use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use jiff::tz::TimeZone as JiffTimeZone;
use serde_json::Value;

use crate::{
    LoadedEntry, PricingMap, Result, TimestampMs, TokenUsageRaw, UsageEntry, UsageMessage,
    apply_total_token_fallback, calculate_cost_for_usage, cli::CostMode, format_date_tz,
    json_value_u64, missing_pricing_model_for_candidates, non_empty_json_string,
};

use super::{
    config::KimiConfig,
    paths::{WireFile, WireLayout},
};

const DEFAULT_MODEL: &str = "kimi-for-coding";
const HIGHSPEED_MODEL: &str = "kimi-for-coding-highspeed";
const K3_MODEL: &str = "k3";
/// K3 capped at a 256K context window; same model, same price.
const K3_256K_MODEL: &str = "k3-256k";
const DEFAULT_PROVIDER: &str = "moonshot";
const KIMI_FOR_CODING_K2_6_CUTOFF_MS: i64 = 1_776_698_890_072;
/// Kimi Code's coding-plan models have no LiteLLM key; price them as the
/// public Kimi K2.7 Code API model (an API-equivalent estimate).
const KIMI_CODE_PLAN_PRICING_MODEL: &str = "moonshot/kimi-k2.7-code";
/// The highspeed tier bills at 2x K2.7 Code (LiteLLM `tencent/kimi-k2.7-code-highspeed`).
const HIGHSPEED_PRICING_MODEL: &str = "moonshot/kimi-k2.7-code-highspeed";
const K3_PRICING_MODEL: &str = "moonshot/kimi-k3";

#[derive(Debug, Clone)]
pub(super) struct KimiUsageEntry {
    timestamp: TimestampMs,
    timestamp_text: String,
    session_id: String,
    model: String,
    message_id: Option<String>,
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
    extra_total_tokens: u64,
    layout: WireLayout,
}

pub(super) fn read_wire_file(path: &Path) -> Result<Vec<KimiUsageEntry>> {
    let model = read_model_from_config(path);
    let fallback_timestamp = file_modified_timestamp(path);
    let content = fs::read_to_string(path)?;
    Ok(content
        .lines()
        .filter(|line| line.contains("\"StatusUpdate\"") && line.contains("\"token_usage\""))
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|value| wire_line_to_entry(&value, path, &model, fallback_timestamp))
        .collect::<Vec<_>>())
}

/// Read Kimi Code `usage.record` lines: one per LLM request, each a delta
/// (both `turn` and `session` scopes), so every record is summed. A fork copies
/// the source wire and then appends a `forked` marker, so only records after
/// the last marker belong to this file. `step.end` usage repeats the same
/// request and is ignored.
pub(super) fn read_kimi_code_wire_file(
    file: &WireFile,
    config: &KimiConfig,
) -> Result<Vec<KimiUsageEntry>> {
    let fallback_timestamp = file_modified_timestamp(&file.path);
    let content = fs::read_to_string(&file.path)?;
    let lines = content.lines().collect::<Vec<_>>();
    let start = lines
        .iter()
        .rposition(|line| is_forked_marker(line))
        .map_or(0, |index| index + 1);
    let session_id = kimi_code_session_id(&file.path);
    let agent_id = kimi_code_agent_id(&file.path);
    Ok(lines
        .iter()
        .enumerate()
        .skip(start)
        .filter(|(_, line)| line.contains("\"usage.record\""))
        .filter_map(|(index, line)| {
            let value = serde_json::from_str::<Value>(line).ok()?;
            usage_record_to_entry(
                &value,
                config,
                &session_id,
                format!("{agent_id}:{}", index + 1),
                fallback_timestamp,
            )
        })
        .collect())
}

fn is_forked_marker(line: &str) -> bool {
    line.contains("\"forked\"")
        && serde_json::from_str::<Value>(line)
            .ok()
            .is_some_and(|value| value.get("type").and_then(Value::as_str) == Some("forked"))
}

