use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;
use chrono_tz::Tz;
use serde_json::Value;

pub struct ScheduleNode;

#[async_trait]
impl Node for ScheduleNode {
    fn type_name(&self) -> &'static str {
        "core.schedule"
    }
    fn display_name(&self) -> &'static str {
        "Schedule"
    }
    fn description(&self) -> &'static str {
        "Starts a run on a schedule: every N seconds, minutes, hours, days, weeks or months, or a cron expression."
    }
    fn category(&self) -> crate::node::NodeCategory {
        crate::node::NodeCategory::Trigger
    }
    fn icon(&self) -> &'static str {
        "⏰"
    }

    /// A manual run gets the same item a scheduled firing does.
    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        let tz = timezone(&ctx.parameters).map_err(NodeError::ExecutionFailed)?;
        Ok(vec![vec![fired_item(tz)]])
    }
}

/// The item a firing emits, as n8n's Schedule Trigger builds it
/// (`timestamp`, `Readable date`, `Day of week`, ... in the node's time zone).
pub fn fired_item(tz: Tz) -> Item {
    let item = crate::n8n::server::activation::schedule_item(chrono::Utc::now().with_timezone(&tz), &tz);
    Item { json: serde_json::Value::Object(item.json), binary: serde_json::json!({}) }
}

