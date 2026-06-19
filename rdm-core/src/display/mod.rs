//! Display formatting and index generation for roadmaps, phases, and projects.
//!
//! - [`format`] — terminal formatting functions for human-readable output
//! - [`index`] — INDEX.md generation from pre-aggregated project data
mod format;
mod index;

pub use format::*;
pub use index::*;

/// Formats a roadmap progress label from done/total phase counts.
///
/// Returns `no phases` when `total` is zero, `complete` when every phase is
/// done, and `{done}/{total} done` otherwise. This is the shared wording used
/// by the JSON summary and (with an extra `not started` special-case at the
/// call site) the INDEX tables.
#[must_use]
pub fn roadmap_progress_label(done: usize, total: usize) -> String {
    if total == 0 {
        "no phases".to_string()
    } else if done == total {
        "complete".to_string()
    } else {
        format!("{done}/{total} done")
    }
}

/// Truncates `s` to at most `max_len` bytes, appending `...` when truncated.
///
/// The input is trimmed first. Truncation respects UTF-8 char boundaries: the
/// cut point is moved back to the nearest boundary at or before `max_len`.
pub(crate) fn truncate_snippet(s: &str, max_len: usize) -> String {
    let trimmed = s.trim();
    if trimmed.len() <= max_len {
        return trimmed.to_string();
    }
    let mut end = max_len;
    while end > 0 && !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &trimmed[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_label_no_phases() {
        assert_eq!(roadmap_progress_label(0, 0), "no phases");
    }

    #[test]
    fn progress_label_complete() {
        assert_eq!(roadmap_progress_label(3, 3), "complete");
    }

    #[test]
    fn progress_label_partial() {
        assert_eq!(roadmap_progress_label(1, 3), "1/3 done");
    }

    #[test]
    fn progress_label_zero_done_with_phases() {
        assert_eq!(roadmap_progress_label(0, 3), "0/3 done");
    }

    #[test]
    fn truncate_snippet_short_is_unchanged() {
        assert_eq!(truncate_snippet("hello", 40), "hello");
    }

    #[test]
    fn truncate_snippet_trims_first() {
        assert_eq!(truncate_snippet("  hi  ", 40), "hi");
    }

    #[test]
    fn truncate_snippet_appends_ellipsis() {
        assert_eq!(truncate_snippet("abcdefghij", 5), "abcde...");
    }

    #[test]
    fn truncate_snippet_respects_char_boundary() {
        // "é" is two bytes; cutting near it must not split the codepoint.
        let s = "abcdé fghij";
        let out = truncate_snippet(s, 5);
        assert!(out.ends_with("..."));
    }
}
