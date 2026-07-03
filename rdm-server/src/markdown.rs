//! Markdown-to-HTML rendering using pulldown-cmark.

use std::ops::Range;

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, html};

/// First start-sentinel code point: the start marker for span `i` is
/// `U+E100 + i`. Both markers carry the span's identity through rendering
/// so a marker that gets consumed (e.g. percent-encoded inside a link
/// destination) degrades only *its own* highlight instead of shifting or
/// spilling any other highlight.
const HL_START_BASE: u32 = 0xE100;
/// First end-sentinel code point: the end marker for span `i` is
/// `U+E200 + i`.
const HL_END_BASE: u32 = 0xE200;
/// Maximum number of inline highlights per document (each marker block
/// spans 0x100 code points); spans beyond this degrade to preview-only.
const HL_MAX_SPANS: usize = 0x100;

/// Whether `c` falls in the sentinel block reserved by this module
/// (`U+E000..=U+E2FF`), and must therefore be sanitized out of input.
fn is_sentinel(c: char) -> bool {
    (0xE000..=0xE2FF).contains(&(c as u32))
}

/// The span index carried by a start marker, if `c` is one.
fn start_index(c: char) -> Option<usize> {
    let u = c as u32;
    (HL_START_BASE..HL_START_BASE + HL_MAX_SPANS as u32)
        .contains(&u)
        .then(|| (u - HL_START_BASE) as usize)
}

/// The span index carried by an end marker, if `c` is one.
fn end_index(c: char) -> Option<usize> {
    let u = c as u32;
    (HL_END_BASE..HL_END_BASE + HL_MAX_SPANS as u32)
        .contains(&u)
        .then(|| (u - HL_END_BASE) as usize)
}

/// Renders Markdown to HTML with the core GFM extensions enabled and raw
/// HTML disabled.
///
/// Enabled pulldown-cmark options:
///
/// - `ENABLE_TABLES` — GFM pipe tables
/// - `ENABLE_STRIKETHROUGH` — `~~text~~`
/// - `ENABLE_TASKLISTS` — `- [ ]` / `- [x]`
/// - `ENABLE_GFM` — pulldown-cmark's umbrella GFM flag; enables
///   GitHub-style `[!NOTE]`/`[!TIP]`/etc. blockquote alerts and
///   GFM-spec event handling beyond the per-feature flags above.
///
/// These match the GFM dialect that LLM-authored roadmap, phase, and task
/// bodies routinely emit; without them the source syntax leaks through as
/// literal text in the rendered HTML.
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
    let parser = Parser::new_ext(input, cmark_options()).filter(|event| {
        !matches!(
            event,
            Event::Html(_)
                | Event::InlineHtml(_)
                | Event::Start(Tag::HtmlBlock)
                | Event::End(TagEnd::HtmlBlock)
        )
    });

    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    html_output
}

/// The single set of pulldown-cmark options every rendering entry point
/// uses (see [`render_markdown`] for the rationale behind each flag).
fn cmark_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
}

/// Whether `event` is user-authored raw HTML that must be dropped (the
/// same filter [`render_markdown`] applies inline).
fn is_user_html(event: &Event<'_>) -> bool {
    matches!(
        event,
        Event::Html(_)
            | Event::InlineHtml(_)
            | Event::Start(Tag::HtmlBlock)
            | Event::End(TagEnd::HtmlBlock)
    )
}

/// One visible text run of a markdown document: the source byte range it
/// was parsed from and the text it renders as.
///
/// Runs come from pulldown-cmark `Text` and inline `Code` events (image
/// alt text is excluded — it renders into an attribute, not the DOM text
/// flow). For plain text runs `content` equals the source slice at
/// `range`; for inline code and entity/escape runs the two differ (`range`
/// includes backticks or the entity spelling), which callers detect by
/// comparing `content.as_bytes()` against the source slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRun {
    /// Byte range into the markdown source this run was parsed from.
    pub range: Range<usize>,
    /// The visible text the run renders as.
    pub content: String,
}

