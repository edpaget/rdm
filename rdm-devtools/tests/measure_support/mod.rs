//! Shared helpers for the `rdm-measure` integration tests: run the real
//! binary, locate checked-in fixtures and goldens, and compare outputs.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

/// The `rdm-measure` binary under test.
pub fn measure_bin() -> &'static str {
    env!("CARGO_BIN_EXE_rdm-measure")
}

/// The repository checkout.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("rdm-devtools has a parent directory")
        .to_path_buf()
}

/// `tests/fixtures/<name>` in the checkout.
pub fn fixture(name: &str) -> PathBuf {
    repo_root().join("tests/fixtures").join(name)
}

/// Runs `rdm-measure` with `args`, never reading the real `~/.claude`
/// (`HOME` points at an empty temp dir unless the caller overrides it).
pub fn run(args: &[&str]) -> Output {
    let home = tempfile::tempdir().expect("temp home");
    run_with(
        args,
        &[("HOME", home.path().to_str().expect("utf-8 temp path"))],
    )
}

/// Runs `rdm-measure` with `args` and extra environment.
pub fn run_with(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(measure_bin());
    cmd.args(args).current_dir(repo_root());
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("rdm-measure runs")
}

/// stdout as UTF-8.
pub fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// stderr as UTF-8.
pub fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// Asserts a successful exit and returns stdout.
pub fn ok(o: &Output) -> String {
    assert!(
        o.status.success(),
        "rdm-measure failed ({:?}):\nstdout:\n{}\nstderr:\n{}",
        o.status.code(),
        stdout(o),
        stderr(o)
    );
    stdout(o)
}

/// Asserts exit code 1 and returns stderr.
pub fn fails(o: &Output) -> String {
    assert_eq!(
        o.status.code(),
        Some(1),
        "expected exit 1:\nstdout:\n{}\nstderr:\n{}",
        stdout(o),
        stderr(o)
    );
    stderr(o)
}

/// Replaces every occurrence of `root` with `<ROOT>`.
pub fn normalize(text: &str, root: &Path) -> String {
    text.replace(root.to_str().expect("utf-8 path"), "<ROOT>")
}

/// A checked-in golden file's text.
pub fn golden(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("golden {}: {e}", path.display()))
}

/// Parses JSON, failing with the offending text.
pub fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}):\n{text}"))
}

/// Parses JSON with its `instrument` field (the only intentional schema
/// change from the JavaScript tools) removed, for a structural golden
/// comparison.
pub fn json_without_instrument(text: &str) -> Value {
    let mut v = json(text);
    if let Some(o) = v.as_object_mut() {
        o.remove("instrument");
    }
    v
}

/// A copy of a checked-in doc with `edit` applied, written to a temp file.
pub fn edited_doc(src: &Path, edit: impl FnOnce(&mut Value)) -> tempfile::NamedTempFile {
    let mut doc = json(&golden(src));
    edit(&mut doc);
    let file = tempfile::NamedTempFile::new().expect("temp doc");
    std::fs::write(
        file.path(),
        serde_json::to_string_pretty(&doc).expect("serialize"),
    )
    .expect("write temp doc");
    file
}
