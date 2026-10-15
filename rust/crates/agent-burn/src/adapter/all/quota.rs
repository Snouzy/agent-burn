use serde_json::{Value, json};

use crate::{
    TimestampMs,
    adapter::{claude, codex, cursor, kimi},
    utc_now,
};

/// Private app collector protocol. Never scan logs or label a cached limit as live.
pub(super) fn snapshot(agent: &str, offline: bool) -> Option<Value> {
    let (used, minutes, reset) = match agent {
        "codex" => {
            let window = codex::usage_limits(offline)?.weekly_window()?;
            (
                window.used_percent,
                window.window_minutes,
                TimestampMs::from_unix_seconds(window.resets_at?)?,
            )
        }
        "claude" => {
            let limits = claude::usage_limits(offline)?;
            let window = limits.seven_day?;
            let session = limits
                .five_hour
                .and_then(|session| Some((session.utilization, session.resets_at?)));
            return with_session(
                agent,
                (window.utilization, 10080, window.resets_at?),
                session,
                utc_now(),
            );
        }
        "kimi" => {
            let limits = kimi::usage_limits(offline)?;
            let (window, minutes) = limits.primary_window()?;
            let session = limits
                .five_hour
                .and_then(|session| Some((session.used_percent, session.resets_at?)));
            return with_session(
                agent,
                (window.used_percent, minutes, window.resets_at?),
                session,
                utc_now(),
            );
        }
        "cursor" => {
            let account = cursor::load_account(offline)?;
            cursor::live_window(&account, utc_now())?
        }
        _ => return None,
    };
    reading(agent, used, minutes, reset, utc_now())
}

/// Cursor promo credits are optional. A skip payload keeps collection green
/// when the signed-in account has no burning promotional grant.
pub(super) fn cursor_snapshot(offline: bool) -> Value {
    snapshot("cursor", offline).unwrap_or_else(|| {
        json!({
            "agent": "cursor",
            "observedAt": utc_now().as_millis(),
        })
    })
}

/// A main-window reading plus, when live, the 5-hour session window as `sessionWindow`.
fn with_session(
    agent: &str,
    (used, minutes, reset): (f64, u64, TimestampMs),
    session: Option<(f64, TimestampMs)>,
    now: TimestampMs,
) -> Option<Value> {
    let mut output = reading(agent, used, minutes, reset, now)?;
    if let Some(session) = session.and_then(|(used, reset)| reading(agent, used, 300, reset, now)) {
        output["sessionWindow"] = session["window"].clone();
    }
    Some(output)
}

fn reading(
    agent: &str,
    used: f64,
    minutes: u64,
    reset: TimestampMs,
    now: TimestampMs,
) -> Option<Value> {
    let duration = i64::try_from(minutes).ok()?.checked_mul(60_000)?;
    let start = reset.checked_sub_millis(duration)?;
    if duration == 0
        || !used.is_finite()
        || !(0.0..=100.0).contains(&used)
        || now < start
        || now >= reset
    {
        return None;
    }
    Some(json!({
        "agent": agent,
        "observedAt": now.as_millis(),
        "window": {
            "windowMinutes": minutes,
            "usedPercent": used,
            "elapsedPercent": now.duration_since(start) as f64 / duration as f64 * 100.0,
            "apiEquivalentSpent": 0
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_expired_and_invalid_live_windows() {
        let now = TimestampMs::from_unix_seconds(1_800_000_000).unwrap();
        assert!(reading("codex", 14.0, 10080, now, now).is_none());
        let reset = now.checked_add_millis(86_400_000).unwrap();
        assert!(reading("codex", -1.0, 10080, reset, now).is_none());
        assert!(reading("codex", 101.0, 10080, reset, now).is_none());
        let output = reading("codex", 14.0, 10080, reset, now).unwrap();
        assert_eq!(output["observedAt"], now.as_millis());
        assert_eq!(output["window"]["usedPercent"], 14.0);
    }

    #[test]
    fn monthly_kimi_window_carries_the_five_hour_session_window() {
        let now = TimestampMs::from_unix_seconds(1_800_000_000).unwrap();
        let reset = now.checked_add_millis(10 * 86_400_000).unwrap();
        let session_reset = now.checked_add_millis(3 * 3_600_000).unwrap();
        let output = with_session(
            "kimi",
            (25.0, 30 * 1440, reset),
            Some((46.0, session_reset)),
            now,
        )
        .unwrap();

        assert_eq!(output["agent"], "kimi");
        assert_eq!(output["window"]["windowMinutes"], 43200);
        assert_eq!(output["window"]["usedPercent"], 25.0);
        assert_eq!(output["sessionWindow"]["windowMinutes"], 300);
        assert_eq!(output["sessionWindow"]["usedPercent"], 46.0);
        assert_eq!(output["sessionWindow"]["elapsedPercent"], 40.0);
    }

    #[test]
    fn expired_session_window_is_left_out() {
        let now = TimestampMs::from_unix_seconds(1_800_000_000).unwrap();
        let reset = now.checked_add_millis(86_400_000).unwrap();
        let output = with_session("kimi", (12.0, 10080, reset), Some((5.0, now)), now).unwrap();

        assert!(output.get("sessionWindow").is_none());
    }

    #[test]
    fn five_hour_reading_reports_elapsed_share_of_the_session() {
        let now = TimestampMs::from_unix_seconds(1_800_000_000).unwrap();
        let reset = now.checked_add_millis(3 * 3_600_000).unwrap();
        let output = reading("claude", 46.0, 300, reset, now).unwrap();
        assert_eq!(output["window"]["windowMinutes"], 300);
        assert_eq!(output["window"]["elapsedPercent"], 40.0);
    }
}
