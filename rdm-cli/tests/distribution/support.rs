//! Shared scaffolding for the distribution tests: sandboxed emission into a
//! temp tree, a structural frontmatter parser, a recursive tree snapshot and
//! differ, the plugin-manifest version normalizer, and the marketplace
//! checker. Nothing here inspects prose: frontmatter and manifests are
//! parsed, trees are compared byte for byte.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::Value;
use tempfile::TempDir;

use crate::plan_fixture::Sandbox;

/// The skills `agent-config claude --skills` ships, by directory.
pub const SKILLS: [&str; 11] = [
    "rdm-autopilot",
    "rdm-backlog",
    "rdm-dispatch-phase",
    "rdm-do",
    "rdm-document",
    "rdm-estimate",
    "rdm-land",
    "rdm-plan-review",
    "rdm-review",
    "rdm-revise",
    "rdm-roadmap",
];

/// The command that regenerates the checked-in plugin tree.
pub const REGENERATE_PLUGIN: &str = "env -u RDM_ROOT -u RDM_PROJECT cargo run -q --bin rdm -- agent-config claude --plugin --out plugins/rdm";

/// A temp tree with a sandboxed user context, for running emissions.
pub struct Emit {
    /// The owning temp directory.
    pub dir: TempDir,
    /// The user context every emission runs under.
    pub sandbox: Sandbox,
}

impl Emit {
    /// A fresh temp tree with its sandbox under `<dir>/user`.
    pub fn new() -> Self {
        let dir = TempDir::new().expect("tempdir");
        let sandbox = Sandbox::new(&dir.path().join("user")).expect("sandbox");
        Self { dir, sandbox }
    }

    /// A path under the temp tree.
    pub fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    /// Runs the binary under test with `args` from the temp tree, whatever
    /// the status.
    pub fn rdm(&self, args: &[&str]) -> Output {
        self.rdm_env(args, &[])
    }

    /// Like [`Emit::rdm`], with extra environment.
    pub fn rdm_env(&self, args: &[&str], env: &[(&str, &Path)]) -> Output {
        let mut cmd = self.sandbox.rdm();
        cmd.args(args).current_dir(self.dir.path());
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.output().expect("spawn rdm")
    }

    /// Runs `rdm <args>` and requires success.
    pub fn ok(&self, args: &[&str]) -> Output {
        let out = self.rdm(args);
        assert!(
            out.status.success(),
            "`rdm {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }

    /// `agent-config claude --skills --project distro-check --out <rel>`.
    pub fn skills(&self, rel: &str) -> (PathBuf, Output) {
        let out_dir = self.path(rel);
        let out = self.ok(&[
            "agent-config",
            "claude",
            "--skills",
            "--project",
            "distro-check",
            "--out",
            &out_dir.to_string_lossy(),
        ]);
        (out_dir, out)
    }

    /// `agent-config claude --plugin --out <rel>` (no `--project`).
    pub fn plugin(&self, rel: &str) -> (PathBuf, Output) {
        let out_dir = self.path(rel);
        let out = self.ok(&[
            "agent-config",
            "claude",
            "--plugin",
            "--out",
            &out_dir.to_string_lossy(),
        ]);
        (out_dir, out)
    }
}

/// The workspace root (read only).
pub fn repo_root() -> PathBuf {
    crate::workflow_support::repo_root()
}

/// Stdout lines of the cleanup report (`Removed …`, `Skipped …`,
/// `Failed to remove …`).
pub fn cleanup_report(out: &Output) -> Vec<String> {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| {
            l.starts_with("Removed ")
                || l.starts_with("Skipped ")
                || l.starts_with("Failed to remove ")
        })
        .map(str::to_owned)
        .collect()
}

/// Every file under `root`, by `/`-separated relative path, with its bytes.
pub fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            if entry.file_type().expect("file type").is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .expect("under root")
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, std::fs::read(&path).expect("read file"));
            }
        }
    }
    out
}

/// Every difference between two trees: paths only in one, and paths whose
/// bytes differ.
pub fn tree_diff(
    left: &BTreeMap<String, Vec<u8>>,
    right: &BTreeMap<String, Vec<u8>>,
) -> Vec<String> {
    let mut out = Vec::new();
    for (path, bytes) in left {
        match right.get(path) {
            None => out.push(format!("only in the first tree: {path}")),
            Some(other) if other != bytes => out.push(format!("differs: {path}")),
            Some(_) => {}
        }
    }
    for path in right.keys() {
        if !left.contains_key(path) {
            out.push(format!("only in the second tree: {path}"));
        }
    }
    out
}