/// Collects the visible text runs of `source`, in document order.
///
/// This is the server-side mirror of the DOM text nodes that
/// [`render_markdown_annotated`] wraps in `rdm-src` spans: the `n`-th run
/// here corresponds to the `n`-th annotated span there, with the same
/// source range. Used by selection→anchor validation to reconstruct the
/// rendered text of a source byte range.
#[must_use]
pub fn source_runs(source: &str) -> Vec<SourceRun> {
    let mut runs = Vec::new();
    let mut in_image = 0usize;
    for (event, range) in Parser::new_ext(source, cmark_options()).into_offset_iter() {
        if is_user_html(&event) {
            continue;
        }
        match &event {
            Event::Start(Tag::Image { .. }) => in_image += 1,
            Event::End(TagEnd::Image) => in_image = in_image.saturating_sub(1),
            Event::Text(t) | Event::Code(t) if in_image == 0 => runs.push(SourceRun {
                range,
                content: t.to_string(),
            }),
            _ => {}
        }
    }
    runs
}

/// Renders Markdown to HTML like [`render_markdown`], additionally
/// wrapping every visible text run in
/// `<span class="rdm-src" data-so="{start}" data-se="{end}">`, where the
/// two attributes are the run's **source** byte range.
///
/// This is the read-back half of the select-to-anchor flow: client-side
/// JavaScript maps a DOM selection to source byte offsets by walking these
/// spans, and the server re-validates the mapping before storing an
/// anchor. The spans wrap exactly the runs reported by [`source_runs`],
/// in the same order.
///
/// Rules:
///
/// - Plain text runs are wrapped as-is; their text content equals the
///   source slice, so clients can do exact within-run offset arithmetic.
/// - Inline code runs are wrapped *inside* their `<code>` element; the
///   annotated range includes the backticks, so the span's text content is
///   shorter than its source range — clients treat such runs as opaque and
///   snap selections to the whole run.
/// - Image alt text is **not** annotated: it renders into the `alt`
///   attribute, where an injected span would corrupt the markup.
/// - Raw user HTML is stripped exactly as in [`render_markdown`].
///
/// One `<span>` per text run is a deliberate DOM-weight trade-off: plan
/// bodies are small, and per-run annotation makes the client's offset
/// arithmetic exact instead of heuristic.
///
/// Annotations and inline review highlights
/// ([`render_markdown_with_highlights`]) are mutually exclusive render
/// modes: pages render annotations only while the viewer has an open
/// draft (the only state in which the selection gesture is usable) and
/// highlights otherwise, so the two instrumentation schemes never have to
/// compose.
#[must_use]
pub fn render_markdown_annotated(source: &str) -> String {
    let mut events: Vec<Event> = Vec::new();
    let mut in_image = 0usize;
    for (event, range) in Parser::new_ext(source, cmark_options()).into_offset_iter() {
        if is_user_html(&event) {
            continue;
        }
        match event {
            Event::Start(Tag::Image { .. }) => {
                in_image += 1;
                events.push(event);
            }
            Event::End(TagEnd::Image) => {
                in_image = in_image.saturating_sub(1);
                events.push(event);
            }
            Event::Text(_) if in_image == 0 => {
                events.push(Event::Html(open_run_span(&range).into()));
                events.push(event);
                events.push(Event::Html("</span>".into()));
            }
            Event::Code(text) if in_image == 0 => {
                // Replace the default `<code>…</code>` rendering so the
                // annotation span sits on the content node inside it.
                events.push(Event::Html(
                    format!("<code>{}", open_run_span(&range)).into(),
                ));
                // Re-emitted as a text event so push_html escapes it.
                events.push(Event::Text(text));
                events.push(Event::Html("</span></code>".into()));
            }
            _ => events.push(event),
        }
    }
    let mut out = String::new();
    html::push_html(&mut out, events.into_iter());
    out
}

/// The opening tag of one `rdm-src` annotation span.
fn open_run_span(range: &Range<usize>) -> String {
    format!(
        "<span class=\"rdm-src\" data-so=\"{}\" data-se=\"{}\">",
        range.start, range.end
    )
}

/// One resolved review-comment anchor to highlight inline in a rendered
/// body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightSpan {
    /// Byte range into the markdown **source** (not the rendered HTML), as
    /// produced by `rdm_core::anchor::resolve_comments` against the same
    /// body being rendered. Must lie on `char` boundaries; ranges that
    /// don't (or that fall outside the source) are silently dropped.
    pub range: Range<usize>,
    /// Value emitted as the `<mark>`'s `data-rdm-anchor` attribute, tying
    /// the inline highlight back to its comment in the reviews section.
    pub anchor_ref: String,
}

