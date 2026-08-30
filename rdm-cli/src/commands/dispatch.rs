//! `rdm dispatch` — inspect what the autonomous dispatch lane resolves for a project.
//!
//! Read-only: nothing here writes to the plan repo, the source repo, or git.

use std::path::Path;

use anyhow::{Context, Result};
use rdm_core::directives::{DirectiveRole, DirectiveSet};

use crate::OutputFormat;
use crate::cli::{DirectiveRoleArg, DispatchCommand};

/// Runs a `rdm dispatch` subcommand.
///
/// `declared` is the plan repo's `dispatch.directives` value, or `None` when no
/// plan repo was reachable or the key is unset — in which case rdm falls back to
/// discovering the known agent-instruction locations rather than erroring, so a
/// downstream repo with no plan repo still gets the feature.
///
/// # Errors
///
/// Returns an error if the scanned directory cannot be resolved, does not exist,
/// or is not a directory, or if the output cannot be serialized. An absent,
/// unreadable, or over-bound source *inside* a valid scan root is reported in the
/// payload's `skipped` array, never as a failure.
pub fn run(
    command: DispatchCommand,
    declared: Option<Vec<String>>,
    format: OutputFormat,
) -> Result<()> {
    match command {
        DispatchCommand::Directives {
            dir,
            role,
            format: sub_format,
        } => {
            let dir = match dir {
                Some(d) => d,
                None => {
                    std::env::current_dir().context("failed to resolve the current directory")?
                }
            };
            validate_scan_root(&dir)?;
            let set = rdm_core::directives::resolve(&dir, declared.as_deref())
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let set = filter_role(set, role);
            // The subcommand's own --format wins when given; the global --format is
            // the fallback so `rdm --format json dispatch directives` also works.
            let chosen = if sub_format == OutputFormat::Human {
                format
            } else {
                sub_format
            };
            match chosen {
                OutputFormat::Json => print_json(&set)?,
                _ => print_text(&set),
            }
            Ok(())
        }
    }
}

/// Rejects a scan root that does not exist or is not a directory.
///
/// An absent *source location* inside a valid root is normal and is silently
/// nothing (a project simply has no `.claude/rules/`). An absent scan ROOT is
/// not: it is a typo'd `--dir`, or a caller that handed over a file. Without
/// this check both render byte-for-byte identically to a healthy project with no
/// directives, so the one command an operator runs to see what would be injected
/// would answer "nothing" to a question it never actually asked.
///
/// # Errors
///
/// Returns an error naming the path and what to do about it when `dir` does not
/// exist or is not a directory.
fn validate_scan_root(dir: &Path) -> Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    let what = if dir.exists() {
        "is not a directory"
    } else {
        "does not exist"
    };
    anyhow::bail!(
        "cannot scan {} for project directives — that path {what}. \
         Pass --dir <path> naming the root of the source repo to scan, \
         or omit it to scan the current directory.",
        dir.display()
    )
}

/// Drops directives not addressed to `role`. `both` always survives, and no filter
/// keeps everything.
fn filter_role(mut set: DirectiveSet, role: Option<DirectiveRoleArg>) -> DirectiveSet {
    let Some(want) = role else { return set };
    let want = match want {
        DirectiveRoleArg::Implementer => DirectiveRole::Implementer,
        DirectiveRoleArg::Reviewer => DirectiveRole::Reviewer,
    };
    set.directives
        .retain(|d| d.role == want || d.role == DirectiveRole::Both);
    set
}