fn usage_record_to_entry(
    value: &Value,
    config: &KimiConfig,
    session_id: &str,
    record_id: String,
    fallback_timestamp: TimestampMs,
) -> Option<KimiUsageEntry> {
    if value.get("type").and_then(Value::as_str) != Some("usage.record") {
        return None;
    }
    let usage = value.get("usage")?;
    let input_tokens = json_value_u64(usage.get("inputOther"));
    let output_tokens = json_value_u64(usage.get("output"));
    let cache_read_tokens = json_value_u64(usage.get("inputCacheRead"));
    let cache_creation_tokens = json_value_u64(usage.get("inputCacheCreation"));
    if input_tokens + output_tokens + cache_read_tokens + cache_creation_tokens == 0 {
        return None;
    }
    let model = non_empty_json_string(value.get("model")).map_or_else(
        || DEFAULT_MODEL.to_string(),
        |alias| config.model_for_alias(&alias),
    );
    let timestamp = value
        .get("time")
        .and_then(Value::as_i64)
        .map(TimestampMs::from_millis)
        .unwrap_or(fallback_timestamp);
    Some(KimiUsageEntry {
        timestamp,
        timestamp_text: crate::format_rfc3339_millis(timestamp),
        session_id: session_id.to_string(),
        model,
        message_id: Some(record_id),
        input_tokens,
        output_tokens,
        cache_creation_tokens,
        cache_read_tokens,
        extra_total_tokens: 0,
        layout: WireLayout::KimiCode,
    })
}

/// `sessions/<workDirKey>/<session>/agents/<agentId>/wire.jsonl` → `<session>`,
/// so sub-agent usage rolls up into its parent session.
fn kimi_code_session_id(file_path: &Path) -> String {
    file_path
        .ancestors()
        .nth(3)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("unknown")
        .to_string()
}

fn kimi_code_agent_id(file_path: &Path) -> String {
    extract_session_id(file_path)
}

fn read_model_from_config(file_path: &Path) -> String {
    let Some(root) = kimi_root_from_wire_path(file_path) else {
        return DEFAULT_MODEL.to_string();
    };
    let Ok(content) = fs::read_to_string(root.join("config.json")) else {
        return DEFAULT_MODEL.to_string();
    };
    let Ok(value) = serde_json::from_str::<Value>(&content) else {
        return DEFAULT_MODEL.to_string();
    };
    non_empty_json_string(value.get("model")).unwrap_or_else(|| DEFAULT_MODEL.to_string())
}

fn kimi_root_from_wire_path(file_path: &Path) -> Option<PathBuf> {
    file_path
        .parent()?
        .parent()?
        .parent()?
        .parent()
        .map(Path::to_path_buf)
}

fn file_modified_timestamp(path: &Path) -> TimestampMs {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .map(TimestampMs::from_millis)
        .unwrap_or(TimestampMs::UNIX_EPOCH)
}

fn wire_line_to_entry(
    value: &Value,
    file_path: &Path,
    model: &str,
    fallback_timestamp: TimestampMs,
) -> Option<KimiUsageEntry> {
    if value.get("type").and_then(Value::as_str) == Some("metadata") {
        return None;
    }
    let message = value.get("message")?;
    if message.get("type").and_then(Value::as_str) != Some("StatusUpdate") {
        return None;
    }
    let payload = message.get("payload")?;
    let token_usage = payload.get("token_usage")?;
    let input_tokens = json_value_u64(token_usage.get("input_other"));
    let output_tokens = json_value_u64(token_usage.get("output"));
    let cache_creation_tokens = json_value_u64(token_usage.get("input_cache_creation"));
    let cache_read_tokens = json_value_u64(token_usage.get("input_cache_read"));
    let total_tokens = json_value_u64(token_usage.get("total"));
    let usage = TokenUsageRaw {
        input_tokens,
        output_tokens,
        cache_creation_input_tokens: cache_creation_tokens,
        cache_read_input_tokens: cache_read_tokens,
        speed: None,
        cache_creation: None,
    };
    let (usage, extra_total_tokens) = apply_total_token_fallback(usage, 0, total_tokens);
    if crate::total_usage_tokens(usage) + extra_total_tokens == 0 {
        return None;
    }
    let timestamp = value
        .get("timestamp")
        .and_then(Value::as_f64)
        .and_then(timestamp_from_seconds)
        .unwrap_or(fallback_timestamp);
    Some(KimiUsageEntry {
        timestamp,
        timestamp_text: crate::format_rfc3339_millis(timestamp),
        session_id: extract_session_id(file_path),
        model: model.to_string(),
        message_id: non_empty_json_string(payload.get("message_id")),
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cache_creation_tokens: usage.cache_creation_input_tokens,
        cache_read_tokens: usage.cache_read_input_tokens,
        extra_total_tokens,
        layout: WireLayout::Legacy,
    })
}

