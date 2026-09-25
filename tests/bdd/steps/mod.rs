//! Step definitions. Generic steps are registered for Given, When and Then
//! alike so feature files can use whichever keyword reads best.

pub mod ai;
pub mod api;
pub mod cli;
pub mod credentials;
pub mod execution;
pub mod expressions;
pub mod http;
pub mod mocks;
pub mod perf;
pub mod push;
pub mod server;
pub mod workflow;

use cucumber::gherkin::Step;

pub fn docstring(step: &Step) -> &str {
    step.docstring.as_deref().unwrap_or_else(|| panic!("step \"{}\" needs a \"\"\" doc string", step.value))
}

pub fn table(step: &Step) -> &Vec<Vec<String>> {
    &step.table.as_ref().unwrap_or_else(|| panic!("step \"{}\" needs a data table", step.value)).rows
}

/// A docstring of one item per line, or a single-column table, or a
/// comma-separated inline string.
pub fn list(step: &Step, inline: Option<&str>) -> Vec<String> {
    if let Some(inline) = inline {
        return inline.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    }
    if let Some(doc) = &step.docstring {
        return doc.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    }
    table(step).iter().map(|r| r[0].trim().to_string()).collect()
}

/// Polls `check` every 100 ms until it returns `Some` or `timeout` passes.
pub async fn eventually<T, F, Fut>(timeout: std::time::Duration, mut check: F) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Some(v) = check().await {
            return Some(v);
        }
        if std::time::Instant::now() > deadline {
            return None;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
