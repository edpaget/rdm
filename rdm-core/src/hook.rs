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

/// Errors returned when formatting a `Done:` directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatDoneError {
    /// A required identifier was empty or whitespace-only.
    EmptyIdentifier {
        /// Which identifier was empty (`roadmap`, `phase`, or `task`).
        field: &'static str,
    },
    /// An identifier contained a `/`, which would produce an ambiguous
    /// directive that [`parse_done_directives`] cannot round-trip.
    ContainsSlash {
        /// Which identifier contained the `/`.
        field: &'static str,
    },
    /// `task` was used as a roadmap slug. `task` is a reserved prefix — a
    /// `Done: task/<x>` line always parses as a task directive.
    ReservedRoadmapSlug,
}

impl std::fmt::Display for FormatDoneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyIdentifier { field } => {
                write!(f, "{field} must not be empty")
            }
            Self::ContainsSlash { field } => {
                write!(
                    f,
                    "{field} must not contain '/' — a Done: directive splits on the first '/'"
                )
            }
            Self::ReservedRoadmapSlug => write!(
                f,
                "'task' is a reserved prefix and cannot be used as a roadmap slug"
            ),
        }
    }
}

impl std::error::Error for FormatDoneError {}

/// Formats a [`DoneDirective`] as the commit-message trailer line.
///
/// This is the single home of the `Done:` format string. Every surface that
/// writes the trailer — the `rdm-review` skill's gate step, `rdm-land`'s
/// land-time synthesis — goes through here (via `rdm hook done-line`) rather
/// than hand-typing the format.
///
/// The output round-trips: `parse_done_directives(&format_done_directive(d)?)`
/// yields `[d]`.
///
/// # Errors
///
/// - [`FormatDoneError::EmptyIdentifier`] if any identifier is empty or
///   whitespace-only.
/// - [`FormatDoneError::ContainsSlash`] if any identifier contains `/`, which
///   would produce a directive that does not round-trip.
/// - [`FormatDoneError::ReservedRoadmapSlug`] if the roadmap slug is `task`
///   (case-insensitive), which would parse back as a task directive.
///
/// # Examples
///
/// ```
/// use rdm_core::hook::{format_done_directive, parse_done_directives, DoneDirective};
///
/// let directive = DoneDirective::Phase {
///     roadmap: "search-feature".to_string(),
///     phase: "phase-2-indexing".to_string(),
/// };
/// let line = format_done_directive(&directive).unwrap();
/// assert_eq!(line, "Done: search-feature/phase-2-indexing");
/// assert_eq!(parse_done_directives(&line), vec![directive]);
/// ```
pub fn format_done_directive(directive: &DoneDirective) -> Result<String, FormatDoneError> {
    fn check(value: &str, field: &'static str) -> Result<(), FormatDoneError> {
        if value.trim().is_empty() {
            return Err(FormatDoneError::EmptyIdentifier { field });
        }
        if value.contains('/') {
            return Err(FormatDoneError::ContainsSlash { field });
        }
        Ok(())
    }

    match directive {
        DoneDirective::Phase { roadmap, phase } => {
            check(roadmap, "roadmap")?;
            check(phase, "phase")?;
            if roadmap.trim().eq_ignore_ascii_case("task") {
                return Err(FormatDoneError::ReservedRoadmapSlug);
            }
            Ok(format!("Done: {}/{}", roadmap.trim(), phase.trim()))
        }
        DoneDirective::Task { slug } => {
            check(slug, "task")?;
            Ok(format!("Done: task/{}", slug.trim()))
        }
    }
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

    #[test]
    fn format_phase_directive() {
        let directive = DoneDirective::Phase {
            roadmap: "search-feature".to_string(),
            phase: "phase-2-indexing".to_string(),
        };
        assert_eq!(
            format_done_directive(&directive).unwrap(),
            "Done: search-feature/phase-2-indexing"
        );
    }

    #[test]
    fn format_task_directive() {
        let directive = DoneDirective::Task {
            slug: "fix-bug".to_string(),
        };
        assert_eq!(
            format_done_directive(&directive).unwrap(),
            "Done: task/fix-bug"
        );
    }

    #[test]
    fn format_round_trips_through_parse() {
        let directives = vec![
            DoneDirective::Phase {
                roadmap: "unify-code-review".to_string(),
                phase: "phase-4-canonical".to_string(),
            },
            DoneDirective::Task {
                slug: "done-trailer".to_string(),
            },
        ];
        for directive in directives {
            let line = format_done_directive(&directive).unwrap();
            assert_eq!(parse_done_directives(&line), vec![directive]);
        }
    }

    #[test]
    fn format_trims_surrounding_whitespace() {
        let directive = DoneDirective::Phase {
            roadmap: "  r  ".to_string(),
            phase: "  p  ".to_string(),
        };
        assert_eq!(format_done_directive(&directive).unwrap(), "Done: r/p");
    }

    #[test]
    fn format_rejects_empty_identifiers() {
        assert_eq!(
            format_done_directive(&DoneDirective::Phase {
                roadmap: "  ".to_string(),
                phase: "p".to_string(),
            }),
            Err(FormatDoneError::EmptyIdentifier { field: "roadmap" })
        );
        assert_eq!(
            format_done_directive(&DoneDirective::Phase {
                roadmap: "r".to_string(),
                phase: String::new(),
            }),
            Err(FormatDoneError::EmptyIdentifier { field: "phase" })
        );
        assert_eq!(
            format_done_directive(&DoneDirective::Task {
                slug: String::new()
            }),
            Err(FormatDoneError::EmptyIdentifier { field: "task" })
        );
    }

    #[test]
    fn format_rejects_embedded_slash() {
        assert_eq!(
            format_done_directive(&DoneDirective::Phase {
                roadmap: "a/b".to_string(),
                phase: "p".to_string(),
            }),
            Err(FormatDoneError::ContainsSlash { field: "roadmap" })
        );
        assert_eq!(
            format_done_directive(&DoneDirective::Task {
                slug: "a/b".to_string(),
            }),
            Err(FormatDoneError::ContainsSlash { field: "task" })
        );
    }

    #[test]
    fn format_rejects_task_as_roadmap_slug() {
        for slug in ["task", "Task", "TASK"] {
            assert_eq!(
                format_done_directive(&DoneDirective::Phase {
                    roadmap: slug.to_string(),
                    phase: "p".to_string(),
                }),
                Err(FormatDoneError::ReservedRoadmapSlug),
                "failed for: {slug}"
            );
        }
    }
}
