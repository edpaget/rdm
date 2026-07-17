//! Read-only backlog grooming report.
//!
//! Surfaces four signals over a project's active tasks and roadmaps without
//! mutating anything: stale tasks, likely-duplicate task clusters (via the
//! existing fuzzy [`search`](crate::search) infrastructure), thematic tag
//! clusters, and archivable roadmaps. This is a sensor, not an actuator — it
//! never writes, and callers decide what (if anything) to do with the
//! results.

use chrono::NaiveDate;
use serde::Serialize;

use crate::error::Result;
use crate::model::TaskStatus;
use crate::ops::roadmap::{RoadmapStatus, computed_status};
use crate::search::{ItemKind, SearchFilter};
use crate::store::Store;

/// Default staleness threshold, in days, used by [`ReportOptions::default`].
///
/// A task is flagged as stale once its `created` date is at least this many
/// days in the past (inclusive) and it is still `open` or `in-progress`.
pub const DEFAULT_STALE_THRESHOLD_DAYS: i64 = 60;

/// Minimum score, as a fraction of a candidate's self-match score, for
/// another active task to be unioned into the same duplicate cluster.
///
/// Tuned empirically against the fuzzy matcher in [`crate::search`] so that
/// near-identical titles cluster together while unrelated titles do not.
/// Paired with the title-vs-body snippet check in [`duplicate_clusters`]:
/// without that check, a candidate's short title can score *higher*
/// against an unrelated task's long, prose-y body than against that task's
/// own title (fuzzy subsequence matching doesn't penalize target length),
/// which chains unrelated tasks together through shared vocabulary even at
/// a strict ratio. Verified against this repo's own backlog during
/// development: an early version without the snippet check clustered most
/// of the active backlog into one group.
const DUPLICATE_SCORE_RATIO: f64 = 0.85;

/// Options controlling a [`report`] run.
#[derive(Debug, Clone)]
pub struct ReportOptions {
    /// Staleness threshold in days. See [`DEFAULT_STALE_THRESHOLD_DAYS`].
    pub older_than_days: i64,
    /// Restrict every section to items carrying this tag.
    pub tag: Option<String>,
}

impl Default for ReportOptions {
    fn default() -> Self {
        ReportOptions {
            older_than_days: DEFAULT_STALE_THRESHOLD_DAYS,
            tag: None,
        }
    }
}

/// A task flagged as stale: still active (`open` or `in-progress`) with a
/// `created` date past the configured threshold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StaleTask {
    /// Task slug.
    pub slug: String,
    /// Task title.
    pub title: String,
    /// Current status (always `open` or `in-progress`).
    pub status: TaskStatus,
    /// Date the task was created.
    pub created: NaiveDate,
    /// Age of the task in days, as of today.
    pub age_days: i64,
    /// Tags carried by the task, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

/// A single member of a [`DuplicateCluster`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DuplicateMember {
    /// Task slug.
    pub slug: String,
    /// Task title.
    pub title: String,
}

/// A group of active tasks whose titles/bodies fuzzy-match each other closely
/// enough to be likely duplicates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DuplicateCluster {
    /// The tasks in this cluster, in slug order.
    pub members: Vec<DuplicateMember>,
}

/// A single member of a [`TagCluster`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TagClusterMember {
    /// Task slug.
    pub slug: String,
    /// Task title.
    pub title: String,
}

/// A group of active tasks sharing a single tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TagCluster {
    /// The shared tag.
    pub tag: String,
    /// The tasks carrying this tag, in slug order.
    pub tasks: Vec<TagClusterMember>,
}

/// A roadmap whose phases are all terminal but which has not yet been
/// archived.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ArchivableRoadmap {
    /// Roadmap slug.
    pub roadmap: String,
    /// Roadmap title.
    pub title: String,
    /// Number of phases in the roadmap.
    pub phase_count: usize,
}

/// The full backlog grooming report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BacklogReport {
    /// Stale tasks, sorted by age descending.
    pub stale_tasks: Vec<StaleTask>,
    /// Likely-duplicate task clusters, sorted by first member slug ascending.
    pub duplicate_clusters: Vec<DuplicateCluster>,
    /// Thematic tag clusters, sorted by tag ascending.
    pub tag_clusters: Vec<TagCluster>,
    /// Archivable roadmaps, sorted by roadmap slug ascending.
    pub archivable_roadmaps: Vec<ArchivableRoadmap>,
}

/// Returns whether `item_tags` contains `needle`, treating `None` as no tags.
fn has_tag(item_tags: Option<&[String]>, needle: &str) -> bool {
    item_tags.is_some_and(|tags| tags.iter().any(|t| t == needle))
}

