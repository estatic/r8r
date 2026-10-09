use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;

pub struct WaitNode;

#[async_trait]
impl Node for WaitNode {
    fn type_name(&self) -> &'static str {
        "core.wait"
    }
    fn display_name(&self) -> &'static str {
        "Wait"
    }
    fn description(&self) -> &'static str {
        "Pauses the run for a while, or until a given time, then passes its items on."
    }
    fn category(&self) -> crate::node::NodeCategory {
        crate::node::NodeCategory::Action
    }
    fn icon(&self) -> &'static str {
        "⏳"
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        let wait = wait_for(&ctx.parameters, chrono::Utc::now()).map_err(|e| NodeError::ExecutionFailed(format!("core.wait: {e}")))?;
        tokio::time::sleep(wait).await;
        Ok(vec![ctx.input_items.clone()])
    }
}

/// The longest wait: the run is held in memory meanwhile, so a restart ends it.
const MAX_WAIT: Duration = Duration::from_secs(31 * 24 * 3600);

/// How long to wait: `resume: "interval"` (`amount` of `unit`), `"at"` a
/// `date_time`, or `seconds` as saved before.
fn wait_for(p: &Value, now: chrono::DateTime<chrono::Utc>) -> Result<Duration, String> {
    let number = |key: &str| -> Result<Option<f64>, String> {
        match p.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Number(n)) => Ok(n.as_f64()),
            Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
            Some(Value::String(s)) => s.trim().parse().map(Some).map_err(|_| format!("\"{key}\" must be a number, got \"{s}\"")),
            Some(other) => Err(format!("\"{key}\" must be a number, got {other}")),
        }
    };
    let wait = match p.get("resume").and_then(Value::as_str) {
        None => Duration::from_secs_f64(number("seconds")?.unwrap_or(0.0).max(0.0)),
        Some("interval") => {
            let amount = number("amount")?.unwrap_or(1.0);
            if amount < 0.0 {
                return Err("\"amount\" can't be negative".into());
            }
            let unit = match p.get("unit").and_then(Value::as_str).unwrap_or("seconds") {
                "seconds" => 1.0,
                "minutes" => 60.0,
                "hours" => 3600.0,
                "days" => 86400.0,
                other => return Err(format!("unknown unit \"{other}\" (expected seconds, minutes, hours or days)")),
            };
            Duration::from_secs_f64((amount * unit).min(MAX_WAIT.as_secs_f64() + 1.0))
        }
        Some("at") => {
            let text = p.get("date_time").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).ok_or("enter the date and time to wait until")?;
            let at = parse_time(text, p).ok_or_else(|| format!("\"{text}\" isn't a date and time (e.g. 2026-10-09T18:30 or 2026-10-09T18:30:00+02:00)"))?;
            // A time already past continues at once, as n8n does.
            (at - now).to_std().unwrap_or(Duration::ZERO)
        }
        Some(other) => return Err(format!("unknown resume \"{other}\" (expected \"interval\" or \"at\")")),
    };
    if wait > MAX_WAIT {
        return Err("waits longer than 31 days aren't supported (the run is held in memory meanwhile)".into());
    }
    Ok(wait)
}

/// An RFC 3339 time, or a local one (no offset) read in the node's time zone.
fn parse_time(text: &str, p: &Value) -> Option<chrono::DateTime<chrono::Utc>> {
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(text) {
        return Some(t.with_timezone(&chrono::Utc));
    }
    let local = ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"]
        .iter()
        .find_map(|f| chrono::NaiveDateTime::parse_from_str(text, f).ok())?;
    let tz = super::schedule::timezone(p).ok()?;
    local.and_local_timezone(tz).earliest().map(|t| t.with_timezone(&chrono::Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> chrono::DateTime<chrono::Utc> {
        "2026-10-09T10:00:00Z".parse().unwrap()
    }

    #[test]
    fn waits_an_amount_of_a_unit() {
        let w = |p: serde_json::Value| wait_for(&p, now());
        assert_eq!(w(serde_json::json!({"resume": "interval", "amount": 2, "unit": "minutes"})).unwrap(), Duration::from_secs(120));
        assert_eq!(w(serde_json::json!({"resume": "interval", "amount": "1.5", "unit": "hours"})).unwrap(), Duration::from_secs(5400));
        assert_eq!(w(serde_json::json!({"seconds": 3})).unwrap(), Duration::from_secs(3));
        assert!(w(serde_json::json!({"resume": "interval", "amount": 40, "unit": "days"})).unwrap_err().contains("31 days"));
        assert!(w(serde_json::json!({"resume": "interval", "amount": 1, "unit": "weeks"})).unwrap_err().contains("unknown unit"));
    }

    #[test]
    fn waits_until_a_time() {
        let w = |p: serde_json::Value| wait_for(&p, now());
        assert_eq!(w(serde_json::json!({"resume": "at", "date_time": "2026-10-09T10:05:00Z"})).unwrap(), Duration::from_secs(300));
        // No offset: read in the node's time zone (Berlin is UTC+2 in October).
        assert_eq!(w(serde_json::json!({"resume": "at", "date_time": "2026-10-09T12:30", "timezone": "Europe/Berlin"})).unwrap(), Duration::from_secs(1800));
        assert_eq!(w(serde_json::json!({"resume": "at", "date_time": "2026-10-08T00:00:00Z"})).unwrap(), Duration::ZERO);
        assert!(w(serde_json::json!({"resume": "at", "date_time": "tomorrow"})).unwrap_err().contains("isn't a date and time"));
    }
    use crate::domain::Item;

    #[tokio::test]
    async fn passes_items_through_after_waiting() {
        let node = WaitNode;
        let items = vec![Item { json: serde_json::json!({"x": 1}), binary: serde_json::json!({}) }];
        let ctx = NodeExecutionContext { parameters: serde_json::json!({"seconds": 0.01}), input_items: items.clone(), ..Default::default() };
        let start = std::time::Instant::now();
        let result = node.execute(&ctx).await.unwrap();
        assert!(start.elapsed() >= std::time::Duration::from_millis(10));
        assert_eq!(result[0], items);
    }

    #[tokio::test]
    async fn missing_seconds_defaults_to_zero_wait() {
        let node = WaitNode;
        let ctx = NodeExecutionContext { parameters: serde_json::json!({}), input_items: vec![], ..Default::default() };
        let result = node.execute(&ctx).await.unwrap();
        assert!(result[0].is_empty());
    }
}
