//! Project-authored dispatch directives — discovery, verbatim reading, and bounding.
//!
//! A project expresses optional development discipline (mutation testing, coverage
//! floors, fuzzing, review emphases) as prose it already writes for its own agents.
//! This module RESOLVES that prose into a bounded set of [`Directive`] records; the
//! dispatch lane then injects the text **verbatim** into the implementer's or
//! reviewer's prompt. rdm never summarizes, paraphrases, or re-words a directive —
//! a paraphrase would be rdm's reading of the project's rule interposed between the
//! project and the agent, which is exactly what must not happen.
//!
//! This is the **guidance** half of the dispatch lane, deliberately not enforcement.
//! Declared checks (`dispatch.verify`) are executed and gate the outcome; a directive
//! is text that shapes what an agent chooses to do. Instruction files shape behavior
//! and are not a hard enforcement layer. Canonical write-up: `docs/project-directives.md`.
//!
//! # Sources
//!
//! With no `dispatch.directives` key declared, [`discover_sources`] looks in this
//! FIXED order (the order is the emitted order, so output is deterministic):
//!
//! 1. `.claude/rules/**/*.md` (recursive)
//! 2. `AGENTS.md`
//! 3. `.cursor/rules/*.mdc`
//! 4. `.clinerules` (a file, or `*.md` inside it when it is a directory)
//! 5. `.windsurf/rules/**/*.md` (recursive)
//! 6. `.github/copilot-instructions.md`
//!
//! `CLAUDE.md` is deliberately EXCLUDED: Claude Code already loads it into every
//! subagent, so injecting it would pay its token cost twice per dispatched agent.
//!
//! A declared `dispatch.directives` list REPLACES that discovery entirely — it never
//! merges with it. An explicitly empty list means "this project declares no directive
//! sources" and is a legal, meaningful value.
//!
//! # Staying inside the scanned tree
//!
//! Every walk refuses a scan ROOT that is itself a symlink, not merely a symlinked
//! entry found inside one. `std::fs::read_dir` and
//! `Path::is_dir` both follow links, so a `.claude/rules` (or a declared directory
//! entry) pointing out of the tree would otherwise splice arbitrary readable files
//! on the dispatching machine into an agent's prompt — a real exfiltration sink,
//! since the scanned tree may itself be less-trusted code under review.
//!
//! # Bounds
//!
//! Injected text is paid for once per dispatched agent, so the set is bounded by
//! [`MAX_BYTES_PER_SOURCE`] and [`MAX_BYTES_TOTAL`]. An over-bound source is SKIPPED
//! WHOLE with a stated reason rather than truncated: a rule cut mid-sentence can
//! invert its own meaning ("never do X … unless Y"), which is the same failure mode
//! as a paraphrase. Every skip is reported in [`DirectiveSet::skipped`] — never
//! silently dropped.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::markdown::split_frontmatter;

/// Maximum size, in bytes, of a single directive source's post-frontmatter body.
///
/// At CLAUDE.md's measured 2.49 chars/token this is roughly 3.2k tokens for one
/// source. A source over this bound is skipped whole, never truncated.
pub const MAX_BYTES_PER_SOURCE: usize = 8_000;

/// Maximum combined size, in bytes, of every admitted directive body.
///
/// Roughly 6.4k tokens at 2.49 chars/token, paid PER DISPATCHED AGENT. Sources are
/// admitted in discovery order until admitting the next one would exceed this;
/// each remaining source is skipped with the total-budget reason.
pub const MAX_BYTES_TOTAL: usize = 16_000;

/// How deep the recursive directory walks (`.claude/rules/`, `.windsurf/rules/`)
/// descend before stopping.
const MAX_WALK_DEPTH: usize = 8;

/// Which dispatched role a directive is addressed to.
///
/// Set by a `role:` (or `rdm-role:`) key in the source file's YAML frontmatter.
/// Anything absent, unrecognized, or malformed degrades to [`DirectiveRole::Both`] —
/// a project's typo must never break a dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DirectiveRole {
    /// Addressed to the agent that writes the code.
    Implementer,
    /// Addressed to the agents that review the code.
    Reviewer,
    /// Addressed to both (the default).
    Both,
}

