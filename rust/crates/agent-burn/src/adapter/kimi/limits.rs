use std::{env, fs, path::Path, time::Duration};

use serde_json::{Map, Value, json};

use crate::{TimestampMs, utc_now};

use super::{config::KimiConfig, paths::kimi_code_home};

/// Mainland-China managed endpoint; Kimi Code's default `oauth/kimi-code` slot.
const DEFAULT_BASE_URL: &str = "https://api.kimi.com/coding/v1";
const BASE_URL_ENV: &str = "KIMI_CODE_BASE_URL";
const MANAGED_PROVIDER: &str = "managed:kimi-code";
const DEFAULT_TOKEN_NAME: &str = "kimi-code";
const FETCH_TIMEOUT_SECONDS: u64 = 5;
const FETCH_MAX_BYTES: u64 = 1_000_000;
const WEEKLY_MINUTES: u64 = 7 * 24 * 60;
/// Booster wallet amounts are fixed-point: 1_000_000 units per cent.
const BOOSTER_UNITS_PER_CENT: f64 = 1_000_000.0;

/// One live Kimi Code quota window from `GET {baseUrl}/usages`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct KimiQuotaWindow {
    pub(crate) used_percent: f64,
    pub(crate) resets_at: Option<TimestampMs>,
}

/// Prepaid booster credits, in major currency units.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct KimiBooster {
    pub(crate) remaining: f64,
    pub(crate) total: f64,
    pub(crate) currency: Option<String>,
}

/// Live Kimi Code plan meters. Every window is optional: current plans report
/// a 5-hour and monthly windows, legacy plans a 7-day one.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct KimiUsageLimits {
    pub(crate) five_hour: Option<KimiQuotaWindow>,
    pub(crate) seven_day: Option<KimiQuotaWindow>,
    pub(crate) month_code: Option<KimiQuotaWindow>,
    pub(crate) month_total: Option<KimiQuotaWindow>,
    pub(crate) booster: Option<KimiBooster>,
}

impl KimiUsageLimits {
    /// The plan's main quota window and its length in minutes: the 7-day window
    /// when the plan has one, else the monthly coding (then total) window,
    /// which spans the calendar month before its reset.
    pub(crate) fn primary_window(&self) -> Option<(KimiQuotaWindow, u64)> {
        if let Some(window) = self.seven_day {
            return Some((window, WEEKLY_MINUTES));
        }
        let window = self.month_code.or(self.month_total)?;
        Some((window, month_window_minutes(window.resets_at?)?))
    }
}

/// Fetch the signed-in Kimi Code account's live quota. Returns `None` when
/// offline, when no unexpired access token is stored, or on any network or
/// parse error (never fatal). Only reads the access token: the OAuth refresh
/// endpoint is never called, so the CLI's own session is never rotated.
pub(crate) fn usage_limits(offline: bool) -> Option<KimiUsageLimits> {
    if offline {
        return None;
    }
    let credentials = credentials(&kimi_code_home()?, env::var(BASE_URL_ENV).ok(), utc_now())?;
    fetch_usage_limits(&credentials.usage_url(), &credentials.access_token)
}

/// Current Kimi meters for `summary --value --json`, omitted when offline or empty.
pub(crate) fn load_account(offline: bool) -> Option<Value> {
    let limits = usage_limits(offline)?;
    if limits == KimiUsageLimits::default() {
        return None;
    }
    Some(account_json(&limits))
}

#[derive(Debug, PartialEq)]
struct Credentials {
    access_token: String,
    base_url: String,
}

impl Credentials {
    fn usage_url(&self) -> String {
        format!("{}/usages", self.base_url.trim_end_matches('/'))
    }
}