/// The names of the entries directly under `dir` (files and directories).
pub fn entries(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// A skill or agent file's frontmatter: the top-level `key: value` pairs
/// between the opening and closing `---` fences (a key whose value is an
/// indented block maps to the empty string).
///
/// # Errors
///
/// A message naming what is malformed: no opening fence on line 1, or no
/// closing fence.
pub fn frontmatter(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut lines = text.lines();
    if lines.next() != Some("---") {
        return Err("the first line is not a `---` fence".to_owned());
    }
    let mut fields = BTreeMap::new();
    for line in lines {
        if line == "---" {
            return Ok(fields);
        }
        if line.starts_with([' ', '\t']) || line.trim().is_empty() {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            fields.insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    Err("no closing `---` fence".to_owned())
}

/// Checks that `dir/<name>/SKILL.md` exists for exactly `names`, each with a
/// frontmatter whose `name` is the directory and whose `description` is
/// non-empty; returns every problem.
pub fn skill_problems(dir: &Path, names: &BTreeSet<String>) -> Vec<String> {
    let mut problems = Vec::new();
    let found = entries(dir);
    if &found != names {
        problems.push(format!("skill directories {found:?}, expected {names:?}"));
    }
    for name in names {
        let path = dir.join(name).join("SKILL.md");
        match std::fs::read_to_string(&path) {
            Err(e) => problems.push(format!("{}: {e}", path.display())),
            Ok(text) => match frontmatter(&text) {
                Err(e) => problems.push(format!("{}: {e}", path.display())),
                Ok(f) => {
                    if f.get("name").map(String::as_str) != Some(name.as_str()) {
                        problems.push(format!(
                            "{}: frontmatter name {:?}",
                            path.display(),
                            f.get("name")
                        ));
                    }
                    if f.get("description").is_none_or(String::is_empty) {
                        problems.push(format!("{}: empty description", path.display()));
                    }
                }
            },
        }
    }
    problems
}

/// Replaces the manifest's `version` value — read from the parsed JSON — with
/// a placeholder, touching no other byte.
///
/// # Errors
///
/// When the bytes are not a JSON object with a string `version`, or that
/// value's JSON spelling does not occur exactly once.
pub fn normalize_manifest_version(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let parsed: Value = serde_json::from_slice(bytes).map_err(|e| format!("manifest: {e}"))?;
    let version = parsed["version"]
        .as_str()
        .ok_or_else(|| "manifest has no string version".to_owned())?;
    let quoted = serde_json::to_string(version).map_err(|e| e.to_string())?;
    let text = String::from_utf8(bytes.to_vec()).map_err(|e| e.to_string())?;
    match text.matches(&quoted).count() {
        1 => Ok(text.replacen(&quoted, "\"<VERSION>\"", 1).into_bytes()),
        n => Err(format!(
            "the version {quoted} occurs {n} times in the manifest"
        )),
    }
}

/// A plugin tree with its manifest's version normalized.
pub fn normalized_plugin_tree(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut t = tree(root);
    let key = ".claude-plugin/plugin.json".to_owned();
    let manifest = t
        .get(&key)
        .ok_or_else(|| format!("{}: no {key}", root.display()))?;
    let normalized = normalize_manifest_version(manifest)?;
    t.insert(key, normalized);
    Ok(t)
}

fn non_empty_str(v: &Value) -> bool {
    v.as_str().is_some_and(|s| !s.trim().is_empty())
}

/// Every problem with a marketplace document whose plugin sources resolve
/// against `root`.
pub fn marketplace_problems(market: &Value, root: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    for (path, v) in [
        ("name", &market["name"]),
        ("description", &market["description"]),
        ("owner.name", &market["owner"]["name"]),
        ("owner.url", &market["owner"]["url"]),
    ] {
        if !non_empty_str(v) {
            problems.push(format!("marketplace `{path}` is missing or empty"));
        }
    }
    let plugins = market["plugins"].as_array().cloned().unwrap_or_default();
    if plugins.is_empty() {
        problems.push("marketplace `plugins` is missing or empty".to_owned());
    }
    for (i, entry) in plugins.iter().enumerate() {
        if !non_empty_str(&entry["name"]) {
            problems.push(format!("plugins[{i}].name is missing or empty"));
        }
        let Some(source) = entry["source"].as_str().filter(|s| !s.trim().is_empty()) else {
            problems.push(format!("plugins[{i}].source is missing or empty"));
            continue;
        };
        let dir = root.join(source);
        let manifest = dir.join(".claude-plugin/plugin.json");
        match std::fs::read(&manifest)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        {
            None => problems.push(format!(
                "plugins[{i}].source {source:?} does not resolve to a plugin with a readable manifest"
            )),
            Some(m) => {
                if m["name"] != entry["name"] {
                    problems.push(format!(
                        "plugins[{i}] is named {} but its manifest says {}",
                        entry["name"], m["name"]
                    ));
                }
            }
        }
    }
    problems
}
