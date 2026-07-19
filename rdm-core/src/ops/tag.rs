//! Tag inventory: which tags are in use across a project, and how many items
//! carry each one.
//!
//! This is a read-only sensor over the existing document scans — it never
//! writes. Scope decisions, made deliberately rather than by accident:
//!
//! - **Roadmaps and tasks only.** Phase tags are *not* counted. A later phase
//!   could extend the scan to phases behind a flag.
//! - **Archived roadmaps are excluded**, because
//!   [`list_roadmaps`](crate::ops::roadmap::list_roadmaps) never walks the
//!   archive directory.
//! - **Every task status is counted**, including terminal (`done`,
//!   `wont-fix`) and `blocked` ones. This is an inventory of tags in use, not
//!   an active-work view — a deliberate divergence from
//!   [`ops::backlog`](crate::ops::backlog), which filters to active tasks.
//! - **Reserved tags are counted like any other tag.** `needs-plan-review`
//!   (see [`crate::tags`]) is a real tag in use and is not special-cased or
//!   filtered; with the plan-review gate enabled it will often top the list,
//!   which is correct and informative.
//! - **Tag strings are compared verbatim.** No case folding and no trimming:
//!   `CLI` and `cli` are two distinct tags. Normalizing here would silently
//!   misreport the user's data.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::error::Result;
use crate::store::Store;

/// One tag in use, with the number of items carrying it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TagCount {
    /// The tag, verbatim as written in the documents' frontmatter.
    pub tag: String,
    /// Total number of items (roadmaps + tasks) carrying this tag.
    pub count: usize,
    /// Number of roadmaps carrying this tag.
    pub roadmaps: usize,
    /// Number of tasks carrying this tag.
    pub tasks: usize,
}

/// Lists every tag in use across `project`'s roadmaps and tasks, with counts.
///
/// A tag listed more than once within a single document's frontmatter is
/// counted once for that document. The returned vec is sorted by `count`
/// descending, ties broken by `tag` ascending — a deterministic,
/// most-used-first order that callers and tests may rely on.
///
/// See the [module docs](self) for what is and is not scanned.
///
/// # Errors
///
/// Returns [`crate::error::Error::ProjectNotFound`] if `project` does not
/// exist. Propagates [`crate::error::Error::Io`] from the underlying
/// directory/file reads, and
/// [`crate::error::Error::FrontmatterMissing`] /
/// [`crate::error::Error::FrontmatterParse`] from malformed documents rather
/// than silently skipping them.
pub fn tag_list(store: &impl Store, project: &str) -> Result<Vec<TagCount>> {
    // Tasks first: `list_tasks` raises `ProjectNotFound`, so an unknown
    // project fails before any roadmap I/O happens.
    let tasks = crate::ops::task::list_tasks(store, project)?;
    let roadmaps = crate::ops::roadmap::list_roadmaps(store, project, None, None)?;

    // tag -> (roadmap count, task count)
    let mut counts: BTreeMap<String, (usize, usize)> = BTreeMap::new();

    for doc in &roadmaps {
        for tag in unique_tags(doc.frontmatter.tags.as_deref()) {
            counts.entry(tag.to_string()).or_default().0 += 1;
        }
    }
    for (_, doc) in &tasks {
        for tag in unique_tags(doc.frontmatter.tags.as_deref()) {
            counts.entry(tag.to_string()).or_default().1 += 1;
        }
    }

    let mut out: Vec<TagCount> = counts
        .into_iter()
        .map(|(tag, (roadmaps, tasks))| TagCount {
            tag,
            count: roadmaps + tasks,
            roadmaps,
            tasks,
        })
        .collect();
    // BTreeMap already yields tag-ascending; a stable sort by count
    // descending therefore leaves ties in tag-ascending order.
    out.sort_by(|a, b| b.count.cmp(&a.count));
    Ok(out)
}

