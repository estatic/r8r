//! Headless execution (`r8r execute --file`) and other CLI commands.
//!
//! Contract (spec §2.1 `r8n-cli`, Phase 1 "CLI execute", mirroring
//! `n8n execute`): `r8r execute --file <workflow.json> --rawOutput` runs the
//! workflow from its manual trigger as a manual execution (pin data is
//! honoured) and prints the n8n `IRun` JSON (`status`, `mode`,
//! `data.resultData.runData`, ...) on stdout. Exit code 0 on success,
//! non-zero on a failed execution; the JSON is printed either way.

use super::docstring;
use crate::support::json::{assert_matches, parse_strict, Mode};
use crate::support::process::split_args;
use crate::world::{pretty, R8rWorld};
use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use serde_json::Value;
use std::time::Duration;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Extracts the IRun JSON object from stdout, tolerating log lines around it.
pub fn parse_run_output(stdout: &str) -> Option<Value> {
    if let Ok(v) = serde_json::from_str::<Value>(stdout.trim()) {
        return Some(v);
    }
    let start = stdout.find("\n{").map(|i| i + 1).or_else(|| stdout.starts_with('{').then_some(0))?;
    let mut stream = serde_json::Deserializer::from_str(&stdout[start..]).into_iter::<Value>();
    stream.next()?.ok()
}

pub async fn execute_current(w: &mut R8rWorld, timeout: Duration) {
    let spec = w.wf().clone();
    let json = w.workflow_json(&spec);
    let file = w.dir.path().join(format!("workflow-{}.json", uuid::Uuid::new_v4()));
    std::fs::write(&file, serde_json::to_vec_pretty(&json).unwrap()).unwrap();
    let args = vec!["execute".to_string(), format!("--file={}", file.display()), "--rawOutput".to_string()];
    let out = w.cli(&args, timeout).await;
    w.run = parse_run_output(&out.stdout);
}

#[when(expr = "I execute the workflow")]
async fn execute(w: &mut R8rWorld) {
    execute_current(w, DEFAULT_TIMEOUT).await;
}

#[when(expr = "I execute the workflow {string}")]
async fn execute_named(w: &mut R8rWorld, name: String) {
    w.wf_named(&name);
    execute_current(w, DEFAULT_TIMEOUT).await;
}

#[when(expr = "I execute the workflow allowing {int} seconds")]
async fn execute_with_timeout(w: &mut R8rWorld, seconds: u64) {
    execute_current(w, Duration::from_secs(seconds)).await;
}

#[given(expr = "the file {string} contains:")]
async fn write_file(w: &mut R8rWorld, name: String, step: &Step) {
    let content = w.expand(docstring(step));
    std::fs::write(w.dir.path().join(name), content).unwrap();
}

/// Runs a command line; a leading `r8r` is optional. Relative file
/// arguments resolve against the scenario folder (the working directory).
#[given(expr = "I run {string}")]
#[when(expr = "I run {string}")]
async fn run_command(w: &mut R8rWorld, line: String) {
    let line = w.expand(&line);
    let mut args = split_args(&line);
    if args.first().map(String::as_str) == Some("r8r") {
        args.remove(0);
    }
    let out = w.cli(&args, DEFAULT_TIMEOUT).await;
    if args.first().map(String::as_str) == Some("execute") {
        w.run = parse_run_output(&out.stdout);
    }
}

#[given(expr = "I successfully run {string}")]
async fn run_command_ok(w: &mut R8rWorld, line: String) {
    run_command(w, line).await;
    let out = w.cli.as_ref().unwrap();
    assert!(out.code == Some(0), "command failed:\n{}", out.describe());
}

fn last(w: &R8rWorld) -> &crate::support::process::CliOutput {
    w.cli.as_ref().expect("no command has run")
}

#[then(expr = "the command succeeds")]
async fn command_succeeds(w: &mut R8rWorld) {
    let out = last(w);
    assert_eq!(out.code, Some(0), "expected exit code 0:\n{}", out.describe());
}

#[then(expr = "the command fails")]
async fn command_fails(w: &mut R8rWorld) {
    let out = last(w);
    assert!(!out.timed_out && out.code.is_some_and(|c| c != 0), "expected a non-zero exit:\n{}", out.describe());
}

#[then(expr = "the command output contains {string}")]
async fn output_contains(w: &mut R8rWorld, needle: String) {
    let out = last(w);
    let needle = w.expand(&needle);
    assert!(
        out.stdout.contains(&needle) || out.stderr.contains(&needle),
        "expected output to contain {needle:?}:\n{}",
        out.describe()
    );
}

#[then(expr = "the command output does not contain {string}")]
async fn output_not_contains(w: &mut R8rWorld, needle: String) {
    let out = last(w);
    let needle = w.expand(&needle);
    assert!(
        !out.stdout.contains(&needle) && !out.stderr.contains(&needle),
        "expected output NOT to contain {needle:?}:\n{}",
        out.describe()
    );
}

#[then(expr = "the command finished within {int} ms")]
async fn finished_within(w: &mut R8rWorld, ms: u64) {
    let out = last(w);
    assert!(!out.timed_out && out.elapsed <= Duration::from_millis(ms), "took {:?}:\n{}", out.elapsed, out.describe());
}

#[then(expr = "the command output is JSON matching:")]
async fn output_json(w: &mut R8rWorld, step: &Step) {
    let out = last(w);
    let actual = parse_run_output(&out.stdout).unwrap_or_else(|| panic!("stdout is not JSON:\n{}", out.describe()));
    let expected = parse_strict(&w.expand(docstring(step)), "expected JSON");
    assert_matches(&expected, &actual, Mode::Subset).unwrap_or_else(|e| panic!("{e}\nactual:\n{}", pretty(&actual)));
}

pub fn read_json_file(w: &R8rWorld, name: &str) -> (String, Value) {
    let path = w.dir.path().join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("cannot read {}: {e}\nlast command:\n{}", path.display(), w.cli.as_ref().map(|c| c.describe()).unwrap_or_default())
    });
    let json = parse_strict(&text, name);
    (text, json)
}

#[then(expr = "the file {string} contains JSON matching:")]
async fn file_json(w: &mut R8rWorld, name: String, step: &Step) {
    let (_, actual) = read_json_file(w, &name);
    let expected = parse_strict(&w.expand(docstring(step)), "expected JSON");
    assert_matches(&expected, &actual, Mode::Subset).unwrap_or_else(|e| panic!("{e}\nactual:\n{}", pretty(&actual)));
}

#[then(expr = "the file {string} does not contain {string}")]
async fn file_not_contains(w: &mut R8rWorld, name: String, needle: String) {
    let text = std::fs::read_to_string(w.dir.path().join(&name)).unwrap_or_default();
    assert!(!text.contains(&needle), "{name} contains {needle:?}");
}

#[then(expr = "the object at {string} in the file {string} has the keys in the order {string}")]
async fn file_key_order(w: &mut R8rWorld, path: String, name: String, keys: String) {
    let (text, _) = read_json_file(w, &name);
    let actual = crate::support::json::key_order(&text, &path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let expected: Vec<String> = keys.split(',').map(|k| k.trim().to_string()).collect();
    let filtered: Vec<String> = actual.iter().filter(|k| expected.contains(k)).cloned().collect();
    assert_eq!(filtered, expected, "all keys at {path}: {actual:?}");
}
