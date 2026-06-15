//! Markdown-to-terminal rendering for phase/task bodies.
//!
//! [`render_markdown`] walks `pulldown-cmark` events with a small stateful
//! builder, accumulating owned, styled [`Span`]s into [`Line`]s so the result
//! is `'static` and can be handed straight to a [`ratatui::widgets::Paragraph`].
//! The same GFM options the server's HTML renderer uses are enabled, so tables,
//! strikethrough, and task lists parse the same way they do on the web.
//!
//! Each block kind is rendered to be visually distinguishable in a colorless
//! terminal: headings are bold (H1 also underlined and prefixed `# `), code
//! blocks carry a `│ ` gutter, block quotes a `> ` prefix, and GFM tables are
//! laid out with aligned columns and a separator rule.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};

/// Renders a Markdown string into styled terminal [`Text`].
///
/// Soft breaks become spaces (the consuming widget re-wraps to its width); hard
/// breaks start a new line. Inline emphasis (strong, emphasis, strikethrough,
/// code) maps to terminal modifiers. Returns owned `'static` text with no
/// borrow on `input`.
pub fn render_markdown(input: &str) -> Text<'static> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM;
    let mut builder = Builder::default();
    for event in Parser::new_ext(input, options) {
        builder.handle(event);
    }
    builder.finish()
}

/// The distinct style applied to inline and block code spans.
fn code_style() -> Style {
    Style::default().fg(Color::Cyan)
}

/// Buffered state for a single GFM table while its cells stream in.
#[derive(Default)]
struct TableBuf {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
    cur_row: Vec<String>,
    cur_cell: String,
    in_head: bool,
}

/// Stateful accumulator that turns a stream of `pulldown-cmark` events into
/// styled lines.
#[derive(Default)]
struct Builder {
    lines: Vec<Line<'static>>,
    spans: Vec<Span<'static>>,
    bold: u32,
    italic: u32,
    strike: u32,
    heading: Option<HeadingLevel>,
    quote_depth: u32,
    /// One entry per open list; `Some(n)` is the next number of an ordered
    /// list, `None` an unordered list.
    list_stack: Vec<Option<u64>>,
    in_code_block: bool,
    code_buffer: String,
    table: Option<TableBuf>,
    in_cell: bool,
}

