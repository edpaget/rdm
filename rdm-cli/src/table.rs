use rdm_core::display::RoadmapWithPhases;
use rdm_core::document::Document;
#[cfg(feature = "git")]
use rdm_core::model::Review;
use rdm_core::model::{Phase, Task};
use rdm_core::search::SearchResult;
use tabled::builder::Builder;
use tabled::settings::peaker::Priority;
use tabled::settings::{Style, Width};

fn terminal_width() -> usize {
    terminal_size::terminal_size()
        .map(|(w, _)| w.0 as usize)
        .unwrap_or(120)
}

/// Renders roadmaps as a table.
///
/// A trailing `Tags` column is appended after `Priority` only when at least
/// one listed roadmap carries a non-empty tag list; untagged rows render an
/// empty cell.
pub fn format_roadmap_table(entries: &[RoadmapWithPhases]) -> String {
    if entries.is_empty() {
        return "No roadmaps found.\n".to_string();
    }
    let show_tags = entries
        .iter()
        .any(|(doc, _)| has_tags(&doc.frontmatter.tags));
    let rows = entries
        .iter()
        .map(|(doc, phases)| {
            let total = phases.len();
            let done = phases
                .iter()
                .filter(|(_, p)| p.frontmatter.status.is_terminal())
                .count();
            let mut row = vec![
                doc.frontmatter.roadmap.clone(),
                doc.frontmatter.title.clone(),
                format!("{done}/{total} phases done"),
                doc.frontmatter
                    .priority
                    .map(|p| p.to_string())
                    .unwrap_or_default(),
            ];
            if show_tags {
                row.push(join_tags(doc.frontmatter.tags.as_deref()));
            }
            row
        })
        .collect();
    let mut headers = vec!["Slug", "Title", "Progress", "Priority"];
    if show_tags {
        headers.push("Tags");
    }
    build_table_dyn(headers, rows)
}

pub fn format_phase_table(phases: &[(String, Document<Phase>)]) -> String {
    if phases.is_empty() {
        return "No phases yet.\n".to_string();
    }
    let rows = phases
        .iter()
        .map(|(stem, doc)| {
            [
                doc.frontmatter.phase.to_string(),
                doc.frontmatter.title.clone(),
                doc.frontmatter.status.to_string(),
                stem.clone(),
            ]
        })
        .collect();
    build_table(["#", "Phase", "Status", "Stem"], rows)
}

/// Renders tasks as a table.
///
/// A trailing `Tags` column is appended after `Priority` only when at least
/// one listed task carries a non-empty tag list; untagged rows render an
/// empty cell.
pub fn format_task_table(tasks: &[(String, Document<Task>)]) -> String {
    if tasks.is_empty() {
        return "No tasks found.\n".to_string();
    }
    let show_tags = tasks.iter().any(|(_, doc)| has_tags(&doc.frontmatter.tags));
    let rows = tasks
        .iter()
        .map(|(slug, doc)| {
            let mut row = vec![
                slug.clone(),
                doc.frontmatter.title.clone(),
                doc.frontmatter.status.to_string(),
                doc.frontmatter.priority.to_string(),
            ];
            if show_tags {
                row.push(join_tags(doc.frontmatter.tags.as_deref()));
            }
            row
        })
        .collect();
    let mut headers = vec!["Slug", "Title", "Status", "Priority"];
    if show_tags {
        headers.push("Tags");
    }
    build_table_dyn(headers, rows)
}

#[cfg(feature = "git")]
pub fn format_review_table(reviews: &[(String, Document<Review>)]) -> String {
    if reviews.is_empty() {
        return "No reviews found.\n".to_string();
    }
    let rows = reviews
        .iter()
        .map(|(id, doc)| {
            let fm = &doc.frontmatter;
            let open = fm
                .comments
                .iter()
                .filter(|c| !c.status.is_terminal())
                .count();
            [
                id.clone(),
                fm.target.label(),
                fm.state.to_string(),
                fm.verdict.map(|v| v.to_string()).unwrap_or_default(),
                fm.author.clone(),
                format!("{open}/{} open", fm.comments.len()),
            ]
        })
        .collect();
    build_table(
        ["Id", "Target", "State", "Verdict", "Author", "Comments"],
        rows,
    )
}

pub fn format_search_table(results: &[SearchResult]) -> String {
    if results.is_empty() {
        return "No results found.\n".to_string();
    }
    let rows = results
        .iter()
        .enumerate()
        .map(|(i, r)| {
            [
                (i + 1).to_string(),
                format!("{:?}", r.kind),
                r.title.clone(),
                r.identifier.clone(),
                r.snippet.clone(),
            ]
        })
        .collect();
    build_table(["#", "Type", "Title", "Identifier", "Snippet"], rows)
}

/// Returns whether `tags` holds at least one tag — the gate for emitting a
/// conditional trailing `Tags` column.
fn has_tags(tags: &Option<Vec<String>>) -> bool {
    tags.as_ref().is_some_and(|t| !t.is_empty())
}

/// Renders a tag list as a comma-space joined cell (`bug, ui`); absent or
/// empty tags render as the empty string.
fn join_tags(tags: Option<&[String]>) -> String {
    tags.map(|t| t.join(", ")).unwrap_or_default()
}

pub fn build_table<const N: usize>(headers: [&str; N], rows: Vec<[String; N]>) -> String {
    build_table_dyn(headers.to_vec(), rows.into_iter().map(Vec::from).collect())
}