impl DirectiveRole {
    /// Parses a frontmatter `role:` value, defaulting to [`DirectiveRole::Both`].
    fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "implementer" | "implement" => DirectiveRole::Implementer,
            "reviewer" | "review" => DirectiveRole::Reviewer,
            _ => DirectiveRole::Both,
        }
    }

    /// The lowercase wire name used in JSON output.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            DirectiveRole::Implementer => "implementer",
            DirectiveRole::Reviewer => "reviewer",
            DirectiveRole::Both => "both",
        }
    }
}

/// One resolved directive: a source path, its addressing, its path scoping, and
/// its VERBATIM post-frontmatter body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Directive {
    /// The source path, relative to the scanned directory, with `/` separators.
    pub path: String,
    /// Which dispatched role this directive is addressed to.
    pub role: DirectiveRole,
    /// Glob patterns from `paths:` / `globs:` frontmatter. Empty means unscoped
    /// (always applicable).
    pub paths: Vec<String>,
    /// The source's post-frontmatter body, byte-for-byte as it was on disk.
    pub text: String,
    /// `text.len()` — UTF-8 bytes, the unit the size bound is expressed in.
    pub bytes: usize,
    /// `text.chars().count()` — Unicode code points, the unit the JS transport
    /// check compares against (JavaScript has no byte length).
    pub chars: usize,
}

/// A source that was found but NOT injected, together with why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedDirective {
    /// The source path, relative to the scanned directory, with `/` separators.
    pub path: String,
    /// The source's size in bytes, or `0` when it could not be read.
    pub bytes: usize,
    /// Why it was not injected, in prose an operator can act on.
    pub reason: String,
}

/// Whether the source list came from a declared config key or from discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DirectiveOrigin {
    /// The `dispatch.directives` key named the sources explicitly.
    Config,
    /// No key was declared; the known locations were scanned.
    Discovery,
}

impl DirectiveOrigin {
    /// The lowercase wire name used in JSON output.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            DirectiveOrigin::Config => "config",
            DirectiveOrigin::Discovery => "discovery",
        }
    }
}

/// The full resolution result: where the list came from, the bound in force, the
/// admitted directives in discovery order, and every source that was skipped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectiveSet {
    /// Whether the sources were declared or discovered.
    pub origin: DirectiveOrigin,
    /// The per-source byte bound in force ([`MAX_BYTES_PER_SOURCE`]).
    pub max_bytes_per_source: usize,
    /// The total byte budget in force ([`MAX_BYTES_TOTAL`]).
    pub max_bytes_total: usize,
    /// Admitted directives, in discovery order.
    pub directives: Vec<Directive>,
    /// Sources found but not injected, with a stated reason each.
    pub skipped: Vec<SkippedDirective>,
}

/// Returns the known directive-source locations under `dir`, in the fixed
/// documented order, capping recursion depth and never traversing a symlink —
/// neither a linked entry inside a location nor a location whose own root is a link.
///
/// Absent locations contribute nothing — a project with none of them yields an
/// empty vector, which is normal and not an error.
#[must_use]
pub fn discover_sources(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    // 1. .claude/rules/**/*.md — recursive.
    collect_recursive(&dir.join(".claude/rules"), "md", &mut out);
    // 2. AGENTS.md — a single file.
    push_if_file(&dir.join("AGENTS.md"), &mut out);
    // 3. .cursor/rules/*.mdc — one directory level, Cursor's own extension.
    collect_flat(&dir.join(".cursor/rules"), "mdc", &mut out);
    // 4. .clinerules — a plain FILE in some projects, a directory of *.md in others.
    let cline = dir.join(".clinerules");
    // is_plain_dir, not Path::is_dir: the latter follows a symlink.
    if is_plain_dir(&cline) {
        collect_flat(&cline, "md", &mut out);
    } else {
        push_if_file(&cline, &mut out);
    }
    // 5. .windsurf/rules/**/*.md — recursive.
    collect_recursive(&dir.join(".windsurf/rules"), "md", &mut out);
    // 6. .github/copilot-instructions.md — a single file.
    push_if_file(&dir.join(".github/copilot-instructions.md"), &mut out);
    out
}