fn timestamp_from_seconds(seconds: f64) -> Option<TimestampMs> {
    if !seconds.is_finite() {
        return None;
    }
    let millis = (seconds * 1000.0).trunc();
    if millis < i64::MIN as f64 || millis > i64::MAX as f64 {
        return None;
    }
    Some(TimestampMs::from_millis(millis as i64))
}

fn extract_session_id(file_path: &Path) -> String {
    file_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("unknown")
        .to_string()
}

pub(super) fn kimi_entry_key(entry: &KimiUsageEntry) -> String {
    format!(
        "{}:{}:{}:{}:{}:{}:{}:{}:{}",
        entry.session_id,
        entry.message_id.as_deref().unwrap_or_default(),
        entry.timestamp_text,
        entry.model,
        entry.input_tokens,
        entry.output_tokens,
        entry.cache_creation_tokens,
        entry.cache_read_tokens,
        entry.extra_total_tokens
    )
}

pub(super) fn kimi_entry_to_loaded(
    entry: KimiUsageEntry,
    tz: Option<&JiffTimeZone>,
    mode: CostMode,
    pricing: &PricingMap,
) -> LoadedEntry {
    let usage = TokenUsageRaw {
        input_tokens: entry.input_tokens,
        output_tokens: entry.output_tokens,
        cache_creation_input_tokens: entry.cache_creation_tokens,
        cache_read_input_tokens: entry.cache_read_tokens,
        speed: None,
        cache_creation: None,
    };
    let cost = calculate_kimi_cost(&entry, mode, pricing, usage);
    let missing_pricing_model = missing_kimi_pricing(&entry, mode, pricing, usage);
    let data = UsageEntry {
        session_id: Some(entry.session_id.clone()),
        timestamp: entry.timestamp_text,
        version: None,
        message: UsageMessage {
            usage,
            model: Some(entry.model.clone()),
            id: entry.message_id.clone(),
        },
        cost_usd: None,
        request_id: None,
        is_api_error_message: None,
        is_sidechain: None,
    };
    LoadedEntry {
        date: format_date_tz(entry.timestamp, tz),
        timestamp: entry.timestamp,
        project: Arc::from("kimi"),
        session_id: Arc::from(entry.session_id),
        project_path: Arc::from("Kimi"),
        cost,
        extra_total_tokens: entry.extra_total_tokens,
        credits: None,
        message_count: None,
        model: Some(entry.model),
        usage_limit_reset_time: None,
        missing_pricing_model,
        data,
    }
}

fn calculate_kimi_cost(
    entry: &KimiUsageEntry,
    mode: CostMode,
    pricing: &PricingMap,
    usage: TokenUsageRaw,
) -> f64 {
    match mode {
        CostMode::Display => 0.0,
        CostMode::Auto | CostMode::Calculate => {
            for candidate in model_candidates(entry) {
                if pricing.find(&candidate).is_some() {
                    return calculate_cost_for_usage(
                        Some(&candidate),
                        usage,
                        None,
                        CostMode::Calculate,
                        Some(pricing),
                    );
                }
            }
            0.0
        }
    }
}

fn missing_kimi_pricing(
    entry: &KimiUsageEntry,
    mode: CostMode,
    pricing: &PricingMap,
    usage: TokenUsageRaw,
) -> Option<String> {
    if mode == CostMode::Display {
        return None;
    }
    missing_pricing_model_for_candidates(
        &entry.model,
        model_candidates(entry),
        crate::total_usage_tokens(usage).saturating_add(entry.extra_total_tokens),
        Some(pricing),
    )
}

fn model_candidates(entry: &KimiUsageEntry) -> Vec<String> {
    let mut candidates = Vec::new();
    if let Some(model) = coding_plan_pricing_model(entry) {
        candidates.push(model.to_string());
    }
    candidates.extend([
        format!("{DEFAULT_PROVIDER}/{}", entry.model),
        format!("kimi/{}", entry.model),
        entry.model.clone(),
    ]);
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|candidate| seen.insert(candidate.clone()));
    candidates
}

