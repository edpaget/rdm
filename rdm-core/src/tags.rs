//! Reserved-tag primitives.
//!
//! Some tags carry internal, tool-managed meaning rather than being purely
//! user-facing labels. This module defines those reserved names and the pure
//! helpers that stamp/clear them, so callers (CLI command handlers today,
//! skills/hooks in later phases) manipulate them consistently instead of
//! hand-rolling `Vec<String>` mutations.
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
