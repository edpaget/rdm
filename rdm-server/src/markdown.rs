//! Markdown-to-HTML rendering using pulldown-cmark.

use pulldown_cmark::{Options, Parser, Tag, TagEnd, html};

/// Renders CommonMark markdown to HTML with raw HTML disabled.
///
/// Raw HTML tags in the input are escaped rather than passed through.
/// This is safe for author-controlled content from the plan repo.
///
/// # Examples
///
/// ```
/// use rdm_server::markdown::render_markdown;
/// let html = render_markdown("**bold**");
/// assert!(html.contains("<strong>bold</strong>"));
/// ```
pub fn render_markdown(input: &str) -> String {
    let options = Options::empty();
    // Parse then filter out any raw HTML events.
    let parser = Parser::new_ext(input, options).filter(|event| {
        !matches!(
            event,
            pulldown_cmark::Event::Html(_)
                | pulldown_cmark::Event::InlineHtml(_)
                | pulldown_cmark::Event::Start(Tag::HtmlBlock)
                | pulldown_cmark::Event::End(TagEnd::HtmlBlock)
        )
    });

    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    html_output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_heading() {
        let html = render_markdown("# Hello");
        assert!(html.contains("<h1>Hello</h1>"));
    }

    #[test]
    fn renders_bold_and_links() {
        let html = render_markdown("**bold** and [link](https://example.com)");
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<a href=\"https://example.com\">link</a>"));
    }

    #[test]
    fn renders_code_block() {
        let html = render_markdown("```\nfn main() {}\n```");
        assert!(html.contains("<code>"));
        assert!(html.contains("fn main()"));
    }

    #[test]
    fn empty_input_returns_empty() {
        assert_eq!(render_markdown(""), "");
    }

    #[test]
    fn raw_html_is_stripped() {
        let html = render_markdown("<script>alert('xss')</script>");
        assert!(!html.contains("<script>"));
        assert!(!html.contains("alert"));
    }

    #[test]
    fn inline_html_is_stripped() {
        let html = render_markdown("text <b>bold</b> more");
        assert!(!html.contains("<b>"));
    }
}