/// Pushes `p` when it is a regular file (never a symlink, directory, or FIFO).
fn push_if_file(p: &Path, out: &mut Vec<PathBuf>) {
    if is_plain_file(p) {
        out.push(p.to_path_buf());
    }
}

/// True only for a regular file reached without traversing a symlink.
fn is_plain_file(p: &Path) -> bool {
    match std::fs::symlink_metadata(p) {
        Ok(md) => md.file_type().is_file(),
        Err(_) => false,
    }
}

/// True only for a real directory — never a symlink that happens to point at one.
///
/// Applied to the SCAN ROOT of every walk, not only to entries discovered inside an
/// already-opened directory. `std::fs::read_dir` and `Path::is_dir` both FOLLOW a
/// symlink, so without this a `.claude/rules` (or `.windsurf/rules`, or a declared
/// directory entry) that is itself a link out of the tree would let a crafted link
/// committed into the scanned repo splice arbitrary readable files on the
/// dispatching machine into an agent's prompt.
fn is_plain_dir(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|md| md.file_type().is_dir())
}

/// Lists `dir`'s entries, sorted lexicographically, refusing a scan root that is
/// itself a symlink.
///
/// Sorting here rather than at each call site is what makes the emitted order
/// independent of filesystem iteration order; returning `None` for a symlinked or
/// unreadable root is what keeps the walk inside the scanned tree.
fn read_plain_dir(dir: &Path) -> Option<Vec<PathBuf>> {
    if !is_plain_dir(dir) {
        return None;
    }
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    paths.sort();
    Some(paths)
}

/// Collects `*.<ext>` files directly inside `dir`, sorted lexicographically.
///
/// A `dir` that is itself a symlink yields nothing — see `read_plain_dir`.
fn collect_flat(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Some(paths) = read_plain_dir(dir) else {
        return;
    };
    out.extend(
        paths
            .into_iter()
            .filter(|p| is_plain_file(p) && has_ext(p, ext)),
    );
}

/// Collects `*.<ext>` files under `dir` recursively, depth-capped and sorted so the
/// emitted order does not depend on filesystem iteration order.
fn collect_recursive(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    collect_recursive_at(dir, ext, 0, out);
}

fn collect_recursive_at(dir: &Path, ext: &str, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_WALK_DEPTH {
        return;
    }
    // read_plain_dir, never a bare read_dir: it refuses a symlinked ROOT, which is
    // the case a per-entry check alone cannot see.
    let Some(paths) = read_plain_dir(dir) else {
        return;
    };
    // Files first at each level, then descend — a stable, documented order.
    for p in &paths {
        if is_plain_file(p) && has_ext(p, ext) {
            out.push(p.clone());
        }
    }
    for p in &paths {
        // A symlinked directory is skipped, so a loop cannot hang the walk and a
        // link out of the repo cannot exfiltrate.
        if is_plain_dir(p) {
            collect_recursive_at(p, ext, depth + 1, out);
        }
    }
}