/// Renders Markdown to HTML like [`render_markdown`], additionally wrapping
/// each highlight span in `<mark class="rdm-anchor" data-rdm-anchor="…">`.
///
/// Mechanism: private-use sentinel characters are spliced into a sanitized
/// copy of the source at each span boundary, the instrumented source is
/// rendered through the same pipeline as [`render_markdown`], and the
/// sentinels — which survive rendering as ordinary text characters — are
/// replaced by `<mark>`/`</mark>` tags in a linear post-pass. Both markers
/// of a span carry the span's index in the code point itself, and a
/// survivorship pre-pass over the rendered output determines which spans
/// have **both** markers surviving, outside HTML tags, in start-before-end
/// order; only those spans materialize as `<mark>` pairs, and every other
/// sentinel is stripped. The output is therefore always well-formed: no
/// orphan close tags, no auto-closed spills past unrelated text.
///
/// Robustness rules, in order:
///
/// - Pre-existing characters in the reserved sentinel block
///   (`U+E000..=U+E2FF`) are replaced with U+FFFD *before* splicing, so
///   adversarial or accidental private-use characters can never masquerade
///   as markers and misplace a highlight. (Every character involved is
///   3 bytes in UTF-8, so span offsets remain valid.)
/// - Spans out of bounds or off `char` boundaries are dropped.
/// - Overlapping spans: the earlier (by start offset) is kept, later
///   overlapping ones are dropped from inline rendering — their comments
///   still show the quote preview.
/// - A marker consumed during rendering (e.g. percent-encoded inside a
///   link destination) or surfacing only inside an HTML tag (attribute
///   values) fails its span's survivorship check: that span degrades to
///   quote-preview-only — whether it lost its start, its end, or both —
///   and every other span renders under its own `anchor_ref`, unshifted.
/// - A highlight spanning block boundaries emits one `<mark>` per source
///   span; the browser's parser may truncate it at the first block edge —
///   the quote preview remains the authoritative fallback.
///
/// # Known edge case: emphasis-classification drift
///
/// A sentinel character counts as an "other" (non-punctuation,
/// non-whitespace) character in CommonMark's emphasis flanking rules, so a
/// highlight boundary that lands *directly against* underscore emphasis
/// which is only valid because of adjacent punctuation can change how that
/// emphasis parses — e.g. `(_bar_)` renders `<em>bar</em>`, but with a
/// highlight starting immediately after `(` the `_` is no longer preceded
/// by punctuation and the emphasis is lost (the literal `_bar_` is
/// rendered, still correctly highlighted). This is an accepted, rare
/// formatting drift: the highlighted text is always right, only its
/// emphasis styling may degrade. Pinned by
/// `highlight_boundary_can_drop_punctuation_dependent_emphasis`.
pub fn render_markdown_with_highlights(source: &str, highlights: &[HighlightSpan]) -> String {
    // Sanitize pre-existing sentinel-block characters first (same-width
    // replacement, so the caller's byte offsets stay valid).
    let sanitized: String = source
        .chars()
        .map(|c| if is_sentinel(c) { '\u{FFFD}' } else { c })
        .collect();
    debug_assert_eq!(sanitized.len(), source.len());

    // Keep only in-bounds, char-boundary spans; then drop overlaps,
    // keeping the earliest span by start offset.
    let mut spans: Vec<&HighlightSpan> = highlights
        .iter()
        .filter(|h| {
            h.range.start <= h.range.end
                && h.range.end <= sanitized.len()
                && sanitized.is_char_boundary(h.range.start)
                && sanitized.is_char_boundary(h.range.end)
        })
        .collect();
    spans.sort_by_key(|h| (h.range.start, h.range.end));
    let mut kept: Vec<&HighlightSpan> = Vec::with_capacity(spans.len());
    for span in spans {
        match kept.last() {
            Some(prev) if span.range.start < prev.range.end => {} // overlap: drop
            _ => kept.push(span),
        }
    }
    kept.truncate(HL_MAX_SPANS);

    if kept.is_empty() {
        return render_markdown(&sanitized);
    }

    // Splice sentinels in descending offset order so earlier offsets stay
    // valid. Spans are non-overlapping, so per span the end goes in first.
    let mut instrumented = sanitized;
    for (i, span) in kept.iter().enumerate().rev() {
        let start_sentinel =
            char::from_u32(HL_START_BASE + i as u32).expect("PUA code point is a valid char");
        let end_sentinel =
            char::from_u32(HL_END_BASE + i as u32).expect("PUA code point is a valid char");
        instrumented.insert(span.range.end, end_sentinel);
        instrumented.insert(span.range.start, start_sentinel);
    }

    let rendered = render_markdown(&instrumented);

    // Survivorship pre-pass: a span materializes only when both its markers
    // survived rendering, outside HTML tags, in start-before-end order.
    // `in_tag` bracket tracking is exact on this machine-generated output:
    // attribute values entity-escape `<`/`>`. A marker consumed by
    // rendering (e.g. percent-encoded into a link href) or surfacing only
    // inside a tag fails the check, degrading exactly that span to its
    // quote preview.
    let mut start_seen = vec![false; kept.len()];
    let mut survives = vec![false; kept.len()];
    let mut in_tag = false;
    for c in rendered.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if in_tag => {}
            c => {
                if let Some(i) = start_index(c) {
                    if let Some(seen) = start_seen.get_mut(i) {
                        *seen = true;
                    }
                } else if let Some(i) = end_index(c)
                    && start_seen.get(i).copied().unwrap_or(false)
                    && let Some(s) = survives.get_mut(i)
                {
                    *s = true;
                }
            }
        }
    }

    // Emit pass: materialize <mark> pairs for surviving spans, strip every
    // other sentinel (including any surfacing inside a tag).
    let mut out = String::with_capacity(rendered.len() + kept.len() * 48);
    let mut in_tag = false;
    for c in rendered.chars() {
        match c {
            '<' => {
                in_tag = true;
                out.push(c);
            }
            '>' => {
                in_tag = false;
                out.push(c);
            }
            c if is_sentinel(c) => {
                if in_tag {
                    continue;
                }
                if let Some(i) = start_index(c)
                    && survives.get(i).copied().unwrap_or(false)
                {
                    let span = &kept[i];
                    out.push_str("<mark class=\"rdm-anchor\" data-rdm-anchor=\"");
                    push_attr_escaped(&mut out, &span.anchor_ref);
                    out.push_str("\">");
                } else if let Some(i) = end_index(c)
                    && survives.get(i).copied().unwrap_or(false)
                {
                    out.push_str("</mark>");
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Appends `value` to `out` with the five HTML attribute-significant
/// characters escaped.
fn push_attr_escaped(out: &mut String, value: &str) {
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
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

    #[test]
    fn renders_pipe_table() {
        let html = render_markdown("| a | b |\n|---|---|\n| 1 | 2 |\n");
        let thead_idx = html.find("<thead>").expect("thead present");
        let tbody_idx = html.find("<tbody>").expect("tbody present");
        assert!(thead_idx < tbody_idx, "<thead> must precede <tbody>");
        assert!(html.contains("<th>a</th>"));
        assert!(html.contains("<th>b</th>"));
        assert!(html.contains("<td>1</td>"));
        assert!(html.contains("<td>2</td>"));
        let thead_section = &html[thead_idx..tbody_idx];
        assert!(thead_section.contains("<th>a</th>"));
        assert!(thead_section.contains("<th>b</th>"));
    }

    #[test]
    fn renders_strikethrough() {
        let html = render_markdown("~~gone~~");
        assert!(html.contains("<del>gone</del>"));
    }

    #[test]
    fn renders_task_list_item() {
        let html = render_markdown("- [x] done\n- [ ] todo\n");
        assert!(
            html.contains(r#"<input disabled="" type="checkbox" checked=""/>"#),
            "expected a checked checkbox: {html}",
        );
        assert!(
            html.contains(r#"<input disabled="" type="checkbox"/>"#),
            "expected an unchecked checkbox (no `checked` attribute): {html}",
        );
    }

    #[test]
    fn renders_note_callout() {
        let html = render_markdown("> [!NOTE]\n> heads up\n");
        assert!(
            html.contains(r#"<blockquote class="markdown-alert-note">"#),
            "expected GFM alert blockquote: {html}",
        );
        assert!(html.contains("heads up"));
    }

    // -- render_markdown_with_highlights --

    fn hl(range: Range<usize>, anchor_ref: &str) -> HighlightSpan {
        HighlightSpan {
            range,
            anchor_ref: anchor_ref.to_string(),
        }
    }

    /// Byte range of `needle` within `haystack`, for readable span setup.
    fn range_of(haystack: &str, needle: &str) -> Range<usize> {
        let start = haystack.find(needle).expect("needle present");
        start..start + needle.len()
    }

    #[test]
    fn highlight_wraps_single_word() {
        let src = "The quick brown fox.";
        let html = render_markdown_with_highlights(src, &[hl(range_of(src, "quick"), "r1-c1")]);
        assert!(
            html.contains(r#"<mark class="rdm-anchor" data-rdm-anchor="r1-c1">quick</mark>"#),
            "got: {html}"
        );
    }

    #[test]
    fn highlight_spans_bold_boundary() {
        let src = "start **bold** end";
        let html =
            render_markdown_with_highlights(src, &[hl(range_of(src, "start **bold**"), "a")]);
        assert!(
            html.contains(r#"<mark class="rdm-anchor" data-rdm-anchor="a">start <strong>bold</strong></mark> end"#),
            "got: {html}"
        );
    }

    #[test]
    fn highlight_multiple_non_overlapping_ranges() {
        let src = "alpha beta gamma delta";
        let html = render_markdown_with_highlights(
            src,
            &[
                hl(range_of(src, "gamma"), "second"),
                hl(range_of(src, "alpha"), "first"),
            ],
        );
        let first = html.find(r#"data-rdm-anchor="first">alpha</mark>"#);
        let second = html.find(r#"data-rdm-anchor="second">gamma</mark>"#);
        assert!(first.is_some() && second.is_some(), "got: {html}");
        assert!(first < second, "refs must follow span order: {html}");
    }

    #[test]
    fn highlight_overlapping_range_keeps_earlier_drops_later() {
        let src = "one two three four";
        let html = render_markdown_with_highlights(
            src,
            &[
                hl(range_of(src, "one two"), "kept"),
                hl(range_of(src, "two three"), "dropped"),
            ],
        );
        assert!(
            html.contains(r#"data-rdm-anchor="kept">one two</mark>"#),
            "got: {html}"
        );
        assert!(!html.contains("dropped"), "overlap must be dropped: {html}");
    }

    #[test]
    fn highlight_out_of_bounds_range_is_dropped() {
        let src = "short body";
        let html = render_markdown_with_highlights(src, &[hl(0..999, "x")]);
        assert_eq!(html, render_markdown(src));
    }

    #[test]
    fn highlight_non_char_boundary_range_is_dropped() {
        let src = "héllo world";
        // é occupies bytes 1..3, so an end offset of 2 lands mid-char.
        let html = render_markdown_with_highlights(src, &[hl(1..2, "x")]);
        assert_eq!(html, render_markdown(src));
    }

    #[test]
    fn highlight_empty_slice_matches_plain_render() {
        let src = "# Heading\n\nSome **bold** text.\n";
        assert_eq!(
            render_markdown_with_highlights(src, &[]),
            render_markdown(src)
        );
    }

    #[test]
    fn highlight_still_strips_raw_html() {
        let src = "before <script>alert('x')</script> after";
        let html = render_markdown_with_highlights(src, &[hl(range_of(src, "before"), "a")]);
        assert!(!html.contains("<script>"), "got: {html}");
        assert!(
            html.contains(r#"data-rdm-anchor="a">before</mark>"#),
            "got: {html}"
        );
    }

    #[test]
    fn highlight_multibyte_content_is_char_boundary_safe() {
        let src = "Café notes: the résumé draft — naïve.";
        let html = render_markdown_with_highlights(src, &[hl(range_of(src, "résumé"), "mb")]);
        assert!(
            html.contains(r#"data-rdm-anchor="mb">résumé</mark>"#),
            "got: {html}"
        );
    }

    #[test]
    fn highlight_inside_list_item_and_table_cell() {
        let src = "- first item\n- second item\n\n| a | b |\n|---|---|\n| cell one | cell two |\n";
        let html = render_markdown_with_highlights(
            src,
            &[
                hl(range_of(src, "second item"), "li"),
                hl(range_of(src, "cell two"), "td"),
            ],
        );
        assert!(
            html.contains(r#"data-rdm-anchor="li">second item</mark>"#),
            "got: {html}"
        );
        assert!(
            html.contains(r#"data-rdm-anchor="td">cell two</mark>"#),
            "got: {html}"
        );
    }

    #[test]
    fn highlight_anchor_ref_attribute_is_escaped() {
        let src = "hello world";
        let html =
            render_markdown_with_highlights(src, &[hl(range_of(src, "hello"), r#"a"b<c>&d"#)]);
        assert!(
            html.contains(r#"data-rdm-anchor="a&quot;b&lt;c&gt;&amp;d">hello</mark>"#),
            "got: {html}"
        );
    }

    /// A body carrying a literal pre-existing sentinel character must not
    /// consume a highlight marker or misplace any highlight: the sentinel
    /// is sanitized to U+FFFD before instrumentation.
    #[test]
    fn highlight_sanitizes_pre_existing_sentinel_chars() {
        let src = "evil \u{E000} and \u{E100} and \u{E001} then the quoted span here.";
        let html = render_markdown_with_highlights(src, &[hl(range_of(src, "quoted span"), "c1")]);
        assert!(
            html.contains(r#"data-rdm-anchor="c1">quoted span</mark>"#),
            "highlight must land despite hostile sentinels: {html}"
        );
        assert_eq!(
            html.matches("<mark").count(),
            1,
            "exactly one mark, no strays: {html}"
        );
        assert!(!html.contains('\u{E000}') && !html.contains('\u{E001}'));
        assert!(
            html.contains('\u{FFFD}'),
            "sanitized chars become U+FFFD: {html}"
        );
    }

    /// Pins the accepted emphasis-classification drift documented on
    /// [`render_markdown_with_highlights`]: a highlight boundary directly
    /// against punctuation-dependent underscore emphasis suppresses the
    /// `<em>` (the text renders literally, still correctly highlighted).
    #[test]
    fn highlight_boundary_can_drop_punctuation_dependent_emphasis() {
        let src = "(_bar_)";
        assert!(
            render_markdown(src).contains("<em>bar</em>"),
            "baseline: plain render keeps the emphasis"
        );
        let html = render_markdown_with_highlights(src, &[hl(range_of(src, "_bar_"), "e")]);
        assert!(
            !html.contains("<em>"),
            "known drift: sentinel breaks the preceded-by-punctuation exception: {html}"
        );
        assert!(
            html.contains(r#"data-rdm-anchor="e">_bar_</mark>"#),
            "the span itself is still highlighted, literally: {html}"
        );
    }

    /// A span whose END marker is consumed by rendering (percent-encoded
    /// into a link href) while its start survives must not emit a mark at
    /// all — previously the open `<mark>` was auto-closed at end of
    /// document, producing `</mark>` after `</p>` (invalid nesting).
    #[test]
    fn highlight_consumed_end_emits_no_mark_and_stays_well_formed() {
        let src = "intro [text](http://example.com/target) tail";
        let url_mid = src.find("target").unwrap() + 3;
        let html = render_markdown_with_highlights(src, &[hl(0..url_mid, "gone")]);
        assert!(
            !html.contains("<mark"),
            "span must degrade entirely: {html}"
        );
        assert!(!html.contains("</mark>"), "no orphan close tag: {html}");
        assert!(
            !html.contains("</p></mark>"),
            "no invalid nesting after the paragraph close: {html}"
        );
        for c in html.chars() {
            assert!(!is_sentinel(c), "no sentinel may leak: {html}");
        }
        assert!(html.contains("<a href="), "link must survive: {html}");
    }

    /// Multi-span variant of the consumed-END case: the broken span
    /// degrades alone; a later unrelated span still renders under its own
    /// `anchor_ref`, unshifted, and the output stays balanced.
    #[test]
    fn highlight_consumed_end_does_not_spill_into_sibling_span() {
        let src = "intro [text](http://example.com/target) mid tail";
        let url_mid = src.find("target").unwrap() + 3;
        let html = render_markdown_with_highlights(
            src,
            &[hl(0..url_mid, "broken"), hl(range_of(src, "tail"), "ok")],
        );
        assert!(
            !html.contains("broken"),
            "consumed-end span must not render: {html}"
        );
        assert!(
            html.contains(r#"<mark class="rdm-anchor" data-rdm-anchor="ok">tail</mark>"#),
            "sibling span must render unshifted: {html}"
        );
        assert_eq!(html.matches("<mark").count(), 1, "exactly one mark: {html}");
        assert_eq!(html.matches("</mark>").count(), 1, "balanced marks: {html}");
        assert!(
            !html.contains("mid</mark>"),
            "mark must not spill over unrelated text: {html}"
        );
    }

    /// Multi-span variant of the consumed-START case: a span whose start
    /// marker is percent-encoded into a link href degrades alone; the
    /// surviving sibling renders correctly under its own `anchor_ref`.
    #[test]
    fn highlight_consumed_start_does_not_shift_sibling_span() {
        let src = "pre [a](http://example.com/xyz) mid tail";
        let url_mid = src.find("xyz").unwrap() + 1;
        let end_in_text = src.find(" tail").unwrap();
        let html = render_markdown_with_highlights(
            src,
            &[
                hl(url_mid..end_in_text, "broken"),
                hl(range_of(src, "tail"), "sib"),
            ],
        );
        assert!(
            !html.contains("broken"),
            "consumed-start span must not render: {html}"
        );
        assert!(
            html.contains(r#"<mark class="rdm-anchor" data-rdm-anchor="sib">tail</mark>"#),
            "sibling span must render unshifted: {html}"
        );
        assert_eq!(html.matches("<mark").count(), 1, "exactly one mark: {html}");
        assert_eq!(html.matches("</mark>").count(), 1, "balanced marks: {html}");
        for c in html.chars() {
            assert!(!is_sentinel(c), "no sentinel may leak: {html}");
        }
    }

    /// A range inside a link destination never surfaces its sentinel in the
    /// output (pulldown percent-encodes it into the href); the highlight is
    /// silently dropped and the output stays valid, sentinel-free HTML.
    #[test]
    fn highlight_inside_link_destination_degrades_cleanly() {
        let src = "[text](http://example.com/path) tail";
        let html = render_markdown_with_highlights(src, &[hl(range_of(src, "path"), "u")]);
        assert!(!html.contains("<mark"), "got: {html}");
        for c in html.chars() {
            assert!(!is_sentinel(c), "no sentinel may leak: {html}");
        }
        assert!(html.contains("<a href="), "link must survive: {html}");
    }

    // -- render_markdown_annotated / source_runs --

    /// Extracts each annotated span's `(start, end)` from rendered HTML, in
    /// document order.
    fn annotated_ranges(html: &str) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut rest = html;
        while let Some(i) = rest.find("data-so=\"") {
            let after = &rest[i + 9..];
            let so: usize = after[..after.find('"').unwrap()].parse().unwrap();
            let j = after.find("data-se=\"").unwrap();
            let after_e = &after[j + 9..];
            let se: usize = after_e[..after_e.find('"').unwrap()].parse().unwrap();
            out.push((so, se));
            rest = after_e;
        }
        out
    }

    #[test]
    fn annotated_wraps_text_runs_with_source_offsets() {
        let src = "start **bold** end";
        let html = render_markdown_annotated(src);
        // Each run's data range slices back to exactly its rendered text.
        for (so, se) in annotated_ranges(&html) {
            let slice = &src[so..se];
            assert!(
                html.contains(&format!(
                    "<span class=\"rdm-src\" data-so=\"{so}\" data-se=\"{se}\">{slice}</span>"
                )),
                "span {so}..{se} must wrap its source slice {slice:?}: {html}"
            );
        }
        // The three runs of a bold-crossing paragraph, structure intact.
        assert_eq!(annotated_ranges(&html).len(), 3, "got: {html}");
        assert!(html.contains("<strong>"), "got: {html}");
    }

    #[test]
    fn annotated_list_heading_and_table_runs_slice_back() {
        let src =
            "# Big Heading\n\n- first item\n- second item\n\n| a | b |\n|---|---|\n| c1 | c2 |\n";
        let html = render_markdown_annotated(src);
        for needle in ["Big Heading", "second item", "c2"] {
            let so = src.find(needle).unwrap();
            let se = so + needle.len();
            assert!(
                html.contains(&format!(
                    "data-so=\"{so}\" data-se=\"{se}\">{needle}</span>"
                )),
                "run for {needle:?} must be annotated: {html}"
            );
        }
    }

    #[test]
    fn annotated_inline_code_run_includes_backticks() {
        let src = "use `foo()` here";
        let html = render_markdown_annotated(src);
        let code_range = range_of(src, "`foo()`");
        // The annotation sits inside the <code> element and spans the
        // backtick-inclusive source range while wrapping only the content.
        assert!(
            html.contains(&format!(
                "<code><span class=\"rdm-src\" data-so=\"{}\" data-se=\"{}\">foo()</span></code>",
                code_range.start, code_range.end
            )),
            "got: {html}"
        );
    }

    #[test]
    fn annotated_skips_image_alt_text() {
        let src = "before ![alt text](http://example.com/i.png) after";
        let html = render_markdown_annotated(src);
        assert!(
            html.contains(r#"alt="alt text""#),
            "alt attribute must stay clean: {html}"
        );
        assert!(
            !html.contains("alt=\"<span"),
            "no span may leak into the alt attribute: {html}"
        );
        // The surrounding text runs are still annotated.
        assert!(html.contains(">before </span>"), "got: {html}");
        assert!(html.contains("> after</span>"), "got: {html}");
    }

    #[test]
    fn annotated_still_strips_raw_html() {
        // Inline: the tags are dropped, the inner text stays inert —
        // matching render_markdown.
        let src = "before <script>alert('x')</script> after";
        let html = render_markdown_annotated(src);
        assert!(!html.contains("<script>"), "got: {html}");
        // Block: the whole HTML block (tags and content) is dropped.
        let block = render_markdown_annotated("<script>alert('x')</script>");
        assert!(!block.contains("script"), "got: {block}");
        assert!(!block.contains("alert"), "got: {block}");
    }

    #[test]
    fn annotated_multibyte_offsets_slice_back() {
        let src = "Café **résumé** naïve";
        let html = render_markdown_annotated(src);
        let r = range_of(src, "résumé");
        assert!(
            html.contains(&format!(
                "data-so=\"{}\" data-se=\"{}\">résumé</span>",
                r.start, r.end
            )),
            "got: {html}"
        );
    }

    #[test]
    fn annotated_code_block_lines_are_annotated() {
        let src = "```\nfn main() {}\n```\n";
        let html = render_markdown_annotated(src);
        let r = range_of(src, "fn main() {}\n");
        assert!(
            html.contains(&format!("data-so=\"{}\" data-se=\"{}\">", r.start, r.end)),
            "fenced code content must be annotated: {html}"
        );
    }

    #[test]
    fn source_runs_match_annotated_offsets() {
        let src = "# H\n\npara with `code` and **bold** plus AT&amp;T.\n\n- item une\n";
        let runs = source_runs(src);
        let spans = annotated_ranges(&render_markdown_annotated(src));
        assert_eq!(
            runs.iter()
                .map(|r| (r.range.start, r.range.end))
                .collect::<Vec<_>>(),
            spans,
            "source_runs and the annotated renderer must agree run-for-run"
        );
    }

    #[test]
    fn source_runs_plain_text_content_equals_source_slice() {
        let src = "plain text run";
        let runs = source_runs(src);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].content, "plain text run");
        assert_eq!(&src[runs[0].range.clone()], "plain text run");
    }

    #[test]
    fn source_runs_entity_run_is_opaque() {
        let src = "AT&amp;T works";
        let runs = source_runs(src);
        // The entity run's content ("&") differs from its source slice
        // ("&amp;") — the opacity signal clients and the server key off.
        let entity = runs
            .iter()
            .find(|r| r.content == "&")
            .expect("entity run present");
        assert_eq!(&src[entity.range.clone()], "&amp;");
    }

    #[test]
    fn source_runs_exclude_image_alt() {
        let src = "before ![alt text](http://example.com/i.png) after";
        let runs = source_runs(src);
        assert!(
            runs.iter().all(|r| r.content != "alt text"),
            "image alt must not be a run: {runs:?}"
        );
        assert!(runs.iter().any(|r| r.content == "before "));
    }
}
