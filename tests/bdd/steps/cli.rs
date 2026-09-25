//! Headless execution and other CLI commands.
//!
//! Contract (spec §2.1 `r8n-cli`, Phase 1 "CLI execute"), identical to n8n
//! 2.x: `r8r import:workflow --input=<file>` stores the workflow, then
//! `r8r execute --id=<id> --rawOutput` runs it in "cli" mode and prints the
//! `IRun` JSON (`status`, `mode`, `data.resultData.runData`, ...) on stdout
//! after any log lines. Pin data does not apply in this mode; see
//! `support::workflow::prepare_for_cli` for how trigger input is fed.

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

/// Imports the current workflow and runs it headless, as with n8n:
/// `import:workflow --input=<file>` then `execute --id=<id> --rawOutput`.
pub async fn execute_current(w: &mut R8rWorld, timeout: Duration) {
    let spec = w.wf().clone();
    let mut json = w.workflow_json(&spec);
    let id = crate::support::workflow::prepare_for_cli(&mut json);
    let file = w.dir.path().join(format!("workflow-{id}.json"));
    std::fs::write(&file, serde_json::to_vec_pretty(&json).unwrap()).unwrap();
    let import = vec!["import:workflow".to_string(), format!("--input={}", file.display())];
    let out = w.cli(&import, timeout).await;
    if out.code != Some(0) {
        w.run = None;
        return;
    }
    let args = vec!["execute".to_string(), format!("--id={id}"), "--rawOutput".to_string()];
    let started = std::time::Instant::now();
    let mut out = w.cli(&args, timeout).await;
    // "finished within" assertions time the execution, not the import.
    out.elapsed = started.elapsed();
    w.cli = Some(out.clone());
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
    // A non-zero exit or death by signal both count; a timeout does not.
    assert!(!out.timed_out && out.code != Some(0), "expected the command to fail:\n{}", out.describe());
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