/// Resolve the managed provider's base URL and stored access token. The
/// credential slot comes from `[providers."managed:kimi-code".oauth].key`
/// (`oauth/kimi-code` or a region-scoped `oauth/kimi-code-env-<hash>`), stored
/// as `credentials/<key without oauth/>.json`; the base URL comes from
/// `KIMI_CODE_BASE_URL`, then the provider's `base_url`, then the default.
fn credentials(home: &Path, env_base_url: Option<String>, now: TimestampMs) -> Option<Credentials> {
    let config = KimiConfig::load(home);
    let provider = |key: &str| config.get(&["providers", MANAGED_PROVIDER, key]);
    let token_name = config
        .get(&["providers", MANAGED_PROVIDER, "oauth", "key"])
        .map_or(DEFAULT_TOKEN_NAME, |key| {
            key.strip_prefix("oauth/").unwrap_or(key)
        });
    if token_name.is_empty() || token_name.starts_with('.') || token_name.contains(['/', '\\']) {
        return None;
    }
    let token_json =
        fs::read_to_string(home.join("credentials").join(format!("{token_name}.json"))).ok()?;
    let access_token = access_token_from_credentials(&token_json, now)?;
    let base_url = env_base_url
        .filter(|url| !url.trim().is_empty())
        .or_else(|| provider("base_url").map(str::to_string))
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
    Some(Credentials {
        access_token,
        base_url: base_url.trim().trim_end_matches('/').to_string(),
    })
}

/// The stored `access_token`, unless `expires_at` (unix seconds) has passed.
/// Kimi Code treats `expires_at == 0` as "no known expiry".
fn access_token_from_credentials(json: &str, now: TimestampMs) -> Option<String> {
    let value = serde_json::from_str::<Value>(json.trim()).ok()?;
    let token = value
        .get("access_token")?
        .as_str()
        .filter(|token| !token.is_empty())?;
    let expires_at = value
        .get("expires_at")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    if expires_at > 0.0 && expires_at * 1000.0 <= now.as_millis() as f64 {
        return None;
    }
    Some(token.to_string())
}

fn fetch_usage_limits(url: &str, token: &str) -> Option<KimiUsageLimits> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(FETCH_TIMEOUT_SECONDS)))
        .build()
        .new_agent();
    let mut response = agent
        .get(url)
        .header("Authorization", &format!("Bearer {token}"))
        .header("Accept", "application/json")
        .call()
        .ok()?;
    if response.status().as_u16() != 200 {
        return None;
    }
    let body = response
        .body_mut()
        .with_config()
        .limit(FETCH_MAX_BYTES)
        .read_to_string()
        .ok()?;
    parse_usage_limits(&body)
}

fn parse_usage_limits(body: &str) -> Option<KimiUsageLimits> {
    let value = serde_json::from_str::<Value>(body).ok()?;
    let mut limits = match value.get("usages").filter(|usages| usages.is_object()) {
        Some(usages) => KimiUsageLimits {
            five_hour: parse_ratio_window(usages.get("limit_5h")),
            seven_day: parse_ratio_window(usages.get("limit_7d")),
            month_code: parse_ratio_window(usages.get("limit_month_code")),
            month_total: parse_ratio_window(usages.get("limit_month_total")),
            booster: None,
        },
        None => parse_legacy_usage(&value),
    };
    limits.booster = parse_booster(value.get("boosterWallet"));
    Some(limits)
}

/// Current shape: `{"used_ratio": 0..1 (number or string), "reset_time": RFC 3339}`.
fn parse_ratio_window(value: Option<&Value>) -> Option<KimiQuotaWindow> {
    let window = value?.as_object()?;
    let ratio = number(window.get("used_ratio"))?;
    Some(KimiQuotaWindow {
        used_percent: percent(ratio * 100.0),
        resets_at: window
            .get("reset_time")
            .and_then(Value::as_str)
            .and_then(parse_reset_time),
    })
}

/// The pre-`usages` payload the Python kimi-cli parsed: a weekly `usage`
/// summary plus `limits[]` rows with `window: {duration, timeUnit}` and
/// `detail: {limit, used | remaining, resetTime}`.
fn parse_legacy_usage(value: &Value) -> KimiUsageLimits {
    let mut limits = KimiUsageLimits {
        seven_day: value.get("usage").and_then(parse_count_window),
        ..KimiUsageLimits::default()
    };
    for item in value
        .get("limits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let detail = item.get("detail").filter(|detail| detail.is_object());
        if limits.five_hour.is_none()
            && window_minutes(item) == Some(300)
            && let Some(window) = parse_count_window(detail.unwrap_or(item))
        {
            limits.five_hour = Some(window);
        }
    }
    limits
}

