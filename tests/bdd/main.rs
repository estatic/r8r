//! BDD conformance suite for r8r, the Rust reimplementation of n8n.
//!
//! Scenarios live in `tests/bdd/features/` and describe the target behaviour
//! from the reimplementation spec (see tests/bdd/README.md). The binary is
//! driven black-box, so a scenario fails until r8r implements what it
//! describes.
//!
//!   cargo test --test bdd                          # default set
//!   cargo test --test bdd -- --tags @phase-1       # one roadmap phase
//!   cargo test --test bdd -- -i 'features/03-expressions/*'
//!   R8R_BDD_INCLUDE=@perf cargo test --test bdd    # add opt-in tags

mod steps;
mod support;
mod world;

use cucumber::World as _;

/// Scenarios with these tags need infrastructure or time the default run
/// shouldn't assume. Opt in via `R8R_BDD_INCLUDE` (comma-separated), or
/// select them explicitly with `--tags`, which replaces this filter.
const OPT_IN_TAGS: &[&str] = &["perf", "slow", "requires-redis", "requires-postgres", "requires-python-runner"];

#[tokio::main]
async fn main() {
    let include: Vec<String> = std::env::var("R8R_BDD_INCLUDE")
        .unwrap_or_default()
        .split(',')
        .map(|t| t.trim().trim_start_matches('@').to_string())
        .filter(|t| !t.is_empty())
        .collect();
    let concurrency = std::env::var("R8R_BDD_CONCURRENCY").ok().and_then(|c| c.parse().ok()).unwrap_or(8);

    world::R8rWorld::cucumber()
        .max_concurrent_scenarios(concurrency)
        // An undefined step means a typo or a missing step definition; count
        // it as a failure instead of silently skipping the scenario.
        .fail_on_skipped()
        .filter_run_and_exit(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/bdd/features"), move |feature, rule, scenario| {
            let tags = feature.tags.iter().chain(rule.iter().flat_map(|r| &r.tags)).chain(&scenario.tags);
            let mut opt_in = tags.filter(|t| OPT_IN_TAGS.contains(&t.as_str()));
            opt_in.all(|t| include.iter().any(|i| i == t))
        })
        .await;
}
