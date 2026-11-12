/// Generic document wrapper combining YAML frontmatter with a markdown body.
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::frontmatter;
use crate::markdown;

/// The number of lines of `content` that precede `part`, a slice cut out of
/// it — what turns a line counted within the frontmatter into a line of the
/// file.
fn lines_before(content: &str, part: &str) -> u64 {
    let offset = part.as_ptr().addr().saturating_sub(content.as_ptr().addr());
    content
        .get(..offset)
        .map_or(0, |head| head.matches('\n').count() as u64)
}

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
    /// deserialized into `T`. The reported line counts from the top of
    /// `content`; no document is named, because a bare string has none.
    pub fn parse(content: &str) -> Result<Self> {
        let (yaml, body) = markdown::split_frontmatter(content)?;
        let frontmatter: T = frontmatter::from_str(yaml)
            .map_err(|err| Error::FrontmatterParse(err.after_lines(lines_before(content, yaml))))?;
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
        let yaml = frontmatter::to_string(&self.frontmatter).map_err(Error::FrontmatterParse)?;
        Ok(markdown::join_frontmatter(&yaml, &self.body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        Phase, PhaseStatus, Plan, PlanStatus, Priority, ReviewTarget, Task, TaskStatus,
    };
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
                started_head: None,
                difficulty: None,
                model: None,
                blocked_reason: None,
                gate_override: None,
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
                started_head: None,
                difficulty: None,
                model: None,
                blocked_reason: None,
                gate_override: None,
            },
            body: "Body text.\n".to_string(),
        };
        let rendered = original.render().unwrap();
        let parsed: Document<Phase> = Document::parse(&rendered).unwrap();
        assert_eq!(parsed.frontmatter, original.frontmatter);
        assert_eq!(parsed.body, original.body);
    }

    #[test]
    fn parse_render_plan_round_trip() {
        // Every field populated (including `supersedes`) so the `rdm:` serde
        // shim is exercised in BOTH directions.
        let original = Document {
            frontmatter: Plan {
                project: "rdm".to_string(),
                plan: "impl-auth-v2".to_string(),
                title: "Auth implementation, second attempt".to_string(),
                implements: ReviewTarget::Phase {
                    roadmap: "auth".to_string(),
                    stem: "phase-1-design".to_string(),
                },
                supersedes: Some(ReviewTarget::Plan {
                    slug: "impl-auth-v1".to_string(),
                }),
                status: PlanStatus::ChangesRequested,
                created: NaiveDate::from_ymd_opt(2026, 3, 14).unwrap(),
                updated: NaiveDate::from_ymd_opt(2026, 3, 15).unwrap(),
            },
            body: "## Approach\n\nDetails.\n".to_string(),
        };
        let rendered = original.render().unwrap();
        // The canonical `rdm:`-prefixed URI form, not a tagged mapping.
        assert!(rendered.contains("implements: rdm:phase/auth/phase-1-design"));
        assert!(rendered.contains("supersedes: rdm:plan/impl-auth-v1"));
        assert!(rendered.contains("status: changes-requested"));

        let parsed: Document<Plan> = Document::parse(&rendered).unwrap();
        assert_eq!(parsed.frontmatter, original.frontmatter);
        assert_eq!(parsed.body, original.body);
    }

    #[test]
    fn parse_plan_document_accepts_a_bare_item_ref() {
        // Hand-edited files may omit the `rdm:` scheme prefix.
        let content = "---\nproject: rdm\nplan: impl-auth\ntitle: Auth plan\nimplements: task/fix-login\nstatus: draft\ncreated: 2026-03-14\nupdated: 2026-03-14\n---\n\nBody.\n";
        let doc: Document<Plan> = Document::parse(content).unwrap();
        assert_eq!(
            doc.frontmatter.implements,
            ReviewTarget::Task {
                slug: "fix-login".to_string()
            }
        );
        assert_eq!(doc.frontmatter.supersedes, None);
        assert_eq!(doc.frontmatter.status, PlanStatus::Draft);
        // ...and it is re-serialized in the canonical form regardless.
        assert!(
            doc.render()
                .unwrap()
                .contains("implements: rdm:task/fix-login")
        );
    }

    #[test]
    fn parse_plan_document_rejects_a_code_reference() {
        let content = "---\nproject: rdm\nplan: impl-auth\ntitle: Auth plan\nimplements: rdm:src/main.rs\nstatus: draft\ncreated: 2026-03-14\nupdated: 2026-03-14\n---\n\nBody.\n";
        assert!(Document::<Plan>::parse(content).is_err());
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
        ReviewState, Verdict,
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
        // A file with no `applied_commit_repo` key loads as "not recorded",
        // and that absence is not written back.
        assert_eq!(first.applied_commit_repo, None);
        assert!(!doc.render().unwrap().contains("applied_commit_repo"));
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
      anchor_type: heading-path
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
                assert_eq!(anchor_type, "heading-path");
                assert_eq!(
                    raw.get("start").and_then(serde_json::Value::as_i64),
                    Some(3)
                );
                assert_eq!(raw.get("end").and_then(serde_json::Value::as_i64), Some(7));
            }
            other => panic!("expected Anchor::Unknown, got {other:?}"),
        }
        // Render and re-parse: the unknown anchor survives verbatim.
        let rendered = doc.render().unwrap();
        let reparsed: Document<Review> = Document::parse(&rendered).unwrap();
        assert_eq!(reparsed.frontmatter, doc.frontmatter);
        assert_eq!(reparsed.body, doc.body);
    }

    /// A submitted review whose only comment carries `anchor`, a block of
    /// YAML indented four spaces.
    fn review_with_anchor(anchor: &str) -> String {
        format!(
            r#"---
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
{anchor}    body: Anchored by a shape this build does not know.
---

Summary.
"#
        )
    }

    /// Reads a review, writes it back and reads the result: the anchor of
    /// its first comment as it stands after a read followed by a write.
    fn anchor_after_read_then_write(content: &str) -> (String, serde_json::Value) {
        let doc: Document<Review> = Document::parse(content).unwrap();
        let rendered = doc.render().unwrap();
        let reread: Document<Review> = Document::parse(&rendered).unwrap();
        assert_eq!(reread.frontmatter, doc.frontmatter);
        assert_eq!(reread.body, doc.body);
        match reread.frontmatter.comments[0].anchor.clone() {
            Some(Anchor::Unknown { anchor_type, raw }) => (anchor_type, raw),
            other => panic!("expected Anchor::Unknown, got {other:?}"),
        }
    }

    #[test]
    fn an_unrecognised_anchor_keeps_every_field_and_value_through_read_then_write() {
        use serde_json::json;

        let content = review_with_anchor(
            r#"      anchor_type: heading-path
      label: 'true'
      pinned: true
      stale: false
      depth: 3
      weight: 0.5
      parent: null
      path:
        - Intro
        - '12'
      span:
        from: 2
        to: 9
        note: kept
"#,
        );
        let (anchor_type, raw) = anchor_after_read_then_write(&content);

        assert_eq!(anchor_type, "heading-path");
        let fields = raw.as_object().expect("the anchor is a mapping");
        assert_eq!(fields.len(), 9, "{raw}");
        assert_eq!(fields.get("anchor_type"), Some(&json!("heading-path")));
        // A string that spells a boolean stays a string.
        assert_eq!(fields.get("label"), Some(&json!("true")));
        // A boolean stays a boolean.
        assert_eq!(fields.get("pinned"), Some(&json!(true)));
        assert_eq!(fields.get("stale"), Some(&json!(false)));
        assert_eq!(fields.get("depth"), Some(&json!(3)));
        assert_eq!(fields.get("weight"), Some(&json!(0.5)));
        // A null value keeps its key.
        assert_eq!(fields.get("parent"), Some(&serde_json::Value::Null));
        // A string that spells a number stays a string, inside a list too.
        assert_eq!(fields.get("path"), Some(&json!(["Intro", "12"])));
        assert_eq!(
            fields.get("span"),
            Some(&json!({"from": 2, "to": 9, "note": "kept"}))
        );
    }

    #[test]
    fn a_malformed_line_range_anchor_keeps_every_field_and_value_through_read_then_write() {
        use serde_json::json;

        let content = review_with_anchor(
            r#"      anchor_type: line-range
      start_line: 0
      end_line: 4
      snippet: 'no'
      context:
        - before
        - after
"#,
        );
        let (anchor_type, raw) = anchor_after_read_then_write(&content);

        assert_eq!(anchor_type, "line-range");
        let fields = raw.as_object().expect("the anchor is a mapping");
        assert_eq!(fields.len(), 5, "{raw}");
        assert_eq!(fields.get("anchor_type"), Some(&json!("line-range")));
        assert_eq!(fields.get("start_line"), Some(&json!(0)));
        assert_eq!(fields.get("end_line"), Some(&json!(4)));
        assert_eq!(fields.get("snippet"), Some(&json!("no")));
        assert_eq!(fields.get("context"), Some(&json!(["before", "after"])));
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
                        anchor_state: None,
                        intended_path: None,
                        document: None,
                        id: 1,
                        doc: None,
                        status: ReviewCommentStatus::Open,
                        applied_commit: None,
                        applied_commit_repo: None,
                        anchor: Some(Anchor::TextQuote {
                            quote: "## Acceptance Criteria".to_string(),
                            prefix: "right?\n\n".to_string(),
                            suffix: "\n\n- [ ] Criterion".to_string(),
                        }),
                        body: "Multi-line comment body.\n\nSecond paragraph.\n".to_string(),
                        reply: None,
                    },
                    ReviewComment {
                        anchor_state: None,
                        intended_path: None,
                        document: Some(ReviewTarget::Plan {
                            slug: "review-model-plan".to_string(),
                        }),
                        id: 2,
                        doc: Some(CommentDoc {
                            kind: CommentDocKind::Phase,
                            stem: "phase-2-ops".to_string(),
                        }),
                        status: ReviewCommentStatus::Addressed,
                        applied_commit: Some("beefcafe".to_string()),
                        applied_commit_repo: Some(crate::model::CommitRepo::Source),
                        anchor: None,
                        body: "Whole-document comment.".to_string(),
                        reply: Some("Addressed.".to_string()),
                    },
                ],
                implements: None,
                change_branch: None,
                follows: None,
            },
            body: "Overall summary.\n".to_string(),
        };
        let rendered = original.render().unwrap();
        assert!(
            rendered.contains("applied_commit_repo: source"),
            "{rendered}"
        );
        // A comment's document is written as an `rdm:` reference, and only
        // on the comment that has one.
        assert_eq!(
            rendered
                .matches("document: rdm:plan/review-model-plan")
                .count(),
            1,
            "{rendered}"
        );
        assert_eq!(rendered.matches("document:").count(), 1, "{rendered}");
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
                implements: None,
                change_branch: None,
                follows: None,
            },
            body: String::new(),
        };
        let rendered = doc.render().unwrap();
        assert!(!rendered.contains("verdict"));
        assert!(!rendered.contains("submitted"));
        assert!(!rendered.contains("created_commit"));
    }

    // -- what today's writer emits, the reader gives back --

    /// Strings a YAML reader could take for something else: other types,
    /// syntax, indentation, line breaks. In the three tests of this section
    /// none is the last value of its document; the next section puts them
    /// there too.
    fn awkward_strings() -> Vec<String> {
        let mut strings: Vec<String> = [
            "",
            " ",
            "plain",
            // Other types' spellings.
            "yes",
            "No",
            "on",
            "OFF",
            "y",
            "n",
            "true",
            "False",
            "null",
            "~",
            "0",
            "-1",
            "123",
            "007",
            "0755",
            "0o17",
            "0x1F",
            "1_000",
            "1.0",
            ".5",
            "1e3",
            "1e999",
            "12345e6789012345678901234567890123456789",
            ".inf",
            "-.inf",
            ".nan",
            "12:30:45",
            "2026-03-13",
            "2026-03-13T10:00:00Z",
            "18446744073709551616",
            // Syntax.
            "- foo",
            "-",
            "---",
            "...",
            "a: b",
            "x:",
            "key: value # comment",
            "#hash",
            "@at",
            "`tick`",
            "!bang",
            "!!str x",
            "*star",
            "&amp",
            "%pct",
            "'single",
            "it's",
            "\"double",
            "a \"q\" b",
            "\\back",
            "?",
            "|",
            "> x",
            "[a]",
            "{a}",
            "a, b",
            "<<",
            // Whitespace and line breaks.
            " leading",
            "trailing ",
            "\ttab",
            "a\tb",
            "multi\nline",
            "multi\n\nline\n",
            "\nlead-nl",
            "trail-nl\n",
            "trail-nls\n\n",
            "  indented first\nsecond",
            "a\n  b\n c",
            "trailing space \nnext",
            "crlf\r\nline",
            "nel\u{85}x",
            "nul\u{0}x",
            // Text.
            "héllo",
            "日本語",
            "emoji 🎉",
            "naïve—dash",
            "## Acceptance Criteria",
            "```rust\nfn x() {}\n```",
            "rdm:phase/auth/phase-1-design",
            "http://x.y/z?a=b#c",
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        strings.push("word ".repeat(60));
        strings.push(format!("{}\n{}", "long ".repeat(40), "second ".repeat(40)));
        strings
    }

    /// Asserts that `doc`, written and read back, is the document written.
    fn assert_written_then_read<T>(doc: &Document<T>, what: &str)
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let rendered = doc.render().unwrap();
        let read = Document::<T>::parse(&rendered)
            .unwrap_or_else(|err| panic!("{what}: {err}\n{rendered}"));
        assert_eq!(&read, doc, "{what}\n{rendered}");
    }

    #[test]
    fn a_task_with_awkward_strings_reads_back_as_written() {
        for s in awkward_strings() {
            let doc = Document {
                frontmatter: Task {
                    project: "p".to_string(),
                    title: s.clone(),
                    status: TaskStatus::Open,
                    priority: Priority::High,
                    created: NaiveDate::from_ymd_opt(2026, 3, 14).unwrap(),
                    tags: Some(vec![s.clone(), "z".to_string()]),
                    completed: None,
                    commit: Some("abc123".to_string()),
                    review_sha: None,
                    review_branch: None,
                    started_head: None,
                    close_reason: None,
                    gate_override: None,
                },
                body: "Body.\n".to_string(),
            };
            assert_written_then_read(&doc, &format!("task title and tag {s:?}"));
        }
    }

    #[test]
    fn a_review_with_awkward_strings_reads_back_as_written() {
        let comment =
            |id: u32, anchor: Option<Anchor>, body: &str, reply: Option<&str>| ReviewComment {
                anchor_state: None,
                intended_path: None,
                id,
                doc: None,
                status: ReviewCommentStatus::Open,
                applied_commit: None,
                applied_commit_repo: None,
                anchor,
                document: None,
                body: body.to_string(),
                reply: reply.map(str::to_string),
            };
        for s in awkward_strings() {
            let unknown = Anchor::Unknown {
                anchor_type: "heading-path".to_string(),
                raw: serde_json::json!({
                    "anchor_type": "heading-path",
                    "label": s,
                    "path": [s, "z"],
                    "depth": 3,
                    "nested": {"values": [1, 2.5, null, true, s]},
                }),
            };
            let anchors = [
                Anchor::TextQuote {
                    quote: s.clone(),
                    prefix: s.clone(),
                    suffix: s.clone(),
                },
                Anchor::LineRange {
                    start_line: 3,
                    end_line: 4,
                    snippet: s.clone(),
                },
                Anchor::FileQuote {
                    path: s.clone(),
                    quote: s.clone(),
                    occurrence: 1,
                    start_line: 1,
                    end_line: 2,
                },
                unknown,
            ];
            let mut comments: Vec<ReviewComment> = anchors
                .into_iter()
                .zip(1..)
                .map(|(anchor, id)| comment(id, Some(anchor), &s, Some(&s)))
                .collect();
            comments.push(comment(5, None, "The last comment.", Some("Its reply.")));
            let doc = Document {
                frontmatter: Review {
                    id: "2026-07-01-1430-a1b2".to_string(),
                    author: s.clone(),
                    target: ReviewTarget::Change {
                        head: s.clone(),
                        base: Some(s.clone()),
                    },
                    state: ReviewState::Submitted,
                    verdict: Some(Verdict::RequestChanges),
                    created: Utc.with_ymd_and_hms(2026, 7, 1, 14, 30, 0).unwrap(),
                    submitted: Some(Utc.with_ymd_and_hms(2026, 7, 1, 14, 55, 0).unwrap()),
                    created_commit: Some(s.clone()),
                    comments,
                    implements: None,
                    change_branch: Some(s.clone()),
                    follows: Some(s.clone()),
                },
                body: "Summary.\n".to_string(),
            };
            assert_written_then_read(&doc, &format!("review strings {s:?}"));
        }
    }

    #[test]
    fn a_run_with_awkward_strings_reads_back_as_written() {
        use crate::model::{Run, RunDriver, RunStatus, RunTarget, RunUnit};

        let started = Utc.with_ymd_and_hms(2026, 9, 24, 15, 30, 12).unwrap();
        let unit = |name: &str, outcome: &str| RunUnit {
            unit: name.to_string(),
            attempt: 1,
            started,
            ended: Some(started),
            outcome: Some(outcome.to_string()),
        };
        for s in awkward_strings() {
            for target in [RunTarget::Roadmap(s.clone()), RunTarget::Task(s.clone())] {
                let doc = Document {
                    frontmatter: Run {
                        id: s.clone(),
                        project: "rdm".to_string(),
                        driver: RunDriver::Autopilot,
                        target,
                        session_uuid: Some(s.clone()),
                        args: Some(s.clone()),
                        status: RunStatus::Closed,
                        started,
                        ended: Some(started),
                        stop_reason: Some(s.clone()),
                        units: vec![unit(&s, &s), unit("phase-2-last", "reviewed")],
                    },
                    body: String::new(),
                };
                assert_written_then_read(&doc, &format!("run strings {s:?}"));
            }
        }
    }

    // -- a document is read back whole, and written the same a second time --
    //
    // Each string goes in every string position of each kind of document,
    // the last one included: a string that ends its frontmatter is the one
    // whose final line break the reader could take for the text's own.

    /// [`awkward_strings`], and the strings that decide how a value is laid
    /// out or what happens to it at the end of the frontmatter.
    fn strings_for_every_position() -> Vec<String> {
        let mut strings = awkward_strings();
        strings.extend(
            [
                // One line break at the end, two, and nothing but line breaks.
                "a\n",
                "a\nb\n",
                "a\nb\n\n",
                "\n",
                "\n\n",
                // Lines that end in spaces and tabs.
                "a \nb",
                "a\nb ",
                "a\nb \n",
                "a\t\nb",
                "a\nb\t",
                "trail\t",
                // A multi-line string that starts with a space, a tab and a
                // line break.
                " a\nb",
                "\ta\nb",
                "\n\na",
                "a\r\nb\r\n",
                // Characters a block scalar cannot carry.
                "a\u{85}b",
                "a\nb\u{85}",
                "a\u{2028}b",
                "a\nb\u{2028}",
                "a\u{2028}\n",
                "a\u{2029}b",
                "\u{2029}",
                "\u{feff}a",
                "a\nb\u{feff}",
                // Quotes and backslashes.
                "'",
                "''",
                "\"",
                "\"a\"",
                "a'b\"c",
                "\\",
                "a\\nb",
                "C:\\x",
            ]
            .map(String::from),
        );
        strings.push("x".repeat(250));
        strings.push(format!("{}  {}", "a".repeat(120), "b".repeat(120)));
        strings
    }

    /// Asserts that `frontmatter`, written as a document, reads back as
    /// exactly what was written, and that what was read is written to the
    /// same bytes.
    fn assert_stable<T>(frontmatter: T, body: &str, what: &str)
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let doc = Document {
            frontmatter,
            body: body.to_string(),
        };
        let first = doc.render().unwrap();
        let read =
            Document::<T>::parse(&first).unwrap_or_else(|err| panic!("{what}: {err}\n{first}"));
        assert_eq!(read, doc, "{what}\n{first}");
        let second = read.render().unwrap();
        assert_eq!(second, first, "{what}: written a second time");
    }

    #[test]
    fn a_phase_is_read_back_whole_and_written_the_same_twice() {
        use crate::model::{Difficulty, GateOverride, ModelTier};

        for s in strings_for_every_position() {
            // `blocked_reason` is the last value.
            let phase = Phase {
                phase: 3,
                title: s.clone(),
                status: PhaseStatus::Blocked,
                tags: Some(vec![s.clone(), "z".to_string()]),
                completed: Some(NaiveDate::from_ymd_opt(2026, 3, 13).unwrap()),
                commit: Some(s.clone()),
                review_sha: Some(s.clone()),
                review_branch: Some(s.clone()),
                started_head: Some(s.clone()),
                difficulty: Some(Difficulty::Hard),
                model: Some(ModelTier::Large),
                blocked_reason: Some(s.clone()),
                gate_override: None,
            };
            let overridden = Phase {
                gate_override: Some(GateOverride {
                    reason: s.clone(),
                    actor: s.clone(),
                    at: NaiveDate::from_ymd_opt(2026, 1, 2).unwrap(),
                }),
                ..phase.clone()
            };
            assert_stable(phase, "Body.\n", &format!("phase {s:?}"));
            assert_stable(overridden, "Body.\n", &format!("overridden phase {s:?}"));
        }
    }

    #[test]
    fn a_task_is_read_back_whole_and_written_the_same_twice() {
        for s in strings_for_every_position() {
            // `close_reason` is the last value.
            let task = Task {
                project: "p".to_string(),
                title: s.clone(),
                status: TaskStatus::WontFix,
                priority: Priority::High,
                created: NaiveDate::from_ymd_opt(2026, 3, 14).unwrap(),
                tags: Some(vec![s.clone()]),
                completed: None,
                commit: Some(s.clone()),
                review_sha: Some(s.clone()),
                review_branch: Some(s.clone()),
                started_head: Some(s.clone()),
                close_reason: Some(s.clone()),
                gate_override: None,
            };
            assert_stable(task, "Body.\n", &format!("task {s:?}"));
        }
    }

    #[test]
    fn a_roadmap_and_a_project_are_read_back_whole_and_written_the_same_twice() {
        use crate::model::{Project, Roadmap, Source};

        for s in strings_for_every_position() {
            // The last tag is the last value.
            let roadmap = Roadmap {
                project: "p".to_string(),
                roadmap: "r".to_string(),
                title: s.clone(),
                phases: vec!["phase-1-a".to_string(), s.clone()],
                dependencies: Some(vec![s.clone()]),
                priority: Some(Priority::Low),
                tags: Some(vec!["a".to_string(), s.clone()]),
            };
            assert_stable(roadmap, "Body.\n", &format!("roadmap {s:?}"));

            // `default_branch` is the last value.
            let project = Project {
                name: "p".to_string(),
                title: s.clone(),
                source: Some(Source {
                    repo: s.clone(),
                    default_branch: Some(s.clone()),
                }),
            };
            assert_stable(project, "", &format!("project {s:?}"));
        }
    }

    #[test]
    fn a_plan_is_read_back_whole_and_written_the_same_twice() {
        for s in strings_for_every_position() {
            let plan = Plan {
                project: "p".to_string(),
                plan: "impl".to_string(),
                title: s.clone(),
                implements: ReviewTarget::Phase {
                    roadmap: "r".to_string(),
                    stem: "phase-1-a".to_string(),
                },
                supersedes: Some(ReviewTarget::Plan {
                    slug: "old".to_string(),
                }),
                status: PlanStatus::Approved,
                created: NaiveDate::from_ymd_opt(2026, 3, 14).unwrap(),
                updated: NaiveDate::from_ymd_opt(2026, 3, 15).unwrap(),
            };
            assert_stable(plan, "Body.\n", &format!("plan {s:?}"));
        }
    }

    #[test]
    fn a_review_is_read_back_whole_and_written_the_same_twice() {
        use crate::model::{AnchorState, CommitRepo};

        for s in strings_for_every_position() {
            let anchors = [
                Anchor::TextQuote {
                    quote: s.clone(),
                    prefix: s.clone(),
                    suffix: s.clone(),
                },
                Anchor::LineRange {
                    start_line: 3,
                    end_line: 4,
                    snippet: s.clone(),
                },
                Anchor::FileQuote {
                    path: s.clone(),
                    quote: s.clone(),
                    occurrence: 2,
                    start_line: 1,
                    end_line: 2,
                },
                Anchor::Unknown {
                    anchor_type: "heading-path".to_string(),
                    raw: serde_json::json!({
                        "anchor_type": "heading-path",
                        "label": s,
                        "path": [s, 5, 1.5, true, null, {"deep": s}],
                        "none": [],
                        "nothing": {},
                    }),
                },
            ];
            // The last comment has no reply, so its body is the last value.
            let comments = anchors
                .into_iter()
                .zip(1..)
                .map(|(anchor, id)| ReviewComment {
                    id,
                    doc: Some(CommentDoc {
                        kind: CommentDocKind::Phase,
                        stem: s.clone(),
                    }),
                    status: ReviewCommentStatus::Addressed,
                    applied_commit: Some(s.clone()),
                    applied_commit_repo: Some(CommitRepo::Plan),
                    anchor: Some(anchor),
                    anchor_state: Some(AnchorState::Unknown(s.clone())),
                    intended_path: Some(s.clone()),
                    document: Some(ReviewTarget::Phase {
                        roadmap: "r".to_string(),
                        stem: "phase-1-a".to_string(),
                    }),
                    body: s.clone(),
                    reply: (id < 4).then(|| s.clone()),
                })
                .collect();
            let review = Review {
                id: "2026-07-01-1430-a1b2".to_string(),
                author: s.clone(),
                target: ReviewTarget::Change {
                    head: s.clone(),
                    base: Some(s.clone()),
                },
                state: ReviewState::Submitted,
                verdict: Some(Verdict::RequestChanges),
                created: Utc.with_ymd_and_hms(2026, 7, 1, 14, 30, 0).unwrap(),
                submitted: Some(Utc.with_ymd_and_hms(2026, 7, 1, 14, 55, 0).unwrap()),
                created_commit: Some(s.clone()),
                implements: Some(ReviewTarget::Plan {
                    slug: "impl".to_string(),
                }),
                change_branch: Some(s.clone()),
                follows: Some(s.clone()),
                comments,
            };
            assert_stable(review, "Summary.\n", &format!("review {s:?}"));
        }
    }

    #[test]
    fn a_run_is_read_back_whole_and_written_the_same_twice() {
        use crate::model::{Run, RunDriver, RunStatus, RunTarget, RunUnit};

        let started = Utc.with_ymd_and_hms(2026, 9, 24, 15, 30, 12).unwrap();
        for s in strings_for_every_position() {
            // The last unit's `outcome` is the last value.
            let run = Run {
                id: s.clone(),
                project: "rdm".to_string(),
                driver: RunDriver::Autopilot,
                target: RunTarget::Roadmap(s.clone()),
                session_uuid: Some(s.clone()),
                args: Some(s.clone()),
                status: RunStatus::Closed,
                started,
                ended: Some(started),
                stop_reason: Some(s.clone()),
                units: vec![RunUnit {
                    unit: s.clone(),
                    attempt: 2,
                    started,
                    ended: Some(started),
                    outcome: Some(s.clone()),
                }],
            };
            // Nothing follows `stop_reason` but an empty list.
            let without_units = Run {
                target: RunTarget::Task(s.clone()),
                units: vec![],
                ..run.clone()
            };
            assert_stable(run, "", &format!("run {s:?}"));
            assert_stable(without_units, "", &format!("run without units {s:?}"));
        }
    }
}
