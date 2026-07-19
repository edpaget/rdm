//! Tag primitives: the shared tag-filter predicate plus reserved-tag helpers.
//!
//! [`matches_all_tags`] is the single source of truth for `--tag` filtering
//! across every rdm surface, so AND semantics, case sensitivity, and the
//! empty-filter contract cannot drift between the CLI, core ops, search, and
//! the MCP tools.
//!
//! Some tags additionally carry internal, tool-managed meaning rather than
//! being purely user-facing labels. This module also defines those reserved
//! names and the pure helpers that stamp/clear them, so callers (CLI command
//! handlers today, skills/hooks in later phases) manipulate them consistently
//! instead of hand-rolling `Vec<String>` mutations.
//!
//! This is a documentation-level contract, not an enforced one: nothing
//! prevents a user from passing a reserved tag via `--tags` themselves (the
//! same convention `rdm` already uses for the `task` roadmap-slug prefix).

/// Reserved tag stamped on roadmaps/phases/tasks created while the
/// `plan_review` config flag is enabled, marking them as pending an
/// agent-driven plan review. Cleared by the (not-yet-implemented) review
/// skill once a plan passes.
///
/// Internal/reserved: user code should treat this name as owned by rdm's
/// plan-review workflow rather than inventing a tag that collides with it.
pub const NEEDS_PLAN_REVIEW_TAG: &str = "needs-plan-review";

/// Returns whether `item_tags` carries **every** tag in `required` (logical
/// AND).
///
/// This is the single source of truth for tag filtering across every rdm
/// surface (`task list`, `roadmap list`, `rdm list`, `search`, and the MCP
/// list tools), so the semantics below cannot drift between them:
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

/// Stamps [`NEEDS_PLAN_REVIEW_TAG`] onto `tags` when `enabled` is `true`.
///
/// When `enabled` is `false`, `tags` is returned completely unchanged
/// (including a `None` staying `None`) — this is the gate that keeps the
/// `plan_review` config flag a true no-op when off.
///
/// When `enabled` is `true`, the reserved tag is appended after any existing
/// tags (insertion order is preserved; nothing is sorted or deduplicated
/// except the reserved tag itself, which is only added if not already
/// present by exact string match). The result is always `Some(...)` in this
/// branch, even if `tags` was `None`.
#[must_use]
pub fn stamp_plan_review_tag(tags: Option<Vec<String>>, enabled: bool) -> Option<Vec<String>> {
    if !enabled {
        return tags;
    }
    let mut tags = tags.unwrap_or_default();
    if !tags.iter().any(|t| t == NEEDS_PLAN_REVIEW_TAG) {
        tags.push(NEEDS_PLAN_REVIEW_TAG.to_string());
    }
    Some(tags)
}

/// Removes [`NEEDS_PLAN_REVIEW_TAG`] from `tags`, if present.
///
/// Idempotent and total:
/// - `None` stays `None`.
/// - A tag list that doesn't contain the reserved tag is returned unchanged
///   (a no-op, matching the "clearing when absent" requirement).
/// - If removing the reserved tag empties the list, returns `None` rather
///   than `Some(vec![])`, matching the empty-is-`None` convention used by
///   [`crate::ops::update::TagsUpdate::apply`](crate::ops).
///
/// Not yet wired into any CLI command — this phase only adds the primitive;
/// a later phase's plan-review skill will call it once a plan passes.
#[must_use]
pub fn clear_plan_review_tag(tags: Option<Vec<String>>) -> Option<Vec<String>> {
    let mut tags = tags?;
    tags.retain(|t| t != NEEDS_PLAN_REVIEW_TAG);
    if tags.is_empty() { None } else { Some(tags) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
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

    #[test]
    fn stamp_plan_review_tag_disabled_is_noop() {
        let tags = Some(vec!["bug".to_string()]);
        assert_eq!(stamp_plan_review_tag(tags.clone(), false), tags);
    }

    #[test]
    fn stamp_plan_review_tag_disabled_with_no_tags_stays_none() {
        assert_eq!(stamp_plan_review_tag(None, false), None);
    }

    #[test]
    fn stamp_plan_review_tag_enabled_adds_to_user_tags() {
        let tags = Some(vec!["bug".to_string(), "urgent".to_string()]);
        let result = stamp_plan_review_tag(tags, true).unwrap();
        assert_eq!(
            result,
            vec![
                "bug".to_string(),
                "urgent".to_string(),
                NEEDS_PLAN_REVIEW_TAG.to_string(),
            ]
        );
    }

    #[test]
    fn stamp_plan_review_tag_enabled_with_no_user_tags() {
        let result = stamp_plan_review_tag(None, true).unwrap();
        assert_eq!(result, vec![NEEDS_PLAN_REVIEW_TAG.to_string()]);
    }

    #[test]
    fn stamp_plan_review_tag_enabled_does_not_duplicate_if_user_already_passed_it() {
        let tags = Some(vec!["bug".to_string(), NEEDS_PLAN_REVIEW_TAG.to_string()]);
        let result = stamp_plan_review_tag(tags, true).unwrap();
        assert_eq!(
            result,
            vec!["bug".to_string(), NEEDS_PLAN_REVIEW_TAG.to_string()]
        );
    }

    #[test]
    fn clear_plan_review_tag_removes_when_present() {
        let tags = Some(vec!["bug".to_string(), NEEDS_PLAN_REVIEW_TAG.to_string()]);
        let result = clear_plan_review_tag(tags).unwrap();
        assert_eq!(result, vec!["bug".to_string()]);
    }

    #[test]
    fn clear_plan_review_tag_absent_is_noop() {
        let tags = Some(vec!["bug".to_string()]);
        assert_eq!(clear_plan_review_tag(tags.clone()), tags);
    }

    #[test]
    fn clear_plan_review_tag_none_is_noop() {
        assert_eq!(clear_plan_review_tag(None), None);
    }

    #[test]
    fn clear_plan_review_tag_only_tag_present_yields_none() {
        let tags = Some(vec![NEEDS_PLAN_REVIEW_TAG.to_string()]);
        assert_eq!(clear_plan_review_tag(tags), None);
    }

    #[test]
    fn clear_plan_review_tag_is_idempotent_when_called_twice() {
        let tags = Some(vec!["bug".to_string(), NEEDS_PLAN_REVIEW_TAG.to_string()]);
        let once = clear_plan_review_tag(tags);
        let twice = clear_plan_review_tag(once.clone());
        assert_eq!(once, twice);
        assert_eq!(once, Some(vec!["bug".to_string()]));
    }
}
