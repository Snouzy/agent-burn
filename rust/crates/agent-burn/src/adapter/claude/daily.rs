use std::{
    borrow::Cow,
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
};

use jiff::tz::TimeZone as JiffTimeZone;
use memchr::memmem;
use serde::Deserialize;

use crate::{
    ModelBreakdown, PricingMap, ProjectBreakdown, Result, Speed, TimestampMs, TokenCounts,
    TokenUsageRaw, UsageSummary, calculate_cost_for_usage,
    cli::{CostMode, SharedArgs},
    fast::{FxHashMap, SmallIndexVec, byte_lines, suffix_string},
    format_date_tz, log_level, missing_pricing_model_for_usage, parse_ts_timestamp, parse_tz,
    project_root::TurnRepositories,
    touched_paths,
};

use super::{
    chunk_file_indexes_by_size, has_unsupported_null_field, is_semver_prefix,
    paths::{claude_paths, extract_project, usage_files},
    usage_dedupe_hash,
};

pub(super) fn load_daily_summaries_inner(
    shared: &SharedArgs,
    project_filter: Option<&str>,
    group_by_project: bool,
) -> Result<Vec<UsageSummary>> {
    let paths = claude_paths()?;
    let files = usage_files(&paths, project_filter);
    if files.is_empty() {
        return Ok(Vec::new());
    }

    let pricing = if shared.mode == CostMode::Display {
        None
    } else {
        Some(PricingMap::load_with_overrides(
            shared.offline,
            log_level() != Some(0),
            shared.pricing_overrides.iter(),
        ))
    };
    let tz = parse_tz(shared.timezone.as_deref());
    let mode = shared.mode;
    let loaded_files = if shared.single_thread {
        files
            .iter()
            .map(|file| read_daily_usage_file(file, tz.as_ref(), mode, pricing.as_ref()))
            .collect::<Vec<_>>()
    } else {
        read_daily_usage_files_parallel(&files, tz.as_ref(), mode, pricing.as_ref())
    };

    let mut deduped_indexes: FxHashMap<u64, SmallIndexVec> = FxHashMap::default();
    let mut deduped = Vec::with_capacity(loaded_files.iter().map(|file| file.entries.len()).sum());
    for loaded_file in loaded_files {
        for entry in loaded_file.entries {
            if let Some(filter) = project_filter
                && entry.project.as_ref() != filter
            {
                continue;
            }
            push_deduped_daily_entry(entry, &mut deduped_indexes, &mut deduped);
        }
    }

    if group_by_project {
        let mut groups = BTreeMap::<(String, Arc<str>), DailyAccumulator>::new();
        for entry in &deduped {
            groups
                .entry((entry.date.clone(), Arc::clone(&entry.project)))
                .or_default()
                .add_entry(entry);
        }
        return Ok(groups
            .into_iter()
            .map(|((date, project), group)| {
                let mut summary = group.into_summary();
                summary.date = Some(date);
                summary.project = Some(project.to_string());
                summary
            })
            .collect());
    }

    let mut groups = BTreeMap::<String, DailyAccumulator>::new();
    for entry in &deduped {
        groups
            .entry(entry.date.clone())
            .or_default()
            .add_entry(entry);
    }
    Ok(groups
        .into_iter()
        .map(|(key, group)| {
            let mut summary = group.into_summary();
            summary.date = Some(key);
            summary
        })
        .collect())
}

#[derive(Debug)]
struct DailyLoadedFile {
    timestamp: Option<TimestampMs>,
    entries: Vec<DailyLoadedEntry>,
}

