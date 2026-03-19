//! Internal Markdown AST types for structured document generation.
//!
//! Instead of building markdown output via raw string concatenation, rdm
//! constructs an AST and renders it to a string. This module defines the
//! tree types; rendering lives in a separate phase.

/// A complete Markdown document — an ordered sequence of block-level nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// The block-level nodes that make up the document.
    pub nodes: Vec<Block>,
}

impl Document {
    /// Create an empty document.
    pub fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    /// Append a block-level node to the document.
    pub fn push(&mut self, block: Block) {
        self.nodes.push(block);
    }

    /// Append a heading with plain text content.
    ///
    /// # Panics
    ///
    /// Panics if `level` is outside the range 1–6.
    pub fn heading(&mut self, level: u8, text: &str) {
        assert!((1..=6).contains(&level), "heading level must be 1–6");
        self.push(Block::Heading {
            level,
            content: vec![Inline::text(text)],
        });
    }

    /// Append raw content that is rendered as-is.
    pub fn raw(&mut self, content: &str) {
        self.push(Block::Raw(content.to_owned()));
    }

    /// Append a paragraph with plain text content.
    pub fn paragraph(&mut self, text: &str) {
        self.push(Block::Paragraph {
            content: vec![Inline::text(text)],
        });
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

/// Block-level Markdown elements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// ATX heading: level (1–6) + inline content.
    Heading {
        /// Heading level (1–6).
        level: u8,
        /// Inline content of the heading.
        content: Vec<Inline>,
    },
    /// A paragraph of inline content.
    Paragraph {
        /// Inline content of the paragraph.
        content: Vec<Inline>,
    },
    /// A markdown table with headers and data rows.
    Table {
        /// Header cells, each containing inline content.
        headers: Vec<Vec<Inline>>,
        /// Data rows, each a vector of cells containing inline content.
        rows: Vec<Vec<Vec<Inline>>>,
    },
    /// Unordered bullet list.
    UnorderedList {
        /// List items, each containing inline content.
        items: Vec<Vec<Inline>>,
    },
    /// Raw HTML comment (`<!-- ... -->`).
    HtmlComment(String),
    /// Blank line (controls spacing between blocks).
    BlankLine,
    /// Raw content rendered as-is with no trailing newline.
    Raw(String),
}

/// Inline-level Markdown elements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    /// Plain text.
    Text(String),
    /// Bold/strong text.
    Bold(Vec<Inline>),
    /// Markdown link: `[text](url)`.
    Link {
        /// Link display text.
        text: Vec<Inline>,
        /// Link URL.
        url: String,
    },
}

impl Inline {
    /// Create a plain text inline element.
    pub fn text(s: &str) -> Self {
        Inline::Text(s.to_owned())
    }

    /// Create a bold inline element wrapping plain text.
    pub fn bold(s: &str) -> Self {
        Inline::Bold(vec![Inline::text(s)])
    }

    /// Create a link inline element with plain text.
    pub fn link(text: &str, url: &str) -> Self {
        Inline::Link {
            text: vec![Inline::text(text)],
            url: url.to_owned(),
        }
    }
}

use std::fmt;

impl fmt::Display for Inline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Inline::Text(s) => write!(f, "{s}"),
            Inline::Bold(content) => {
                write!(f, "**")?;
                for node in content {
                    write!(f, "{node}")?;
                }
                write!(f, "**")
            }
            Inline::Link { text, url } => {
                write!(f, "[")?;
                for node in text {
                    write!(f, "{node}")?;
                }
                write!(f, "]({url})")
            }
        }
    }
}

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Block::Heading { level, content } => {
                for _ in 0..*level {
                    write!(f, "#")?;
                }
                write!(f, " ")?;
                for node in content {
                    write!(f, "{node}")?;
                }
                writeln!(f)
            }
            Block::Paragraph { content } => {
                for node in content {
                    write!(f, "{node}")?;
                }
                writeln!(f)
            }
            Block::Table { headers, rows } => {
                // Header row
                write!(f, "|")?;
                for header in headers {
                    write!(f, " ")?;
                    for node in header {
                        write!(f, "{node}")?;
                    }
                    write!(f, " |")?;
                }
                writeln!(f)?;
                // Separator row
                write!(f, "|")?;
                for _ in headers {
                    write!(f, "---|")?;
                }
                writeln!(f)?;
                // Data rows
                for row in rows {
                    write!(f, "|")?;
                    for cell in row {
                        write!(f, " ")?;
                        for node in cell {
                            write!(f, "{node}")?;
                        }
                        write!(f, " |")?;
                    }
                    writeln!(f)?;
                }
                Ok(())
            }
            Block::UnorderedList { items } => {
                for item in items {
                    write!(f, "- ")?;
                    for node in item {
                        write!(f, "{node}")?;
                    }
                    writeln!(f)?;
                }
                Ok(())
            }
            Block::HtmlComment(content) => writeln!(f, "<!-- {content} -->"),
            Block::BlankLine => writeln!(f),
            Block::Raw(content) => write!(f, "{content}"),
        }
    }
}

