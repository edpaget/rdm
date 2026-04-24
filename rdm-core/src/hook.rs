//! Git hook helpers for parsing `Done:` directives from commit messages.

/// A parsed `Done:` directive from a commit message.
///
/// Supports two forms:
/// - `Done: <roadmap>/<phase>` — marks a roadmap phase as done
/// - `Done: task/<slug>` — marks a standalone task as done
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DoneDirective {
    /// A phase completion directive.
    Phase {
        /// The roadmap slug.
        roadmap: String,
        /// The phase stem or number.
        phase: String,
    },
    /// A task completion directive.
    Task {
        /// The task slug.
        slug: String,
    },
}

/// Parses `Done:` directives from a commit message.
///
/// Iterates lines, matches case-insensitive `^Done:` prefix, splits the value
/// on the first `/`, trims whitespace, and skips malformed lines (no `/`,
/// empty parts).
///
/// When the left side of the `/` is `task` (case-insensitive), emits a
/// [`DoneDirective::Task`]; otherwise emits a [`DoneDirective::Phase`].
///
/// # Examples
///
/// ```
/// use rdm_core::hook::{parse_done_directives, DoneDirective};
///
/// let msg = "feat: implement search\n\nDone: search-feature/phase-2-indexing\n";
/// let directives = parse_done_directives(msg);
/// assert_eq!(directives.len(), 1);
/// assert_eq!(directives[0], DoneDirective::Phase {
///     roadmap: "search-feature".to_string(),
///     phase: "phase-2-indexing".to_string(),
/// });
/// ```
pub fn parse_done_directives(message: &str) -> Vec<DoneDirective> {
    let mut directives = Vec::new();
    for line in message.lines() {
        let trimmed = line.trim();
        // Byte-level prefix check — safe for lines starting with multi-byte
        // UTF-8 chars. "done:" is pure ASCII, so if the first 5 bytes match,
        // byte 5 is guaranteed to be a char boundary.
        let bytes = trimmed.as_bytes();
        if bytes.len() < 5 || !bytes[..5].eq_ignore_ascii_case(b"done:") {
            continue;
        }
        let value = trimmed[5..].trim();
        let Some((left, right)) = value.split_once('/') else {
            continue;
        };
        let left = left.trim();
        let right = right.trim();
        if left.is_empty() || right.is_empty() {
            continue;
        }
        if left.eq_ignore_ascii_case("task") {
            directives.push(DoneDirective::Task {
                slug: right.to_string(),
            });
        } else {
            directives.push(DoneDirective::Phase {
                roadmap: left.to_string(),
                phase: right.to_string(),
            });
        }
    }
    directives
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_message() {
        assert!(parse_done_directives("").is_empty());
    }

    #[test]
    fn single_valid_phase_directive() {
        let directives = parse_done_directives("Done: search-feature/phase-2-indexing");
        assert_eq!(
            directives,
            vec![DoneDirective::Phase {
                roadmap: "search-feature".to_string(),
                phase: "phase-2-indexing".to_string(),
            }]
        );
    }

    #[test]
    fn single_valid_task_directive() {
        let directives = parse_done_directives("Done: task/fix-bug");
        assert_eq!(
            directives,
            vec![DoneDirective::Task {
                slug: "fix-bug".to_string(),
            }]
        );
    }

    #[test]
    fn task_directive_case_insensitive_prefix() {
        for task_word in ["task", "Task", "TASK", "tAsK"] {
            let msg = format!("Done: {task_word}/my-slug");
            let directives = parse_done_directives(&msg);
            assert_eq!(directives.len(), 1, "failed for: {task_word}");
            assert_eq!(
                directives[0],
                DoneDirective::Task {
                    slug: "my-slug".to_string(),
                }
            );
        }
    }

    #[test]
    fn case_insensitive_done_prefix() {
        for prefix in ["done:", "DONE:", "DoNe:", "dOnE:"] {
            let msg = format!("{prefix} my-roadmap/my-phase");
            let directives = parse_done_directives(&msg);
            assert_eq!(directives.len(), 1, "failed for prefix: {prefix}");
            assert_eq!(
                directives[0],
                DoneDirective::Phase {
                    roadmap: "my-roadmap".to_string(),
                    phase: "my-phase".to_string(),
                }
            );
        }
    }

    #[test]
    fn mixed_phase_and_task_directives() {
        let msg =
            "feat: big merge\n\nDone: search/phase-1\nDone: task/fix-bug\nDone: perf/phase-2\n";
        let directives = parse_done_directives(msg);
        assert_eq!(directives.len(), 3);
        assert_eq!(
            directives[0],
            DoneDirective::Phase {
                roadmap: "search".to_string(),
                phase: "phase-1".to_string(),
            }
        );
        assert_eq!(
            directives[1],
            DoneDirective::Task {
                slug: "fix-bug".to_string(),
            }
        );
        assert_eq!(
            directives[2],
            DoneDirective::Phase {
                roadmap: "perf".to_string(),
                phase: "phase-2".to_string(),
            }
        );
    }

    #[test]
    fn skips_non_done_lines() {
        let msg = "feat: something\nNot a done line\nDone: r/p\nAnother line";
        let directives = parse_done_directives(msg);
        assert_eq!(directives.len(), 1);
        assert_eq!(
            directives[0],
            DoneDirective::Phase {
                roadmap: "r".to_string(),
                phase: "p".to_string(),
            }
        );
    }

    #[test]
    fn skips_lines_with_multibyte_chars_before_byte_five() {
        // Regression: previously panicked because `trimmed[..5]` sliced
        // into the middle of a multi-byte UTF-8 character.
        let msg = "map — both `lambda` AND `λ` parsed as the lambda special form\nDone: r/p\n";
        let directives = parse_done_directives(msg);
        assert_eq!(directives.len(), 1);
        assert_eq!(
            directives[0],
            DoneDirective::Phase {
                roadmap: "r".to_string(),
                phase: "p".to_string(),
            }
        );
    }

    #[test]
    fn skips_short_multibyte_lines() {
        // A line shorter than 5 bytes worth of ASCII but with multi-byte chars.
        assert!(parse_done_directives("λλ").is_empty());
        assert!(parse_done_directives("— hi").is_empty());
    }

    #[test]
    fn skips_malformed_no_slash() {
        let directives = parse_done_directives("Done: no-slash-here");
        assert!(directives.is_empty());
    }

    #[test]
    fn skips_malformed_empty_parts() {
        assert!(parse_done_directives("Done: /phase").is_empty());
        assert!(parse_done_directives("Done: roadmap/").is_empty());
        assert!(parse_done_directives("Done: /").is_empty());
        assert!(parse_done_directives("Done: task/").is_empty());
    }

    #[test]
    fn trims_whitespace() {
        let directives = parse_done_directives("Done:   my-roadmap  /  my-phase  ");
        assert_eq!(directives.len(), 1);
        assert_eq!(
            directives[0],
            DoneDirective::Phase {
                roadmap: "my-roadmap".to_string(),
                phase: "my-phase".to_string(),
            }
        );
    }

    #[test]
    fn trims_whitespace_task() {
        let directives = parse_done_directives("Done:   task  /  fix-bug  ");
        assert_eq!(directives.len(), 1);
        assert_eq!(
            directives[0],
            DoneDirective::Task {
                slug: "fix-bug".to_string(),
            }
        );
    }
}