fn parse_count_window(data: &Value) -> Option<KimiQuotaWindow> {
    let limit = number(data.get("limit")).filter(|limit| *limit > 0.0)?;
    let used = number(data.get("used"))
        .or_else(|| number(data.get("remaining")).map(|remaining| limit - remaining))?;
    let resets_at = ["reset_at", "resetAt", "reset_time", "resetTime"]
        .iter()
        .find_map(|key| data.get(*key).and_then(Value::as_str))
        .and_then(parse_reset_time);
    Some(KimiQuotaWindow {
        used_percent: percent(used / limit * 100.0),
        resets_at,
    })
}

fn window_minutes(item: &Value) -> Option<u64> {
    let window = item.get("window").unwrap_or(item);
    let duration = number(window.get("duration"))? as u64;
    let unit = window.get("timeUnit").and_then(Value::as_str).unwrap_or("");
    if unit.contains("MINUTE") {
        Some(duration)
    } else if unit.contains("HOUR") {
        Some(duration * 60)
    } else if unit.contains("DAY") {
        Some(duration * 1440)
    } else {
        None
    }
}

/// `boosterWallet.balance` of type `BOOSTER` carries fixed-point `amount` /
/// `amountLeft` (1e6 units per cent); the currency is read from the monthly
/// charge limit or monthly usage money objects.
fn parse_booster(value: Option<&Value>) -> Option<KimiBooster> {
    let wallet = value?.as_object()?;
    let balance = wallet.get("balance")?;
    if balance.get("type").and_then(Value::as_str) != Some("BOOSTER") {
        return None;
    }
    let total = number(balance.get("amount")).filter(|amount| *amount > 0.0)?;
    let remaining = number(balance.get("amountLeft")).unwrap_or(0.0);
    let currency = ["monthlyChargeLimit", "monthlyUsed"]
        .iter()
        .find_map(|key| {
            wallet
                .get(*key)?
                .get("currency")?
                .as_str()
                .filter(|currency| !currency.is_empty())
        })
        .map(str::to_string);
    Some(KimiBooster {
        remaining: fixed_point_to_major(remaining),
        total: fixed_point_to_major(total),
        currency,
    })
}

/// Mirror Kimi Code's rounding: whole cents, with any positive sub-cent amount
/// shown as one cent.
fn fixed_point_to_major(amount: f64) -> f64 {
    let cents = amount.trunc() / BOOSTER_UNITS_PER_CENT;
    let cents = if cents > 0.0 && cents < 1.0 {
        1.0
    } else {
        cents.round()
    };
    cents / 100.0
}

/// Clamp to 0–100 and drop float noise from the ratio scaling (0.3 → 30, not 30.000000000000004).
fn percent(value: f64) -> f64 {
    (value.clamp(0.0, 100.0) * 1e6).round() / 1e6
}

fn number(value: Option<&Value>) -> Option<f64> {
    let value = value?;
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse::<f64>().ok())
        .filter(|number| number.is_finite())
}

fn parse_reset_time(value: &str) -> Option<TimestampMs> {
    let timestamp = value.trim().parse::<jiff::Timestamp>().ok()?;
    Some(TimestampMs::from_millis(timestamp.as_millisecond()))
}

/// Minutes from one calendar month before `reset` up to `reset` (UTC).
fn month_window_minutes(reset: TimestampMs) -> Option<u64> {
    let end = jiff::Timestamp::from_millisecond(reset.as_millis()).ok()?;
    let start = end
        .to_zoned(jiff::tz::TimeZone::UTC)
        .checked_sub(jiff::Span::new().months(1))
        .ok()?
        .timestamp();
    u64::try_from((end.as_millisecond() - start.as_millisecond()) / 60_000).ok()
}

