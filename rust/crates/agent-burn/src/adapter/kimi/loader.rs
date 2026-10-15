use std::collections::{HashMap, HashSet};

use crate::{LoadedEntry, PricingMap, Result, cli::SharedArgs, parse_tz};

use super::{
    config::KimiConfig,
    parser::{kimi_entry_key, kimi_entry_to_loaded, read_kimi_code_wire_file, read_wire_file},
    paths::{WireLayout, discover_wire_files},
};

pub(crate) fn load_entries(shared: &SharedArgs, pricing: &PricingMap) -> Result<Vec<LoadedEntry>> {
    crate::progress::track_usage_load(crate::progress::UsageLoadAgent::Kimi, shared.json, || {
        load_entries_inner(shared, pricing)
    })
}

fn load_entries_inner(shared: &SharedArgs, pricing: &PricingMap) -> Result<Vec<LoadedEntry>> {
    let tz = parse_tz(shared.timezone.as_deref());
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut configs = HashMap::new();
    for file in discover_wire_files()? {
        let file_entries = match file.layout {
            WireLayout::Legacy => read_wire_file(&file.path)?,
            WireLayout::KimiCode => {
                let config = configs
                    .entry(file.root.clone())
                    .or_insert_with(|| KimiConfig::load(&file.root));
                read_kimi_code_wire_file(&file, config)?
            }
        };
        for entry in file_entries {
            let key = kimi_entry_key(&entry);
            if seen.insert(key) {
                entries.push(kimi_entry_to_loaded(
                    entry,
                    tz.as_ref(),
                    shared.mode,
                    pricing,
                ));
            }
        }
    }
    entries.sort_by_key(|entry| entry.timestamp);
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::super::paths::KIMI_DATA_DIR_ENV;
    use super::*;
    use agent_burn_test_support::{EnvVarGuard, fs_fixture};

    #[test]
    fn loads_status_update_token_usage_from_wire_files() {
        let fixture = fs_fixture!({
            "config.json": r#"{"model":"kimi-k2"}"#,
            "sessions/group/session-a/wire.jsonl": [
                r#"{"type":"metadata","protocol_version":"1.3"}"#,
                r#"{"timestamp":1770983426.420942,"message":{"type":"TurnBegin","payload":{"user_input":"hello"}}}"#,
                r#"{"timestamp":1770983427.123,"message":{"type":"StatusUpdate","payload":{"token_usage":{"input_other":100,"output":50,"input_cache_read":10,"input_cache_creation":20},"message_id":"msg-1"}}}"#,
            ]
            .join("\n"),
        });
        let _cleanup = EnvVarGuard::set(KIMI_DATA_DIR_ENV, fixture.root());
        let shared = SharedArgs {
            timezone: Some("UTC".to_string()),
            ..SharedArgs::default()
        };
        let entries = load_entries(&shared, &PricingMap::load_embedded()).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].date, "2026-02-13");
        assert_eq!(entries[0].session_id.as_ref(), "session-a");
        assert_eq!(entries[0].model.as_deref(), Some("kimi-k2"));
        assert_eq!(entries[0].data.message.usage.input_tokens, 100);
        assert_eq!(entries[0].data.message.usage.output_tokens, 50);
        assert_eq!(
            entries[0].data.message.usage.cache_creation_input_tokens,
            20
        );
        assert_eq!(entries[0].data.message.usage.cache_read_input_tokens, 10);
    }

    #[test]
    fn skips_malformed_and_zero_token_wire_lines() {
        let fixture = fs_fixture!({
            "sessions/group/session-a/wire.jsonl": [
                "not json",
                r#"{"timestamp":1770983427,"message":{"type":"StatusUpdate","payload":{"token_usage":{"input_other":0,"output":0,"input_cache_read":0,"input_cache_creation":0}}}}"#,
            ]
            .join("\n"),
        });
        let _cleanup = EnvVarGuard::set(KIMI_DATA_DIR_ENV, fixture.root());
        let entries = load_entries(&SharedArgs::default(), &PricingMap::load_embedded()).unwrap();

        assert!(entries.is_empty());
    }

    fn kimi_code_entries(fixture: &agent_burn_test_support::Fixture) -> Vec<LoadedEntry> {
        let _cleanup = EnvVarGuard::set(KIMI_DATA_DIR_ENV, fixture.root());
        let shared = SharedArgs {
            timezone: Some("UTC".to_string()),
            ..SharedArgs::default()
        };
        load_entries(&shared, &PricingMap::load_embedded()).unwrap()
    }

    #[test]
    fn loads_kimi_code_usage_records_from_the_official_wire_shape() {
        // Lines from kimi-code apps/vis/server/test/fixtures/sessions/sample-main.
        let fixture = fs_fixture!({
            "sessions/wd/session_a/agents/main/wire.jsonl": [
                r#"{"type":"metadata","protocol_version":"1.1","created_at":1779256800000}"#,
                r#"{"type":"context.append_loop_event","event":{"type":"step.end","uuid":"s1","turnId":"t1","step":0,"usage":{"inputOther":10,"output":5,"inputCacheRead":0,"inputCacheCreation":0},"finishReason":"end_turn"},"time":1779256800300}"#,
                r#"{"type":"usage.record","model":"kimi-k2","usage":{"inputOther":10,"output":5,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1779256800302}"#,
            ]
            .join("\n"),
        });
        let entries = kimi_code_entries(&fixture);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].date, "2026-05-20");
        assert_eq!(entries[0].timestamp.as_millis(), 1_779_256_800_302);
        assert_eq!(entries[0].session_id.as_ref(), "session_a");
        assert_eq!(entries[0].model.as_deref(), Some("kimi-k2"));
        assert_eq!(entries[0].data.message.usage.input_tokens, 10);
        assert_eq!(entries[0].data.message.usage.output_tokens, 5);
    }

    #[test]
    fn sums_turn_and_session_scoped_usage_records() {
        let fixture = fs_fixture!({
            "sessions/wd/session_a/agents/main/wire.jsonl": [
                r#"{"type":"metadata","protocol_version":"1.1"}"#,
                r#"{"type":"usage.record","agentId":"main","model":"kimi-code/kimi-for-coding","usage":{"inputOther":100,"output":50,"inputCacheRead":1000,"inputCacheCreation":20},"usageScope":"turn","time":1779256800000}"#,
                r#"{"type":"usage.record","agentId":"main","model":"kimi-code/kimi-for-coding","usage":{"inputOther":7,"output":3,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"session","time":1779256801000}"#,
                r#"{"type":"usage.record","agentId":"main","model":"kimi-code/kimi-for-coding","usage":{"inputOther":0,"output":0,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1779256802000}"#,
            ]
            .join("\n"),
        });
        let entries = kimi_code_entries(&fixture);

        assert_eq!(entries.len(), 2);
        let usage = &entries[0].data.message.usage;
        assert_eq!(
            (
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_input_tokens,
                usage.cache_creation_input_tokens
            ),
            (100, 50, 1000, 20)
        );
        assert_eq!(entries[1].data.message.usage.input_tokens, 7);
    }

    #[test]
    fn counts_only_usage_after_the_last_fork_marker() {
        let source = r#"{"type":"usage.record","agentId":"main","model":"kimi-k3","usage":{"inputOther":100,"output":10,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1779256800000}"#;
        let fixture = fs_fixture!({
            "sessions/wd/session_src/agents/main/wire.jsonl": [
                r#"{"type":"metadata","protocol_version":"1.1"}"#,
                source,
            ]
            .join("\n"),
            "sessions/wd/session_fork/agents/main/wire.jsonl": [
                r#"{"type":"metadata","protocol_version":"1.1"}"#,
                source,
                r#"{"type":"forked","agentId":"main","time":1779256900000}"#,
                r#"{"type":"usage.record","agentId":"main","model":"kimi-k3","usage":{"inputOther":5,"output":1,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1779257000000}"#,
                r#"{"type":"forked","agentId":"main","time":1779257100000}"#,
                r#"{"type":"usage.record","agentId":"main","model":"kimi-k3","usage":{"inputOther":2,"output":1,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1779257200000}"#,
            ]
            .join("\n"),
        });
        let entries = kimi_code_entries(&fixture);

        let tokens = entries
            .iter()
            .map(|entry| {
                (
                    entry.session_id.to_string(),
                    entry.data.message.usage.input_tokens,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            tokens,
            vec![
                ("session_src".to_string(), 100),
                ("session_fork".to_string(), 2),
            ]
        );
    }

    #[test]
    fn rolls_sub_agent_wire_usage_into_its_parent_session() {
        let fixture = fs_fixture!({
            "sessions/wd/session_a/agents/main/wire.jsonl": r#"{"type":"usage.record","agentId":"main","model":"kimi-code/kimi-for-coding","usage":{"inputOther":10,"output":1,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1779256800000}"#,
            "sessions/wd/session_a/agents/agent-0/wire.jsonl": [
                r#"{"type":"metadata","protocol_version":"1.1"}"#,
                r#"{"type":"usage.record","agentId":"agent-0","model":"kimi-code/kimi-for-coding","usage":{"inputOther":10,"output":1,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1779256800000}"#,
            ]
            .join("\n"),
        });
        let entries = kimi_code_entries(&fixture);

        assert_eq!(entries.len(), 2);
        assert!(
            entries
                .iter()
                .all(|entry| entry.session_id.as_ref() == "session_a")
        );
    }

    #[test]
    fn resolves_model_aliases_through_config_toml_and_prices_plan_models() {
        let fixture = fs_fixture!({
            "config.toml": [
                r#"[models."kimi-code/kimi-for-coding"]"#,
                r#"provider = "managed:kimi-code""#,
                r#"model = "kimi-for-coding""#,
                r#"[models."my-k3"]"#,
                r#"model = "k3""#,
            ]
            .join("\n"),
            "sessions/wd/session_a/agents/main/wire.jsonl": [
                r#"{"type":"usage.record","agentId":"main","model":"kimi-code/kimi-for-coding","usage":{"inputOther":1000000,"output":0,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1779256800000}"#,
                r#"{"type":"usage.record","agentId":"main","model":"my-k3","usage":{"inputOther":1000000,"output":0,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1779256801000}"#,
                r#"{"type":"usage.record","agentId":"main","model":"kimi-code/kimi-for-coding-highspeed","usage":{"inputOther":1000000,"output":0,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1779256802000}"#,
                r#"{"type":"usage.record","agentId":"main","model":"kimi-code/k3-256k","usage":{"inputOther":1000000,"output":0,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1779256803000}"#,
            ]
            .join("\n"),
        });
        let entries = kimi_code_entries(&fixture);

        let models = entries
            .iter()
            .map(|entry| (entry.model.clone().unwrap(), entry.cost))
            .collect::<Vec<_>>();
        assert_eq!(
            models,
            vec![
                ("kimi-for-coding".to_string(), 0.95),
                ("k3".to_string(), 3.0),
                ("kimi-for-coding-highspeed".to_string(), 1.9),
                ("k3-256k".to_string(), 3.0),
            ]
        );
        assert!(
            entries
                .iter()
                .all(|entry| entry.missing_pricing_model.is_none())
        );
    }

    #[test]
    fn counts_a_migrated_session_once_across_legacy_and_kimi_code_trees() {
        // kimi-code's migration-legacy translator drops `_usage` rows and writes no
        // usage.record (only a token_counting.measured anchor), so the legacy tree
        // keeps owning the imported spend while live usage after import is new.
        let legacy = fs_fixture!({
            "sessions/hash/0b5d/wire.jsonl": r#"{"timestamp":1770983427.123,"message":{"type":"StatusUpdate","payload":{"token_usage":{"input_other":100,"output":50,"input_cache_read":0,"input_cache_creation":0},"message_id":"msg-1"}}}"#,
        });
        let kimi_code = fs_fixture!({
            "sessions/wd/ses_0b5d/state.json": r#"{"id":"ses_0b5d","custom":{"imported_from_kimi_cli":true,"kimi_cli_session_id":"0b5d"}}"#,
            "sessions/wd/ses_0b5d/agents/main/wire.jsonl": [
                r#"{"type":"metadata","protocol_version":"1.0","created_at":1770983400000}"#,
                r#"{"type":"turn.prompt","agentId":"main","time":1770983400000}"#,
                r#"{"type":"token_counting.measured","agentId":"main","length":2,"tokens":150,"time":1770983400000}"#,
                r#"{"type":"usage.record","agentId":"main","model":"kimi-code/kimi-for-coding","usage":{"inputOther":30,"output":3,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1779256800000}"#,
            ]
            .join("\n"),
        });
        let roots = format!("{},{}", legacy.root().display(), kimi_code.root().display());
        let _cleanup = EnvVarGuard::set(KIMI_DATA_DIR_ENV, roots);
        let entries = load_entries(&SharedArgs::default(), &PricingMap::load_embedded()).unwrap();

        let tokens = entries
            .iter()
            .map(|entry| {
                (
                    entry.session_id.to_string(),
                    entry.data.message.usage.input_tokens,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            tokens,
            vec![("0b5d".to_string(), 100), ("ses_0b5d".to_string(), 30)]
        );
    }
}
