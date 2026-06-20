//! Request types for the mutating `update` operations.
//!
//! These collapse the per-field "a value **or** its `clear_*` flag" protocol
//! that the CLI, server, and MCP each hand-rolled (reject-both, then map to a
//! sentinel) into three small types whose [`from_args`](BodyUpdate::from_args)
//! constructors validate the mutually-exclusive flags **once** and return a
//! matchable [`Error::ConflictingUpdate`].

use crate::error::{Error, Result};
use crate::model::{Difficulty, ModelTier, Priority};

/// How an update should treat a document's markdown body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyUpdate {
    /// Leave the existing body unchanged.
    Keep,
    /// Replace the body with this content. An empty string here is only
    /// honored when the existing body is also empty; otherwise the op rejects
    /// it with [`Error::BodyClobberRefused`]. Use [`BodyUpdate::Clear`] to
    /// confirm emptying a non-empty body.
    Set(String),
    /// Clear the body (set it to empty), confirming the clobber of any
    /// existing content.
    Clear,
}

impl BodyUpdate {
    /// Builds a [`BodyUpdate`] from a frontend's `body` value and `clear` flag.
    ///
    /// `Some(b)` → [`Set`](BodyUpdate::Set); `clear` → [`Clear`](BodyUpdate::Clear);
    /// neither → [`Keep`](BodyUpdate::Keep).
    ///
    /// # Errors
    ///
    /// Returns [`Error::ConflictingUpdate`] if both `body` and `clear` are set.
    pub fn from_args(body: Option<String>, clear: bool) -> Result<Self> {
        match (body, clear) {
            (Some(_), true) => Err(Error::ConflictingUpdate {
                field: "body".to_string(),
            }),
            (Some(b), false) => Ok(BodyUpdate::Set(b)),
            (None, true) => Ok(BodyUpdate::Clear),
            (None, false) => Ok(BodyUpdate::Keep),
        }
    }

    /// Applies this update to a document's `body` in place.
    ///
    /// # Errors
    ///
    /// Returns [`Error::BodyClobberRefused`] when [`Set`](BodyUpdate::Set) would
    /// replace a non-empty body with an empty string (use
    /// [`Clear`](BodyUpdate::Clear) to confirm that).
    pub(crate) fn apply(self, body: &mut String) -> Result<()> {
        match self {
            BodyUpdate::Keep => {}
            BodyUpdate::Clear => body.clear(),
            BodyUpdate::Set(new) => {
                if new.is_empty() && !body.is_empty() {
                    return Err(Error::BodyClobberRefused);
                }
                *body = new;
            }
        }
        Ok(())
    }
}

/// How an update should treat a document's tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagsUpdate {
    /// Leave the existing tags unchanged.
    Keep,
    /// Replace the tags with this list (an empty list clears them).
    Set(Vec<String>),
    /// Clear all tags.
    Clear,
}

impl TagsUpdate {
    /// Builds a [`TagsUpdate`] from a frontend's `tags` value and `clear` flag.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ConflictingUpdate`] if both `tags` and `clear` are set.
    pub fn from_args(tags: Option<Vec<String>>, clear: bool) -> Result<Self> {
        match (tags, clear) {
            (Some(_), true) => Err(Error::ConflictingUpdate {
                field: "tags".to_string(),
            }),
            (Some(t), false) => Ok(TagsUpdate::Set(t)),
            (None, true) => Ok(TagsUpdate::Clear),
            (None, false) => Ok(TagsUpdate::Keep),
        }
    }

    /// Applies this update to a document's optional `tags` in place. An empty
    /// [`Set`](TagsUpdate::Set) list clears the tags, matching the historical
    /// behavior.
    pub(crate) fn apply(self, tags: &mut Option<Vec<String>>) {
        match self {
            TagsUpdate::Keep => {}
            TagsUpdate::Clear => *tags = None,
            TagsUpdate::Set(new) => *tags = if new.is_empty() { None } else { Some(new) },
        }
    }
}

/// How an update should treat a document's optional priority.
///
/// Used for roadmaps, whose priority is optional. Tasks carry a required
/// priority and so use a plain `Option<Priority>` (set-or-keep) instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorityUpdate {
    /// Leave the existing priority unchanged.
    Keep,
    /// Set the priority to this value.
    Set(Priority),
    /// Clear the priority (set it to none).
    Clear,
}

impl PriorityUpdate {
    /// Builds a [`PriorityUpdate`] from a frontend's `priority` value and
    /// `clear` flag.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ConflictingUpdate`] if both `priority` and `clear` are
    /// set.
    pub fn from_args(priority: Option<Priority>, clear: bool) -> Result<Self> {
        match (priority, clear) {
            (Some(_), true) => Err(Error::ConflictingUpdate {
                field: "priority".to_string(),
            }),
            (Some(p), false) => Ok(PriorityUpdate::Set(p)),
            (None, true) => Ok(PriorityUpdate::Clear),
            (None, false) => Ok(PriorityUpdate::Keep),
        }
    }

    /// Applies this update to a document's optional `priority` in place.
    pub(crate) fn apply(self, priority: &mut Option<Priority>) {
        match self {
            PriorityUpdate::Keep => {}
            PriorityUpdate::Set(p) => *priority = Some(p),
            PriorityUpdate::Clear => *priority = None,
        }
    }
}

/// How an update should treat a phase's optional difficulty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DifficultyUpdate {
    /// Leave the existing difficulty unchanged.
    Keep,
    /// Set the difficulty to this value.
    Set(Difficulty),
    /// Clear the difficulty (set it to none).
    Clear,
}

