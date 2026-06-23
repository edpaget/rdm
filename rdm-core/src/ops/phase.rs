//! Phase operations.

use chrono::Local;

use crate::document::Document;
use crate::error::{Error, Result};
use crate::model::{Phase, PhaseStatus};
use crate::ops::update::{BodyUpdate, DifficultyUpdate, ModelTierUpdate, TagsUpdate};
use crate::store::{DirEntryKind, Store};

/// Lists all phases in a roadmap, sorted by phase number.
///
/// Returns `(stem, Document<Phase>)` tuples.
///
/// # Errors
///
/// Returns [`Error::RoadmapNotFound`] if the roadmap doesn't exist,
/// [`Error::Io`] if the directory cannot be read, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if a
/// phase file has invalid frontmatter.
pub fn list_phases(
    store: &impl Store,
    project: &str,
    roadmap: &str,
) -> Result<Vec<(String, Document<Phase>)>> {
    let roadmap_file = crate::paths::roadmap_path(project, roadmap);
    if !store.exists(&roadmap_file) {
        return Err(Error::RoadmapNotFound(roadmap.to_string()));
    }

    let dir = crate::paths::roadmap_dir(project, roadmap);
    let entries = store.list(&dir)?;

    let mut phases: Vec<(String, Document<Phase>)> = Vec::new();
    for entry in entries {
        if entry.kind != DirEntryKind::File {
            continue;
        }
        if entry.name == "roadmap.md" || !entry.name.ends_with(".md") {
            continue;
        }
        let stem = entry.name.trim_end_matches(".md").to_string();
        let doc = crate::io::load_phase(store, project, roadmap, &stem)?;
        phases.push((stem, doc));
    }
    phases.sort_by_key(|(_, doc)| doc.frontmatter.phase);
    Ok(phases)
}

/// Creates a new phase within a roadmap.
///
/// If `phase_number` is `None`, auto-assigns the next number.
/// `body` sets the markdown body below the frontmatter. Pass `None` for
/// an empty body. `tags` sets optional tags for categorization.
///
/// # Errors
///
/// Returns [`Error::RoadmapNotFound`] if the roadmap doesn't exist,
/// [`Error::DuplicateSlug`] if a phase with the same stem already exists,
/// [`Error::Io`] if file creation fails, or
/// [`Error::FrontmatterParse`] if frontmatter serialization fails.
#[allow(clippy::too_many_arguments)]
pub fn create_phase(
    store: &mut impl Store,
    project: &str,
    roadmap: &str,
    slug: &str,
    title: &str,
    phase_number: Option<u32>,
    body: Option<&str>,
    tags: Option<Vec<String>>,
) -> Result<Document<Phase>> {
    let roadmap_file = crate::paths::roadmap_path(project, roadmap);
    if !store.exists(&roadmap_file) {
        return Err(Error::RoadmapNotFound(roadmap.to_string()));
    }

    let number = match phase_number {
        Some(n) => n,
        None => {
            let existing = list_phases(store, project, roadmap)?;
            existing
                .last()
                .map(|(_, doc)| doc.frontmatter.phase + 1)
                .unwrap_or(1)
        }
    };

    let stem = crate::model::phase_stem(number, slug);
    let path = crate::paths::phase_path(project, roadmap, &stem);
    if store.exists(&path) {
        return Err(Error::DuplicateSlug(stem));
    }

    let doc = Document {
        frontmatter: Phase {
            phase: number,
            title: title.to_string(),
            status: PhaseStatus::NotStarted,
            tags,
            completed: None,
            commit: None,
            review_sha: None,
            difficulty: None,
            model: None,
        },
        body: body.unwrap_or_default().to_string(),
    };
    crate::io::write_phase(store, project, roadmap, &stem, &doc)?;

    // Update roadmap's phases list
    let mut roadmap_doc = crate::io::load_roadmap(store, project, roadmap)?;
    roadmap_doc.frontmatter.phases.push(stem);
    crate::io::write_roadmap(store, project, roadmap, &roadmap_doc)?;

    Ok(doc)
}

/// Updates a phase's status, tags, body, and/or commit SHA.
///
/// When `status` is `Some` of a terminal state (`Done` or `WontFix`),
/// auto-sets `completed` to today and stores the optional `commit` SHA.
/// Re-setting the same terminal state preserves the existing `completed`
/// date and only updates `commit` if a new value is provided. When `status`
/// transitions to a non-terminal state, both `completed` and `commit` are
/// cleared. When `status` is `None`, the existing status, `completed`, and
/// `commit` are preserved.
///
/// The `review_sha` parameter stamps the source-repo HEAD SHA that produced
/// the item. When `status` transitions to [`PhaseStatus::NeedsReview`], the
/// provided `review_sha` is stored on the phase; any other status change clears
/// it to `None`; when `status` is `None`, the existing `review_sha` is
/// preserved.
/// When `tags`/`body` are `Keep`, the existing values are preserved; otherwise
/// see [`TagsUpdate`] and [`BodyUpdate`].
///
/// # Errors
///
/// Returns [`Error::PhaseNotFound`] if the phase file doesn't exist,
/// [`Error::BodyClobberRefused`] if `body` is [`BodyUpdate::Set("")`](BodyUpdate::Set)
/// over a non-empty body (use [`BodyUpdate::Clear`] to confirm),
/// [`Error::Io`] if reading or writing fails, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if the
/// existing phase file has invalid frontmatter.
#[allow(clippy::too_many_arguments)]
pub fn update_phase(
    store: &mut impl Store,
    project: &str,
    roadmap: &str,
    phase_stem: &str,
    status: Option<PhaseStatus>,
    tags: TagsUpdate,
    body: BodyUpdate,
    commit: Option<String>,
    review_sha: Option<String>,
) -> Result<Document<Phase>> {
    let path = crate::paths::phase_path(project, roadmap, phase_stem);
    if !store.exists(&path) {
        return Err(Error::PhaseNotFound(phase_stem.to_string()));
    }

    let mut doc = crate::io::load_phase(store, project, roadmap, phase_stem)?;
    if let Some(status) = status {
        if status.is_terminal() && doc.frontmatter.status == status {
            // Already at this terminal state: only update commit if a new one is provided
            if let Some(sha) = commit {
                doc.frontmatter.commit = Some(sha);
            }
        } else {
            doc.frontmatter.status = status;
            if status.is_terminal() {
                doc.frontmatter.completed = Some(Local::now().date_naive());
                doc.frontmatter.commit = commit;
            } else {
                doc.frontmatter.completed = None;
                doc.frontmatter.commit = None;
            }
            // Stamp the source-repo SHA on entry to needs-review; clear it on
            // any other transition so a stale discriminator never lingers.
            if status == PhaseStatus::NeedsReview {
                doc.frontmatter.review_sha = review_sha;
            } else {
                doc.frontmatter.review_sha = None;
            }
        }
    }
    tags.apply(&mut doc.frontmatter.tags);
    body.apply(&mut doc.body)?;
    crate::io::write_phase(store, project, roadmap, phase_stem, &doc)?;
    Ok(doc)
}