/// Emits the payload the workflow lane consumes.
///
/// `budget` echoes the bound so it is single-sourced in Rust — the JS helpers and
/// the harness render what `skipped` reports and never restate the numbers.
fn print_json(set: &DirectiveSet) -> Result<()> {
    let payload = serde_json::json!({
        "origin": set.origin.as_str(),
        "budget": {
            "maxBytesPerSource": set.max_bytes_per_source,
            "maxBytesTotal": set.max_bytes_total,
        },
        "directives": set
            .directives
            .iter()
            .map(|d| serde_json::json!({
                "path": d.path,
                "role": d.role.as_str(),
                "paths": d.paths,
                "text": d.text,
                "chars": d.chars,
                "bytes": d.bytes,
            }))
            .collect::<Vec<_>>(),
        "skipped": set
            .skipped
            .iter()
            .map(|s| serde_json::json!({
                "path": s.path,
                "bytes": s.bytes,
                "reason": s.reason,
            }))
            .collect::<Vec<_>>(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&payload).context("failed to serialize directives")?
    );
    Ok(())
}

fn print_text(set: &DirectiveSet) {
    print!("{}", render_text(set));
}

/// Renders the human-readable summary.
///
/// Split out from [`print_text`] so it is assertable without capturing stdout — an
/// empty `directives` or `skipped` list is a normal outcome (a project with no
/// directive sources), so the empty-list branches have to be exercised.
fn render_text(set: &DirectiveSet) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "origin: {}", set.origin.as_str());
    let _ = writeln!(
        out,
        "budget: {} bytes per source, {} bytes total",
        set.max_bytes_per_source, set.max_bytes_total
    );
    if set.directives.is_empty() {
        let _ = writeln!(out, "directives: (none)");
    } else {
        let _ = writeln!(out, "directives:");
        for d in &set.directives {
            let scope = if d.paths.is_empty() {
                "unscoped".to_string()
            } else {
                d.paths.join(", ")
            };
            let _ = writeln!(
                out,
                "  {} [role: {}] [paths: {}] ({} bytes)",
                d.path,
                d.role.as_str(),
                scope,
                d.bytes
            );
        }
    }
    if !set.skipped.is_empty() {
        let _ = writeln!(out, "skipped:");
        for s in &set.skipped {
            let _ = writeln!(out, "  {} — {}", s.path, s.reason);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdm_core::directives::{
        Directive, DirectiveOrigin, MAX_BYTES_PER_SOURCE, MAX_BYTES_TOTAL, SkippedDirective,
    };

    fn directive(path: &str, role: DirectiveRole, paths: &[&str]) -> Directive {
        let text = format!("body of {path}\n");
        Directive {
            path: path.to_string(),
            role,
            paths: paths.iter().map(|s| (*s).to_string()).collect(),
            bytes: text.len(),
            chars: text.chars().count(),
            text,
        }
    }

    fn set_of(directives: Vec<Directive>, skipped: Vec<SkippedDirective>) -> DirectiveSet {
        DirectiveSet {
            origin: DirectiveOrigin::Discovery,
            max_bytes_per_source: MAX_BYTES_PER_SOURCE,
            max_bytes_total: MAX_BYTES_TOTAL,
            directives,
            skipped,
        }
    }

    fn paths_of(set: &DirectiveSet) -> Vec<&str> {
        set.directives.iter().map(|d| d.path.as_str()).collect()
    }

    fn seeded() -> DirectiveSet {
        set_of(
            vec![
                directive("impl.md", DirectiveRole::Implementer, &[]),
                directive("rev.md", DirectiveRole::Reviewer, &[]),
                directive("both.md", DirectiveRole::Both, &["rdm-core/**/*.rs"]),
            ],
            vec![],
        )
    }

    #[test]
    fn a_missing_scan_root_is_an_actionable_error_not_an_empty_result() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let missing = tmp.path().join("no-such-dir");
        let err = validate_scan_root(&missing).expect_err(
            "a --dir that does not exist must fail loudly — an empty result is \
             indistinguishable from a healthy project with no directives",
        );
        let msg = format!("{err:#}");
        assert!(msg.contains("does not exist"), "{msg}");
        assert!(
            msg.contains(&missing.display().to_string()),
            "the error must name the offending path: {msg}"
        );
        assert!(
            msg.contains("--dir"),
            "the error must say what the reader can do about it: {msg}"
        );
    }

    #[test]
    fn a_scan_root_that_is_a_file_is_rejected_with_its_own_reason() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let file = tmp.path().join("AGENTS.md");
        std::fs::write(&file, "not a directory\n").expect("write");
        let err = validate_scan_root(&file).expect_err("a file is not a scan root");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("is not a directory"),
            "a path that exists but is the wrong kind gets its own reason: {msg}"
        );
    }

    #[test]
    fn a_real_directory_is_accepted_even_with_no_sources_in_it() {
        let tmp = tempfile::tempdir().expect("tempdir");
        validate_scan_root(tmp.path())
            .expect("an existing directory with no directive sources is a normal, valid scan root");
    }

    #[test]
    fn no_role_filter_keeps_every_directive() {
        let out = filter_role(seeded(), None);
        assert_eq!(paths_of(&out), vec!["impl.md", "rev.md", "both.md"]);
    }

    #[test]
    fn implementer_filter_drops_reviewer_only_and_keeps_both() {
        let out = filter_role(seeded(), Some(DirectiveRoleArg::Implementer));
        assert_eq!(
            paths_of(&out),
            vec!["impl.md", "both.md"],
            "a `both`-role directive is addressed to the implementer too"
        );
    }

    #[test]
    fn reviewer_filter_drops_implementer_only_and_keeps_both() {
        let out = filter_role(seeded(), Some(DirectiveRoleArg::Reviewer));
        assert_eq!(
            paths_of(&out),
            vec!["rev.md", "both.md"],
            "a `both`-role directive is addressed to the reviewer too"
        );
    }

    #[test]
    fn the_role_filter_leaves_skipped_and_the_budget_alone() {
        let set = set_of(
            vec![directive("impl.md", DirectiveRole::Implementer, &[])],
            vec![SkippedDirective {
                path: "huge.md".to_string(),
                bytes: 9_000,
                reason: "exceeds the per-source byte bound (8000)".to_string(),
            }],
        );
        let out = filter_role(set, Some(DirectiveRoleArg::Reviewer));
        assert!(out.directives.is_empty());
        assert_eq!(
            out.skipped.len(),
            1,
            "a withheld source stays reported whatever role is asked for"
        );
        assert_eq!(out.max_bytes_total, MAX_BYTES_TOTAL);
    }

    #[test]
    fn text_output_names_every_directive_with_its_role_and_scope() {
        let rendered = render_text(&seeded());
        assert!(rendered.contains("origin: discovery"));
        assert!(rendered.contains("budget: 8000 bytes per source, 16000 bytes total"));
        assert!(rendered.contains("impl.md [role: implementer] [paths: unscoped]"));
        assert!(rendered.contains("both.md [role: both] [paths: rdm-core/**/*.rs]"));
        assert!(
            !rendered.contains("skipped:"),
            "with nothing skipped the section is omitted entirely"
        );
    }

    #[test]
    fn text_output_handles_an_empty_set_and_reports_skips() {
        let empty = render_text(&set_of(vec![], vec![]));
        assert!(
            empty.contains("directives: (none)"),
            "no sources is a normal outcome, not an error: {empty}"
        );
        let with_skip = render_text(&set_of(
            vec![],
            vec![SkippedDirective {
                path: "huge.md".to_string(),
                bytes: 9_000,
                reason: "exceeds the per-source byte bound (8000)".to_string(),
            }],
        ));
        assert!(with_skip.contains("skipped:"));
        assert!(with_skip.contains("huge.md — exceeds the per-source byte bound (8000)"));
    }
}
