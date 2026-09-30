//! Usage history from the Dashboard, and the Remaining limit from `hermes usage --json`.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// Token and cost sums over some Sessions. A `daily` row names them plainly; `totals` prefixes
/// them (`total_input`, …), and a Session record suffixes the cost (`estimated_cost_usd`).
/// Numbers stay as Hermes's SQLite sums them, and a sum over no rows is null.
#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
pub struct Tally {
    #[serde(alias = "total_input", default, deserialize_with = "zero_if_null")]
    pub input_tokens: f64,
    #[serde(alias = "total_output", default, deserialize_with = "zero_if_null")]
    pub output_tokens: f64,
    #[serde(alias = "total_cache_read", default, deserialize_with = "zero_if_null")]
    pub cache_read_tokens: f64,
    #[serde(alias = "total_estimated_cost", alias = "estimated_cost_usd", default, deserialize_with = "zero_if_null")]
    pub estimated_cost: f64,
    #[serde(alias = "total_sessions", default, deserialize_with = "zero_if_null")]
    pub sessions: f64,
}

fn zero_if_null<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    Ok(Option::<f64>::deserialize(d)?.unwrap_or_default())
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Day {
    /// Local calendar day on the Hermes host, `YYYY-MM-DD`.
    pub day: String,
    #[serde(flatten)]
    pub tally: Tally,
}

impl Tally {
    fn plus(self, other: &Tally) -> Tally {
        Tally {
            input_tokens: self.input_tokens + other.input_tokens,
            output_tokens: self.output_tokens + other.output_tokens,
            cache_read_tokens: self.cache_read_tokens + other.cache_read_tokens,
            estimated_cost: self.estimated_cost + other.estimated_cost,
            sessions: self.sessions + other.sessions,
        }
    }
}

/// `GET /api/analytics/usage?days=N`: Sessions started in the last N days, by the day they started.
#[derive(Serialize, Debug, PartialEq)]
pub struct Analytics {
    /// Main agent calls only.
    pub daily: Vec<Day>,
    /// Main agent calls plus `side_calls`.
    pub totals: Tally,
    /// Auxiliary calls (compression, titles, vision, …) of those Sessions. They never reach the
    /// Session counters, so Hermes sums them per task (`by_task`), not per day or Session.
    pub side_calls: Tally,
}

pub fn analytics(body: Value) -> Result<Analytics, serde_json::Error> {
    #[derive(Deserialize)]
    struct Body {
        #[serde(default)]
        daily: Vec<Day>,
        totals: Tally,
        #[serde(default)]
        by_task: Vec<Tally>,
    }
    let body: Body = serde_json::from_value(body)?;
    let side_calls = body.by_task.iter().fold(Tally::default(), Tally::plus);
    Ok(Analytics { daily: body.daily, totals: body.totals.plus(&side_calls), side_calls })
}

/// API server Session records (`GET /api/sessions`) started after `cutoff` (epoch seconds),
/// summed the way the analytics route sums them. Side calls aren't in these records.
pub fn tally_since(sessions: Value, cutoff: f64) -> Result<Tally, serde_json::Error> {
    #[derive(Deserialize)]
    struct Record {
        started_at: f64,
        #[serde(flatten)]
        tally: Tally,
    }
    let records: Vec<Record> = serde_json::from_value(sessions)?;
    let started = records.into_iter().filter(|r| r.started_at > cutoff);
    Ok(started.fold(Tally::default(), |sum, r| sum.plus(&Tally { sessions: 1.0, ..r.tally })))
}

/// One rate-limit or credit window of the provider account.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Window {
    pub label: String,
    #[serde(default)]
    pub used_percent: Option<f64>,
    #[serde(default)]
    pub resets_at: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
}

/// `hermes usage --json` (hermes_cli/subcommands/usage.py): the Profile's provider account limits.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Limit {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub fetched_at: Option<String>,
    #[serde(default)]
    pub windows: Vec<Window>,
    #[serde(default)]
    pub details: Vec<String>,
    #[serde(default)]
    pub unavailable_reason: Option<String>,
}

