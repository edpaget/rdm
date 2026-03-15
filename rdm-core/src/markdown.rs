/// Markdown frontmatter splitting and joining utilities.
use crate::error::{Error, Result};

const DELIMITER: &str = "---";

/// Splits a markdown document into its YAML frontmatter and body.
///
/// The frontmatter must be delimited by `---` on its own line at the start
/// and end of the frontmatter block.
///
/// Returns `(yaml, body)` where `yaml` does not include the delimiters and
/// `body` does not include the trailing delimiter line.
pub fn split_frontmatter(content: &str) -> Result<(&str, &str)> {
    let trimmed = content.trim_start();
    let Some(after_open) = trimmed.strip_prefix(DELIMITER) else {
        return Err(Error::FrontmatterMissing);
    };

    // The opening delimiter must be followed by a newline (or be at EOF)
    let after_open = after_open.strip_prefix('\n').unwrap_or(after_open);

    let Some(close_pos) = after_open.find("\n---") else {
        return Err(Error::FrontmatterMissing);
    };

    let yaml = &after_open[..close_pos];
    let after_close = &after_open[close_pos + 4..]; // skip \n---

    // Skip the newline after the closing delimiter, and the blank separator line
    let body = after_close.strip_prefix('\n').unwrap_or(after_close);
    let body = body.strip_prefix('\n').unwrap_or(body);

    Ok((yaml, body))
}

/// Joins YAML frontmatter and a markdown body into a complete document.
pub fn join_frontmatter(yaml: &str, body: &str) -> String {
    let mut out = String::new();
    out.push_str(DELIMITER);
    out.push('\n');
    out.push_str(yaml);
    if !yaml.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(DELIMITER);
    out.push('\n');
    if !body.is_empty() {
        out.push('\n');
        out.push_str(body);
        if !body.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_valid_frontmatter() {
        let content = "---\ntitle: Hello\n---\nBody text here.\n";
        let (yaml, body) = split_frontmatter(content).unwrap();
        assert_eq!(yaml, "title: Hello");
        assert_eq!(body, "Body text here.\n");
    }

    #[test]
    fn split_missing_frontmatter() {
        let content = "No frontmatter here.";
        assert!(split_frontmatter(content).is_err());
    }

    #[test]
    fn split_only_opening_delimiter() {
        let content = "---\ntitle: Hello\nNo closing delimiter.";
        assert!(split_frontmatter(content).is_err());
    }

    #[test]
    fn split_empty_body() {
        let content = "---\ntitle: Hello\n---\n";
        let (yaml, body) = split_frontmatter(content).unwrap();
        assert_eq!(yaml, "title: Hello");
        assert_eq!(body, "");
    }

    #[test]
    fn join_yaml_and_body() {
        let result = join_frontmatter("title: Hello", "Body text.");
        assert_eq!(result, "---\ntitle: Hello\n---\n\nBody text.\n");
    }

    #[test]
    fn join_with_trailing_newlines() {
        let result = join_frontmatter("title: Hello\n", "Body text.\n");
        assert_eq!(result, "---\ntitle: Hello\n---\n\nBody text.\n");
    }

    #[test]
    fn join_empty_body() {
        let result = join_frontmatter("title: Hello", "");
        assert_eq!(result, "---\ntitle: Hello\n---\n");
    }

    #[test]
    fn split_then_join_round_trip() {
        let original = "---\ntitle: Hello\nstatus: open\n---\n\nSome body content.\n";
        let (yaml, body) = split_frontmatter(original).unwrap();
        let rejoined = join_frontmatter(yaml, body);
        assert_eq!(rejoined, original);
    }
}