impl Builder {
    /// The style for ordinary text given the current inline/block context.
    fn text_style(&self) -> Style {
        let mut style = Style::default();
        if let Some(level) = self.heading {
            style = style.add_modifier(Modifier::BOLD);
            if level == HeadingLevel::H1 {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
        }
        if self.bold > 0 {
            style = style.add_modifier(Modifier::BOLD);
        }
        if self.italic > 0 {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if self.strike > 0 {
            style = style.add_modifier(Modifier::CROSSED_OUT);
        }
        if self.quote_depth > 0 {
            style = style.add_modifier(Modifier::DIM | Modifier::ITALIC);
        }
        style
    }

    /// Pushes the accumulated spans as a finished line, prefixing the block
    /// quote marker when inside a quote. Empty, non-quoted lines are dropped.
    fn flush_line(&mut self) {
        if self.spans.is_empty() && self.quote_depth == 0 {
            return;
        }
        let mut spans = std::mem::take(&mut self.spans);
        if self.quote_depth > 0 {
            let prefix = "> ".repeat(self.quote_depth as usize);
            spans.insert(
                0,
                Span::styled(prefix, Style::default().add_modifier(Modifier::DIM)),
            );
        }
        self.lines.push(Line::from(spans));
    }

    /// Appends a blank separator line, collapsing leading and repeated blanks.
    fn push_blank(&mut self) {
        match self.lines.last() {
            None => {}
            Some(line) if line.spans.is_empty() => {}
            Some(_) => self.lines.push(Line::default()),
        }
    }

    /// Pushes a styled text span onto the current line.
    fn push_text(&mut self, text: String, style: Style) {
        self.spans.push(Span::styled(text, style));
    }

    fn handle(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => {
                if self.in_code_block {
                    self.code_buffer.push_str(&text);
                } else if self.in_cell {
                    if let Some(table) = &mut self.table {
                        table.cur_cell.push_str(&text);
                    }
                } else {
                    let style = self.text_style();
                    self.push_text(text.into_string(), style);
                }
            }
            Event::Code(code) => {
                if self.in_cell {
                    if let Some(table) = &mut self.table {
                        table.cur_cell.push_str(&code);
                    }
                } else {
                    self.push_text(code.into_string(), code_style());
                }
            }
            Event::SoftBreak => {
                if self.in_cell {
                    if let Some(table) = &mut self.table {
                        table.cur_cell.push(' ');
                    }
                } else {
                    self.push_text(" ".to_string(), Style::default());
                }
            }
            Event::HardBreak => self.flush_line(),
            Event::Rule => {
                self.push_blank();
                self.lines.push(Line::from(Span::styled(
                    "-".repeat(20),
                    Style::default().add_modifier(Modifier::DIM),
                )));
                self.push_blank();
            }
            Event::TaskListMarker(checked) => {
                let marker = if checked { "[x] " } else { "[ ] " };
                self.push_text(marker.to_string(), Style::default());
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { level, .. } => {
                self.heading = Some(level);
                let prefix = format!("{} ", "#".repeat(level as usize));
                let style = self.text_style();
                self.push_text(prefix, style);
            }
            Tag::Paragraph => {}
            Tag::Strong => self.bold += 1,
            Tag::Emphasis => self.italic += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::BlockQuote(_) => self.quote_depth += 1,
            Tag::List(start) => self.list_stack.push(start),
            Tag::Item => {
                // A nested list opens while the parent item's line is still
                // buffered; flush it so the nested marker starts a fresh line
                // rather than trailing the parent's text.
                self.flush_line();
                let depth = self.list_stack.len();
                let indent = "  ".repeat(depth.saturating_sub(1));
                let marker = match self.list_stack.last_mut() {
                    Some(Some(n)) => {
                        let marker = format!("{n}. ");
                        *n += 1;
                        marker
                    }
                    _ => "• ".to_string(),
                };
                self.push_text(format!("{indent}{marker}"), Style::default());
            }
            Tag::CodeBlock(_) => {
                self.push_blank();
                self.in_code_block = true;
                self.code_buffer.clear();
            }
            Tag::Table(_) => self.table = Some(TableBuf::default()),
            Tag::TableHead => {
                if let Some(table) = &mut self.table {
                    table.in_head = true;
                    table.cur_row.clear();
                }
            }
            Tag::TableRow => {
                if let Some(table) = &mut self.table {
                    table.cur_row.clear();
                }
            }
            Tag::TableCell => {
                if let Some(table) = &mut self.table {
                    table.cur_cell.clear();
                }
                self.in_cell = true;
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => {
                self.heading = None;
                self.flush_line();
                self.push_blank();
            }
            TagEnd::Paragraph => {
                self.flush_line();
                if self.list_stack.is_empty() {
                    self.push_blank();
                }
            }
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::BlockQuote(_) => {
                self.quote_depth = self.quote_depth.saturating_sub(1);
                if self.quote_depth == 0 {
                    self.push_blank();
                }
            }
            TagEnd::List(_) => {
                self.list_stack.pop();
                if self.list_stack.is_empty() {
                    self.push_blank();
                }
            }
            TagEnd::Item => self.flush_line(),
            TagEnd::CodeBlock => {
                let buffer = std::mem::take(&mut self.code_buffer);
                for line in buffer.lines() {
                    let spans = vec![
                        Span::styled(
                            "│ ".to_string(),
                            Style::default().add_modifier(Modifier::DIM),
                        ),
                        Span::styled(line.to_string(), code_style()),
                    ];
                    self.lines.push(Line::from(spans));
                }
                self.in_code_block = false;
                self.push_blank();
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    self.render_table(table);
                }
            }
            TagEnd::TableHead => {
                if let Some(table) = &mut self.table {
                    table.headers = std::mem::take(&mut table.cur_row);
                    table.in_head = false;
                }
            }
            TagEnd::TableRow => {
                if let Some(table) = &mut self.table {
                    let row = std::mem::take(&mut table.cur_row);
                    if !table.in_head {
                        table.rows.push(row);
                    }
                }
            }
            TagEnd::TableCell => {
                if let Some(table) = &mut self.table {
                    let cell = std::mem::take(&mut table.cur_cell);
                    table.cur_row.push(cell);
                }
                self.in_cell = false;
            }
            _ => {}
        }
    }

    /// Lays a buffered table out into aligned lines: a bold header row, a
    /// `-`/`+` separator rule, then body rows separated by `|`.
    ///
    /// Column widths use `chars().count()`, so cells with wide glyphs (CJK,
    /// emoji) under-pad and misalign; this is a cosmetic limitation accepted for
    /// an English-dominant tool.
    fn render_table(&mut self, table: TableBuf) {
        let cols = table.headers.len();
        if cols == 0 {
            return;
        }
        let mut widths = vec![0usize; cols];
        for (i, h) in table.headers.iter().enumerate() {
            widths[i] = widths[i].max(h.chars().count());
        }
        for row in &table.rows {
            for (i, cell) in row.iter().enumerate() {
                if i < cols {
                    widths[i] = widths[i].max(cell.chars().count());
                }
            }
        }

        self.push_blank();
        self.lines.push(Line::from(Span::styled(
            render_row(&table.headers, &widths),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        let separator = widths
            .iter()
            .map(|w| "-".repeat(*w))
            .collect::<Vec<_>>()
            .join("-+-");
        self.lines.push(Line::from(Span::styled(
            separator,
            Style::default().add_modifier(Modifier::DIM),
        )));
        for row in &table.rows {
            self.lines.push(Line::from(render_row(row, &widths)));
        }
        self.push_blank();
    }

    /// Finalizes the builder, flushing any trailing line.
    fn finish(mut self) -> Text<'static> {
        self.flush_line();
        Text::from(self.lines)
    }
}

/// Left-pads `cell` to `width` characters.
fn pad(cell: &str, width: usize) -> String {
    let len = cell.chars().count();
    let mut out = cell.to_string();
    if len < width {
        out.push_str(&" ".repeat(width - len));
    }
    out
}

/// Renders one table row as padded cells joined by ` | `.
fn render_row(cells: &[String], widths: &[usize]) -> String {
    cells
        .iter()
        .enumerate()
        .map(|(i, c)| pad(c, widths.get(i).copied().unwrap_or(0)))
        .collect::<Vec<_>>()
        .join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Concatenates a line's span contents into one string.
    fn line_text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    /// Finds the first span across all lines whose content equals `needle`.
    fn find_span<'a>(text: &'a Text, needle: &str) -> &'a Span<'a> {
        text.lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.as_ref() == needle)
            .unwrap_or_else(|| panic!("no span with content {needle:?}"))
    }

    #[test]
    fn heading_is_bold_with_prefix() {
        let text = render_markdown("# Hello");
        let prefix = find_span(&text, "# ");
        assert!(prefix.style.add_modifier.contains(Modifier::BOLD));
        let span = find_span(&text, "Hello");
        assert!(span.style.add_modifier.contains(Modifier::BOLD));
        assert!(span.style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn h2_uses_double_hash_prefix_without_underline() {
        let text = render_markdown("## Sub");
        find_span(&text, "## ");
        let span = find_span(&text, "Sub");
        assert!(span.style.add_modifier.contains(Modifier::BOLD));
        assert!(!span.style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn strong_text_is_bold() {
        let text = render_markdown("**loud**");
        let span = find_span(&text, "loud");
        assert!(span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn emphasis_text_is_italic() {
        let text = render_markdown("*lean*");
        let span = find_span(&text, "lean");
        assert!(span.style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn strikethrough_text_is_crossed_out() {
        let text = render_markdown("~~gone~~");
        let span = find_span(&text, "gone");
        assert!(span.style.add_modifier.contains(Modifier::CROSSED_OUT));
    }

    #[test]
    fn inline_code_has_distinct_style() {
        let text = render_markdown("call `run()` now");
        let span = find_span(&text, "run()");
        assert_eq!(span.style.fg, Some(Color::Cyan));
    }

    #[test]
    fn unordered_list_uses_bullet_prefix() {
        let text = render_markdown("- one\n- two\n");
        let first = &text.lines[0];
        assert_eq!(first.spans[0].content.as_ref(), "• ");
        assert_eq!(first.spans[1].content.as_ref(), "one");
    }

    #[test]
    fn ordered_list_numbers_items() {
        let text = render_markdown("1. one\n2. two\n");
        assert_eq!(text.lines[0].spans[0].content.as_ref(), "1. ");
        assert_eq!(text.lines[1].spans[0].content.as_ref(), "2. ");
    }

    #[test]
    fn nested_list_items_start_on_their_own_indented_lines() {
        let text = render_markdown("- a\n  - nested\n- b\n");
        let rendered: Vec<String> = text.lines.iter().map(|l| line_text(l)).collect();
        // Each item is on its own line; the nested item carries a 2-space indent
        // and does not trail the parent's text.
        assert_eq!(rendered[0], "• a");
        assert_eq!(rendered[1], "  • nested");
        assert_eq!(rendered[2], "• b");
    }

    #[test]
    fn code_block_carries_gutter_and_style() {
        let text = render_markdown("```\nfn main() {}\n```");
        let line = text
            .lines
            .iter()
            .find(|l| line_text(l).contains("fn main()"))
            .expect("code line present");
        assert_eq!(line.spans[0].content.as_ref(), "│ ");
        assert!(line.spans[0].style.add_modifier.contains(Modifier::DIM));
        let code = &line.spans[1];
        assert_eq!(code.content.as_ref(), "fn main() {}");
        assert_eq!(code.style.fg, Some(Color::Cyan));
    }

    #[test]
    fn gfm_table_renders_aligned_rows() {
        let text = render_markdown("| a | bb |\n|---|----|\n| 1 | 2 |\n");
        let rendered: Vec<String> = text.lines.iter().map(|l| line_text(l)).collect();
        assert_eq!(rendered[0], "a | bb");
        assert_eq!(rendered[1], "--+---");
        assert_eq!(rendered[2], "1 | 2 ");
        // Header row is bold.
        assert!(
            text.lines[0].spans[0]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn block_quote_has_prefix_and_dim_italic() {
        let text = render_markdown("> quoted");
        let line = &text.lines[0];
        assert_eq!(line.spans[0].content.as_ref(), "> ");
        let body = find_span(&text, "quoted");
        assert!(body.style.add_modifier.contains(Modifier::DIM));
        assert!(body.style.add_modifier.contains(Modifier::ITALIC));
    }
}