/// Builds a read-only backlog grooming report for `project`.
///
/// Combines four independent, side-effect-free scans:
///
/// - **Stale tasks**: tasks still `open` or `in-progress` whose `created`
///   date is at least `opts.older_than_days` days in the past (inclusive
///   boundary — a task created exactly `older_than_days` days ago is
///   flagged).
/// - **Duplicate clusters**: active tasks (non-terminal) whose *titles*
///   fuzzy-match each other via [`crate::search::search`] closely enough
///   (within [`DUPLICATE_SCORE_RATIO`] of each task's own self-match score,
///   and with the match driven by title similarity rather than incidental
///   body overlap — see [`DUPLICATE_SCORE_RATIO`]'s doc comment) are grouped
///   by transitive union. This is `O(n^2)` in the number of active tasks
///   (one search per candidate) — acceptable for a phase-1 backlog sensor,
///   but a scaling caveat for large backlogs.
/// - **Tag clusters**: active tasks grouped by each individual tag they
///   carry (a task with two tags contributes to two clusters). Groups of one
///   are not reported.
/// - **Archivable roadmaps**: roadmaps (already excludes archived — those
///   live in a separate directory) whose phases are all terminal per
///   [`computed_status`].
///
/// If `opts.tag` is set, every section is scoped to items carrying that tag
/// first.
///
/// # Errors
///
/// Returns [`crate::error::Error::ProjectNotFound`] if `project` does not
/// exist. Propagates I/O and frontmatter errors from the underlying
/// task/roadmap/phase reads.
pub fn report(store: &impl Store, project: &str, opts: &ReportOptions) -> Result<BacklogReport> {
    let required_tag = opts.tag.as_deref();

    let all_tasks = crate::ops::task::list_tasks(store, project)?;
    let tasks: Vec<_> = all_tasks
        .into_iter()
        .filter(|(_, doc)| required_tag.is_none_or(|t| has_tag(doc.frontmatter.tags.as_deref(), t)))
        .collect();

    let all_roadmaps = crate::ops::roadmap::list_roadmaps(store, project, None, None)?;
    let roadmaps: Vec<_> = all_roadmaps
        .into_iter()
        .filter(|doc| required_tag.is_none_or(|t| has_tag(doc.frontmatter.tags.as_deref(), t)))
        .collect();

    let stale_tasks = stale_tasks(&tasks, opts.older_than_days);
    let duplicate_clusters = duplicate_clusters(store, project, &tasks, required_tag)?;
    let tag_clusters = tag_clusters(&tasks);
    let archivable_roadmaps = archivable_roadmaps(store, project, &roadmaps)?;

    Ok(BacklogReport {
        stale_tasks,
        duplicate_clusters,
        tag_clusters,
        archivable_roadmaps,
    })
}