fn has_ext(p: &Path, ext: &str) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// Renders `p` relative to `dir` with `/` separators, for stable JSON output.
fn rel_display(dir: &Path, p: &Path) -> String {
    let rel = p.strip_prefix(dir).unwrap_or(p);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Pulls the `role:` and `paths:`/`globs:` keys out of a YAML frontmatter block.
///
/// Every failure mode — no frontmatter, invalid YAML, a non-mapping document, an
/// unrecognized role, a non-string/non-sequence `paths` — degrades to the permissive
/// default `(Both, [])` rather than erroring. A project's typo must not break every
/// dispatch.
fn parse_meta(yaml: &str) -> (DirectiveRole, Vec<String>) {
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(yaml) else {
        return (DirectiveRole::Both, Vec::new());
    };
    let Some(map) = value.as_mapping() else {
        return (DirectiveRole::Both, Vec::new());
    };
    let get = |k: &str| -> Option<&serde_yaml::Value> {
        map.get(serde_yaml::Value::String(k.to_string()))
    };
    let role = get("role")
        .or_else(|| get("rdm-role"))
        .and_then(|v| v.as_str())
        .map_or(DirectiveRole::Both, DirectiveRole::parse);
    let scope = get("paths").or_else(|| get("globs"));
    let paths = match scope {
        Some(serde_yaml::Value::String(s)) => s
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        Some(serde_yaml::Value::Sequence(seq)) => seq
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    };
    (role, paths)
}

/// Expands one declared source entry into concrete paths.
///
/// A literal path is used as-is; a directory expands to the `*.md`/`*.mdc` files
/// under it; a trailing `/**` or `/*.ext` suffix expands the same way. A declared
/// entry that matches nothing is returned as-is so the caller can report it in
/// `skipped` — the operator named it explicitly, so its absence is signal.
fn expand_declared(dir: &Path, entry: &str) -> Vec<PathBuf> {
    let trimmed = entry.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let joined = dir.join(trimmed);
    if let Some(idx) = trimmed.find('*') {
        // Split at the last separator before the first wildcard; everything after is
        // an extension filter (`*.md` → "md", bare `*`/`**` → every extension).
        let head = &trimmed[..idx];
        let base = dir.join(head.trim_end_matches('/'));
        let tail = &trimmed[idx..];
        let ext = tail.rsplit_once('.').map(|(_, e)| e.to_string());
        let mut out = Vec::new();
        if tail.contains("**") {
            collect_recursive(&base, ext.as_deref().unwrap_or(""), &mut out);
            if ext.is_none() {
                out.clear();
                collect_any_recursive(&base, &mut out);
            }
        } else if let Some(e) = ext {
            collect_flat(&base, &e, &mut out);
        } else {
            collect_any_flat(&base, &mut out);
        }
        return out;
    }
    // is_plain_dir, not Path::is_dir: a declared entry that is a symlinked directory
    // falls through to the literal-path branch below, where the reader's own
    // is_plain_file gate reports it in `skipped` instead of walking out of the tree.
    if is_plain_dir(&joined) {
        let mut out = Vec::new();
        collect_recursive(&joined, "md", &mut out);
        collect_recursive(&joined, "mdc", &mut out);
        out.sort();
        return out;
    }
    vec![joined]
}

fn collect_any_flat(dir: &Path, out: &mut Vec<PathBuf>) {
    let Some(paths) = read_plain_dir(dir) else {
        return;
    };
    out.extend(paths.into_iter().filter(|p| is_plain_file(p)));
}

fn collect_any_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    let Some(paths) = read_plain_dir(dir) else {
        return;
    };
    for p in &paths {
        if is_plain_file(p) {
            out.push(p.clone());
        }
    }
    for p in &paths {
        if is_plain_dir(p) {
            collect_any_recursive(p, out);
        }
    }
}