impl DifficultyUpdate {
    /// Builds a [`DifficultyUpdate`] from a frontend's `difficulty` value and
    /// `clear` flag.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ConflictingUpdate`] if both `difficulty` and `clear`
    /// are set.
    pub fn from_args(difficulty: Option<Difficulty>, clear: bool) -> Result<Self> {
        match (difficulty, clear) {
            (Some(_), true) => Err(Error::ConflictingUpdate {
                field: "difficulty".to_string(),
            }),
            (Some(d), false) => Ok(DifficultyUpdate::Set(d)),
            (None, true) => Ok(DifficultyUpdate::Clear),
            (None, false) => Ok(DifficultyUpdate::Keep),
        }
    }

    /// Applies this update to a phase's optional `difficulty` in place.
    pub(crate) fn apply(self, difficulty: &mut Option<Difficulty>) {
        match self {
            DifficultyUpdate::Keep => {}
            DifficultyUpdate::Set(d) => *difficulty = Some(d),
            DifficultyUpdate::Clear => *difficulty = None,
        }
    }
}

/// How an update should treat a phase's optional model tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelTierUpdate {
    /// Leave the existing model tier unchanged.
    Keep,
    /// Set the model tier to this value.
    Set(ModelTier),
    /// Clear the model tier (set it to none).
    Clear,
}

impl ModelTierUpdate {
    /// Builds a [`ModelTierUpdate`] from a frontend's `model` value and
    /// `clear` flag.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ConflictingUpdate`] if both `model` and `clear` are
    /// set.
    pub fn from_args(model: Option<ModelTier>, clear: bool) -> Result<Self> {
        match (model, clear) {
            (Some(_), true) => Err(Error::ConflictingUpdate {
                field: "model".to_string(),
            }),
            (Some(m), false) => Ok(ModelTierUpdate::Set(m)),
            (None, true) => Ok(ModelTierUpdate::Clear),
            (None, false) => Ok(ModelTierUpdate::Keep),
        }
    }

    /// Applies this update to a phase's optional `model` in place.
    pub(crate) fn apply(self, model: &mut Option<ModelTier>) {
        match self {
            ModelTierUpdate::Keep => {}
            ModelTierUpdate::Set(m) => *model = Some(m),
            ModelTierUpdate::Clear => *model = None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_from_args_maps_each_combination() {
        assert_eq!(
            BodyUpdate::from_args(None, false).unwrap(),
            BodyUpdate::Keep
        );
        assert_eq!(
            BodyUpdate::from_args(Some("hi".to_string()), false).unwrap(),
            BodyUpdate::Set("hi".to_string())
        );
        assert_eq!(
            BodyUpdate::from_args(None, true).unwrap(),
            BodyUpdate::Clear
        );
    }

    #[test]
    fn from_args_rejects_value_and_clear_together() {
        let err = BodyUpdate::from_args(Some(String::new()), true).unwrap_err();
        assert_eq!(err.to_string(), "cannot set both 'body' and 'clear_body'");
        assert!(matches!(err, Error::ConflictingUpdate { field } if field == "body"));

        let err = TagsUpdate::from_args(Some(vec![]), true).unwrap_err();
        assert!(matches!(err, Error::ConflictingUpdate { field } if field == "tags"));

        let err = PriorityUpdate::from_args(Some(Priority::High), true).unwrap_err();
        assert_eq!(
            err.to_string(),
            "cannot set both 'priority' and 'clear_priority'"
        );
        assert!(matches!(err, Error::ConflictingUpdate { field } if field == "priority"));

        let err = DifficultyUpdate::from_args(Some(Difficulty::Hard), true).unwrap_err();
        assert_eq!(
            err.to_string(),
            "cannot set both 'difficulty' and 'clear_difficulty'"
        );
        assert!(matches!(err, Error::ConflictingUpdate { field } if field == "difficulty"));

        let err = ModelTierUpdate::from_args(Some(ModelTier::Large), true).unwrap_err();
        assert_eq!(err.to_string(), "cannot set both 'model' and 'clear_model'");
        assert!(matches!(err, Error::ConflictingUpdate { field } if field == "model"));
    }

    #[test]
    fn difficulty_apply_arms() {
        // Keep: leaves both Some and None untouched.
        let mut d = Some(Difficulty::Hard);
        DifficultyUpdate::Keep.apply(&mut d);
        assert_eq!(d, Some(Difficulty::Hard));
        let mut d = None;
        DifficultyUpdate::Keep.apply(&mut d);
        assert_eq!(d, None);

        // Set: overwrites None and an existing value.
        let mut d = None;
        DifficultyUpdate::Set(Difficulty::Easy).apply(&mut d);
        assert_eq!(d, Some(Difficulty::Easy));
        DifficultyUpdate::Set(Difficulty::Moderate).apply(&mut d);
        assert_eq!(d, Some(Difficulty::Moderate));

        // Clear: drops an existing value.
        DifficultyUpdate::Clear.apply(&mut d);
        assert_eq!(d, None);
    }

    #[test]
    fn model_tier_apply_arms() {
        let mut m = Some(ModelTier::Large);
        ModelTierUpdate::Keep.apply(&mut m);
        assert_eq!(m, Some(ModelTier::Large));

        let mut m = None;
        ModelTierUpdate::Set(ModelTier::Small).apply(&mut m);
        assert_eq!(m, Some(ModelTier::Small));
        ModelTierUpdate::Set(ModelTier::Medium).apply(&mut m);
        assert_eq!(m, Some(ModelTier::Medium));

        ModelTierUpdate::Clear.apply(&mut m);
        assert_eq!(m, None);
    }
}