/// Computes the stale-tasks section, sorted by age descending.
fn stale_tasks(
    tasks: &[(String, crate::document::Document<crate::model::Task>)],
    older_than_days: i64,
) -> Vec<StaleTask> {
    let today = chrono::Local::now().date_naive();
    let mut out: Vec<StaleTask> = tasks
        .iter()
        .filter_map(|(slug, doc)| {
            let t = &doc.frontmatter;
            if !matches!(t.status, TaskStatus::Open | TaskStatus::InProgress) {
                return None;
            }
            let age_days = today.signed_duration_since(t.created).num_days();
            if age_days < older_than_days {
                return None;
            }
            Some(StaleTask {
                slug: slug.clone(),
                title: t.title.clone(),
                status: t.status,
                created: t.created,
                age_days,
                tags: t.tags.clone(),
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.age_days
            .cmp(&a.age_days)
            .then_with(|| a.slug.cmp(&b.slug))
    });
    out
}

/// Computes the duplicate-clusters section via union-find over fuzzy search
/// results, sorted by first member slug ascending.
fn duplicate_clusters(
    store: &impl Store,
    project: &str,
    tasks: &[(String, crate::document::Document<crate::model::Task>)],
    required_tag: Option<&str>,
) -> Result<Vec<DuplicateCluster>> {
    let candidates: Vec<&(String, crate::document::Document<crate::model::Task>)> = tasks
        .iter()
        .filter(|(_, doc)| !doc.frontmatter.status.is_terminal())
        .collect();

    // Union-find over candidate indices.
    let mut parent: Vec<usize> = (0..candidates.len()).collect();
    fn find(parent: &mut [usize], x: usize) -> usize {
        if parent[x] != x {
            parent[x] = find(parent, parent[x]);
        }
        parent[x]
    }
    fn union(parent: &mut [usize], a: usize, b: usize) {
        let ra = find(parent, a);
        let rb = find(parent, b);
        if ra != rb {
            parent[ra] = rb;
        }
    }

    let tag_filter = required_tag.map(|t| vec![t.to_string()]);

    for (i, (slug_i, doc_i)) in candidates.iter().enumerate() {
        let filter = SearchFilter {
            kind: Some(ItemKind::Task),
            project: Some(project.to_string()),
            status: None,
            tags: tag_filter.clone(),
            min_score_ratio: Some(0.0),
        };
        let results = crate::search::search(store, &doc_i.frontmatter.title, &filter)?;
        let Some(self_score) = results
            .iter()
            .find(|r| &r.identifier == slug_i)
            .map(|r| r.score)
            .filter(|&s| s > 0)
        else {
            // No self-match (e.g. empty title) or a zero self-score — skip
            // clustering for this candidate rather than dividing by zero.
            continue;
        };
        let threshold = (self_score as f64 * DUPLICATE_SCORE_RATIO) as u32;
        for r in &results {
            if &r.identifier == slug_i {
                continue;
            }
            if r.score < threshold {
                continue;
            }
            // A task's body can be long enough that querying with i's
            // (short) title scores higher against an unrelated task's body
            // than it does against that task's own title — chaining
            // unrelated tasks together through shared vocabulary in prose.
            // `search::score_item` sets `snippet` to the title verbatim
            // only when the *title* match won over the body match (see
            // `crate::search`), so this reuses that existing signal to
            // keep the cluster driven by title similarity, not incidental
            // body overlap.
            if r.snippet != r.title {
                continue;
            }
            if let Some(j) = candidates.iter().position(|(s, _)| s == &r.identifier) {
                union(&mut parent, i, j);
            }
        }
    }

    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> =
        std::collections::BTreeMap::new();
    for i in 0..candidates.len() {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(i);
    }

    let mut clusters: Vec<DuplicateCluster> = groups
        .into_values()
        .filter(|members| members.len() >= 2)
        .map(|members| {
            let mut members: Vec<DuplicateMember> = members
                .into_iter()
                .map(|idx| DuplicateMember {
                    slug: candidates[idx].0.clone(),
                    title: candidates[idx].1.frontmatter.title.clone(),
                })
                .collect();
            members.sort_by(|a, b| a.slug.cmp(&b.slug));
            DuplicateCluster { members }
        })
        .collect();
    clusters.sort_by(|a, b| a.members[0].slug.cmp(&b.members[0].slug));
    Ok(clusters)
}

/// Computes the tag-clusters section, sorted by tag ascending.
fn tag_clusters(
    tasks: &[(String, crate::document::Document<crate::model::Task>)],
) -> Vec<TagCluster> {
    let mut by_tag: std::collections::BTreeMap<String, Vec<TagClusterMember>> =
        std::collections::BTreeMap::new();
    for (slug, doc) in tasks {
        let t = &doc.frontmatter;
        if t.status.is_terminal() {
            continue;
        }
        let Some(tags) = &t.tags else { continue };
        for tag in tags {
            by_tag
                .entry(tag.clone())
                .or_default()
                .push(TagClusterMember {
                    slug: slug.clone(),
                    title: t.title.clone(),
                });
        }
    }
    by_tag
        .into_iter()
        .filter(|(_, members)| members.len() >= 2)
        .map(|(tag, mut tasks)| {
            tasks.sort_by(|a, b| a.slug.cmp(&b.slug));
            TagCluster { tag, tasks }
        })
        .collect()
}

/// Computes the archivable-roadmaps section, sorted by roadmap slug
/// ascending.
fn archivable_roadmaps(
    store: &impl Store,
    project: &str,
    roadmaps: &[crate::document::Document<crate::model::Roadmap>],
) -> Result<Vec<ArchivableRoadmap>> {
    let mut out = Vec::new();
    for doc in roadmaps {
        let slug = &doc.frontmatter.roadmap;
        let phases = crate::ops::phase::list_phases(store, project, slug)?;
        let statuses: Vec<_> = phases.iter().map(|(_, d)| d.frontmatter.status).collect();
        if computed_status(&statuses) == RoadmapStatus::Done {
            out.push(ArchivableRoadmap {
                roadmap: slug.clone(),
                title: doc.frontmatter.title.clone(),
                phase_count: phases.len(),
            });
        }
    }
    out.sort_by(|a, b| a.roadmap.cmp(&b.roadmap));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::model::{Priority, TaskStatus};
    use crate::store::MemoryStore;

    /// Sets up a store with a single project, ready for task/roadmap setup.
    fn setup_store() -> MemoryStore {
        let mut store = MemoryStore::new();
        crate::ops::init::init(&mut store).unwrap();
        crate::ops::project::create_project(&mut store, "acme", "Acme Corp").unwrap();
        store
    }

    /// Creates a task and backdates its `created` field, returning the slug.
    fn create_backdated_task(
        store: &mut MemoryStore,
        slug: &str,
        title: &str,
        tags: Option<Vec<String>>,
        days_ago: i64,
    ) {
        crate::ops::task::create_task(
            store,
            crate::ops::task::CreateTask {
                project: "acme",
                slug,
                title,
                priority: Priority::Medium,
                tags,
                body: None,
            },
        )
        .unwrap();
        let mut doc = crate::io::load_task(store, "acme", slug).unwrap();
        doc.frontmatter.created =
            chrono::Local::now().date_naive() - chrono::Duration::days(days_ago);
        crate::io::write_task(store, "acme", slug, &doc).unwrap();
    }

    // -- stale tasks --

    #[test]
    fn stale_task_past_threshold_is_flagged() {
        let mut store = setup_store();
        create_backdated_task(&mut store, "old-task", "Old Task", None, 90);
        let report = report(&store, "acme", &ReportOptions::default()).unwrap();
        assert!(report.stale_tasks.iter().any(|t| t.slug == "old-task"));
    }

    #[test]
    fn fresh_task_under_threshold_not_flagged() {
        let mut store = setup_store();
        create_backdated_task(&mut store, "new-task", "New Task", None, 5);
        let report = report(&store, "acme", &ReportOptions::default()).unwrap();
        assert!(!report.stale_tasks.iter().any(|t| t.slug == "new-task"));
    }

    #[test]
    fn stale_boundary_inclusive_at_exact_threshold() {
        let mut store = setup_store();
        create_backdated_task(&mut store, "boundary-task", "Boundary Task", None, 60);
        let report = report(&store, "acme", &ReportOptions::default()).unwrap();
        assert!(report.stale_tasks.iter().any(|t| t.slug == "boundary-task"));
    }

    #[test]
    fn in_progress_task_past_threshold_flagged() {
        let mut store = setup_store();
        create_backdated_task(&mut store, "wip-task", "WIP Task", None, 90);
        crate::ops::task::update_task(
            &mut store,
            "acme",
            "wip-task",
            Some(TaskStatus::InProgress),
            None,
            crate::ops::TagsUpdate::Keep,
            crate::ops::BodyUpdate::Keep,
            None,
            None,
            None,
            crate::ops::TitleUpdate::Keep,
        )
        .unwrap();
        let report = report(&store, "acme", &ReportOptions::default()).unwrap();
        assert!(report.stale_tasks.iter().any(|t| t.slug == "wip-task"));
    }

    #[test]
    fn needs_review_or_reviewed_never_stale() {
        let mut store = setup_store();
        create_backdated_task(&mut store, "nr-task", "NR Task", None, 90);
        crate::ops::task::update_task(
            &mut store,
            "acme",
            "nr-task",
            Some(TaskStatus::NeedsReview),
            None,
            crate::ops::TagsUpdate::Keep,
            crate::ops::BodyUpdate::Keep,
            None,
            None,
            None,
            crate::ops::TitleUpdate::Keep,
        )
        .unwrap();
        let report = report(&store, "acme", &ReportOptions::default()).unwrap();
        assert!(!report.stale_tasks.iter().any(|t| t.slug == "nr-task"));
    }

    #[test]
    fn terminal_task_never_stale() {
        let mut store = setup_store();
        create_backdated_task(&mut store, "done-task", "Done Task", None, 90);
        crate::ops::task::update_task(
            &mut store,
            "acme",
            "done-task",
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
        let report = report(&store, "acme", &ReportOptions::default()).unwrap();
        assert!(!report.stale_tasks.iter().any(|t| t.slug == "done-task"));
    }

    #[test]
    fn custom_older_than_narrows_widens() {
        let mut store = setup_store();
        create_backdated_task(&mut store, "mid-task", "Mid Task", None, 30);
        let narrow = ReportOptions {
            older_than_days: 10,
            tag: None,
        };
        let wide = ReportOptions {
            older_than_days: 100,
            tag: None,
        };
        let narrow_report = report(&store, "acme", &narrow).unwrap();
        let wide_report = report(&store, "acme", &wide).unwrap();
        assert!(
            narrow_report
                .stale_tasks
                .iter()
                .any(|t| t.slug == "mid-task")
        );
        assert!(!wide_report.stale_tasks.iter().any(|t| t.slug == "mid-task"));
    }

    #[test]
    fn default_older_than_is_60() {
        assert_eq!(DEFAULT_STALE_THRESHOLD_DAYS, 60);
        assert_eq!(ReportOptions::default().older_than_days, 60);
    }

    // -- duplicate clusters --

    #[test]
    fn near_duplicate_titles_cluster() {
        let mut store = setup_store();
        crate::ops::task::create_task(
            &mut store,
            crate::ops::task::CreateTask {
                project: "acme",
                slug: "fix-login-bug",
                title: "Fix login bug on mobile",
                priority: Priority::Medium,
                tags: None,
                body: None,
            },
        )
        .unwrap();
        crate::ops::task::create_task(
            &mut store,
            crate::ops::task::CreateTask {
                project: "acme",
                slug: "fix-login-bug-2",
                title: "Fix login bug on mobile devices",
                priority: Priority::Medium,
                tags: None,
                body: None,
            },
        )
        .unwrap();
        crate::ops::task::create_task(
            &mut store,
            crate::ops::task::CreateTask {
                project: "acme",
                slug: "unrelated-task",
                title: "Write quarterly financial report",
                priority: Priority::Medium,
                tags: None,
                body: None,
            },
        )
        .unwrap();

        let report = report(&store, "acme", &ReportOptions::default()).unwrap();
        assert_eq!(
            report.duplicate_clusters.len(),
            1,
            "expected exactly one cluster, got: {:?}",
            report.duplicate_clusters
        );
        let members: Vec<&str> = report.duplicate_clusters[0]
            .members
            .iter()
            .map(|m| m.slug.as_str())
            .collect();
        assert!(members.contains(&"fix-login-bug"));
        assert!(members.contains(&"fix-login-bug-2"));
        assert!(!members.contains(&"unrelated-task"));
    }

    #[test]
    fn duplicate_cluster_transitive_grouping() {
        let mut store = setup_store();
        for (slug, title) in [
            ("auth-a", "Fix authentication timeout error"),
            ("auth-b", "Fix authentication timeout errors"),
            ("auth-c", "Fix authentication timeout errors urgently"),
        ] {
            crate::ops::task::create_task(
                &mut store,
                crate::ops::task::CreateTask {
                    project: "acme",
                    slug,
                    title,
                    priority: Priority::Medium,
                    tags: None,
                    body: None,
                },
            )
            .unwrap();
        }
        let report = report(&store, "acme", &ReportOptions::default()).unwrap();
        assert_eq!(
            report.duplicate_clusters.len(),
            1,
            "expected one transitive cluster, got: {:?}",
            report.duplicate_clusters
        );
        assert_eq!(report.duplicate_clusters[0].members.len(), 3);
    }

    #[test]
    fn terminal_tasks_excluded_from_clustering() {
        let mut store = setup_store();
        crate::ops::task::create_task(
            &mut store,
            crate::ops::task::CreateTask {
                project: "acme",
                slug: "dup-1",
                title: "Refactor payment gateway module",
                priority: Priority::Medium,
                tags: None,
                body: None,
            },
        )
        .unwrap();
        crate::ops::task::create_task(
            &mut store,
            crate::ops::task::CreateTask {
                project: "acme",
                slug: "dup-2",
                title: "Refactor payment gateway modules",
                priority: Priority::Medium,
                tags: None,
                body: None,
            },
        )
        .unwrap();
        crate::ops::task::update_task(
            &mut store,
            "acme",
            "dup-2",
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
        let report = report(&store, "acme", &ReportOptions::default()).unwrap();
        assert!(
            report.duplicate_clusters.is_empty(),
            "a terminal task must not participate in clustering: {:?}",
            report.duplicate_clusters
        );
    }

    #[test]
    fn duplicate_clusters_respect_tag_scope() {
        let mut store = setup_store();
        crate::ops::task::create_task(
            &mut store,
            crate::ops::task::CreateTask {
                project: "acme",
                slug: "scoped-1",
                title: "Improve search relevance ranking",
                priority: Priority::Medium,
                tags: Some(vec!["search".to_string()]),
                body: None,
            },
        )
        .unwrap();
        crate::ops::task::create_task(
            &mut store,
            crate::ops::task::CreateTask {
                project: "acme",
                slug: "scoped-2",
                title: "Improve search relevance rankings",
                priority: Priority::Medium,
                tags: None,
                body: None,
            },
        )
        .unwrap();
        let opts = ReportOptions {
            older_than_days: DEFAULT_STALE_THRESHOLD_DAYS,
            tag: Some("search".to_string()),
        };
        let report = report(&store, "acme", &opts).unwrap();
        assert!(
            report.duplicate_clusters.is_empty(),
            "untagged candidate must be excluded by tag scope: {:?}",
            report.duplicate_clusters
        );
    }

    #[test]
    fn unrelated_task_with_long_body_does_not_chain_via_body_match() {
        // Regression test: a short title's fuzzy subsequence can match
        // somewhere inside an unrelated task's long body, scoring higher
        // than it does against that task's own (different) title. Without
        // filtering matches to those actually driven by title similarity,
        // this chains unrelated tasks into the same cluster.
        let mut store = setup_store();
        crate::ops::task::create_task(
            &mut store,
            crate::ops::task::CreateTask {
                project: "acme",
                slug: "fix-login-bug",
                title: "Fix login bug on mobile",
                priority: Priority::Medium,
                tags: None,
                body: None,
            },
        )
        .unwrap();
        crate::ops::task::create_task(
            &mut store,
            crate::ops::task::CreateTask {
                project: "acme",
                slug: "fix-login-bug-2",
                title: "Fix login bug on mobile devices",
                priority: Priority::Medium,
                tags: None,
                body: None,
            },
        )
        .unwrap();
        crate::ops::task::create_task(
            &mut store,
            crate::ops::task::CreateTask {
                project: "acme",
                slug: "unrelated-long-body",
                title: "Write quarterly financial report",
                priority: Priority::Medium,
                tags: None,
                body: Some(
                    "A long unrelated body about accounting, invoices, revenue \
                     recognition, budget forecasting, and quarterly numbers that \
                     has nothing to do with login or mobile devices at all, just \
                     general business finance narrative detail padding this out.",
                ),
            },
        )
        .unwrap();

        let report = report(&store, "acme", &ReportOptions::default()).unwrap();
        assert_eq!(
            report.duplicate_clusters.len(),
            1,
            "expected exactly the login-bug cluster, got: {:?}",
            report.duplicate_clusters
        );
        let members: Vec<&str> = report.duplicate_clusters[0]
            .members
            .iter()
            .map(|m| m.slug.as_str())
            .collect();
        assert!(members.contains(&"fix-login-bug"));
        assert!(members.contains(&"fix-login-bug-2"));
        assert!(
            !members.contains(&"unrelated-long-body"),
            "a long, unrelated body must not chain a task into the cluster: {:?}",
            report.duplicate_clusters
        );
    }

    #[test]
    fn moderately_similar_distinct_titles_do_not_cluster() {
        // Pins the discriminating power of DUPLICATE_SCORE_RATIO at the margin:
        // two active tasks that share vocabulary but describe distinct work must
        // NOT be flagged as duplicates. Without this, a loosened ratio (or a
        // scoring change) that started merging overlapping-but-distinct tasks
        // would still pass every other duplicate test, since those only cover
        // near-identical (one trailing word) positives and wildly-dissimilar
        // negatives — nothing in the 0.0..RATIO band that this guards.
        let mut store = setup_store();
        crate::ops::task::create_task(
            &mut store,
            crate::ops::task::CreateTask {
                project: "acme",
                slug: "dark-mode-toggle",
                title: "Add dark mode toggle to settings screen",
                priority: Priority::Medium,
                tags: None,
                body: None,
            },
        )
        .unwrap();
        crate::ops::task::create_task(
            &mut store,
            crate::ops::task::CreateTask {
                project: "acme",
                slug: "export-csv",
                title: "Add CSV export button to reports screen",
                priority: Priority::Medium,
                tags: None,
                body: None,
            },
        )
        .unwrap();
        let report = report(&store, "acme", &ReportOptions::default()).unwrap();
        assert!(
            report.duplicate_clusters.is_empty(),
            "distinct tasks that merely share vocabulary must not cluster: {:?}",
            report.duplicate_clusters
        );
    }

    // -- tag clusters --

    #[test]
    fn tag_cluster_groups_sharing_tag() {
        let mut store = setup_store();
        for slug in ["t1", "t2"] {
            crate::ops::task::create_task(
                &mut store,
                crate::ops::task::CreateTask {
                    project: "acme",
                    slug,
                    title: slug,
                    priority: Priority::Medium,
                    tags: Some(vec!["billing".to_string()]),
                    body: None,
                },
            )
            .unwrap();
        }
        let report = report(&store, "acme", &ReportOptions::default()).unwrap();
        let cluster = report
            .tag_clusters
            .iter()
            .find(|c| c.tag == "billing")
            .expect("billing cluster expected");
        assert_eq!(cluster.tasks.len(), 2);
    }

    #[test]
    fn tag_cluster_excludes_terminal() {
        let mut store = setup_store();
        for slug in ["t1", "t2"] {
            crate::ops::task::create_task(
                &mut store,
                crate::ops::task::CreateTask {
                    project: "acme",
                    slug,
                    title: slug,
                    priority: Priority::Medium,
                    tags: Some(vec!["billing".to_string()]),
                    body: None,
                },
            )
            .unwrap();
        }
        crate::ops::task::update_task(
            &mut store,
            "acme",
            "t2",
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
        let report = report(&store, "acme", &ReportOptions::default()).unwrap();
        assert!(
            !report.tag_clusters.iter().any(|c| c.tag == "billing"),
            "only one active task carries billing; must not cluster: {:?}",
            report.tag_clusters
        );
    }

    #[test]
    fn untagged_never_in_tag_clusters() {
        let mut store = setup_store();
        crate::ops::task::create_task(
            &mut store,
            crate::ops::task::CreateTask {
                project: "acme",
                slug: "untagged",
                title: "Untagged",
                priority: Priority::Medium,
                tags: None,
                body: None,
            },
        )
        .unwrap();
        let report = report(&store, "acme", &ReportOptions::default()).unwrap();
        assert!(report.tag_clusters.is_empty());
    }

    #[test]
    fn single_task_unique_tag_not_clustered() {
        let mut store = setup_store();
        crate::ops::task::create_task(
            &mut store,
            crate::ops::task::CreateTask {
                project: "acme",
                slug: "lonely",
                title: "Lonely",
                priority: Priority::Medium,
                tags: Some(vec!["unique".to_string()]),
                body: None,
            },
        )
        .unwrap();
        let report = report(&store, "acme", &ReportOptions::default()).unwrap();
        assert!(!report.tag_clusters.iter().any(|c| c.tag == "unique"));
    }

    #[test]
    fn tag_clusters_respect_tag_scope() {
        // The `--tag` scope must restrict tag clustering to the scoped tasks:
        // a cluster for a tag carried only by out-of-scope tasks must not appear.
        let mut store = setup_store();
        for slug in ["billing-1", "billing-2"] {
            crate::ops::task::create_task(
                &mut store,
                crate::ops::task::CreateTask {
                    project: "acme",
                    slug,
                    title: slug,
                    priority: Priority::Medium,
                    tags: Some(vec!["billing".to_string()]),
                    body: None,
                },
            )
            .unwrap();
        }
        for slug in ["auth-1", "auth-2"] {
            crate::ops::task::create_task(
                &mut store,
                crate::ops::task::CreateTask {
                    project: "acme",
                    slug,
                    title: slug,
                    priority: Priority::Medium,
                    tags: Some(vec!["auth".to_string()]),
                    body: None,
                },
            )
            .unwrap();
        }
        // Without scoping, both a billing and an auth cluster exist.
        let unscoped = report(&store, "acme", &ReportOptions::default()).unwrap();
        assert!(unscoped.tag_clusters.iter().any(|c| c.tag == "billing"));
        assert!(unscoped.tag_clusters.iter().any(|c| c.tag == "auth"));
        // Scoped to "auth": the billing tasks are out of scope, so no billing
        // cluster may appear; only the in-scope auth cluster survives.
        let opts = ReportOptions {
            older_than_days: DEFAULT_STALE_THRESHOLD_DAYS,
            tag: Some("auth".to_string()),
        };
        let scoped = report(&store, "acme", &opts).unwrap();
        assert!(
            !scoped.tag_clusters.iter().any(|c| c.tag == "billing"),
            "out-of-scope tag must not cluster under --tag auth: {:?}",
            scoped.tag_clusters
        );
        assert!(
            scoped.tag_clusters.iter().any(|c| c.tag == "auth"),
            "in-scope auth cluster must survive scoping: {:?}",
            scoped.tag_clusters
        );
    }

    // -- archivable roadmaps --

    #[test]
    fn fully_terminal_roadmap_flagged() {
        let mut store = setup_store();
        crate::ops::roadmap::create_roadmap(
            &mut store,
            crate::ops::roadmap::CreateRoadmap {
                project: "acme",
                slug: "done-roadmap",
                title: "Done Roadmap",
                ..Default::default()
            },
        )
        .unwrap();
        crate::ops::phase::create_phase(
            &mut store,
            crate::ops::phase::CreatePhase {
                project: "acme",
                roadmap: "done-roadmap",
                slug: "only",
                title: "Only Phase",
                number: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        crate::ops::phase::update_phase(
            &mut store,
            "acme",
            "done-roadmap",
            "phase-1-only",
            Some(crate::model::PhaseStatus::Done),
            crate::ops::TagsUpdate::Keep,
            crate::ops::BodyUpdate::Keep,
            None,
            None,
            None,
            crate::ops::TitleUpdate::Keep,
        )
        .unwrap();
        let report = report(&store, "acme", &ReportOptions::default()).unwrap();
        assert!(
            report
                .archivable_roadmaps
                .iter()
                .any(|r| r.roadmap == "done-roadmap"),
            "got: {:?}",
            report.archivable_roadmaps
        );
    }

    #[test]
    fn wont_fix_counts_as_terminal() {
        let mut store = setup_store();
        crate::ops::roadmap::create_roadmap(
            &mut store,
            crate::ops::roadmap::CreateRoadmap {
                project: "acme",
                slug: "wf-roadmap",
                title: "WF Roadmap",
                ..Default::default()
            },
        )
        .unwrap();
        crate::ops::phase::create_phase(
            &mut store,
            crate::ops::phase::CreatePhase {
                project: "acme",
                roadmap: "wf-roadmap",
                slug: "only",
                title: "Only Phase",
                number: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        crate::ops::phase::update_phase(
            &mut store,
            "acme",
            "wf-roadmap",
            "phase-1-only",
            Some(crate::model::PhaseStatus::WontFix),
            crate::ops::TagsUpdate::Keep,
            crate::ops::BodyUpdate::Keep,
            None,
            None,
            None,
            crate::ops::TitleUpdate::Keep,
        )
        .unwrap();
        let report = report(&store, "acme", &ReportOptions::default()).unwrap();
        assert!(
            report
                .archivable_roadmaps
                .iter()
                .any(|r| r.roadmap == "wf-roadmap")
        );
    }

    #[test]
    fn partially_done_not_archivable() {
        let mut store = setup_store();
        crate::ops::roadmap::create_roadmap(
            &mut store,
            crate::ops::roadmap::CreateRoadmap {
                project: "acme",
                slug: "partial-roadmap",
                title: "Partial Roadmap",
                ..Default::default()
            },
        )
        .unwrap();
        crate::ops::phase::create_phase(
            &mut store,
            crate::ops::phase::CreatePhase {
                project: "acme",
                roadmap: "partial-roadmap",
                slug: "one",
                title: "Phase One",
                number: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        crate::ops::phase::create_phase(
            &mut store,
            crate::ops::phase::CreatePhase {
                project: "acme",
                roadmap: "partial-roadmap",
                slug: "two",
                title: "Phase Two",
                number: Some(2),
                ..Default::default()
            },
        )
        .unwrap();
        crate::ops::phase::update_phase(
            &mut store,
            "acme",
            "partial-roadmap",
            "phase-1-one",
            Some(crate::model::PhaseStatus::Done),
            crate::ops::TagsUpdate::Keep,
            crate::ops::BodyUpdate::Keep,
            None,
            None,
            None,
            crate::ops::TitleUpdate::Keep,
        )
        .unwrap();
        let report = report(&store, "acme", &ReportOptions::default()).unwrap();
        assert!(
            !report
                .archivable_roadmaps
                .iter()
                .any(|r| r.roadmap == "partial-roadmap")
        );
    }

    #[test]
    fn roadmap_with_no_phases_not_archivable() {
        let mut store = setup_store();
        crate::ops::roadmap::create_roadmap(
            &mut store,
            crate::ops::roadmap::CreateRoadmap {
                project: "acme",
                slug: "empty-roadmap",
                title: "Empty Roadmap",
                ..Default::default()
            },
        )
        .unwrap();
        let report = report(&store, "acme", &ReportOptions::default()).unwrap();
        assert!(
            !report
                .archivable_roadmaps
                .iter()
                .any(|r| r.roadmap == "empty-roadmap")
        );
    }

    #[test]
    fn already_archived_excluded() {
        let mut store = setup_store();
        crate::ops::roadmap::create_roadmap(
            &mut store,
            crate::ops::roadmap::CreateRoadmap {
                project: "acme",
                slug: "archive-me",
                title: "Archive Me",
                ..Default::default()
            },
        )
        .unwrap();
        crate::ops::phase::create_phase(
            &mut store,
            crate::ops::phase::CreatePhase {
                project: "acme",
                roadmap: "archive-me",
                slug: "only",
                title: "Only Phase",
                number: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        crate::ops::phase::update_phase(
            &mut store,
            "acme",
            "archive-me",
            "phase-1-only",
            Some(crate::model::PhaseStatus::Done),
            crate::ops::TagsUpdate::Keep,
            crate::ops::BodyUpdate::Keep,
            None,
            None,
            None,
            crate::ops::TitleUpdate::Keep,
        )
        .unwrap();

        // Sanity: flagged before archiving.
        let before = report(&store, "acme", &ReportOptions::default()).unwrap();
        assert!(
            before
                .archivable_roadmaps
                .iter()
                .any(|r| r.roadmap == "archive-me")
        );

        crate::ops::roadmap::archive_roadmap(&mut store, "acme", "archive-me", false).unwrap();

        let after = report(&store, "acme", &ReportOptions::default()).unwrap();
        assert!(
            !after
                .archivable_roadmaps
                .iter()
                .any(|r| r.roadmap == "archive-me"),
            "archived roadmap must not appear: {:?}",
            after.archivable_roadmaps
        );
    }

    #[test]
    fn archivable_respect_tag_scope() {
        let mut store = setup_store();
        crate::ops::roadmap::create_roadmap(
            &mut store,
            crate::ops::roadmap::CreateRoadmap {
                project: "acme",
                slug: "tagged-done",
                title: "Tagged Done",
                tags: Some(vec!["legacy".to_string()]),
                ..Default::default()
            },
        )
        .unwrap();
        crate::ops::phase::create_phase(
            &mut store,
            crate::ops::phase::CreatePhase {
                project: "acme",
                roadmap: "tagged-done",
                slug: "only",
                title: "Only Phase",
                number: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        crate::ops::phase::update_phase(
            &mut store,
            "acme",
            "tagged-done",
            "phase-1-only",
            Some(crate::model::PhaseStatus::Done),
            crate::ops::TagsUpdate::Keep,
            crate::ops::BodyUpdate::Keep,
            None,
            None,
            None,
            crate::ops::TitleUpdate::Keep,
        )
        .unwrap();

        let opts = ReportOptions {
            older_than_days: DEFAULT_STALE_THRESHOLD_DAYS,
            tag: Some("nonexistent-tag".to_string()),
        };
        let report = report(&store, "acme", &opts).unwrap();
        assert!(
            !report
                .archivable_roadmaps
                .iter()
                .any(|r| r.roadmap == "tagged-done"),
            "tag scope must exclude untagged-match roadmap"
        );
    }

    // -- general --

    #[test]
    fn empty_project_returns_empty_report() {
        let store = setup_store();
        let report = report(&store, "acme", &ReportOptions::default()).unwrap();
        assert!(report.stale_tasks.is_empty());
        assert!(report.duplicate_clusters.is_empty());
        assert!(report.tag_clusters.is_empty());
        assert!(report.archivable_roadmaps.is_empty());
    }

    #[test]
    fn report_errors_on_missing_project() {
        let store = MemoryStore::new();
        let err = report(&store, "nope", &ReportOptions::default()).unwrap_err();
        assert!(matches!(err, Error::ProjectNotFound(_)));
    }
}
