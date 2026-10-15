use std::{
    collections::BTreeMap,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
};

use jiff::tz::TimeZone as JiffTimeZone;
use rustc_hash::FxHasher;

use crate::{
    CodexGroup, CodexModelUsage, CodexTokenUsageEvent, Result,
    cli::{AgentReportKind, SharedArgs, WeekDay},
    fast::FxHashSet,
    format_date_tz, parse_ts_timestamp, parse_tz, wants_json, week_start,
};

use super::{parser, paths};

type CodexEventKey = (
    u64,
    usize,
    crate::TimestampMs,
    u64,
    usize,
    u64,
    u64,
    u64,
    u64,
    u64,
);
type CodexDedupeShards = [Mutex<FxHashSet<CodexEventKey>>];

struct CodexAggregation {
    groups: BTreeMap<String, CodexGroup>,
    seen: FxHashSet<CodexEventKey>,
}

pub(crate) fn load_groups(
    shared: &SharedArgs,
    kind: AgentReportKind,
) -> Result<BTreeMap<String, CodexGroup>> {
    let sources = paths::codex_usage_sources()?;
    if sources.len() == 1 && !wants_json(shared) {
        return load_groups_from_directory(&sources[0].dir, shared, kind);
    }
    load_groups_from_sources(&sources, shared, kind)
}

fn load_groups_from_sources(
    sources: &[paths::CodexUsageSource],
    shared: &SharedArgs,
    kind: AgentReportKind,
) -> Result<BTreeMap<String, CodexGroup>> {
    let mut groups = BTreeMap::new();
    let seen = create_dedupe_shards();
    for group in paths::collect_deduped_codex_usage_files(sources) {
        merge_groups(
            &mut groups,
            aggregate_files_with_dedupe(&group.dir, &group.files, shared, kind, &seen)?,
        );
    }
    Ok(groups)
}

pub(super) fn load_groups_from_directory(
    sessions_dir: &Path,
    shared: &SharedArgs,
    kind: AgentReportKind,
) -> Result<BTreeMap<String, CodexGroup>> {
    let files = paths::collect_codex_usage_files(sessions_dir);
    if shared.single_thread {
        return aggregate_files_local(sessions_dir, &files, shared, kind);
    }
    let seen = create_dedupe_shards();
    aggregate_files_parallel(sessions_dir, &files, shared, kind, &seen)
}

fn aggregate_files_with_dedupe(
    sessions_dir: &Path,
    files: &[PathBuf],
    shared: &SharedArgs,
    kind: AgentReportKind,
    seen: &CodexDedupeShards,
) -> Result<BTreeMap<String, CodexGroup>> {
    if shared.single_thread {
        return aggregate_files(sessions_dir, files, shared, kind, seen);
    }
    aggregate_files_parallel(sessions_dir, files, shared, kind, seen)
}

fn aggregate_files(
    sessions_dir: &Path,
    files: &[PathBuf],
    shared: &SharedArgs,
    kind: AgentReportKind,
    seen: &CodexDedupeShards,
) -> Result<BTreeMap<String, CodexGroup>> {
    let mut groups = BTreeMap::new();
    let timezone = parse_tz(shared.timezone.as_deref()).or_else(|| Some(JiffTimeZone::system()));
    for file in files {
        aggregate_file(
            sessions_dir,
            file,
            kind,
            timezone.as_ref(),
            shared,
            seen,
            &mut groups,
        )?;
    }
    Ok(groups)
}

fn aggregate_files_parallel(
    sessions_dir: &Path,
    files: &[PathBuf],
    shared: &SharedArgs,
    kind: AgentReportKind,
    seen: &CodexDedupeShards,
) -> Result<BTreeMap<String, CodexGroup>> {
    let worker_count = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(files.len());
    if worker_count <= 1 {
        return aggregate_files(sessions_dir, files, shared, kind, seen);
    }

    let chunks = crate::chunk_file_indexes_by_size(files, worker_count);
    thread::scope(|scope| {
        let mut handles = Vec::with_capacity(chunks.len());
        for chunk in chunks {
            handles.push(scope.spawn(move || {
                let mut groups = BTreeMap::new();
                let timezone =
                    parse_tz(shared.timezone.as_deref()).or_else(|| Some(JiffTimeZone::system()));
                for index in chunk {
                    aggregate_file(
                        sessions_dir,
                        &files[index],
                        kind,
                        timezone.as_ref(),
                        shared,
                        seen,
                        &mut groups,
                    )?;
                }
                Result::<BTreeMap<String, CodexGroup>>::Ok(groups)
            }));
        }

        let mut groups = BTreeMap::new();
        for handle in handles {
            merge_groups(
                &mut groups,
                handle
                    .join()
                    .map_err(|_| crate::cli_error("codex worker panicked"))??,
            );
        }
        Ok(groups)
    })
}