#[derive(Debug)]
struct DailyLoadedEntry {
    date: String,
    project: Arc<str>,
    cwd: Option<Arc<str>>,
    /// Repository the turn's tool calls touched, inherited across the session.
    repository: Option<Arc<str>>,
    usage: TokenUsageRaw,
    cost: f64,
    model: Option<String>,
    missing_pricing_model: Option<String>,
    message_id: Option<String>,
    request_id: Option<String>,
    is_sidechain: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DailyUsageEntry<'a> {
    #[serde(borrow, default)]
    cwd: Option<Cow<'a, str>>,
    timestamp: String,
    message: DailyUsageMessage,
    version: Option<String>,
    session_id: Option<String>,
    #[serde(rename = "costUSD")]
    cost_usd: Option<f64>,
    request_id: Option<String>,
    is_sidechain: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum DailyUsageLine<'a> {
    #[serde(borrow)]
    Direct(DailyUsageEntry<'a>),
    #[serde(borrow)]
    AgentProgress(DailyAgentProgressEntry<'a>),
}

impl<'a> DailyUsageLine<'a> {
    fn into_entry(self) -> DailyUsageEntry<'a> {
        match self {
            DailyUsageLine::Direct(entry) => entry,
            DailyUsageLine::AgentProgress(entry) => DailyUsageEntry {
                cwd: entry.cwd,
                timestamp: entry.data.message.timestamp,
                message: entry.data.message.message,
                version: None,
                session_id: None,
                cost_usd: entry.data.message.cost_usd,
                request_id: entry.data.message.request_id,
                is_sidechain: entry.data.message.is_sidechain,
            },
        }
    }
}

#[derive(Debug, Deserialize)]
struct DailyAgentProgressEntry<'a> {
    #[serde(borrow, default)]
    cwd: Option<Cow<'a, str>>,
    data: DailyAgentProgressData,
}

