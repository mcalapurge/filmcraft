//! Whisper runs in the web build's speech worker on wasm32, where `std::time::Instant::now()`
//! panics with "time not implemented on this platform" (every transcription crashed there).
//! `web_time::Instant` is `std::time::Instant` on native targets and `performance.now()` on the
//! web, so there is never a reason to name the std one in this crate.

use std::path::{Path, PathBuf};

/// Every non-test source file of the crate, as (path, contents).
fn sources() -> Vec<(PathBuf, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    let mut stack = vec![dir];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            if !name.ends_with(".rs") || name.ends_with("_tests.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap();
            out.push((p, text));
        }
    }
    assert!(out.iter().any(|(p, _)| p.ends_with("whisper.rs")), "found {} source files", out.len());
    out
}

#[test]
fn no_std_instant_in_code_the_web_build_runs() {
    let mut hits = Vec::new();
    for (path, text) in sources() {
        for (i, line) in text.lines().enumerate() {
            // `use std::time::{.., Instant, ..}` or a fully qualified `std::time::Instant`
            let names_std_instant = line.contains("std::time::Instant") || (line.contains("use std::time::") && line.contains("Instant"));
            if names_std_instant && !line.contains("web_time") && !line.trim_start().starts_with("//") {
                hits.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
            }
        }
    }
    assert!(hits.is_empty(), "std::time::Instant panics on wasm32; use web_time::Instant:\n{}", hits.join("\n"));
}