/// The node's `timezone`, else `GENERIC_TIMEZONE`, else UTC.
pub fn timezone(p: &Value) -> Result<Tz, String> {
    let name = p
        .get("timezone")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| std::env::var("GENERIC_TIMEZONE").ok().filter(|s| !s.trim().is_empty()));
    match name {
        Some(n) => n.parse::<Tz>().map_err(|_| format!("core.schedule: unknown time zone \"{n}\" (e.g. Europe/Berlin)")),
        None => Ok(Tz::UTC),
    }
}

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// The 6-field cron (`sec min hour day month weekday`) the node's schedule
/// means: its `rule`, or a `cron` saved before rules.
pub fn cron_for(p: &Value) -> Result<String, String> {
    let fail = |m: String| format!("core.schedule: {m}");
    let Some(rule) = p.get("rule") else {
        let cron = p.get("cron").and_then(Value::as_str).ok_or_else(|| fail("choose when it runs".into()))?;
        return normalize_cron(cron).map_err(fail);
    };
    let num = |key: &str, default: u64, max: u64| -> Result<u64, String> {
        let n = match rule.get(key) {
            None | Some(Value::Null) => default,
            Some(Value::Number(n)) => n.as_u64().ok_or_else(|| fail(format!("\"{key}\" must be a whole number")))?,
            Some(Value::String(s)) if s.trim().is_empty() => default,
            Some(Value::String(s)) => s.trim().parse().map_err(|_| fail(format!("\"{key}\" must be a whole number")))?,
            Some(other) => return Err(fail(format!("\"{key}\" must be a whole number, got {other}"))),
        };
        if n > max {
            return Err(fail(format!("\"{key}\" must be at most {max}")));
        }
        Ok(n)
    };
    let every = |max: u64| -> Result<u64, String> {
        let n = num("every", 1, max)?;
        if n == 0 { Err(fail("\"every\" must be at least 1".into())) } else { Ok(n) }
    };
    let (minute, hour) = (num("minute", 0, 59)?, num("hour", 0, 23)?);
    let step = |n: u64| if n == 1 { "*".to_string() } else { format!("*/{n}") };
    Ok(match rule.get("interval").and_then(Value::as_str).unwrap_or("days") {
        "seconds" => format!("{} * * * * *", step(every(59)?)),
        "minutes" => format!("0 {} * * * *", step(every(59)?)),
        "hours" => format!("0 {minute} {} * * *", step(every(23)?)),
        "days" => format!("0 {minute} {hour} {} * *", step(every(31)?)),
        "weeks" => {
            if every(52)? != 1 {
                return Err(fail("weekly schedules run every week; use a cron expression for other spacing".into()));
            }
            let days: Vec<&str> = match rule.get("weekdays").and_then(Value::as_array) {
                Some(list) if !list.is_empty() => list
                    .iter()
                    .map(|d| match d {
                        Value::Number(n) => n.as_u64().and_then(|n| WEEKDAYS.get(n as usize).copied()),
                        Value::String(s) => WEEKDAYS.iter().copied().find(|w| s.to_lowercase().starts_with(&w.to_lowercase())),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(|| fail("weekdays are 0 (Sunday) to 6 (Saturday)".into()))?,
                _ => vec!["Mon"],
            };
            format!("0 {minute} {hour} * * {}", days.join(","))
        }
        "months" => {
            let day = num("day_of_month", 1, 31)?.max(1);
            format!("0 {minute} {hour} {day} {} *", step(every(12)?))
        }
        "cron" => normalize_cron(rule.get("expression").and_then(Value::as_str).unwrap_or("")).map_err(fail)?,
        other => return Err(fail(format!("unknown interval \"{other}\" (expected seconds, minutes, hours, days, weeks, months or cron)"))),
    })
}

/// A 5-field cron (n8n's, minute first) gains a leading second; 6 fields stay.
fn normalize_cron(expr: &str) -> Result<String, String> {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    let cron = match fields.len() {
        5 => format!("0 {}", fields.join(" ")),
        6 => fields.join(" "),
        0 => return Err("enter a cron expression".into()),
        _ => return Err(format!("\"{expr}\" isn't a cron expression (5 fields: minute hour day month weekday)")),
    };
    // Checked now, so a bad expression is reported on the node, not at activation.
    cron.parse::<cron::Schedule>().map_err(|e| format!("\"{expr}\" isn't a valid cron expression: {e}"))?;
    Ok(cron)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cron(rule: Value) -> Result<String, String> {
        cron_for(&json!({"rule": rule}))
    }

    #[tokio::test]
    async fn a_manual_run_emits_n8ns_schedule_item_in_the_nodes_time_zone() {
        let ctx = NodeExecutionContext { parameters: json!({"timezone": "Europe/Berlin"}), ..Default::default() };
        let item = &ScheduleNode.execute(&ctx).await.unwrap()[0][0].json;
        assert!(item["timestamp"].is_string() && item["Day of week"].is_string());
        assert!(item["Timezone"].as_str().unwrap().starts_with("Europe/Berlin (UTC+0"), "{item}");
    }

    #[test]
    fn intervals_become_cron() {
        assert_eq!(cron(json!({"interval": "seconds", "every": 30})).unwrap(), "*/30 * * * * *");
        assert_eq!(cron(json!({"interval": "minutes", "every": "15"})).unwrap(), "0 */15 * * * *");
        assert_eq!(cron(json!({"interval": "hours", "every": 2, "minute": 30})).unwrap(), "0 30 */2 * * *");
        assert_eq!(cron(json!({"interval": "days", "hour": 9, "minute": 5})).unwrap(), "0 5 9 * * *");
        assert_eq!(cron(json!({"interval": "weeks", "weekdays": [1, 5], "hour": 8})).unwrap(), "0 0 8 * * Mon,Fri");
        assert_eq!(cron(json!({"interval": "months", "every": 3, "day_of_month": 15, "hour": 12})).unwrap(), "0 0 12 15 */3 *");
        assert_eq!(cron(json!({"interval": "cron", "expression": "*/5 9-17 * * 1-5"})).unwrap(), "0 */5 9-17 * * 1-5");
    }

    #[test]
    fn an_older_cron_parameter_still_works() {
        assert_eq!(cron_for(&json!({"cron": "0 0 * * * *"})).unwrap(), "0 0 * * * *");
    }

    #[test]
    fn says_what_is_wrong_with_a_schedule() {
        for (rule, says) in [
            (json!({"interval": "minutes", "every": 0}), "at least 1"),
            (json!({"interval": "days", "hour": 24}), "\"hour\" must be at most 23"),
            (json!({"interval": "weeks", "every": 2}), "use a cron expression"),
            (json!({"interval": "weeks", "weekdays": [9]}), "0 (Sunday) to 6"),
            (json!({"interval": "cron", "expression": "every day"}), "isn't a cron expression"),
            (json!({"interval": "cron", "expression": "99 * * * *"}), "isn't a valid cron expression"),
            (json!({"interval": "fortnights"}), "unknown interval"),
        ] {
            let err = cron(rule.clone()).unwrap_err();
            assert!(err.contains(says), "{rule}: {err}");
        }
        assert!(timezone(&json!({"timezone": "Mars/Olympus"})).unwrap_err().contains("unknown time zone"));
    }
}
