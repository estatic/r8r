//! Fails the build when the embedded editor (`frontend/dist`, via rust-embed)
//! is missing or older than its sources. `frontend/dist` is gitignored, so a
//! `git pull` that changes `frontend/src` leaves the old build in place, and
//! the binary would serve a UI calling API routes that no longer exist (this
//! broke login when r8r's routes moved to `/rest/r8r`).
//!
//! Set `R8R_SKIP_FRONTEND_CHECK=1` to build anyway.

use std::path::Path;
use std::time::SystemTime;

/// Files whose change requires `npm run build`.
const FRONTEND_INPUTS: &[&str] = &[
    "frontend/src",
    "frontend/index.html",
    "frontend/package.json",
    "frontend/package-lock.json",
    "frontend/vite.config.ts",
    "frontend/tailwind.config.js",
    "frontend/postcss.config.js",
];
const BUILT_MARKER: &str = "frontend/dist/index.html";
const HOW_TO_FIX: &str = "run `cd frontend && npm ci && npm run build`, then build again \
    (or set R8R_SKIP_FRONTEND_CHECK=1 to skip this check)";

fn main() {
    for input in FRONTEND_INPUTS {
        println!("cargo:rerun-if-changed={input}");
    }
    println!("cargo:rerun-if-changed={BUILT_MARKER}");
    println!("cargo:rerun-if-env-changed=R8R_SKIP_FRONTEND_CHECK");

    if std::env::var_os("R8R_SKIP_FRONTEND_CHECK").is_some_and(|v| v != "0") {
        return;
    }

    let built = match modified(Path::new(BUILT_MARKER)) {
        Some(t) => t,
        None => fail(&format!("the editor is not built ({BUILT_MARKER} is missing): {HOW_TO_FIX}")),
    };

    let mut newest: Option<(SystemTime, String)> = None;
    for input in FRONTEND_INPUTS {
        newest_in(Path::new(input), &mut newest);
    }
    if let Some((changed, path)) = newest {
        if changed > built {
            fail(&format!(
                "the embedded editor is out of date: {path} changed after frontend/dist was built; {HOW_TO_FIX}"
            ));
        }
    }
}

fn fail(message: &str) -> ! {
    // A panic in build.rs stops the build and prints the message.
    panic!("\n\nr8r: {message}\n\n");
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Newest modification time of `path` (recursing into directories).
fn newest_in(path: &Path, newest: &mut Option<(SystemTime, String)>) {
    let Ok(meta) = std::fs::metadata(path) else { return };
    if meta.is_dir() {
        let Ok(entries) = std::fs::read_dir(path) else { return };
        for entry in entries.flatten() {
            newest_in(&entry.path(), newest);
        }
    } else if let Ok(time) = meta.modified() {
        if newest.as_ref().is_none_or(|(t, _)| time > *t) {
            *newest = Some((time, path.display().to_string()));
        }
    }
}
