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
                completed: Some(NaiveDate::from_ymd_opt(2026, 3, 13).unwrap()),
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
                completed: None,
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
}