fn aggregate_file(
    sessions_dir: &Path,
    file: &Path,
    kind: AgentReportKind,
    timezone: Option<&JiffTimeZone>,
    shared: &SharedArgs,
    seen: &CodexDedupeShards,
    groups: &mut BTreeMap<String, CodexGroup>,
) -> Result<()> {
    parser::visit_codex_session_file(sessions_dir, file, |event| {
        add_event_to_groups(&event, kind, timezone, shared, seen, groups)
    })
}

fn aggregate_files_local(
    sessions_dir: &Path,
    files: &[PathBuf],
    shared: &SharedArgs,
    kind: AgentReportKind,
) -> Result<BTreeMap<String, CodexGroup>> {
    Ok(aggregate_files_local_with_seen(sessions_dir, files, shared, kind)?.groups)
}

fn aggregate_files_local_with_seen(
    sessions_dir: &Path,
    files: &[PathBuf],
    shared: &SharedArgs,
    kind: AgentReportKind,
) -> Result<CodexAggregation> {
    let mut aggregation = CodexAggregation {
        groups: BTreeMap::new(),
        seen: FxHashSet::default(),
    };
    let timezone = parse_tz(shared.timezone.as_deref()).or_else(|| Some(JiffTimeZone::system()));
    for file in files {
        aggregate_file_local(
            sessions_dir,
            file,
            kind,
            timezone.as_ref(),
            shared,
            &mut aggregation,
        )?;
    }
    Ok(aggregation)
}

fn aggregate_file_local(
    sessions_dir: &Path,
    file: &Path,
    kind: AgentReportKind,
    timezone: Option<&JiffTimeZone>,
    shared: &SharedArgs,
    aggregation: &mut CodexAggregation,
) -> Result<()> {
    parser::visit_codex_session_file(sessions_dir, file, |event| {
        add_event_to_groups_local(&event, kind, timezone, shared, aggregation)
    })
}

fn add_event_to_groups(
    event: &CodexTokenUsageEvent,
    kind: AgentReportKind,
    timezone: Option<&JiffTimeZone>,
    shared: &SharedArgs,
    seen: &CodexDedupeShards,
    groups: &mut BTreeMap<String, CodexGroup>,
) -> Result<()> {
    let Some(model) = event.model.as_deref().filter(|model| !model.is_empty()) else {
        return Ok(());
    };
    let timestamp = parse_ts_timestamp(&event.timestamp)
        .ok_or_else(|| crate::cli_error(format!("Invalid Codex timestamp: {}", event.timestamp)))?;
    if !insert_event_key(event, timestamp, model, kind, seen) {
        return Ok(());
    }
    add_deduped_event_to_groups(event, model, timestamp, kind, timezone, shared, groups)
}

fn add_event_to_groups_local(
    event: &CodexTokenUsageEvent,
    kind: AgentReportKind,
    timezone: Option<&JiffTimeZone>,
    shared: &SharedArgs,
    aggregation: &mut CodexAggregation,
) -> Result<()> {
    let Some(model) = event.model.as_deref().filter(|model| !model.is_empty()) else {
        return Ok(());
    };
    let timestamp = parse_ts_timestamp(&event.timestamp)
        .ok_or_else(|| crate::cli_error(format!("Invalid Codex timestamp: {}", event.timestamp)))?;
    if !aggregation
        .seen
        .insert(codex_event_key(event, timestamp, model, kind))
    {
        return Ok(());
    }
    add_deduped_event_to_groups(
        event,
        model,
        timestamp,
        kind,
        timezone,
        shared,
        &mut aggregation.groups,
    )
}

