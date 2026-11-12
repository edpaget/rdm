//! Tag primitives: the shared tag-filter predicate and tag normalization.
//!
//! [`matches_all_tags`] is the single source of truth for `--tag` filtering
//! across every rdm surface, so AND semantics, case sensitivity, and the
//! empty-filter contract cannot drift between the CLI, core ops, and search.
//!
//! [`normalize_tags`] is what every write path runs its tags through.
//!
//! No tag is special to this code: every tag is a user-facing label, and
//! nothing here stamps or clears one on the caller's behalf.

/// Returns whether `item_tags` carries **every** tag in `required` (logical
/// AND).
///
/// This is the single source of truth for tag filtering across every rdm
/// surface (`task list`, `roadmap list`, `rdm list`, and `search`), so the
/// semantics below cannot drift between them:
///
/// - An empty `required` imposes no constraint and always matches — zero
///   `--tag` flags is the identity filter, never "items with no tags".
/// - An item with `None` tags, or an empty tag list, never matches a
///   non-empty `required`.
/// - Matching is **exact and case-sensitive**: `--tag Bug` does not match an
///   item tagged `bug`. This deliberately differs from the fuzzy matching
///   `search` applies to its query text; tags are a hard pre-filter.
/// - Repeating a tag in `required` is idempotent.
///
/// # Examples
///
/// ```
/// use rdm_core::tags::matches_all_tags;
///
/// let item = vec!["bug".to_string(), "ui".to_string()];
/// assert!(matches_all_tags(Some(&item), &[]));
/// assert!(matches_all_tags(Some(&item), &["bug".to_string()]));
/// assert!(matches_all_tags(
///     Some(&item),
///     &["bug".to_string(), "ui".to_string()]
/// ));
/// assert!(!matches_all_tags(
///     Some(&item),
///     &["bug".to_string(), "perf".to_string()]
/// ));
/// assert!(!matches_all_tags(None, &["bug".to_string()]));
/// ```
#[must_use]
pub fn matches_all_tags(item_tags: Option<&[String]>, required: &[String]) -> bool {
    if required.is_empty() {
        return true;
    }
    match item_tags {
        None => false,
        Some(tags) => required.iter().all(|t| tags.iter().any(|it| it == t)),
    }
}

/// Normalizes a tag list: trims each element, drops empty (or
/// whitespace-only) elements, and removes duplicates, keeping the first
/// occurrence.
///
/// Order is otherwise preserved, and de-duplication is exact and
/// case-sensitive, matching [`matches_all_tags`]. Every write path (create
/// `--tags`, update `--tags`, `--add-tag`, `--remove-tag`) runs its tags
/// through this, so frontends can pass raw user text without cleaning it.
///
/// # Examples
///
/// ```
/// use rdm_core::tags::normalize_tags;
///
/// let raw = vec![" a".to_string(), " ".to_string(), "b".to_string(), "a".to_string()];
/// assert_eq!(normalize_tags(raw), vec!["a".to_string(), "b".to_string()]);
/// ```
#[must_use]
pub fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(tags.len());
    for tag in tags {
        let trimmed = tag.trim();
        if !trimmed.is_empty() && !out.iter().any(|t| t == trimmed) {
            out.push(trimmed.to_string());
        }
    }
    out
}

/// Normalizes an optional tag list with [`normalize_tags`], mapping a list
/// that normalizes to empty to `None` (the empty-is-`None` storage
/// convention).
pub(crate) fn normalize_optional_tags(tags: Option<Vec<String>>) -> Option<Vec<String>> {
    tags.map(normalize_tags).filter(|t| !t.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn normalize_tags_trims_drops_empty_and_dedupes() {
        assert_eq!(
            normalize_tags(tags(&["a", " ", "b ", "", " a", "c", "b"])),
            tags(&["a", "b", "c"])
        );
    }

    #[test]
    fn normalize_tags_is_case_sensitive() {
        assert_eq!(normalize_tags(tags(&["Bug", "bug"])), tags(&["Bug", "bug"]));
    }

    #[test]
    fn normalize_optional_tags_maps_empty_to_none() {
        assert_eq!(normalize_optional_tags(None), None);
        assert_eq!(normalize_optional_tags(Some(tags(&["", "  "]))), None);
        assert_eq!(
            normalize_optional_tags(Some(tags(&[" x "]))),
            Some(tags(&["x"]))
        );
    }

    #[test]
    fn matches_all_tags_empty_required_always_matches() {
        assert!(matches_all_tags(None, &[]));
        assert!(matches_all_tags(Some(&[]), &[]));
        assert!(matches_all_tags(Some(&tags(&["bug"])), &[]));
    }

    #[test]
    fn matches_all_tags_single_tag_match() {
        assert!(matches_all_tags(
            Some(&tags(&["bug", "ui"])),
            &tags(&["ui"])
        ));
    }

    #[test]
    fn matches_all_tags_requires_every_tag() {
        let item = tags(&["ui", "bug", "perf"]);
        assert!(matches_all_tags(Some(&item), &tags(&["bug", "ui"])));
        // Partial match is not enough.
        assert!(!matches_all_tags(
            Some(&tags(&["bug"])),
            &tags(&["bug", "ui"])
        ));
    }

    #[test]
    fn matches_all_tags_none_item_tags_never_match_non_empty_filter() {
        assert!(!matches_all_tags(None, &tags(&["bug"])));
    }

    #[test]
    fn matches_all_tags_empty_item_tags_never_match_non_empty_filter() {
        assert!(!matches_all_tags(Some(&[]), &tags(&["bug"])));
    }

    #[test]
    fn matches_all_tags_duplicate_required_is_idempotent() {
        let item = tags(&["bug"]);
        assert_eq!(
            matches_all_tags(Some(&item), &tags(&["bug"])),
            matches_all_tags(Some(&item), &tags(&["bug", "bug"])),
        );
        assert!(matches_all_tags(Some(&item), &tags(&["bug", "bug"])));
    }

    #[test]
    fn matches_all_tags_is_case_sensitive() {
        assert!(!matches_all_tags(Some(&tags(&["bug"])), &tags(&["Bug"])));
    }
}