/// Resolves the project directives under `dir`.
///
/// `declared` is the `dispatch.directives` config value. `Some(list)` REPLACES
/// discovery entirely (origin [`DirectiveOrigin::Config`]) — including `Some(&[])`,
/// which declares that the project has no directive sources. `None` runs
/// [`discover_sources`] (origin [`DirectiveOrigin::Discovery`]).
///
/// Each source's post-frontmatter body is read VERBATIM — no trimming, re-indenting,
/// or normalization of any kind. Sources over [`MAX_BYTES_PER_SOURCE`] are skipped
/// whole, and admission then stops once [`MAX_BYTES_TOTAL`] would be exceeded; every
/// skip lands in [`DirectiveSet::skipped`] with a reason.
///
/// Finding nothing is normal: a directory with none of the known locations yields an
/// empty set, not an error.
///
/// # Errors
///
/// Never returns `Err` today — an unreadable, non-UTF-8, or missing source is
/// reported in `skipped` rather than failing the resolution. The `Result` is part of
/// the signature so a future hard failure (an unreadable scan root, say) can be
/// surfaced without a breaking change.
pub fn resolve(dir: &Path, declared: Option<&[String]>) -> Result<DirectiveSet> {
    let (origin, sources, declared_entries): (DirectiveOrigin, Vec<PathBuf>, Vec<String>) =
        match declared {
            Some(list) => {
                let mut paths = Vec::new();
                let mut entries = Vec::new();
                for entry in list {
                    let expanded = expand_declared(dir, entry);
                    if expanded.is_empty() {
                        entries.push(entry.trim().to_string());
                    } else {
                        paths.extend(expanded);
                    }
                }
                (DirectiveOrigin::Config, paths, entries)
            }
            None => (
                DirectiveOrigin::Discovery,
                discover_sources(dir),
                Vec::new(),
            ),
        };

    let mut directives: Vec<Directive> = Vec::new();
    let mut skipped: Vec<SkippedDirective> = Vec::new();
    let mut total: usize = 0;

    for src in sources {
        let display = rel_display(dir, &src);
        if !is_plain_file(&src) {
            skipped.push(SkippedDirective {
                path: display,
                bytes: 0,
                reason: "declared source not found".to_string(),
            });
            continue;
        }
        let raw = match std::fs::read(&src) {
            Ok(b) => b,
            Err(e) => {
                skipped.push(SkippedDirective {
                    path: display,
                    bytes: 0,
                    reason: format!("could not be read ({e})"),
                });
                continue;
            }
        };
        let Ok(content) = String::from_utf8(raw) else {
            skipped.push(SkippedDirective {
                path: display,
                bytes: 0,
                reason: "is not valid UTF-8".to_string(),
            });
            continue;
        };
        // An Err from split_frontmatter means the file has none — the WHOLE file is
        // the body, role `both`, unscoped. Never propagated as an error.
        let (role, paths, text) = match split_frontmatter(&content) {
            Ok((yaml, body)) => {
                let (role, paths) = parse_meta(yaml);
                (role, paths, body.to_string())
            }
            Err(_) => (DirectiveRole::Both, Vec::new(), content.clone()),
        };
        let bytes = text.len();
        if bytes > MAX_BYTES_PER_SOURCE {
            skipped.push(SkippedDirective {
                path: display,
                bytes,
                reason: format!("exceeds the per-source byte bound ({MAX_BYTES_PER_SOURCE})"),
            });
            continue;
        }
        // Admission is `<=`: a set landing exactly on the budget is admitted.
        if total + bytes > MAX_BYTES_TOTAL {
            skipped.push(SkippedDirective {
                path: display,
                bytes,
                reason: format!("exceeds the total byte budget ({MAX_BYTES_TOTAL})"),
            });
            continue;
        }
        total += bytes;
        let chars = text.chars().count();
        directives.push(Directive {
            path: display,
            role,
            paths,
            text,
            bytes,
            chars,
        });
    }

    // A declared entry that expanded to nothing is reported, never silently dropped.
    for entry in declared_entries {
        skipped.push(SkippedDirective {
            path: entry,
            bytes: 0,
            reason: "declared source not found".to_string(),
        });
    }

    Ok(DirectiveSet {
        origin,
        max_bytes_per_source: MAX_BYTES_PER_SOURCE,
        max_bytes_total: MAX_BYTES_TOTAL,
        directives,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &Path, rel: &str, body: &str) {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    #[test]
    fn discovers_every_known_source_location() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        write(d, ".claude/rules/testing.md", "claude rule\n");
        write(d, ".claude/rules/nested/deep.md", "nested rule\n");
        write(d, "AGENTS.md", "agents rule\n");
        write(d, ".cursor/rules/style.mdc", "cursor rule\n");
        write(d, ".clinerules", "cline rule\n");
        write(d, ".windsurf/rules/perf.md", "windsurf rule\n");
        write(d, ".github/copilot-instructions.md", "copilot rule\n");

        let set = resolve(d, None).unwrap();
        let paths: Vec<&str> = set.directives.iter().map(|x| x.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                ".claude/rules/testing.md",
                ".claude/rules/nested/deep.md",
                "AGENTS.md",
                ".cursor/rules/style.mdc",
                ".clinerules",
                ".windsurf/rules/perf.md",
                ".github/copilot-instructions.md",
            ],
            "every known location resolves, in the documented order"
        );
        assert_eq!(set.origin, DirectiveOrigin::Discovery);
        assert!(set.skipped.is_empty());
    }

    #[test]
    fn clinerules_as_a_directory_resolves_its_markdown() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        write(d, ".clinerules/b.md", "second\n");
        write(d, ".clinerules/a.md", "first\n");
        let set = resolve(d, None).unwrap();
        let paths: Vec<&str> = set.directives.iter().map(|x| x.path.as_str()).collect();
        assert_eq!(paths, vec![".clinerules/a.md", ".clinerules/b.md"]);
    }

    #[test]
    fn claude_md_is_never_a_directive_source() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        write(d, "CLAUDE.md", "already loaded into every subagent\n");
        let set = resolve(d, None).unwrap();
        assert!(
            set.directives.is_empty(),
            "CLAUDE.md must not be discovered — it is already loaded into every subagent"
        );
    }

    #[test]
    fn absent_sources_are_normal_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let set = resolve(tmp.path(), None).unwrap();
        assert_eq!(set.origin, DirectiveOrigin::Discovery);
        assert!(set.directives.is_empty());
        assert!(set.skipped.is_empty());
    }

    #[test]
    fn role_and_paths_parse_from_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        write(
            d,
            ".claude/rules/scoped.md",
            "---\nrole: reviewer\npaths:\n  - \"rdm-core/**/*.rs\"\n---\n\nbody text\n",
        );
        write(
            d,
            ".claude/rules/cursorish.md",
            "---\nrole: implementer\nglobs: \"a/*.rs, b/*.rs\"\n---\n\nother\n",
        );
        let set = resolve(d, None).unwrap();
        assert_eq!(set.directives[0].role, DirectiveRole::Implementer);
        assert_eq!(set.directives[0].paths, vec!["a/*.rs", "b/*.rs"]);
        assert_eq!(set.directives[1].role, DirectiveRole::Reviewer);
        assert_eq!(set.directives[1].paths, vec!["rdm-core/**/*.rs"]);
    }

    #[test]
    fn malformed_frontmatter_degrades_to_the_permissive_default() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        write(
            d,
            ".claude/rules/bad.md",
            "---\nrole: maintainer\npaths: 42\n---\n\nstill injected\n",
        );
        write(
            d,
            ".claude/rules/broken.md",
            "---\n: : :\nnot yaml\n---\n\nb\n",
        );
        let set = resolve(d, None).unwrap();
        for dir in &set.directives {
            assert_eq!(
                dir.role,
                DirectiveRole::Both,
                "{} degrades to both",
                dir.path
            );
            assert!(dir.paths.is_empty(), "{} degrades to unscoped", dir.path);
        }
        assert_eq!(set.directives.len(), 2);
    }

    #[test]
    fn a_file_without_frontmatter_is_read_whole() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let body = "# Agents\n\nRun the fuzzers.\n";
        write(d, "AGENTS.md", body);
        let set = resolve(d, None).unwrap();
        assert_eq!(set.directives[0].text, body);
        assert_eq!(set.directives[0].role, DirectiveRole::Both);
    }

    #[test]
    fn directive_text_is_byte_identical() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        // Trailing whitespace, a tab-indented line, a curly quote, an em-dash, and a
        // fenced code block — none of it may be normalized.
        const EXPECTED: &str = "Line with trailing space   \n\tTab-indented line\nA \u{201c}curly\u{201d} quote \u{2014} and an em-dash.\n\n```sh\n  cargo mutants   --check\n```\n";
        write(
            d,
            ".claude/rules/exact.md",
            &format!("---\nrole: both\n---\n\n{EXPECTED}"),
        );
        let set = resolve(d, None).unwrap();
        assert_eq!(set.directives[0].text, EXPECTED);
        assert_eq!(set.directives[0].bytes, EXPECTED.len());
        assert_eq!(set.directives[0].chars, EXPECTED.chars().count());
        assert_ne!(
            set.directives[0].bytes, set.directives[0].chars,
            "the non-ASCII fixture must prove bytes and chars are distinct numbers"
        );
    }

    #[test]
    fn declared_sources_replace_discovery() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        write(d, ".claude/rules/discovered.md", "discovered body\n");
        write(d, "docs/rules/only.md", "declared body\n");
        let declared = vec!["docs/rules/only.md".to_string()];
        let set = resolve(d, Some(&declared)).unwrap();
        assert_eq!(set.origin, DirectiveOrigin::Config);
        assert_eq!(set.directives.len(), 1);
        assert_eq!(set.directives[0].path, "docs/rules/only.md");
        // The load-bearing NEGATIVE: replace, never merge. A discovered path must
        // appear in NEITHER output array when the key is declared.
        assert!(
            !set.directives
                .iter()
                .any(|x| x.path.contains("discovered.md")),
            "a declared list must REPLACE discovery, not add to it"
        );
        assert!(
            !set.skipped.iter().any(|x| x.path.contains("discovered.md")),
            "a discovered path must not even appear in skipped when the key is declared"
        );
    }

    #[test]
    fn an_explicitly_empty_declaration_yields_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        write(d, ".claude/rules/discovered.md", "discovered body\n");
        let set = resolve(d, Some(&[])).unwrap();
        assert_eq!(set.origin, DirectiveOrigin::Config);
        assert!(set.directives.is_empty());
        assert!(set.skipped.is_empty());
    }

    #[test]
    fn a_declared_but_missing_path_is_reported_not_dropped() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let declared = vec!["docs/rules/absent.md".to_string()];
        let set = resolve(d, Some(&declared)).unwrap();
        assert!(set.directives.is_empty());
        assert_eq!(set.skipped.len(), 1);
        assert_eq!(set.skipped[0].path, "docs/rules/absent.md");
        assert_eq!(set.skipped[0].reason, "declared source not found");
    }

    #[test]
    fn oversize_source_is_skipped_not_truncated() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let big = "X".repeat(9_000);
        write(d, ".claude/rules/big.md", &big);
        write(d, ".claude/rules/small.md", "small body\n");
        let set = resolve(d, None).unwrap();
        let paths: Vec<&str> = set.directives.iter().map(|x| x.path.as_str()).collect();
        assert_eq!(paths, vec![".claude/rules/small.md"]);
        assert_eq!(set.skipped.len(), 1);
        assert_eq!(set.skipped[0].path, ".claude/rules/big.md");
        assert!(
            set.skipped[0].reason.contains("8000"),
            "the reason names the cap: {}",
            set.skipped[0].reason
        );
        // No truncated remnant anywhere in the admitted output.
        let joined: String = set.directives.iter().map(|x| x.text.clone()).collect();
        assert!(
            !joined.contains("XXXX"),
            "an over-bound source must be skipped WHOLE, never truncated"
        );
    }

    #[test]
    fn total_budget_admits_in_order_then_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        for name in ["a", "b", "c", "d"] {
            write(d, &format!(".claude/rules/{name}.md"), &"Y".repeat(5_000));
        }
        let set = resolve(d, None).unwrap();
        let paths: Vec<&str> = set.directives.iter().map(|x| x.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                ".claude/rules/a.md",
                ".claude/rules/b.md",
                ".claude/rules/c.md"
            ],
            "sources are admitted in discovery order until the budget would be exceeded"
        );
        assert_eq!(set.skipped.len(), 1);
        assert_eq!(set.skipped[0].path, ".claude/rules/d.md");
        assert!(set.skipped[0].reason.contains("16000"));
    }

    #[test]
    fn exactly_at_the_total_budget_is_admitted() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        write(d, ".claude/rules/a.md", &"Z".repeat(8_000));
        write(d, ".claude/rules/b.md", &"Z".repeat(8_000));
        let set = resolve(d, None).unwrap();
        assert_eq!(
            set.directives.len(),
            2,
            "admission is <=, so a set landing exactly on the budget is admitted"
        );
        assert!(set.skipped.is_empty());
        let total: usize = set.directives.iter().map(|x| x.bytes).sum();
        assert_eq!(total, MAX_BYTES_TOTAL);
    }

    #[test]
    fn an_unreadable_source_is_reported_not_fatal() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        // Invalid UTF-8 in a discovered location.
        let p = d.join(".claude/rules/bad.md");
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, [0xff, 0xfe, 0xfd]).unwrap();
        write(d, ".claude/rules/good.md", "good\n");
        let set = resolve(d, None).unwrap();
        assert_eq!(set.directives.len(), 1);
        assert_eq!(set.directives[0].path, ".claude/rules/good.md");
        assert_eq!(set.skipped.len(), 1);
        assert!(set.skipped[0].reason.contains("UTF-8"));
    }

    #[test]
    fn a_symlinked_source_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        write(d, "outside.md", "outside body\n");
        fs::create_dir_all(d.join(".claude/rules")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(d.join("outside.md"), d.join(".claude/rules/link.md")).unwrap();
        let set = resolve(d, None).unwrap();
        #[cfg(unix)]
        assert!(
            set.directives.is_empty(),
            "a symlink must never be followed out of the scanned tree"
        );
        let _ = set;
    }

    /// The scan ROOT case the per-entry check above cannot see.
    ///
    /// `read_dir` follows a symlink, so a `.claude/rules` that is ITSELF a link out
    /// of the tree used to admit whatever it pointed at. A dispatched agent's prompt
    /// is an exfiltration sink, and the scanned tree may be less-trusted code under
    /// review, so this is the load-bearing direction.
    #[test]
    #[cfg(unix)]
    fn a_symlinked_scan_root_is_never_followed() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let outside = tmp.path().join("outside-tree");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("leak.md"), "SECRET CONTENTS\n").unwrap();

        let project = d.join("project");
        fs::create_dir_all(project.join(".claude")).unwrap();
        std::os::unix::fs::symlink(&outside, project.join(".claude/rules")).unwrap();
        // ...and the recursive sibling location, which shares the same walker.
        fs::create_dir_all(project.join(".windsurf")).unwrap();
        std::os::unix::fs::symlink(&outside, project.join(".windsurf/rules")).unwrap();
        // ...and the two flat ones.
        fs::create_dir_all(project.join(".cursor")).unwrap();
        std::os::unix::fs::symlink(&outside, project.join(".cursor/rules")).unwrap();
        std::os::unix::fs::symlink(&outside, project.join(".clinerules")).unwrap();

        let set = resolve(&project, None).unwrap();
        assert!(
            set.directives.is_empty(),
            "a symlinked scan root must yield nothing, got {:?}",
            set.directives.iter().map(|x| &x.path).collect::<Vec<_>>()
        );
        let blob = serde_json::to_string(&set).unwrap();
        assert!(
            !blob.contains("SECRET CONTENTS"),
            "the out-of-tree file's contents must not appear anywhere in the resolved set"
        );
    }

    /// The same escape through the operator-declared list rather than discovery.
    #[test]
    #[cfg(unix)]
    fn a_declared_symlinked_directory_is_never_followed() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().join("outside-tree");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("leak.md"), "SECRET CONTENTS\n").unwrap();

        let project = tmp.path().join("project");
        fs::create_dir_all(project.join("docs")).unwrap();
        std::os::unix::fs::symlink(&outside, project.join("docs/rules")).unwrap();

        let declared = vec!["docs/rules".to_string()];
        let set = resolve(&project, Some(&declared)).unwrap();
        assert_eq!(set.origin, DirectiveOrigin::Config);
        assert!(
            set.directives.is_empty(),
            "a declared directory entry that is a symlink must not be walked"
        );
        // Never silently dropped: the operator named it, so its non-admission is signal.
        assert_eq!(set.skipped.len(), 1);
        assert_eq!(set.skipped[0].reason, "declared source not found");
        let blob = serde_json::to_string(&set).unwrap();
        assert!(!blob.contains("SECRET CONTENTS"));
    }

    /// A symlinked FILE named directly by the declared list is refused the same way.
    #[test]
    #[cfg(unix)]
    fn a_declared_symlinked_file_is_never_read() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().join("outside-tree");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("leak.md"), "SECRET CONTENTS\n").unwrap();

        let project = tmp.path().join("project");
        fs::create_dir_all(project.join("docs")).unwrap();
        std::os::unix::fs::symlink(outside.join("leak.md"), project.join("docs/only.md")).unwrap();

        let declared = vec!["docs/only.md".to_string()];
        let set = resolve(&project, Some(&declared)).unwrap();
        assert!(set.directives.is_empty());
        assert_eq!(set.skipped.len(), 1);
        let blob = serde_json::to_string(&set).unwrap();
        assert!(!blob.contains("SECRET CONTENTS"));
    }

    /// A real directory nested under a real scan root still resolves — the root
    /// guard must not have turned the recursive walk into a no-op.
    #[test]
    fn a_real_nested_directory_still_resolves() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        write(d, ".claude/rules/nested/deep.md", "deep body\n");
        let set = resolve(d, None).unwrap();
        assert_eq!(
            set.directives
                .iter()
                .map(|x| x.path.as_str())
                .collect::<Vec<_>>(),
            vec![".claude/rules/nested/deep.md"]
        );
    }
}