fn add_deduped_event_to_groups(
    event: &CodexTokenUsageEvent,
    model: &str,
    timestamp: crate::TimestampMs,
    kind: AgentReportKind,
    timezone: Option<&JiffTimeZone>,
    shared: &SharedArgs,
    groups: &mut BTreeMap<String, CodexGroup>,
) -> Result<()> {
    let date = format_date_tz(timestamp, timezone);
    if shared.since.is_some() || shared.until.is_some() {
        let date_key = date.replace('-', "");
        if shared.since.as_ref().is_some_and(|since| &date_key < since)
            || shared.until.as_ref().is_some_and(|until| &date_key > until)
        {
            return Ok(());
        }
    }
    let period = match kind {
        AgentReportKind::Daily => date,
        AgentReportKind::Weekly => week_start(&date, WeekDay::Monday).unwrap_or(date),
        AgentReportKind::Monthly => date[..7].to_string(),
        AgentReportKind::Session => event.session_id.clone(),
    };
    let group = groups.entry(period).or_default();
    accumulate_codex_event_into_group(group, event, model);
    Ok(())
}

fn accumulate_codex_event_into_group(
    group: &mut CodexGroup,
    event: &CodexTokenUsageEvent,
    model: &str,
) {
    group.input_tokens += event.input_tokens;
    group.cached_input_tokens += event.cached_input_tokens;
    group.output_tokens += event.output_tokens;
    group.reasoning_output_tokens += event.reasoning_output_tokens;
    group.total_tokens += event.total_tokens;
    if group
        .last_activity
        .as_deref()
        .is_none_or(|current| event.timestamp.as_str() > current)
    {
        group.last_activity = Some(event.timestamp.clone());
    }

    add_event_to_models(&mut group.models, event, model);
    let cwd = event.cwd.as_deref().unwrap_or_default();
    let project = match group.projects.get_mut(cwd) {
        Some(project) => project,
        None => group.projects.entry(Arc::from(cwd)).or_default(),
    };
    add_event_to_models(project, event, model);
}

fn add_event_to_models(
    models: &mut BTreeMap<String, CodexModelUsage>,
    event: &CodexTokenUsageEvent,
    model: &str,
) {
    let model_usage = match models.get_mut(model) {
        Some(model_usage) => model_usage,
        None => models.entry(model.to_string()).or_default(),
    };
    model_usage.input_tokens += event.input_tokens;
    model_usage.cached_input_tokens += event.cached_input_tokens;
    model_usage.output_tokens += event.output_tokens;
    model_usage.reasoning_output_tokens += event.reasoning_output_tokens;
    model_usage.total_tokens += event.total_tokens;
    model_usage.is_fallback |= event.is_fallback_model;
}

fn create_dedupe_shards() -> Vec<Mutex<FxHashSet<CodexEventKey>>> {
    let shard_count = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    (0..shard_count.max(1))
        .map(|_| Mutex::new(FxHashSet::default()))
        .collect()
}

fn insert_event_key(
    event: &CodexTokenUsageEvent,
    timestamp: crate::TimestampMs,
    model: &str,
    kind: AgentReportKind,
    seen: &CodexDedupeShards,
) -> bool {
    let key = codex_event_key(event, timestamp, model, kind);
    let mut hasher = FxHasher::default();
    key.hash(&mut hasher);
    let shard_index = hasher.finish() as usize % seen.len();
    seen[shard_index].lock().unwrap().insert(key)
}

fn codex_event_key(
    event: &CodexTokenUsageEvent,
    timestamp: crate::TimestampMs,
    model: &str,
    kind: AgentReportKind,
) -> CodexEventKey {
    let (session_hash, session_len) = if kind == AgentReportKind::Session {
        (hash_text(&event.session_id), event.session_id.len())
    } else {
        (0, 0)
    };
    (
        session_hash,
        session_len,
        timestamp,
        hash_text(model),
        model.len(),
        event.input_tokens,
        event.cached_input_tokens,
        event.output_tokens,
        event.reasoning_output_tokens,
        event.total_tokens,
    )
}

fn hash_text(value: &str) -> u64 {
    let mut hasher = FxHasher::default();
    value.hash(&mut hasher);
    hasher.finish()
}

fn merge_groups(target: &mut BTreeMap<String, CodexGroup>, source: BTreeMap<String, CodexGroup>) {
    for (period, group) in source {
        let target_group = target.entry(period).or_default();
        target_group.input_tokens += group.input_tokens;
        target_group.cached_input_tokens += group.cached_input_tokens;
        target_group.output_tokens += group.output_tokens;
        target_group.reasoning_output_tokens += group.reasoning_output_tokens;
        target_group.total_tokens += group.total_tokens;
        if target_group.last_activity.as_deref().is_none_or(|current| {
            group
                .last_activity
                .as_deref()
                .is_some_and(|next| next > current)
        }) {
            target_group.last_activity = group.last_activity;
        }
        merge_models(&mut target_group.models, group.models);
        for (cwd, models) in group.projects {
            merge_models(target_group.projects.entry(cwd).or_default(), models);
        }
    }
}