/// Builds a table with a column count only known at runtime.
///
/// Same styling and truncation policy as [`build_table`]; used by the views
/// that append a conditional trailing `Tags` column, which cannot be expressed
/// with the const-generic column count.
pub fn build_table_dyn(headers: Vec<&str>, rows: Vec<Vec<String>>) -> String {
    let mut builder = Builder::default();
    builder.push_record(headers);
    for row in rows {
        builder.push_record(row);
    }
    let mut table = builder.build();
    table
        .with(Style::rounded())
        .with(Width::truncate(terminal_width()).priority(Priority::max(false)));
    format!("{table}\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rdm_core::model::{
        Review, ReviewComment, ReviewCommentStatus, ReviewState, ReviewTarget, Verdict,
    };

    fn task_doc(slug: &str, tags: Option<Vec<String>>) -> (String, Document<Task>) {
        use rdm_core::model::{Priority, TaskStatus};
        let doc = Document {
            frontmatter: Task {
                project: "fbm".to_string(),
                title: slug.to_string(),
                status: TaskStatus::Open,
                priority: Priority::Medium,
                created: chrono::NaiveDate::from_ymd_opt(2026, 3, 15).unwrap(),
                tags,
                completed: None,
                commit: None,
                review_sha: None,
                review_branch: None,
                close_reason: None,
            },
            body: String::new(),
        };
        (slug.to_string(), doc)
    }

    fn roadmap_entry(slug: &str, tags: Option<Vec<String>>) -> RoadmapWithPhases {
        use rdm_core::model::Roadmap;
        let doc = Document {
            frontmatter: Roadmap {
                project: "fbm".to_string(),
                roadmap: slug.to_string(),
                title: slug.to_string(),
                phases: Vec::new(),
                dependencies: None,
                priority: None,
                tags,
            },
            body: String::new(),
        };
        (doc, Vec::new())
    }

    #[test]
    fn task_table_appends_tags_column_only_when_tagged() {
        let untagged = vec![task_doc("plain", None)];
        let out = format_task_table(&untagged);
        assert!(!out.contains("Tags"), "unexpected Tags column in:\n{out}");

        let mixed = vec![
            task_doc("tagged", Some(vec!["bug".to_string(), "ui".to_string()])),
            task_doc("plain", None),
        ];
        let out = format_task_table(&mixed);
        assert!(out.contains("Tags"), "missing Tags column in:\n{out}");
        assert!(out.contains("bug, ui"), "missing joined tags in:\n{out}");
    }

    #[test]
    fn roadmap_table_appends_tags_column_only_when_tagged() {
        let untagged = vec![roadmap_entry("plain-rm", None)];
        let out = format_roadmap_table(&untagged);
        assert!(!out.contains("Tags"), "unexpected Tags column in:\n{out}");

        let mixed = vec![
            roadmap_entry("tagged-rm", Some(vec!["bug".to_string(), "ui".to_string()])),
            roadmap_entry("plain-rm", None),
        ];
        let out = format_roadmap_table(&mixed);
        assert!(out.contains("Tags"), "missing Tags column in:\n{out}");
        assert!(out.contains("bug, ui"), "missing joined tags in:\n{out}");
    }

    #[test]
    fn tags_column_omitted_for_empty_tag_lists() {
        let tasks = vec![task_doc("empty", Some(vec![]))];
        let out = format_task_table(&tasks);
        assert!(!out.contains("Tags"), "unexpected Tags column in:\n{out}");
    }

    fn review_doc() -> (String, Document<Review>) {
        let doc = Document {
            frontmatter: Review {
                id: "rev-1".to_string(),
                author: "ed".to_string(),
                target: ReviewTarget::Task {
                    slug: "fix-login".to_string(),
                },
                state: ReviewState::Submitted,
                verdict: Some(Verdict::RequestChanges),
                created: chrono::Utc.with_ymd_and_hms(2026, 7, 2, 12, 0, 0).unwrap(),
                submitted: None,
                created_commit: None,
                comments: vec![
                    ReviewComment {
                        id: 1,
                        doc: None,
                        status: ReviewCommentStatus::Open,
                        applied_commit: None,
                        anchor: None,
                        body: "Tighten this.".to_string(),
                        reply: None,
                    },
                    ReviewComment {
                        id: 2,
                        doc: None,
                        status: ReviewCommentStatus::Addressed,
                        applied_commit: None,
                        anchor: None,
                        body: "Handled.".to_string(),
                        reply: None,
                    },
                ],
            },
            body: String::new(),
        };
        ("rev-1".to_string(), doc)
    }

    #[test]
    fn review_table_renders_headers_and_row_fields() {
        let out = format_review_table(&[review_doc()]);
        for header in ["Id", "Target", "State", "Verdict", "Author", "Comments"] {
            assert!(out.contains(header), "missing header {header:?} in:\n{out}");
        }
        assert!(out.contains("rev-1"), "missing id in:\n{out}");
        assert!(out.contains("task/fix-login"), "missing target in:\n{out}");
        assert!(out.contains("submitted"), "missing state in:\n{out}");
        assert!(
            out.contains("request-changes"),
            "missing verdict in:\n{out}"
        );
        assert!(out.contains("ed"), "missing author in:\n{out}");
        assert!(
            out.contains("1/2 open"),
            "missing open/total comment count in:\n{out}"
        );
    }

    #[test]
    fn review_table_empty_prints_placeholder() {
        assert_eq!(format_review_table(&[]), "No reviews found.\n");
    }
}