#[derive(Debug, Deserialize)]
struct DailyAgentProgressData {
    message: DailyAgentProgressMessage,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DailyAgentProgressMessage {
    timestamp: String,
    message: DailyUsageMessage,
    #[serde(rename = "costUSD")]
    cost_usd: Option<f64>,
    request_id: Option<String>,
    is_sidechain: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct DailyUsageMessage {
    usage: TokenUsageRaw,
    model: Option<String>,
    id: Option<String>,
}

fn read_daily_usage_files_parallel(
    files: &[PathBuf],
    tz: Option<&JiffTimeZone>,
    mode: CostMode,
    pricing: Option<&PricingMap>,
) -> Vec<DailyLoadedFile> {
    let worker_count = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(files.len());
    if worker_count <= 1 {
        return files
            .iter()
            .map(|file| read_daily_usage_file(file, tz, mode, pricing))
            .collect();
    }

    let chunks = chunk_file_indexes_by_size(files, worker_count);
    thread::scope(|scope| {
        let mut handles = Vec::with_capacity(worker_count);
        for chunk in chunks {
            let tz = tz.cloned();
            handles.push(scope.spawn(move || {
                chunk
                    .into_iter()
                    .map(|index| {
                        (
                            index,
                            read_daily_usage_file(&files[index], tz.as_ref(), mode, pricing),
                        )
                    })
                    .collect::<Vec<_>>()
            }));
        }
        let mut loaded_files = Vec::with_capacity(files.len());
        loaded_files.resize_with(files.len(), || None);
        for (index, file) in handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("daily usage worker panicked"))
        {
            loaded_files[index] = Some(file);
        }
        loaded_files
            .into_iter()
            .map(|file| file.expect("daily usage worker returned every file"))
            .collect()
    })
}

fn read_daily_usage_file(
    path: &Path,
    tz: Option<&JiffTimeZone>,
    mode: CostMode,
    pricing: Option<&PricingMap>,
) -> DailyLoadedFile {
    let project: Arc<str> = Arc::from(extract_project(path));
    let mut loaded_file = DailyLoadedFile {
        timestamp: None,
        entries: Vec::new(),
    };
    let Ok(content) = fs::read(path) else {
        return loaded_file;
    };

    let usage_marker = memmem::Finder::new(br#""usage":{"#);
    let tool_use_marker = memmem::Finder::new(br#""tool_use""#);
    // Sessions can change directory, so usage is attributed per line; lines
    // without a cwd keep the previous one. The repository a turn's tools
    // touched takes precedence over the cwd.
    let mut cwd: Option<Arc<str>> = None;
    let mut turns = TurnRepositories::default();
    for line in byte_lines(&content) {
        if usage_marker.find(line).is_none() {
            continue;
        }
        if has_unsupported_null_field(line) {
            continue;
        }
        let Ok(data) = serde_json::from_slice::<DailyUsageLine>(line) else {
            continue;
        };
        let data = data.into_entry();
        if let Some(line_cwd) = data.cwd.as_deref()
            && cwd.as_deref() != Some(line_cwd)
        {
            cwd = Some(Arc::from(line_cwd));
        }
        if tool_use_marker.find(line).is_some() {
            touch_tool_paths(&mut turns, line);
        }
        let (own_repository, repository) = turns.finish_turn();
        let Some(timestamp) = parse_ts_timestamp(&data.timestamp) else {
            continue;
        };
        loaded_file.timestamp = Some(
            loaded_file
                .timestamp
                .map_or(timestamp, |current| current.min(timestamp)),
        );
        if !is_valid_daily_usage_entry(&data) {
            continue;
        }
        let usage = data.message.usage;
        let cost = calculate_cost_for_usage(
            data.message.model.as_deref(),
            usage,
            data.cost_usd,
            mode,
            pricing,
        );
        let missing_pricing_model = missing_pricing_model_for_usage(
            data.message.model.as_deref(),
            usage,
            data.cost_usd,
            mode,
            pricing,
        );
        let model = data.message.model.as_ref().and_then(|model| {
            if model == "<synthetic>" {
                None
            } else if matches!(usage.speed, Some(Speed::Fast)) {
                Some(suffix_string(model, "-fast"))
            } else {
                Some(model.clone())
            }
        });
        if let Some(own) = &own_repository {
            // Claude logs one line per content block; earlier blocks of the
            // same message belong to the repository its tool call touched.
            for entry in loaded_file.entries.iter_mut().rev() {
                if entry.message_id.is_none() || entry.message_id != data.message.id {
                    break;
                }
                entry.repository = Some(Arc::clone(own));
            }
        }
        loaded_file.entries.push(DailyLoadedEntry {
            date: format_date_tz(timestamp, tz),
            project: Arc::clone(&project),
            cwd: cwd.clone(),
            repository,
            usage,
            cost,
            model,
            missing_pricing_model,
            message_id: data.message.id,
            request_id: data.request_id,
            is_sidechain: data.is_sidechain,
        });
    }
    if let Some(first) = turns.first() {
        for entry in &mut loaded_file.entries {
            entry.repository.get_or_insert_with(|| Arc::clone(first));
        }
    }
    loaded_file
}

fn touch_tool_paths(turns: &mut TurnRepositories, line: &[u8]) {
    for key in ["file_path", "path", "notebook_path"] {
        for path in touched_paths::key_paths(line, key) {
            turns.touch(path);
        }
    }
    for path in touched_paths::cd_paths(line) {
        turns.touch(path);
    }
}

fn is_valid_daily_usage_entry(data: &DailyUsageEntry<'_>) -> bool {
    if data
        .version
        .as_deref()
        .is_some_and(|version| !is_semver_prefix(version))
    {
        return false;
    }
    if data
        .session_id
        .as_deref()
        .is_some_and(|session_id| session_id.is_empty())
    {
        return false;
    }
    if data
        .request_id
        .as_deref()
        .is_some_and(|request_id| request_id.is_empty())
    {
        return false;
    }
    if data
        .message
        .id
        .as_deref()
        .is_some_and(|message_id| message_id.is_empty())
    {
        return false;
    }
    if data
        .message
        .model
        .as_deref()
        .is_some_and(|model| model.is_empty())
    {
        return false;
    }
    true
}
fn daily_usage_token_total(entry: &DailyLoadedEntry) -> u64 {
    entry.usage.input_tokens
        + entry.usage.output_tokens
        + entry.usage.cache_creation_token_count()
        + entry.usage.cache_read_input_tokens
}

fn push_deduped_daily_entry(
    entry: DailyLoadedEntry,
    deduped_indexes: &mut FxHashMap<u64, SmallIndexVec>,
    deduped: &mut Vec<DailyLoadedEntry>,
) {
    let dedupe_lookup = entry.message_id.as_deref().map(|message_id| {
        let request_id = entry.request_id.as_deref();
        let exact_hash = usage_dedupe_hash(message_id, request_id);
        let existing_index = deduped_indexes
            .get(&exact_hash)
            .and_then(|indexes| {
                indexes.iter().copied().find(|&index| {
                    deduped[index].message_id.as_deref() == Some(message_id)
                        && deduped[index].request_id.as_deref() == request_id
                })
            })
            .or_else(|| {
                // /btw sidechain logs can replay parent messages with new request IDs.
                let message_hash = usage_dedupe_hash(message_id, None);
                let candidate_is_sidechain = is_sidechain_daily_entry(&entry);
                deduped_indexes.get(&message_hash).and_then(|indexes| {
                    indexes.iter().copied().find(|&index| {
                        deduped[index].message_id.as_deref() == Some(message_id)
                            && (candidate_is_sidechain || is_sidechain_daily_entry(&deduped[index]))
                    })
                })
            });
        (exact_hash, existing_index)
    });

    if let Some((_, Some(index))) = dedupe_lookup {
        if should_replace_deduped_daily_entry(&entry, &deduped[index]) {
            deduped[index] = entry;
        }
        return;
    }

    let index = deduped.len();
    deduped.push(entry);
    if let Some((hash, None)) = dedupe_lookup {
        push_deduped_daily_index(deduped_indexes, hash, index);
        if let Some(message_id) = deduped[index].message_id.as_deref() {
            push_deduped_daily_index(deduped_indexes, usage_dedupe_hash(message_id, None), index);
        }
    }
}

fn should_replace_deduped_daily_entry(
    candidate: &DailyLoadedEntry,
    existing: &DailyLoadedEntry,
) -> bool {
    let candidate_is_sidechain = is_sidechain_daily_entry(candidate);
    let existing_is_sidechain = is_sidechain_daily_entry(existing);
    if candidate_is_sidechain != existing_is_sidechain {
        return existing_is_sidechain;
    }

    let candidate_total = daily_usage_token_total(candidate);
    let existing_total = daily_usage_token_total(existing);
    if candidate_total != existing_total {
        return candidate_total > existing_total;
    }
    if candidate.cost != existing.cost {
        return candidate.cost > existing.cost;
    }
    candidate.usage.speed.is_some() && existing.usage.speed.is_none()
}

fn is_sidechain_daily_entry(entry: &DailyLoadedEntry) -> bool {
    entry.is_sidechain == Some(true)
}

fn push_deduped_daily_index(
    deduped_indexes: &mut FxHashMap<u64, SmallIndexVec>,
    hash: u64,
    index: usize,
) {
    let indexes = deduped_indexes.entry(hash).or_default();
    if !indexes.contains(&index) {
        indexes.push(index);
    }
}

#[derive(Default)]
struct DailyAccumulator {
    counts: TokenCounts,
    cost: f64,
    models: Vec<String>,
    breakdowns: Vec<ModelBreakdown>,
    breakdown_indexes: FxHashMap<String, usize>,
    projects: Vec<(Option<Arc<str>>, u64, f64)>,
}

impl DailyAccumulator {
    fn add_entry(&mut self, entry: &DailyLoadedEntry) {
        self.counts.add_usage(entry.usage);
        self.cost += entry.cost;
        let tokens = daily_usage_token_total(entry);
        let location = entry.repository.as_ref().or(entry.cwd.as_ref());
        match self
            .projects
            .iter_mut()
            .find(|(cwd, _, _)| cwd.as_ref() == location)
        {
            Some((_, total_tokens, cost)) => {
                *total_tokens += tokens;
                *cost += entry.cost;
            }
            None => self.projects.push((location.cloned(), tokens, entry.cost)),
        }
        if let Some(model) = &entry.model {
            let index = if let Some(index) = self.breakdown_indexes.get(model.as_str()) {
                *index
            } else {
                let index = self.breakdowns.len();
                self.breakdown_indexes.insert(model.clone(), index);
                self.models.push(model.clone());
                self.breakdowns.push(ModelBreakdown {
                    model_name: model.clone(),
                    ..ModelBreakdown::default()
                });
                index
            };
            let breakdown = &mut self.breakdowns[index];
            breakdown.input_tokens += entry.usage.input_tokens;
            breakdown.output_tokens += entry.usage.output_tokens;
            breakdown.cache_creation_tokens += entry.usage.cache_creation_token_count();
            breakdown.cache_read_tokens += entry.usage.cache_read_input_tokens;
            breakdown.cost += entry.cost;
            if entry.missing_pricing_model.is_some() {
                breakdown.missing_pricing = true;
            }
        }
    }

    fn into_summary(mut self) -> UsageSummary {
        self.breakdowns.sort_by(|a, b| b.cost.total_cmp(&a.cost));
        UsageSummary {
            date: None,
            month: None,
            week: None,
            session_id: None,
            project_path: None,
            last_activity: None,
            first_activity: None,
            input_tokens: self.counts.input_tokens,
            output_tokens: self.counts.output_tokens,
            cache_creation_tokens: self.counts.cache_creation_tokens,
            cache_read_tokens: self.counts.cache_read_tokens,
            extra_total_tokens: 0,
            total_cost: self.cost,
            credits: None,
            message_count: None,
            models_used: self.models,
            model_breakdowns: self.breakdowns,
            project: None,
            versions: None,
            project_breakdowns: self
                .projects
                .into_iter()
                .map(|(cwd, total_tokens, cost)| ProjectBreakdown {
                    cwd: cwd.unwrap_or_else(|| Arc::from("")),
                    total_tokens,
                    cost,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use agent_burn_test_support::{EnvVarGuard, Fixture, fs_fixture};

    use super::{DailyLoadedEntry, load_daily_summaries_inner, push_deduped_daily_entry};
    use crate::{
        ProjectBreakdown, TokenUsageRaw, UsageSummary,
        cli::{CostMode, SharedArgs},
    };

    fn usage_line(timestamp: &str, cwd: Option<&str>, id: &str, input: u64, cost: f64) -> String {
        let cwd = cwd.map_or(String::new(), |cwd| format!(r#""cwd":"{cwd}","#));
        format!(
            r#"{{{cwd}"timestamp":"{timestamp}","version":"1.2.3","sessionId":"s","message":{{"id":"{id}","model":"claude-sonnet-4-20250514","usage":{{"input_tokens":{input},"output_tokens":0}}}},"requestId":"req-{id}","costUSD":{cost}}}"#
        )
    }

    fn load_daily(root: &std::path::Path) -> Vec<UsageSummary> {
        let _env = EnvVarGuard::set("CLAUDE_CONFIG_DIR", root);
        let shared = SharedArgs {
            mode: CostMode::Display,
            timezone: Some("UTC".to_string()),
            ..SharedArgs::default()
        };
        load_daily_summaries_inner(&shared, None, false).unwrap()
    }

    fn projects(summary: &UsageSummary) -> Vec<ProjectBreakdown> {
        let mut projects = summary.project_breakdowns.clone();
        projects.sort_by(|a, b| a.cwd.cmp(&b.cwd));
        projects
    }

    fn project(cwd: &str, total_tokens: u64, cost: f64) -> ProjectBreakdown {
        ProjectBreakdown {
            cwd: cwd.into(),
            total_tokens,
            cost,
        }
    }

    #[test]
    fn attributes_daily_usage_to_the_cwd_of_each_line() {
        let fixture = fs_fixture!({
            "projects/app/session.jsonl": [
                usage_line("2026-03-10T06:00:00.000Z", Some("/dev/app"), "m1", 10, 0.25),
                usage_line("2026-03-10T06:01:00.000Z", Some("/dev/app/.claude/worktrees/x"), "m2", 20, 0.5),
                usage_line("2026-03-10T06:02:00.000Z", Some("/dev/app"), "m3", 40, 1.0),
            ]
            .join("\n"),
        });

        let daily = load_daily(fixture.root());

        assert_eq!(
            projects(&daily[0]),
            vec![
                project("/dev/app", 50, 1.25),
                project("/dev/app/.claude/worktrees/x", 20, 0.5),
            ]
        );
    }

    #[test]
    fn lines_without_cwd_inherit_the_previous_line_cwd() {
        let fixture = fs_fixture!({
            "projects/app/session.jsonl": [
                usage_line("2026-03-10T06:00:00.000Z", None, "m1", 10, 0.25),
                usage_line("2026-03-10T06:01:00.000Z", Some("/dev/app"), "m2", 20, 0.5),
                usage_line("2026-03-10T06:02:00.000Z", None, "m3", 40, 1.0),
            ]
            .join("\n"),
        });

        let daily = load_daily(fixture.root());

        assert_eq!(
            projects(&daily[0]),
            vec![project("", 10, 0.25), project("/dev/app", 60, 1.5)]
        );
    }

    #[test]
    fn attributes_agent_progress_usage_to_the_outer_line_cwd() {
        let fixture = fs_fixture!({
            "projects/app/session.jsonl": r#"{"cwd":"/dev/app","sessionId":"s","version":"1.2.3","type":"progress","data":{"message":{"type":"assistant","timestamp":"2026-03-10T06:00:01.000Z","message":{"model":"claude-sonnet-4-20250514","id":"m1","usage":{"input_tokens":20,"output_tokens":2}},"requestId":"req-m1","costUSD":0.5}},"timestamp":"2026-03-10T06:00:01.001Z"}"#,
        });

        let daily = load_daily(fixture.root());

        assert_eq!(projects(&daily[0]), vec![project("/dev/app", 22, 0.5)]);
    }

    #[test]
    fn deduplicated_messages_keep_a_single_project_attribution() {
        let fixture = fs_fixture!({
            "projects/app/a.jsonl": usage_line("2026-03-10T06:00:00.000Z", Some("/dev/app"), "m1", 10, 0.25),
            "projects/app/b.jsonl": usage_line("2026-03-10T06:00:00.000Z", Some("/dev/other"), "m1", 30, 0.75),
        });

        let daily = load_daily(fixture.root());

        assert_eq!(projects(&daily[0]), vec![project("/dev/other", 30, 0.75)]);
    }

    fn tool_line(minute: u32, cwd: &str, id: &str, input: u64, tool: Option<String>) -> String {
        let content = tool.map_or(r#"[{"type":"text","text":"ok"}]"#.to_string(), |input| {
            format!(r#"[{{"type":"tool_use","id":"t-{id}","name":"Tool","input":{input}}}]"#)
        });
        format!(
            r#"{{"cwd":"{cwd}","timestamp":"2026-03-10T06:{minute:02}:00.000Z","version":"1.2.3","sessionId":"s","message":{{"id":"{id}","model":"claude-sonnet-4-20250514","content":{content},"usage":{{"input_tokens":{input},"output_tokens":0}}}},"requestId":"req-{id}","costUSD":0}}"#
        )
    }

    fn edit(path: &str) -> Option<String> {
        Some(format!(
            r#"{{"file_path":"{path}/src/main.rs","old_string":"a","new_string":"b"}}"#
        ))
    }

    struct Workspace {
        fixture: Fixture,
        container: String,
        app: String,
        web: String,
    }

    fn workspace() -> Workspace {
        let fixture = Fixture::new();
        let _ = fixture.create_dir_all("dev/app/.git");
        let _ = fixture.create_dir_all("dev/web/.git");
        let _ = fixture.create_dir_all("tmp");
        let path = |path: &str| fixture.path(path).to_string_lossy().into_owned();
        Workspace {
            container: path("dev"),
            app: path("dev/app"),
            web: path("dev/web"),
            fixture,
        }
    }

    fn session_projects(workspace: &Workspace, lines: &[String]) -> Vec<ProjectBreakdown> {
        let _ = workspace
            .fixture
            .write_file("projects/dev/session.jsonl", lines.join("\n"));
        projects(&load_daily(workspace.fixture.root())[0])
    }

    #[test]
    fn attributes_container_sessions_to_the_repository_they_edit() {
        let ws = workspace();

        let projects =
            session_projects(&ws, &[tool_line(0, &ws.container, "m1", 10, edit(&ws.app))]);

        assert_eq!(projects, vec![project(&ws.app, 10, 0.0)]);
    }

    #[test]
    fn turns_without_tools_inherit_the_last_touched_repository() {
        let ws = workspace();

        let projects = session_projects(
            &ws,
            &[
                tool_line(0, &ws.container, "m1", 10, edit(&ws.app)),
                tool_line(1, &ws.container, "m2", 20, None),
            ],
        );

        assert_eq!(projects, vec![project(&ws.app, 30, 0.0)]);
    }

    #[test]
    fn turns_before_the_first_touch_go_to_the_first_touched_repository() {
        let ws = workspace();
        let read = Some(format!(r#"{{"path":"{}/src","pattern":"x"}}"#, ws.web));

        let projects = session_projects(
            &ws,
            &[
                tool_line(0, &ws.container, "m1", 10, None),
                tool_line(1, &ws.container, "m2", 20, read),
                tool_line(2, &ws.container, "m3", 40, edit(&ws.app)),
            ],
        );

        assert_eq!(
            projects,
            vec![project(&ws.app, 40, 0.0), project(&ws.web, 30, 0.0)]
        );
    }

    #[test]
    fn attributes_bash_cd_targets() {
        let ws = workspace();
        let bash = Some(format!(r#"{{"command":"cd {} && cargo test"}}"#, ws.web));

        let projects = session_projects(&ws, &[tool_line(0, &ws.container, "m1", 10, bash)]);

        assert_eq!(projects, vec![project(&ws.web, 10, 0.0)]);
    }

    #[test]
    fn tool_use_line_attributes_earlier_lines_of_the_same_message() {
        let ws = workspace();

        let projects = session_projects(
            &ws,
            &[
                tool_line(0, &ws.container, "m1", 10, edit(&ws.app)),
                tool_line(1, &ws.container, "m2", 20, None),
                tool_line(1, &ws.container, "m2", 20, edit(&ws.web)),
            ],
        );

        assert_eq!(
            projects,
            vec![project(&ws.app, 10, 0.0), project(&ws.web, 20, 0.0)]
        );
    }

    #[test]
    fn ignores_touched_paths_outside_repositories() {
        let ws = workspace();
        let tmp = ws.fixture.path("tmp").to_string_lossy().into_owned();

        let projects = session_projects(&ws, &[tool_line(0, &ws.app, "m1", 10, edit(&tmp))]);

        assert_eq!(projects, vec![project(&ws.app, 10, 0.0)]);
    }

    #[test]
    fn sessions_without_touches_keep_their_cwd() {
        let ws = workspace();

        let projects = session_projects(&ws, &[tool_line(0, &ws.container, "m1", 10, None)]);

        assert_eq!(projects, vec![project(&ws.container, 10, 0.0)]);
    }

    #[test]
    fn daily_project_breakdowns_sum_to_the_day_totals() {
        let fixture = fs_fixture!({
            "projects/app/session.jsonl": [
                usage_line("2026-03-10T06:00:00.000Z", Some("/dev/app"), "m1", 10, 0.25),
                usage_line("2026-03-10T07:00:00.000Z", Some("/dev/web"), "m2", 20, 0.5),
                usage_line("2026-03-11T06:00:00.000Z", Some("/dev/web"), "m3", 40, 1.0),
            ]
            .join("\n"),
        });

        let daily = load_daily(fixture.root());

        assert_eq!(daily.len(), 2);
        for summary in &daily {
            let projects = &summary.project_breakdowns;
            assert_eq!(
                projects.iter().map(|p| p.total_tokens).sum::<u64>(),
                summary.total_tokens()
            );
            assert_eq!(
                projects.iter().map(|p| p.cost).sum::<f64>(),
                summary.total_cost
            );
        }
    }

    #[test]
    fn keeps_parent_daily_usage_when_sidechain_replays_message_with_new_request_id() {
        let mut deduped_indexes = Default::default();
        let mut deduped = Vec::new();

        push_deduped_daily_entry(
            daily_loaded_entry(DailyEntryFixture {
                message_id: "msg-parent",
                request_id: "req-parent",
                is_sidechain: false,
                cache_read_tokens: 20,
                output_tokens: 10,
            }),
            &mut deduped_indexes,
            &mut deduped,
        );
        push_deduped_daily_entry(
            daily_loaded_entry(DailyEntryFixture {
                message_id: "msg-parent",
                request_id: "req-sidechain-replay",
                is_sidechain: true,
                cache_read_tokens: 50_000,
                output_tokens: 10,
            }),
            &mut deduped_indexes,
            &mut deduped,
        );
        push_deduped_daily_entry(
            daily_loaded_entry(DailyEntryFixture {
                message_id: "msg-sidechain-answer",
                request_id: "req-sidechain-answer",
                is_sidechain: true,
                cache_read_tokens: 700,
                output_tokens: 30,
            }),
            &mut deduped_indexes,
            &mut deduped,
        );

        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].message_id.as_deref(), Some("msg-parent"));
        assert_eq!(deduped[0].request_id.as_deref(), Some("req-parent"));
        assert_eq!(deduped[0].usage.cache_read_input_tokens, 20);
        assert_eq!(
            deduped[1].message_id.as_deref(),
            Some("msg-sidechain-answer")
        );
        assert_eq!(deduped[1].usage.cache_read_input_tokens, 700);
    }

    #[test]
    fn propagates_sidechain_metadata_from_agent_progress_lines() {
        let data = serde_json::from_str::<super::DailyUsageLine>(
            r#"{"data":{"message":{"timestamp":"2026-03-29T07:00:00.000Z","requestId":"req-sidechain","isSidechain":true,"message":{"usage":{"input_tokens":0,"output_tokens":10,"cache_creation_input_tokens":0,"cache_read_input_tokens":20},"model":"claude-sonnet-4-20250514","id":"msg-sidechain"}}}}"#,
        )
        .unwrap()
        .into_entry();

        assert_eq!(data.is_sidechain, Some(true));
    }

    struct DailyEntryFixture {
        message_id: &'static str,
        request_id: &'static str,
        is_sidechain: bool,
        cache_read_tokens: u64,
        output_tokens: u64,
    }

    fn daily_loaded_entry(fixture: DailyEntryFixture) -> DailyLoadedEntry {
        DailyLoadedEntry {
            date: "2026-03-29".to_string(),
            project: Arc::from("project-a"),
            cwd: None,
            repository: None,
            usage: TokenUsageRaw {
                input_tokens: 0,
                output_tokens: fixture.output_tokens,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: fixture.cache_read_tokens,
                speed: None,
                cache_creation: None,
            },
            cost: 0.0,
            model: Some("claude-sonnet-4-20250514".to_string()),
            missing_pricing_model: None,
            message_id: Some(fixture.message_id.to_string()),
            request_id: Some(fixture.request_id.to_string()),
            is_sidechain: Some(fixture.is_sidechain),
        }
    }
}