/// Pricing stand-ins for Kimi's coding-plan model ids, which are not API model
/// names. The legacy kimi-cli keeps its K2.5/K2.6 cutoff; Kimi Code's plan
/// models price as K2.7 Code.
fn coding_plan_pricing_model(entry: &KimiUsageEntry) -> Option<&'static str> {
    match (entry.model.as_str(), entry.layout) {
        (DEFAULT_MODEL, WireLayout::Legacy) => Some(kimi_for_coding_pricing_model(entry.timestamp)),
        (model, _) => plan_pricing_model(model),
    }
}

/// Kimi Code's pricing stand-in for a plan model id, also used to split
/// aggregate spend into token classes (`spendMix`) where no timestamp is known.
pub(crate) fn plan_pricing_model(model: &str) -> Option<&'static str> {
    match model {
        DEFAULT_MODEL => Some(KIMI_CODE_PLAN_PRICING_MODEL),
        HIGHSPEED_MODEL => Some(HIGHSPEED_PRICING_MODEL),
        K3_MODEL | K3_256K_MODEL => Some(K3_PRICING_MODEL),
        _ => None,
    }
}

fn kimi_for_coding_pricing_model(timestamp: TimestampMs) -> &'static str {
    if timestamp.as_millis() < KIMI_FOR_CODING_K2_6_CUTOFF_MS {
        "moonshot/kimi-k2.5"
    } else {
        "moonshot/kimi-k2.6"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_burn_test_support::fs_fixture;

    #[test]
    fn falls_back_to_total_tokens_when_kimi_parts_are_missing() {
        let fixture = fs_fixture!({
            "sessions/group/session-a/wire.jsonl": "",
        });
        let file = fixture.path("sessions/group/session-a/wire.jsonl");
        let value = serde_json::json!({
            "timestamp": 1770983427.123,
            "message": {
                "type": "StatusUpdate",
                "payload": {
                    "token_usage": {
                        "total": 432
                    }
                }
            }
        });

        let entry = wire_line_to_entry(&value, &file, "kimi-k2", TimestampMs::UNIX_EPOCH).unwrap();

        assert_eq!(entry.output_tokens, 432);
        assert_eq!(entry.extra_total_tokens, 0);
    }

    #[test]
    fn prices_default_kimi_model_by_timestamp_without_changing_display_model() {
        let pricing = PricingMap::load_embedded();
        let usage = TokenUsageRaw {
            input_tokens: 100,
            output_tokens: 50,
            cache_creation_input_tokens: 20,
            cache_read_input_tokens: 10,
            speed: None,
            cache_creation: None,
        };
        let before_cutoff = KimiUsageEntry {
            timestamp: TimestampMs::from_millis(1_776_698_890_071),
            timestamp_text: "2026-04-20T15:28:10.071Z".to_string(),
            session_id: "session-a".to_string(),
            model: DEFAULT_MODEL.to_string(),
            message_id: Some("msg-before".to_string()),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_creation_tokens: usage.cache_creation_input_tokens,
            cache_read_tokens: usage.cache_read_input_tokens,
            extra_total_tokens: 0,
            layout: WireLayout::Legacy,
        };
        let at_cutoff = KimiUsageEntry {
            timestamp: TimestampMs::from_millis(1_776_698_890_072),
            timestamp_text: "2026-04-20T15:28:10.072Z".to_string(),
            message_id: Some("msg-at".to_string()),
            ..before_cutoff.clone()
        };

        let before_cost = calculate_kimi_cost(&before_cutoff, CostMode::Calculate, &pricing, usage);
        let at_cost = calculate_kimi_cost(&at_cutoff, CostMode::Calculate, &pricing, usage);
        let loaded = kimi_entry_to_loaded(at_cutoff, None, CostMode::Calculate, &pricing);

        assert!((before_cost - 0.000226).abs() < f64::EPSILON);
        assert!((at_cost - 0.00032035).abs() < f64::EPSILON);
        assert_eq!(loaded.model.as_deref(), Some(DEFAULT_MODEL));
        assert_eq!(loaded.data.message.model.as_deref(), Some(DEFAULT_MODEL));
    }
}