pub fn limit(body: Value) -> Result<Limit, serde_json::Error> {
    serde_json::from_value(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn analytics_days_and_prefixed_totals_share_one_shape() {
        let body = json!({
            "daily": [
                { "day": "2026-09-28", "input_tokens": 1200, "output_tokens": 300, "cache_read_tokens": 800, "reasoning_tokens": null,
                  "estimated_cost": 0.042, "actual_cost": 0, "sessions": 2, "api_calls": 5 },
            ],
            "by_model": [], "by_task": [], "skills": {}, "tools": {}, "period_days": 7,
            "totals": { "total_input": 1200, "total_output": 300, "total_cache_read": 800, "total_reasoning": null,
                        "total_estimated_cost": 0.042, "total_actual_cost": 0, "total_sessions": 2, "total_api_calls": 5 },
        });
        let parsed = analytics(body).unwrap();
        let tally = Tally { input_tokens: 1200.0, output_tokens: 300.0, cache_read_tokens: 800.0, estimated_cost: 0.042, sessions: 2.0 };
        assert_eq!(parsed.daily, [Day { day: "2026-09-28".into(), tally }]);
        assert_eq!(parsed.totals.input_tokens, 1200.0);
        assert_eq!(parsed.totals.sessions, 2.0);
        let out = serde_json::to_value(&parsed).unwrap();
        assert_eq!(out["daily"][0]["day"], "2026-09-28", "the day stays beside its sums for the webview");
        assert_eq!(out["totals"]["cache_read_tokens"], 800.0);
    }

    #[test]
    fn side_calls_count_toward_the_totals_but_not_the_days() {
        // Compression and title calls never reach the Session counters; Hermes sums them per task.
        let body = json!({
            "daily": [{ "day": "2026-09-28", "input_tokens": 1000, "output_tokens": 100, "cache_read_tokens": 0,
                        "estimated_cost": 0.5, "sessions": 1 }],
            "by_task": [
                { "task": "compression", "input_tokens": 400, "output_tokens": 40, "estimated_cost": 0.25, "api_calls": 2, "models": ["m"] },
                { "task": "title_generation", "input_tokens": 100, "output_tokens": 10, "estimated_cost": 0.125, "api_calls": 1, "models": ["m"] },
            ],
            "totals": { "total_input": 1000, "total_output": 100, "total_cache_read": 0, "total_estimated_cost": 0.5, "total_sessions": 1 },
        });
        let parsed = analytics(body).unwrap();
        let side = Tally { input_tokens: 500.0, output_tokens: 50.0, estimated_cost: 0.375, ..Tally::default() };
        assert_eq!(parsed.side_calls, side);
        assert_eq!(parsed.totals, Tally { input_tokens: 1500.0, output_tokens: 150.0, cache_read_tokens: 0.0, estimated_cost: 0.875, sessions: 1.0 });
        assert_eq!(parsed.daily[0].tally.estimated_cost, 0.5);
    }

    #[test]
    fn analytics_over_no_sessions_is_all_zeros() {
        let body = json!({ "daily": [], "totals": { "total_input": null, "total_output": null, "total_cache_read": null,
            "total_reasoning": null, "total_estimated_cost": 0, "total_actual_cost": 0, "total_sessions": 0, "total_api_calls": null } });
        let parsed = analytics(body).unwrap();
        assert!(parsed.daily.is_empty());
        assert_eq!(parsed.totals, Tally::default());
    }

    #[test]
    fn past_hour_sums_the_session_records_started_in_it() {
        let now = 1_800_000_000.0;
        let sessions = json!([
            { "id": "a", "started_at": now - 60.0, "input_tokens": 1000, "output_tokens": 200, "cache_read_tokens": 500,
              "reasoning_tokens": null, "estimated_cost_usd": 0.25, "actual_cost_usd": null, "last_active": now },
            // No cost yet: Hermes starts it at NULL.
            { "id": "b", "started_at": now - 3000.0, "input_tokens": 10, "output_tokens": 5, "estimated_cost_usd": null },
            // Active in the last hour but started before it, like a back-filled pinned Session.
            { "id": "c", "started_at": now - 3600.0, "input_tokens": 9999, "output_tokens": 9999, "estimated_cost_usd": 9.0, "pinned": true },
        ]);
        let tally = Tally { input_tokens: 1010.0, output_tokens: 205.0, cache_read_tokens: 500.0, estimated_cost: 0.25, sessions: 2.0 };
        assert_eq!(tally_since(sessions, now - 3600.0).unwrap(), tally);
        assert_eq!(tally_since(json!([]), now).unwrap(), Tally::default());
    }

    #[test]
    fn remaining_limit_from_hermes_usage_json() {
        // Real `hermes usage --json` output for an OpenRouter key.
        let body = json!({
            "provider": "openrouter", "source": "credits_api", "title": "Account limits", "plan": null,
            "fetched_at": "2026-09-29T21:15:17.647121+00:00",
            "windows": [{ "label": "API key quota", "used_percent": 65.510374511, "resets_at": null,
                          "detail": "$34.49 of $100.00 remaining \u{2022} resets monthly" }],
            "details": ["Credits balance: $23.13"], "unavailable_reason": null,
        });
        let parsed = limit(body).unwrap();
        assert_eq!(parsed.provider, "openrouter");
        assert_eq!(parsed.windows[0].label, "API key quota");
        assert_eq!(parsed.windows[0].used_percent, Some(65.510374511));
        assert_eq!(parsed.windows[0].resets_at, None);
        assert_eq!(parsed.details, ["Credits balance: $23.13"]);

        let unavailable = limit(json!({ "provider": "anthropic", "source": "oauth", "fetched_at": "2026-09-29T00:00:00+00:00",
            "windows": [], "details": [], "unavailable_reason": "Usage endpoint returned 403" })).unwrap();
        assert!(unavailable.windows.is_empty());
        assert_eq!(unavailable.unavailable_reason.as_deref(), Some("Usage endpoint returned 403"));
    }
}