fn merge_models(
    target: &mut BTreeMap<String, CodexModelUsage>,
    source: BTreeMap<String, CodexModelUsage>,
) {
    for (model, usage) in source {
        let target_usage = target.entry(model).or_default();
        target_usage.input_tokens += usage.input_tokens;
        target_usage.cached_input_tokens += usage.cached_input_tokens;
        target_usage.output_tokens += usage.output_tokens;
        target_usage.reasoning_output_tokens += usage.reasoning_output_tokens;
        target_usage.total_tokens += usage.total_tokens;
        target_usage.is_fallback |= usage.is_fallback;
    }
}

pub(crate) fn aggregate_events(
    events: &[CodexTokenUsageEvent],
    kind: AgentReportKind,
    timezone: Option<&str>,
) -> Result<BTreeMap<String, CodexGroup>> {
    let mut groups = BTreeMap::new();
    let timezone = parse_tz(timezone).or_else(|| Some(JiffTimeZone::system()));
    for event in events {
        let Some(model) = event.model.as_deref().filter(|model| !model.is_empty()) else {
            continue;
        };
        let timestamp = parse_ts_timestamp(&event.timestamp).ok_or_else(|| {
            crate::cli_error(format!("Invalid Codex timestamp: {}", event.timestamp))
        })?;
        let date = format_date_tz(timestamp, timezone.as_ref());
        let period = match kind {
            AgentReportKind::Daily => date,
            AgentReportKind::Weekly => week_start(&date, WeekDay::Monday).unwrap_or(date),
            AgentReportKind::Monthly => date[..7].to_string(),
            AgentReportKind::Session => event.session_id.clone(),
        };
        let group = groups.entry(period).or_insert_with(CodexGroup::default);
        accumulate_codex_event_into_group(group, event, model);
    }
    Ok(groups)
}