fn account_json(limits: &KimiUsageLimits) -> Value {
    let mut account = Map::new();
    let mut put_window = |percent_key: &str, reset_key: &str, window: Option<KimiQuotaWindow>| {
        let Some(window) = window else {
            return;
        };
        account.insert(percent_key.to_string(), json!(window.used_percent));
        if let Some(resets_at) = window.resets_at {
            account.insert(reset_key.to_string(), json!(resets_at.as_millis()));
        }
    };
    put_window("sessionUsedPercent", "sessionResetsAtMs", limits.five_hour);
    put_window("weeklyUsedPercent", "weeklyResetsAtMs", limits.seven_day);
    put_window("monthlyUsedPercent", "monthlyResetsAtMs", limits.month_code);
    put_window(
        "monthlyTotalUsedPercent",
        "monthlyTotalResetsAtMs",
        limits.month_total,
    );
    if let Some(booster) = &limits.booster {
        account.insert("boosterRemaining".to_string(), json!(booster.remaining));
        account.insert("boosterTotal".to_string(), json!(booster.total));
        if let Some(currency) = &booster.currency {
            account.insert("boosterCurrency".to_string(), json!(currency));
        }
    }
    Value::Object(account)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format_rfc3339_millis;
    use agent_burn_test_support::fs_fixture;

    fn at(rfc3339: &str) -> TimestampMs {
        parse_reset_time(rfc3339).unwrap()
    }

    #[test]
    fn parses_current_usages_payload_with_string_ratios() {
        let limits = parse_usage_limits(
            r#"{
              "goods_version": 2,
              "usages": {
                "limit_5h": {"used_ratio": 0.3, "reset_time": "2026-09-11T18:00:00Z"},
                "limit_month_total": {"used_ratio": "0.4", "reset_time": "2026-10-01T00:00:00Z"},
                "limit_month_code": {"used_ratio": 0.25, "reset_time": "2026-10-01T00:00:00.443553353Z"}
              }
            }"#,
        )
        .unwrap();

        let five_hour = limits.five_hour.unwrap();
        assert_eq!(five_hour.used_percent, 30.0);
        assert_eq!(
            format_rfc3339_millis(five_hour.resets_at.unwrap()),
            "2026-09-11T18:00:00.000Z"
        );
        assert!(limits.seven_day.is_none());
        assert_eq!(limits.month_code.unwrap().used_percent, 25.0);
        assert_eq!(
            format_rfc3339_millis(limits.month_code.unwrap().resets_at.unwrap()),
            "2026-10-01T00:00:00.443Z"
        );
        assert_eq!(limits.month_total.unwrap().used_percent, 40.0);
    }

    #[test]
    fn drops_windows_without_a_numeric_used_ratio() {
        let limits = parse_usage_limits(
            r#"{"usages":{"limit_5h":"half","limit_7d":{"reset_time":"2026-09-17T00:00:00Z"},"limit_month_code":{"used_ratio":"abc"}}}"#,
        )
        .unwrap();

        assert_eq!(limits, KimiUsageLimits::default());
    }

    #[test]
    fn reads_the_pre_usages_weekly_and_five_hour_shape() {
        let limits = parse_usage_limits(
            r#"{
              "usage": {"limit": 100, "used": 12, "resetTime": "2026-09-17T00:00:00Z"},
              "limits": [
                {"window": {"duration": 300, "timeUnit": "TIME_UNIT_MINUTE"},
                 "detail": {"limit": 50, "remaining": 40, "resetTime": "2025-12-23T05:24:18.443553353Z"}}
              ]
            }"#,
        )
        .unwrap();

        assert_eq!(limits.seven_day.unwrap().used_percent, 12.0);
        let five_hour = limits.five_hour.unwrap();
        assert_eq!(five_hour.used_percent, 20.0);
        assert_eq!(
            format_rfc3339_millis(five_hour.resets_at.unwrap()),
            "2025-12-23T05:24:18.443Z"
        );
    }

    #[test]
    fn converts_booster_fixed_point_amounts_to_major_units() {
        // Upstream test: amount 20000000000 → 20000 cents, amountLeft → 10000 cents.
        let limits = parse_usage_limits(
            r#"{
              "boosterWallet": {
                "balance": {"type": "BOOSTER", "amount": "20000000000", "amountLeft": "10000000000"},
                "monthlyChargeLimitEnabled": true,
                "monthlyChargeLimit": {"currency": "CNY", "priceInCents": "20000"},
                "monthlyUsed": {"currency": "CNY", "priceInCents": "5000"}
              }
            }"#,
        )
        .unwrap();

        assert_eq!(
            limits.booster,
            Some(KimiBooster {
                remaining: 100.0,
                total: 200.0,
                currency: Some("CNY".to_string()),
            })
        );
    }

    #[test]
    fn ignores_non_booster_balances_and_rounds_sub_cent_amounts_up() {
        assert!(
            parse_usage_limits(r#"{"boosterWallet":{"balance":{"type":"PLAN","amount":1}}}"#)
                .unwrap()
                .booster
                .is_none()
        );
        let booster = parse_usage_limits(
            r#"{"boosterWallet":{"balance":{"type":"BOOSTER","amount":1250000000,"amountLeft":1}}}"#,
        )
        .unwrap()
        .booster
        .unwrap();

        assert_eq!(booster.total, 12.5);
        assert_eq!(booster.remaining, 0.01);
        assert_eq!(booster.currency, None);
    }

    #[test]
    fn primary_window_prefers_weekly_then_monthly_code_then_monthly_total() {
        let window = |used_percent, reset: &str| KimiQuotaWindow {
            used_percent,
            resets_at: Some(at(reset)),
        };
        let mut limits = KimiUsageLimits {
            seven_day: Some(window(12.0, "2026-09-17T00:00:00Z")),
            month_code: Some(window(25.0, "2026-10-01T00:00:00Z")),
            month_total: Some(window(40.0, "2026-03-01T00:00:00Z")),
            ..KimiUsageLimits::default()
        };
        assert_eq!(limits.primary_window().unwrap().1, 10080);

        limits.seven_day = None;
        let (code, minutes) = limits.primary_window().unwrap();
        assert_eq!(code.used_percent, 25.0);
        assert_eq!(minutes, 30 * 1440);

        limits.month_code = None;
        let (total, minutes) = limits.primary_window().unwrap();
        assert_eq!(total.used_percent, 40.0);
        assert_eq!(minutes, 28 * 1440);
    }

    #[test]
    fn monthly_window_spans_one_utc_calendar_month_before_the_reset() {
        let days = |reset: &str| month_window_minutes(at(reset)).unwrap() / 1440;

        // 31-day month: Jul 1 → Aug 1.
        assert_eq!(days("2026-08-01T00:00:00Z"), 31);
        // 30-day month: Sep 1 → Oct 1.
        assert_eq!(days("2026-10-01T00:00:00Z"), 30);
        // February: Feb 1 → Mar 1, common and leap years.
        assert_eq!(days("2026-03-01T00:00:00Z"), 28);
        assert_eq!(days("2028-03-01T00:00:00Z"), 29);
        // Mar 31 minus one month clamps to Feb 28.
        assert_eq!(days("2026-03-31T00:00:00Z"), 31);
        // A non-midnight reset (16:00Z = midnight CST) keeps whole days, in UTC.
        assert_eq!(
            month_window_minutes(at("2026-09-30T16:00:00Z")),
            Some(31 * 1440)
        );
    }

    #[test]
    fn account_json_omits_unknown_meters() {
        let limits = parse_usage_limits(
            r#"{
              "usages": {
                "limit_5h": {"used_ratio": 0.3, "reset_time": "2026-09-11T18:00:00Z"},
                "limit_month_code": {"used_ratio": 0.25, "reset_time": "2026-10-01T00:00:00Z"},
                "limit_month_total": {"used_ratio": 0.4}
              },
              "boosterWallet": {"balance": {"type": "BOOSTER", "amount": 1250000000, "amountLeft": 500000000},
                                "monthlyUsed": {"currency": "USD", "priceInCents": "0"}}
            }"#,
        )
        .unwrap();

        assert_eq!(
            account_json(&limits),
            json!({
                "sessionUsedPercent": 30.0,
                "sessionResetsAtMs": 1_789_149_600_000_i64,
                "monthlyUsedPercent": 25.0,
                "monthlyResetsAtMs": 1_790_812_800_000_i64,
                "monthlyTotalUsedPercent": 40.0,
                "boosterRemaining": 5.0,
                "boosterTotal": 12.5,
                "boosterCurrency": "USD",
            })
        );
    }

    #[test]
    fn reads_the_default_slot_token_and_default_base_url() {
        let home = fs_fixture!({
            "credentials/kimi-code.json": r#"{"access_token":"at-1","refresh_token":"rt","expires_at":1900000000,"scope":"","token_type":"Bearer","expires_in":900}"#,
        });

        let credentials = credentials(home.root(), None, at("2026-10-15T00:00:00Z")).unwrap();

        assert_eq!(credentials.access_token, "at-1");
        assert_eq!(
            credentials.usage_url(),
            "https://api.kimi.com/coding/v1/usages"
        );
    }

    #[test]
    fn follows_the_configured_region_slot_and_base_url() {
        let home = fs_fixture!({
            "config.toml": [
                r#"[providers."managed:kimi-code"]"#,
                r#"type = "kimi""#,
                r#"base_url = "https://api.kimi.ai/coding/v1/""#,
                r#"[providers."managed:kimi-code".oauth]"#,
                r#"storage = "file""#,
                r#"key = "oauth/kimi-code-env-0123456789abcdef""#,
            ]
            .join("\n"),
            "credentials/kimi-code.json": r#"{"access_token":"cn-token","expires_at":0}"#,
            "credentials/kimi-code-env-0123456789abcdef.json": r#"{"access_token":"global-token","expires_at":0}"#,
        });

        let credentials = credentials(home.root(), None, at("2026-10-15T00:00:00Z")).unwrap();

        assert_eq!(credentials.access_token, "global-token");
        assert_eq!(
            credentials.usage_url(),
            "https://api.kimi.ai/coding/v1/usages"
        );
    }

    #[test]
    fn env_base_url_overrides_the_configured_endpoint() {
        let home = fs_fixture!({
            "credentials/kimi-code.json": r#"{"access_token":"at-1","expires_at":0}"#,
        });

        let credentials = credentials(
            home.root(),
            Some("https://gw.example.com/coding/v1/".to_string()),
            at("2026-10-15T00:00:00Z"),
        )
        .unwrap();

        assert_eq!(
            credentials.usage_url(),
            "https://gw.example.com/coding/v1/usages"
        );
    }

    #[test]
    fn expired_or_missing_tokens_yield_no_credentials() {
        let now = at("2026-10-15T00:00:00Z");
        let expired = fs_fixture!({
            "credentials/kimi-code.json": r#"{"access_token":"at-1","expires_at":1760486400}"#,
        });
        let empty = fs_fixture!({
            "credentials/kimi-code.json": r#"{"access_token":"","expires_at":0}"#,
        });
        let missing = fs_fixture!({ "config.toml": "" });

        assert_eq!(credentials(expired.root(), None, now), None);
        assert_eq!(credentials(empty.root(), None, now), None);
        assert_eq!(credentials(missing.root(), None, now), None);
    }

    #[test]
    fn rejects_credential_slots_that_escape_the_credentials_dir() {
        let home = fs_fixture!({
            "config.toml": "[providers.\"managed:kimi-code\".oauth]\nkey = \"oauth/../secrets\"\n",
            "secrets.json": r#"{"access_token":"nope","expires_at":0}"#,
        });

        assert_eq!(
            credentials(home.root(), None, at("2026-10-15T00:00:00Z")),
            None
        );
    }
}