impl fmt::Display for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for block in &self.nodes {
            write!(f, "{block}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_new_is_empty() {
        let doc = Document::new();
        assert_eq!(doc.nodes, vec![]);
    }

    #[test]
    fn document_default_is_empty() {
        let doc = Document::default();
        assert_eq!(doc, Document::new());
    }

    #[test]
    fn document_push_appends_block() {
        let mut doc = Document::new();
        doc.push(Block::BlankLine);
        doc.push(Block::BlankLine);
        assert_eq!(doc.nodes.len(), 2);
    }

    #[test]
    fn document_heading_builds_text_heading() {
        let mut doc = Document::new();
        doc.heading(1, "Title");
        assert_eq!(
            doc.nodes,
            vec![Block::Heading {
                level: 1,
                content: vec![Inline::text("Title")],
            }]
        );
    }

    #[test]
    #[should_panic(expected = "heading level must be 1–6")]
    fn document_heading_rejects_level_zero() {
        let mut doc = Document::new();
        doc.heading(0, "bad");
    }

    #[test]
    #[should_panic(expected = "heading level must be 1–6")]
    fn document_heading_rejects_level_seven() {
        let mut doc = Document::new();
        doc.heading(7, "bad");
    }

    #[test]
    fn document_paragraph_builds_text_paragraph() {
        let mut doc = Document::new();
        doc.paragraph("Hello world");
        assert_eq!(
            doc.nodes,
            vec![Block::Paragraph {
                content: vec![Inline::text("Hello world")],
            }]
        );
    }

    #[test]
    fn block_heading_with_rich_content() {
        let block = Block::Heading {
            level: 2,
            content: vec![Inline::text("Phase: "), Inline::bold("Setup")],
        };
        if let Block::Heading { level, content } = &block {
            assert_eq!(*level, 2);
            assert_eq!(content.len(), 2);
        } else {
            panic!("expected Heading");
        }
    }