pub(crate) fn filter_events_by_date(
    events: &mut Vec<CodexTokenUsageEvent>,
    shared: &SharedArgs,
) -> Result<()> {
    if shared.since.is_none() && shared.until.is_none() {
        return Ok(());
    }
    let timezone = parse_tz(shared.timezone.as_deref()).or_else(|| Some(JiffTimeZone::system()));
    let mut kept = Vec::with_capacity(events.len());
    for event in events.drain(..) {
        let timestamp = parse_ts_timestamp(&event.timestamp).ok_or_else(|| {
            crate::cli_error(format!("Invalid Codex timestamp: {}", event.timestamp))
        })?;
        let date = format_date_tz(timestamp, timezone.as_ref()).replace('-', "");
        if shared.since.as_ref().is_none_or(|since| &date >= since)
            && shared.until.as_ref().is_none_or(|until| &date <= until)
        {
            kept.push(event);
        }
    }
    *events = kept;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use agent_burn_test_support::fs_fixture;
    use serde_json::json;

    use crate::{CodexModelUsage, adapter::codex::paths::CodexUsageSource};

    fn token_count_line(timestamp: &str, total_tokens: u64) -> String {
        json!({
            "timestamp": timestamp,
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {
                    "total_token_usage": {
                        "input_tokens": total_tokens,
                        "cached_input_tokens": 0,
                        "output_tokens": 0,
                        "reasoning_output_tokens": 0,
                        "total_tokens": total_tokens,
                    },
                },
            },
        })
        .to_string()
    }

    fn turn_context_line(cwd: &str) -> String {
        json!({
            "timestamp": "2026-05-29T08:00:00.000Z",
            "type": "turn_context",
            "payload": { "cwd": cwd, "model": "gpt-5" },
        })
        .to_string()
    }

    // Codex writes `timestamp`, `type`, then `payload`, with the payload
    // `type` first; `json!` would sort the keys.
    fn response_item_line(payload_type: &str, fields: &str) -> String {
        format!(
            r#"{{"timestamp":"2026-05-29T08:00:30.000Z","type":"response_item","payload":{{"type":"{payload_type}",{fields}}}}}"#
        )
    }

    fn function_call_line(arguments: serde_json::Value) -> String {
        let arguments = json!(arguments.to_string());
        response_item_line(
            "function_call",
            &format!(r#""name":"exec_command","arguments":{arguments}"#),
        )
    }

    fn apply_patch_line(path: &str) -> String {
        let input = json!(format!(
            "*** Begin Patch\n*** Update File: {path}\n@@\n-a\n+b\n*** End Patch"
        ));
        response_item_line(
            "custom_tool_call",
            &format!(r#""name":"apply_patch","input":{input}"#),
        )
    }

    fn project_totals(lines: &[String]) -> Vec<(String, u64)> {
        let fixture = fs_fixture!({ "sessions/session.jsonl": lines.join("\n") });
        let shared = SharedArgs {
            timezone: Some("UTC".to_string()),
            ..SharedArgs::default()
        };
        let groups =
            load_groups_from_directory(&fixture.path("sessions"), &shared, AgentReportKind::Daily)
                .unwrap();
        groups["2026-05-29"]
            .projects
            .iter()
            .map(|(cwd, models)| (cwd.to_string(), models["gpt-5"].total_tokens))
            .collect()
    }

    struct Workspace {
        fixture: agent_burn_test_support::Fixture,
        container: String,
        app: String,
        web: String,
    }

    fn workspace() -> Workspace {
        let fixture = agent_burn_test_support::Fixture::new();
        let _ = fixture.create_dir_all("dev/app/.git");
        let _ = fixture.create_dir_all("dev/web/.git");
        let path = |path: &str| fixture.path(path).to_string_lossy().into_owned();
        Workspace {
            container: path("dev"),
            app: path("dev/app"),
            web: path("dev/web"),
            fixture,
        }
    }

    #[test]
    fn attributes_container_turns_to_the_workdir_repository() {
        let ws = workspace();

        let totals = project_totals(&[
            turn_context_line(&ws.container),
            function_call_line(json!({ "cmd": "cargo test", "workdir": ws.app })),
            token_count_line("2026-05-29T08:01:00.000Z", 100),
        ]);

        assert_eq!(totals, vec![(ws.app.clone(), 100)]);
    }

    #[test]
    fn leading_and_untouched_turns_follow_the_touched_repositories() {
        let ws = workspace();

        let totals = project_totals(&[
            turn_context_line(&ws.container),
            token_count_line("2026-05-29T08:01:00.000Z", 10),
            function_call_line(json!({ "cmd": "ls", "workdir": ws.web })),
            token_count_line("2026-05-29T08:02:00.000Z", 30),
            token_count_line("2026-05-29T08:03:00.000Z", 70),
            function_call_line(json!({ "cmd": "ls", "workdir": ws.app })),
            token_count_line("2026-05-29T08:04:00.000Z", 150),
        ]);

        assert_eq!(totals, vec![(ws.app.clone(), 80), (ws.web.clone(), 70)]);
    }

    #[test]
    fn resolves_relative_patch_paths_against_the_call_workdir() {
        let ws = workspace();
        let arguments = json!({
            "command": ["apply_patch", "*** Begin Patch\n*** Update File: src/a.rs\n*** End Patch"],
            "workdir": ws.web,
        });

        let totals = project_totals(&[
            turn_context_line(&ws.container),
            function_call_line(arguments),
            token_count_line("2026-05-29T08:01:00.000Z", 10),
        ]);

        assert_eq!(totals, vec![(ws.web.clone(), 10)]);
    }

    #[test]
    fn resolves_relative_patch_paths_against_the_turn_cwd() {
        let ws = workspace();

        let totals = project_totals(&[
            turn_context_line(&ws.app),
            function_call_line(json!({ "cmd": "ls", "workdir": ws.web })),
            token_count_line("2026-05-29T08:01:00.000Z", 10),
            apply_patch_line("src/a.rs"),
            token_count_line("2026-05-29T08:02:00.000Z", 30),
        ]);

        assert_eq!(totals, vec![(ws.app.clone(), 20), (ws.web.clone(), 10)]);
    }

    #[test]
    fn sessions_without_touched_repositories_keep_the_turn_cwd() {
        let ws = workspace();
        let tmp = ws.fixture.path("tmp").to_string_lossy().into_owned();

        let totals = project_totals(&[
            turn_context_line(&ws.container),
            apply_patch_line(&format!("{tmp}/notes.md")),
            token_count_line("2026-05-29T08:01:00.000Z", 10),
        ]);

        assert_eq!(totals, vec![(ws.container.clone(), 10)]);
    }

    #[test]
    fn attributes_usage_to_the_latest_turn_context_cwd() {
        let fixture = fs_fixture!({
            "sessions/session.jsonl": [
                turn_context_line("/dev/app"),
                token_count_line("2026-05-29T08:01:00.000Z", 100),
                turn_context_line("/home/.codex/worktrees/fd18/app"),
                token_count_line("2026-05-29T08:02:00.000Z", 250),
            ]
            .join("\n"),
        });
        for single_thread in [true, false] {
            let shared = SharedArgs {
                single_thread,
                timezone: Some("UTC".to_string()),
                ..SharedArgs::default()
            };

            let groups = load_groups_from_directory(
                &fixture.path("sessions"),
                &shared,
                AgentReportKind::Daily,
            )
            .unwrap();

            let projects = &groups["2026-05-29"].projects;
            let totals = projects
                .iter()
                .map(|(cwd, models)| (cwd.as_ref(), models["gpt-5"].total_tokens))
                .collect::<Vec<_>>();
            assert_eq!(
                totals,
                vec![("/dev/app", 100), ("/home/.codex/worktrees/fd18/app", 150)]
            );
        }
    }

    #[test]
    fn aggregates_event_projects_by_cwd() {
        let event = |cwd: Option<&str>, total_tokens| CodexTokenUsageEvent {
            session_id: "s".to_string(),
            timestamp: "2026-05-29T08:01:00.000Z".to_string(),
            model: Some("gpt-5".to_string()),
            cwd: cwd.map(Arc::from),
            input_tokens: total_tokens,
            cached_input_tokens: 0,
            output_tokens: 0,
            reasoning_output_tokens: 0,
            total_tokens,
            is_fallback_model: false,
        };

        let groups = aggregate_events(
            &[
                event(Some("/dev/app"), 10),
                event(None, 20),
                event(Some("/dev/app"), 30),
            ],
            AgentReportKind::Daily,
            Some("UTC"),
        )
        .unwrap();

        let projects = &groups["2026-05-29"].projects;
        assert_eq!(projects[""]["gpt-5"].total_tokens, 20);
        assert_eq!(projects["/dev/app"]["gpt-5"].total_tokens, 40);
    }

    #[test]
    fn merging_groups_keeps_per_project_usage() {
        let group = |cwd: &str, total_tokens| {
            let mut group = CodexGroup::default();
            group.projects.entry(Arc::from(cwd)).or_default().insert(
                "gpt-5".to_string(),
                CodexModelUsage {
                    total_tokens,
                    ..CodexModelUsage::default()
                },
            );
            BTreeMap::from([("2026-05-29".to_string(), group)])
        };
        let mut groups = group("/dev/app", 10);

        merge_groups(&mut groups, group("/dev/app", 5));
        merge_groups(&mut groups, group("/dev/web", 7));

        let projects = &groups["2026-05-29"].projects;
        assert_eq!(projects["/dev/app"]["gpt-5"].total_tokens, 15);
        assert_eq!(projects["/dev/web"]["gpt-5"].total_tokens, 7);
    }

    #[test]
    fn dedupes_copied_token_usage_across_session_files() {
        let usage_line = json!({
            "timestamp": "2026-05-29T08:01:00.000Z",
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {
                    "model": "gpt-5.2",
                    "last_token_usage": {
                        "input_tokens": 1_000,
                        "cached_input_tokens": 100,
                        "output_tokens": 200,
                        "reasoning_output_tokens": 20,
                        "total_tokens": 1_200,
                    },
                },
            },
        })
        .to_string();
        let fixture = fs_fixture!({
            "sessions/root.jsonl": &usage_line,
            "sessions/goal.jsonl": &usage_line,
        });
        for single_thread in [true, false] {
            let shared = SharedArgs {
                single_thread,
                timezone: Some("UTC".to_string()),
                ..SharedArgs::default()
            };

            let groups = load_groups_from_directory(
                &fixture.path("sessions"),
                &shared,
                AgentReportKind::Daily,
            )
            .unwrap();

            assert_eq!(groups.len(), 1);
            let group = groups.get("2026-05-29").unwrap();
            assert_eq!(group.input_tokens, 1_000);
            assert_eq!(group.cached_input_tokens, 100);
            assert_eq!(group.output_tokens, 200);
            assert_eq!(group.reasoning_output_tokens, 20);
            assert_eq!(group.total_tokens, 1_200);
        }
    }

    #[test]
    fn keeps_matching_token_usage_in_distinct_session_groups() {
        let usage_line = json!({
            "timestamp": "2026-05-29T08:01:00.000Z",
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {
                    "model": "gpt-5.2",
                    "last_token_usage": {
                        "input_tokens": 1_000,
                        "cached_input_tokens": 100,
                        "output_tokens": 200,
                        "reasoning_output_tokens": 20,
                        "total_tokens": 1_200,
                    },
                },
            },
        })
        .to_string();
        let fixture = fs_fixture!({
            "sessions/root.jsonl": &usage_line,
            "sessions/goal.jsonl": &usage_line,
        });
        for single_thread in [true, false] {
            let shared = SharedArgs {
                single_thread,
                timezone: Some("UTC".to_string()),
                ..SharedArgs::default()
            };

            let groups = load_groups_from_directory(
                &fixture.path("sessions"),
                &shared,
                AgentReportKind::Session,
            )
            .unwrap();

            assert_eq!(groups.len(), 2);
            assert_eq!(groups["root"].input_tokens, 1_000);
            assert_eq!(groups["goal"].input_tokens, 1_000);
        }
    }

    #[test]
    fn aggregates_active_copy_when_archived_file_has_same_relative_path() {
        let active_usage = [
            json!({
                "timestamp": "2026-05-12T08:00:00.000Z",
                "type": "turn_context",
                "payload": {
                    "model": "gpt-5.2",
                },
            })
            .to_string(),
            json!({
                "timestamp": "2026-05-12T08:01:00.000Z",
                "type": "event_msg",
                "payload": {
                    "type": "token_count",
                    "info": {
                        "total_token_usage": {
                            "input_tokens": 111,
                            "cached_input_tokens": 10,
                            "output_tokens": 20,
                            "reasoning_output_tokens": 1,
                            "total_tokens": 131,
                        },
                    },
                },
            })
            .to_string(),
        ]
        .join("\n");
        let archived_usage = [
            json!({
                "timestamp": "2026-05-12T09:00:00.000Z",
                "type": "turn_context",
                "payload": {
                    "model": "gpt-5.2",
                },
            })
            .to_string(),
            json!({
                "timestamp": "2026-05-12T09:01:00.000Z",
                "type": "event_msg",
                "payload": {
                    "type": "token_count",
                    "info": {
                        "total_token_usage": {
                            "input_tokens": 999,
                            "cached_input_tokens": 90,
                            "output_tokens": 80,
                            "reasoning_output_tokens": 7,
                            "total_tokens": 1_079,
                        },
                    },
                },
            })
            .to_string(),
        ]
        .join("\n");
        let fixture = fs_fixture!({
            "codex/sessions/duplicate.jsonl": active_usage,
            "codex/archived_sessions/duplicate.jsonl": archived_usage,
            "codex/archived_sessions/archived-only.jsonl": [
                json!({
                    "timestamp": "2026-05-13T08:00:00.000Z",
                    "type": "turn_context",
                    "payload": {
                        "model": "gpt-5.2",
                    },
                })
                .to_string(),
                json!({
                    "timestamp": "2026-05-13T08:01:00.000Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "token_count",
                        "info": {
                            "total_token_usage": {
                                "input_tokens": 222,
                                "cached_input_tokens": 20,
                                "output_tokens": 30,
                                "reasoning_output_tokens": 2,
                                "total_tokens": 252,
                            },
                        },
                    },
                })
                .to_string(),
            ]
            .join("\n"),
        });

        for single_thread in [true, false] {
            let shared = SharedArgs {
                single_thread,
                ..SharedArgs::default()
            };
            let sources = vec![
                CodexUsageSource::new_for_test(
                    fixture.path("codex/sessions"),
                    fixture.path("codex"),
                ),
                CodexUsageSource::new_for_test(
                    fixture.path("codex/archived_sessions"),
                    fixture.path("codex"),
                ),
            ];
            let groups =
                load_groups_from_sources(&sources, &shared, AgentReportKind::Daily).unwrap();

            assert_eq!(groups.len(), 2);
            assert_eq!(groups["2026-05-12"].input_tokens, 111);
            assert_eq!(groups["2026-05-13"].input_tokens, 222);
        }
    }
}