/// Sets (or clears) a phase's difficulty and/or model-tier estimate.
///
/// This is the dedicated entry point for difficulty-aware model selection
/// metadata, kept separate from [`update_phase`] so the status/tags/body
/// signature stays untouched. `difficulty` and `model` each follow the
/// keep/set/clear protocol via [`DifficultyUpdate`] and [`ModelTierUpdate`].
///
/// # Model-tier auto-derive
///
/// The difficulty→tier mapping ([`model_tier`](crate::model::Difficulty::model_tier))
/// is authoritative:
/// when `difficulty` is [`DifficultyUpdate::Set`], `model` is
/// [`ModelTierUpdate::Keep`], and no model is already recorded, the model tier
/// is derived from the difficulty. An explicit [`ModelTierUpdate::Set`] or
/// [`ModelTierUpdate::Clear`] always wins, and a previously set model is never
/// overwritten by the derive — so a human override is respected.
///
/// # Errors
///
/// Returns [`Error::PhaseNotFound`] if the phase file doesn't exist,
/// [`Error::Io`] if reading or writing fails, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if the existing
/// phase file has invalid frontmatter.
pub fn set_phase_estimate(
    store: &mut impl Store,
    project: &str,
    roadmap: &str,
    phase_stem: &str,
    difficulty: DifficultyUpdate,
    model: ModelTierUpdate,
) -> Result<Document<Phase>> {
    let path = crate::paths::phase_path(project, roadmap, phase_stem);
    if !store.exists(&path) {
        return Err(Error::PhaseNotFound(phase_stem.to_string()));
    }

    let mut doc = crate::io::load_phase(store, project, roadmap, phase_stem)?;
    difficulty.apply(&mut doc.frontmatter.difficulty);
    model.apply(&mut doc.frontmatter.model);
    // Auto-derive the model tier from the difficulty when the caller set a
    // difficulty but left the model untouched and none is already recorded.
    // An explicit Set/Clear model (applied above) or a pre-existing model wins.
    if let (DifficultyUpdate::Set(d), ModelTierUpdate::Keep) = (difficulty, model)
        && doc.frontmatter.model.is_none()
    {
        doc.frontmatter.model = Some(d.model_tier());
    }
    crate::io::write_phase(store, project, roadmap, phase_stem, &doc)?;
    Ok(doc)
}

/// Removes a phase from a roadmap.
///
/// Deletes the phase file and removes its stem from the roadmap's `phases`
/// list.
///
/// # Errors
///
/// Returns [`Error::PhaseNotFound`] if the phase file doesn't exist,
/// [`Error::Io`] if the file cannot be deleted or the roadmap cannot be
/// updated, or [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`]
/// if the roadmap file has invalid frontmatter.
pub fn remove_phase(
    store: &mut impl Store,
    project: &str,
    roadmap: &str,
    phase_stem: &str,
) -> Result<()> {
    let path = crate::paths::phase_path(project, roadmap, phase_stem);
    if !store.exists(&path) {
        return Err(Error::PhaseNotFound(phase_stem.to_string()));
    }
    store.delete(&path)?;

    // Remove stem from roadmap's phases list
    let mut roadmap_doc = crate::io::load_roadmap(store, project, roadmap)?;
    roadmap_doc.frontmatter.phases.retain(|s| s != phase_stem);
    crate::io::write_roadmap(store, project, roadmap, &roadmap_doc)?;
    Ok(())
}

/// Resolves a phase identifier to a file stem.
///
/// If `identifier` parses as a `u32`, looks up the phase by number.
/// Otherwise, returns `identifier` as-is for downstream validation.
///
/// # Errors
///
/// Returns [`Error::PhaseNotFound`] if `identifier` is numeric but no
/// phase with that number exists. Also propagates errors from
/// [`list_phases`].
pub fn resolve_phase_stem(
    store: &impl Store,
    project: &str,
    roadmap: &str,
    identifier: &str,
) -> Result<String> {
    if let Ok(num) = identifier.parse::<u32>() {
        let phases = list_phases(store, project, roadmap)?;
        for (stem, doc) in phases {
            if doc.frontmatter.phase == num {
                return Ok(stem);
            }
        }
        return Err(Error::PhaseNotFound(identifier.to_string()));
    }
    Ok(identifier.to_string())
}
