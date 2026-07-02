/// Generic document wrapper combining YAML frontmatter with a markdown body.
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::markdown;

/// A document with typed frontmatter and a free-form markdown body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document<T> {
    /// The parsed frontmatter.
    pub frontmatter: T,
    /// The markdown body content.
    pub body: String,
}

impl<T> Document<T>
where
    T: for<'de> Deserialize<'de>,
{
    /// Parses a markdown string with YAML frontmatter into a `Document`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::FrontmatterMissing`] if the content lacks frontmatter
    /// delimiters, or [`Error::FrontmatterParse`] if the YAML cannot be
    /// deserialized into `T`.
    pub fn parse(content: &str) -> Result<Self> {
        let (yaml, body) = markdown::split_frontmatter(content)?;
        let frontmatter: T = serde_yaml::from_str(yaml)?;
        Ok(Document {
            frontmatter,
            body: body.to_string(),
        })
    }
}

impl<T> Document<T>
where
    T: Serialize,
{
    /// Renders the document back to a markdown string with YAML frontmatter.
    ///
    /// # Errors
    ///
    /// Returns [`Error::FrontmatterParse`] if the frontmatter cannot be
    /// serialized to YAML.
    pub fn render(&self) -> Result<String> {
        let yaml = serde_yaml::to_string(&self.frontmatter)?;
        Ok(markdown::join_frontmatter(&yaml, &self.body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Phase, PhaseStatus, Priority, Task, TaskStatus};
    use chrono::NaiveDate;

    #[test]
    fn parse_phase_document() {
        let content = "---\nphase: 1\ntitle: Core valuation\nstatus: done\ncompleted: 2026-03-13\n---\n\n## Context\n\nSome details.\n";
        let doc: Document<Phase> = Document::parse(content).unwrap();
        assert_eq!(doc.frontmatter.phase, 1);
        assert_eq!(doc.frontmatter.title, "Core valuation");
        assert_eq!(doc.frontmatter.status, PhaseStatus::Done);
        assert_eq!(
            doc.frontmatter.completed,
            Some(NaiveDate::from_ymd_opt(2026, 3, 13).unwrap())
        );
        assert_eq!(doc.body, "## Context\n\nSome details.\n");
    }

    #[test]
    fn parse_task_document() {
        let content = "---\nproject: fbm\ntitle: Fix bug\nstatus: open\npriority: high\ncreated: 2026-03-14\ntags:\n- data\n---\n\nDetails.\n";
        let doc: Document<Task> = Document::parse(content).unwrap();
        assert_eq!(doc.frontmatter.project, "fbm");
        assert_eq!(doc.frontmatter.status, TaskStatus::Open);
        assert_eq!(doc.frontmatter.priority, Priority::High);
        assert_eq!(doc.frontmatter.tags, Some(vec!["data".to_string()]));
    }

    #[test]
    fn render_phase_document() {
        let doc = Document {
            frontmatter: Phase {
                phase: 1,
                title: "Core valuation".to_string(),
                status: PhaseStatus::Done,
                tags: None,
                completed: Some(NaiveDate::from_ymd_opt(2026, 3, 13).unwrap()),
                commit: None,
                review_sha: None,
                review_branch: None,
                difficulty: None,
                model: None,
                blocked_reason: None,
            },
            body: "## Context\n\nDetails.\n".to_string(),
        };
        let rendered = doc.render().unwrap();
        assert!(rendered.starts_with("---\n"));
        assert!(rendered.contains("phase: 1"));
        assert!(rendered.contains("status: done"));
        assert!(rendered.contains("## Context"));
    }

    #[test]
    fn parse_render_round_trip() {
        let original = Document {
            frontmatter: Phase {
                phase: 2,
                title: "Keeper service".to_string(),
                status: PhaseStatus::NotStarted,
                tags: None,
                completed: None,
                commit: None,
                review_sha: None,
                review_branch: None,
                difficulty: None,
                model: None,
                blocked_reason: None,
            },
            body: "Body text.\n".to_string(),
        };
        let rendered = original.render().unwrap();
        let parsed: Document<Phase> = Document::parse(&rendered).unwrap();
        assert_eq!(parsed.frontmatter, original.frontmatter);
        assert_eq!(parsed.body, original.body);
    }

    #[test]
    fn parse_missing_frontmatter() {
        let content = "No frontmatter here.";
        let result = Document::<Phase>::parse(content);
        assert!(result.is_err());
    }

    // -- Review document tests --

    use crate::model::{
        Anchor, CommentDoc, CommentDocKind, Review, ReviewComment, ReviewCommentStatus,
        ReviewState, ReviewTarget, Verdict,
    };
    use chrono::{TimeZone, Utc};

    #[test]
    fn parse_review_document_phase_target_submitted() {
        let content = r###"---
id: 2026-07-01-1430-a1b2
author: ed.paget@gmail.com
target:
  kind: phase
  roadmap: comments-and-llm-edits
  stem: core-review-model
state: submitted
verdict: request-changes
created: 2026-07-01T14:30:00Z
submitted: 2026-07-01T14:55:00Z
created_commit: a1b2c3d
comments:
  - id: 1
    status: open
    anchor:
      anchor_type: text-quote
      quote: "## Acceptance Criteria"
      prefix: "right?\n\n"
      suffix: "\n\n- [ ] Criterion"
    body: |
      This criterion contradicts step 3 — tighten it.
---

Overall review summary markdown goes here.
"###;
        let doc: Document<Review> = Document::parse(content).unwrap();
        let review = &doc.frontmatter;
        assert_eq!(review.id, "2026-07-01-1430-a1b2");
        assert_eq!(review.author, "ed.paget@gmail.com");
        assert_eq!(
            review.target,
            ReviewTarget::Phase {
                roadmap: "comments-and-llm-edits".to_string(),
                stem: "core-review-model".to_string(),
            }
        );
        assert_eq!(review.state, ReviewState::Submitted);
        assert_eq!(review.verdict, Some(Verdict::RequestChanges));
        assert_eq!(
            review.created,
            Utc.with_ymd_and_hms(2026, 7, 1, 14, 30, 0).unwrap()
        );
        assert_eq!(
            review.submitted,
            Some(Utc.with_ymd_and_hms(2026, 7, 1, 14, 55, 0).unwrap())
        );
        assert_eq!(review.created_commit.as_deref(), Some("a1b2c3d"));
        assert_eq!(review.comments.len(), 1);
        let comment = &review.comments[0];
        assert_eq!(comment.id, 1);
        assert_eq!(comment.doc, None);
        assert_eq!(comment.status, ReviewCommentStatus::Open);
        assert_eq!(comment.applied_commit, None);
        assert_eq!(
            comment.anchor,
            Some(Anchor::TextQuote {
                quote: "## Acceptance Criteria".to_string(),
                prefix: "right?\n\n".to_string(),
                suffix: "\n\n- [ ] Criterion".to_string(),
            })
        );
        // Note: the block scalar's trailing newline is clipped because the
        // frontmatter splitter cuts the YAML at the closing `---` delimiter.
        assert_eq!(
            comment.body,
            "This criterion contradicts step 3 — tighten it."
        );
        assert_eq!(comment.reply, None);
        assert_eq!(doc.body, "Overall review summary markdown goes here.\n");
    }

    #[test]
    fn parse_review_document_draft_omits_verdict_and_submitted() {
        let content = r#"---
id: 2026-07-01-0900-c3d4
author: reviewer-agent
target:
  kind: task
  slug: fix-login
state: draft
created: 2026-07-01T09:00:00Z
comments: []
---

Draft in progress.
"#;
        let doc: Document<Review> = Document::parse(content).unwrap();
        assert_eq!(doc.frontmatter.state, ReviewState::Draft);
        assert_eq!(doc.frontmatter.verdict, None);
        assert_eq!(doc.frontmatter.submitted, None);
        assert_eq!(doc.frontmatter.created_commit, None);
        assert!(doc.frontmatter.comments.is_empty());
    }

    #[test]
    fn parse_review_document_roadmap_target_with_doc_scoped_comment() {
        let content = r#"---
id: 2026-07-01-1000-e5f6
author: ed
target:
  kind: roadmap
  roadmap: comments-and-llm-edits
state: submitted
verdict: comment
created: 2026-07-01T10:00:00Z
submitted: 2026-07-01T10:05:00Z
comments:
  - id: 1
    doc:
      kind: phase
      stem: phase-2-ops
    status: addressed
    applied_commit: f00dfeed
    body: Scoped to a phase of the roadmap.
    reply: Done in f00dfeed.
  - id: 2
    status: wont-fix
    body: Whole-roadmap comment without doc or anchor.
---

Roadmap-level review.
"#;
        let doc: Document<Review> = Document::parse(content).unwrap();
        assert_eq!(
            doc.frontmatter.target,
            ReviewTarget::Roadmap {
                roadmap: "comments-and-llm-edits".to_string()
            }
        );
        let first = &doc.frontmatter.comments[0];
        assert_eq!(
            first.doc,
            Some(CommentDoc {
                kind: CommentDocKind::Phase,
                stem: "phase-2-ops".to_string(),
            })
        );
        assert_eq!(first.status, ReviewCommentStatus::Addressed);
        assert_eq!(first.applied_commit.as_deref(), Some("f00dfeed"));
        assert_eq!(first.reply.as_deref(), Some("Done in f00dfeed."));
        let second = &doc.frontmatter.comments[1];
        assert_eq!(second.doc, None);
        assert_eq!(second.anchor, None);
        assert_eq!(second.status, ReviewCommentStatus::WontFix);
    }

    #[test]
    fn parse_review_document_unknown_anchor_round_trips_losslessly() {
        let content = r#"---
id: 2026-07-01-1100-abcd
author: future-rdm
target:
  kind: task
  slug: fix-login
state: submitted
verdict: approve
created: 2026-07-01T11:00:00Z
submitted: 2026-07-01T11:01:00Z
comments:
  - id: 1
    status: open
    anchor:
      anchor_type: line-range
      start: 3
      end: 7
    body: Anchored by a future anchor type.
---

Summary.
"#;
        let doc: Document<Review> = Document::parse(content).unwrap();
        let anchor = doc.frontmatter.comments[0].anchor.as_ref().unwrap();
        match anchor {
            Anchor::Unknown { anchor_type, raw } => {
                assert_eq!(anchor_type, "line-range");
                assert_eq!(
                    raw.get("start").and_then(serde_yaml::Value::as_i64),
                    Some(3)
                );
                assert_eq!(raw.get("end").and_then(serde_yaml::Value::as_i64), Some(7));
            }
            other => panic!("expected Anchor::Unknown, got {other:?}"),
        }
        // Render and re-parse: the unknown anchor survives verbatim.
        let rendered = doc.render().unwrap();
        let reparsed: Document<Review> = Document::parse(&rendered).unwrap();
        assert_eq!(reparsed.frontmatter, doc.frontmatter);
        assert_eq!(reparsed.body, doc.body);
    }

    #[test]
    fn parse_review_document_dangling_target_parses_without_error() {
        // The target roadmap/stem need not exist anywhere — parsing never
        // consults the store, so renames/deletions cannot corrupt reviews.
        let content = r#"---
id: 2026-07-01-1200-dead
author: ed
target:
  kind: phase
  roadmap: renamed-away
  stem: phase-9-gone
state: submitted
verdict: request-changes
created: 2026-07-01T12:00:00Z
submitted: 2026-07-01T12:10:00Z
comments: []
---

Still loads.
"#;
        let doc: Document<Review> = Document::parse(content).unwrap();
        assert_eq!(
            doc.frontmatter.target,
            ReviewTarget::Phase {
                roadmap: "renamed-away".to_string(),
                stem: "phase-9-gone".to_string(),
            }
        );
    }

    #[test]
    fn render_review_document_round_trip() {
        let original = Document {
            frontmatter: Review {
                id: "2026-07-01-1430-a1b2".to_string(),
                author: "ed.paget@gmail.com".to_string(),
                target: ReviewTarget::Phase {
                    roadmap: "comments-and-llm-edits".to_string(),
                    stem: "core-review-model".to_string(),
                },
                state: ReviewState::Submitted,
                verdict: Some(Verdict::RequestChanges),
                created: Utc.with_ymd_and_hms(2026, 7, 1, 14, 30, 0).unwrap(),
                submitted: Some(Utc.with_ymd_and_hms(2026, 7, 1, 14, 55, 0).unwrap()),
                created_commit: Some("a1b2c3d".to_string()),
                comments: vec![
                    ReviewComment {
                        id: 1,
                        doc: None,
                        status: ReviewCommentStatus::Open,
                        applied_commit: None,
                        anchor: Some(Anchor::TextQuote {
                            quote: "## Acceptance Criteria".to_string(),
                            prefix: "right?\n\n".to_string(),
                            suffix: "\n\n- [ ] Criterion".to_string(),
                        }),
                        body: "Multi-line comment body.\n\nSecond paragraph.\n".to_string(),
                        reply: None,
                    },
                    ReviewComment {
                        id: 2,
                        doc: Some(CommentDoc {
                            kind: CommentDocKind::Phase,
                            stem: "phase-2-ops".to_string(),
                        }),
                        status: ReviewCommentStatus::Addressed,
                        applied_commit: Some("beefcafe".to_string()),
                        anchor: None,
                        body: "Whole-document comment.".to_string(),
                        reply: Some("Addressed.".to_string()),
                    },
                ],
            },
            body: "Overall summary.\n".to_string(),
        };
        let rendered = original.render().unwrap();
        let parsed: Document<Review> = Document::parse(&rendered).unwrap();
        assert_eq!(parsed.frontmatter, original.frontmatter);
        assert_eq!(parsed.body, original.body);
    }

    #[test]
    fn render_review_document_omits_absent_optionals() {
        let doc = Document {
            frontmatter: Review {
                id: "2026-07-01-0900-c3d4".to_string(),
                author: "ed".to_string(),
                target: ReviewTarget::Task {
                    slug: "fix-login".to_string(),
                },
                state: ReviewState::Draft,
                verdict: None,
                created: Utc.with_ymd_and_hms(2026, 7, 1, 9, 0, 0).unwrap(),
                submitted: None,
                created_commit: None,
                comments: vec![],
            },
            body: String::new(),
        };
        let rendered = doc.render().unwrap();
        assert!(!rendered.contains("verdict"));
        assert!(!rendered.contains("submitted"));
        assert!(!rendered.contains("created_commit"));
    }
}
