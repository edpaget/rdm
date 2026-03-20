//! Git hook helpers for parsing `Done:` directives from commit messages.

/// A parsed `Done: roadmap/phase` directive from a commit message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoneDirective {
    /// The roadmap slug.
    pub roadmap: String,
    /// The phase stem or number.
    pub phase: String,
}

/// Parses `Done: roadmap/phase` directives from a commit message.
///
/// Iterates lines, matches case-insensitive `^Done:` prefix, splits the value
/// on the first `/`, trims whitespace, and skips malformed lines (no `/`,
/// empty roadmap or phase).
///
/// # Examples
///
/// ```
/// use rdm_core::hook::parse_done_directives;
///
/// let msg = "feat: implement search\n\nDone: search-feature/phase-2-indexing\n";
/// let directives = parse_done_directives(msg);
/// assert_eq!(directives.len(), 1);
/// assert_eq!(directives[0].roadmap, "search-feature");
/// assert_eq!(directives[0].phase, "phase-2-indexing");
/// ```
pub fn parse_done_directives(message: &str) -> Vec<DoneDirective> {
    let mut directives = Vec::new();
    for line in message.lines() {
        let trimmed = line.trim();
        if trimmed.len() < 5 {
            continue;
        }
        // Case-insensitive check for "Done:" prefix
        if !trimmed[..5].eq_ignore_ascii_case("done:") {
            continue;
        }
        let value = trimmed[5..].trim();
        let Some((roadmap, phase)) = value.split_once('/') else {
            continue;
        };
        let roadmap = roadmap.trim();
        let phase = phase.trim();
        if roadmap.is_empty() || phase.is_empty() {
            continue;
        }
        directives.push(DoneDirective {
            roadmap: roadmap.to_string(),
            phase: phase.to_string(),
        });
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
    fn single_valid_directive() {
        let directives = parse_done_directives("Done: search-feature/phase-2-indexing");
        assert_eq!(
            directives,
            vec![DoneDirective {
                roadmap: "search-feature".to_string(),
                phase: "phase-2-indexing".to_string(),
            }]
        );
    }

    #[test]
    fn case_insensitive() {
        for prefix in ["done:", "DONE:", "DoNe:", "dOnE:"] {
            let msg = format!("{prefix} my-roadmap/my-phase");
            let directives = parse_done_directives(&msg);
            assert_eq!(directives.len(), 1, "failed for prefix: {prefix}");
            assert_eq!(directives[0].roadmap, "my-roadmap");
            assert_eq!(directives[0].phase, "my-phase");
        }
    }

    #[test]
    fn multiple_directives() {
        let msg = "feat: big merge\n\nDone: search/phase-1\nDone: perf/phase-2\n";
        let directives = parse_done_directives(msg);
        assert_eq!(directives.len(), 2);
        assert_eq!(directives[0].roadmap, "search");
        assert_eq!(directives[0].phase, "phase-1");
        assert_eq!(directives[1].roadmap, "perf");
        assert_eq!(directives[1].phase, "phase-2");
    }

    #[test]
    fn skips_non_done_lines() {
        let msg = "feat: something\nNot a done line\nDone: r/p\nAnother line";
        let directives = parse_done_directives(msg);
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].roadmap, "r");
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
    }

    #[test]
    fn trims_whitespace() {
        let directives = parse_done_directives("Done:   my-roadmap  /  my-phase  ");
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].roadmap, "my-roadmap");
        assert_eq!(directives[0].phase, "my-phase");
    }
}