/// Returns the distinct tags on one document, deduped so a tag repeated in a
/// single item's frontmatter is counted once.
fn unique_tags(tags: Option<&[String]>) -> BTreeSet<&str> {
    tags.unwrap_or(&[]).iter().map(String::as_str).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::model::{Priority, TaskStatus};
    use crate::store::MemoryStore;

    fn setup_store() -> MemoryStore {
        let mut store = MemoryStore::new();
        crate::ops::init::init(&mut store).unwrap();
        crate::ops::project::create_project(&mut store, "acme", "Acme Corp").unwrap();
        store
    }

    fn task(store: &mut MemoryStore, slug: &str, tags: Option<Vec<String>>) {
        crate::ops::task::create_task(
            store,
            crate::ops::task::CreateTask {
                project: "acme",
                slug,
                title: slug,
                priority: Priority::Medium,
                tags,
                body: None,
            },
        )
        .unwrap();
    }

    fn roadmap(store: &mut MemoryStore, slug: &str, tags: Option<Vec<String>>) {
        crate::ops::roadmap::create_roadmap(
            store,
            crate::ops::roadmap::CreateRoadmap {
                project: "acme",
                slug,
                title: slug,
                tags,
                ..Default::default()
            },
        )
        .unwrap();
    }

    fn tags(v: &[&str]) -> Option<Vec<String>> {
        Some(v.iter().map(|s| (*s).to_string()).collect())
    }

    #[test]
    fn counts_tags_across_roadmaps_and_tasks() {
        let mut store = setup_store();
        roadmap(&mut store, "alpha", tags(&["cli"]));
        task(&mut store, "t1", tags(&["cli", "bug"]));
        task(&mut store, "t2", tags(&["cli"]));

        let out = tag_list(&store, "acme").unwrap();
        assert_eq!(
            out,
            vec![
                TagCount {
                    tag: "cli".to_string(),
                    count: 3,
                    roadmaps: 1,
                    tasks: 2,
                },
                TagCount {
                    tag: "bug".to_string(),
                    count: 1,
                    roadmaps: 0,
                    tasks: 1,
                },
            ]
        );
    }

    #[test]
    fn untagged_items_contribute_nothing() {
        let mut store = setup_store();
        roadmap(&mut store, "alpha", None);
        task(&mut store, "t1", None);
        assert!(tag_list(&store, "acme").unwrap().is_empty());
    }

    #[test]
    fn duplicate_tag_within_one_item_counts_once() {
        let mut store = setup_store();
        task(&mut store, "t1", tags(&["cli", "cli"]));
        let out = tag_list(&store, "acme").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].count, 1);
        assert_eq!(out[0].tasks, 1);
    }

    #[test]
    fn empty_project_returns_empty_vec() {
        let store = setup_store();
        assert!(tag_list(&store, "acme").unwrap().is_empty());
    }

    #[test]
    fn unknown_project_is_project_not_found() {
        let store = MemoryStore::new();
        let err = tag_list(&store, "nope").unwrap_err();
        assert!(matches!(err, Error::ProjectNotFound(_)));
    }

    #[test]
    fn ordering_is_count_desc_then_tag_asc() {
        let mut store = setup_store();
        // zeta: 2, alpha: 2, solo: 1 — the 2s must come first, tie-broken
        // alphabetically (alpha before zeta).
        task(&mut store, "t1", tags(&["zeta", "alpha"]));
        task(&mut store, "t2", tags(&["zeta", "alpha", "solo"]));

        let out = tag_list(&store, "acme").unwrap();
        let order: Vec<(&str, usize)> = out.iter().map(|t| (t.tag.as_str(), t.count)).collect();
        assert_eq!(order, vec![("alpha", 2), ("zeta", 2), ("solo", 1)]);
    }

    #[test]
    fn terminal_tasks_are_still_counted() {
        let mut store = setup_store();
        task(&mut store, "t1", tags(&["legacy"]));
        crate::ops::task::update_task(
            &mut store,
            "acme",
            "t1",
            Some(TaskStatus::Done),
            None,
            crate::ops::TagsUpdate::Keep,
            crate::ops::BodyUpdate::Keep,
            None,
            None,
            None,
            crate::ops::TitleUpdate::Keep,
        )
        .unwrap();

        let out = tag_list(&store, "acme").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag, "legacy");
        assert_eq!(out[0].count, 1);
    }

    #[test]
    fn tags_are_compared_verbatim_without_case_folding() {
        let mut store = setup_store();
        task(&mut store, "t1", tags(&["CLI"]));
        task(&mut store, "t2", tags(&["cli"]));

        let out = tag_list(&store, "acme").unwrap();
        let names: Vec<&str> = out.iter().map(|t| t.tag.as_str()).collect();
        assert_eq!(names, vec!["CLI", "cli"]);
        assert!(out.iter().all(|t| t.count == 1));
    }
}