    #[test]
    fn block_table_construction() {
        let table = Block::Table {
            headers: vec![vec![Inline::text("Name")], vec![Inline::text("Status")]],
            rows: vec![vec![
                vec![Inline::text("alpha")],
                vec![Inline::bold("done")],
            ]],
        };
        if let Block::Table { headers, rows } = &table {
            assert_eq!(headers.len(), 2);
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].len(), 2);
        } else {
            panic!("expected Table");
        }
    }

    #[test]
    fn block_unordered_list_construction() {
        let list = Block::UnorderedList {
            items: vec![
                vec![Inline::text("item one")],
                vec![Inline::text("item two")],
            ],
        };
        if let Block::UnorderedList { items } = &list {
            assert_eq!(items.len(), 2);
        } else {
            panic!("expected UnorderedList");
        }
    }

    #[test]
    fn block_html_comment_construction() {
        let comment = Block::HtmlComment("auto-generated".to_owned());
        assert_eq!(comment, Block::HtmlComment("auto-generated".to_owned()));
    }

    #[test]
    fn block_blank_line_equality() {
        assert_eq!(Block::BlankLine, Block::BlankLine);
    }

    #[test]
    fn inline_text_constructor() {
        assert_eq!(Inline::text("hello"), Inline::Text("hello".to_owned()));
    }

    #[test]
    fn inline_bold_constructor() {
        assert_eq!(
            Inline::bold("strong"),
            Inline::Bold(vec![Inline::Text("strong".to_owned())])
        );
    }

    #[test]
    fn inline_link_constructor() {
        assert_eq!(
            Inline::link("click", "https://example.com"),
            Inline::Link {
                text: vec![Inline::Text("click".to_owned())],
                url: "https://example.com".to_owned(),
            }
        );
    }

    #[test]
    fn clone_produces_equal_document() {
        let mut doc = Document::new();
        doc.heading(1, "Title");
        doc.paragraph("Body");
        doc.push(Block::BlankLine);
        let cloned = doc.clone();
        assert_eq!(doc, cloned);
    }

    #[test]
    fn debug_format_is_available() {
        let doc = Document::new();
        let debug = format!("{:?}", doc);
        assert!(debug.contains("Document"));
    }

    // -- Display tests: Inline --

    #[test]
    fn display_inline_text() {
        assert_eq!(Inline::text("hello").to_string(), "hello");
    }

    #[test]
    fn display_inline_bold_plain() {
        assert_eq!(Inline::bold("strong").to_string(), "**strong**");
    }

    #[test]
    fn display_inline_bold_nested() {
        let bold = Inline::Bold(vec![Inline::text("hello"), Inline::text(" world")]);
        assert_eq!(bold.to_string(), "**hello world**");
    }

    #[test]
    fn display_inline_link() {
        assert_eq!(
            Inline::link("click", "https://example.com").to_string(),
            "[click](https://example.com)"
        );
    }

    // -- Display tests: Block --

    #[test]
    fn display_heading_level_1() {
        let block = Block::Heading {
            level: 1,
            content: vec![Inline::text("Title")],
        };
        assert_eq!(block.to_string(), "# Title\n");
    }

    #[test]
    fn display_heading_level_3() {
        let block = Block::Heading {
            level: 3,
            content: vec![Inline::text("Title")],
        };
        assert_eq!(block.to_string(), "### Title\n");
    }

    #[test]
    fn display_paragraph() {
        let block = Block::Paragraph {
            content: vec![Inline::text("Hello world")],
        };
        assert_eq!(block.to_string(), "Hello world\n");
    }

    #[test]
    fn display_table() {
        let table = Block::Table {
            headers: vec![vec![Inline::text("Name")], vec![Inline::text("Status")]],
            rows: vec![
                vec![vec![Inline::text("alpha")], vec![Inline::bold("done")]],
                vec![vec![Inline::text("beta")], vec![Inline::text("open")]],
            ],
        };
        assert_eq!(
            table.to_string(),
            "| Name | Status |\n|---|---|\n| alpha | **done** |\n| beta | open |\n"
        );
    }

    #[test]
    fn display_unordered_list() {
        let list = Block::UnorderedList {
            items: vec![
                vec![Inline::text("item one")],
                vec![Inline::text("item two")],
            ],
        };
        assert_eq!(list.to_string(), "- item one\n- item two\n");
    }

    #[test]
    fn display_html_comment() {
        let comment = Block::HtmlComment("auto-generated".to_owned());
        assert_eq!(comment.to_string(), "<!-- auto-generated -->\n");
    }

    #[test]
    fn display_blank_line() {
        assert_eq!(Block::BlankLine.to_string(), "\n");
    }

    #[test]
    fn display_raw() {
        let raw = Block::Raw("## Overview\n\nSome body content.\n".to_owned());
        assert_eq!(raw.to_string(), "## Overview\n\nSome body content.\n");
    }

    #[test]
    fn display_raw_empty() {
        assert_eq!(Block::Raw(String::new()).to_string(), "");
    }

    // -- Display tests: Document --

    #[test]
    fn display_document_integration() {
        let mut doc = Document::new();
        doc.heading(1, "My Doc");
        doc.push(Block::BlankLine);
        doc.paragraph("A paragraph.");
        doc.push(Block::BlankLine);
        doc.push(Block::Table {
            headers: vec![vec![Inline::text("Col")]],
            rows: vec![vec![vec![Inline::text("val")]]],
        });
        doc.push(Block::UnorderedList {
            items: vec![vec![Inline::text("one")], vec![Inline::text("two")]],
        });
        assert_eq!(
            doc.to_string(),
            "# My Doc\n\nA paragraph.\n\n| Col |\n|---|\n| val |\n- one\n- two\n"
        );
    }
}
