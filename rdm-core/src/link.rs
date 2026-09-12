//! The `rdm:` link scheme: item references (`rdm:roadmap/<slug>`,
//! `rdm:phase/<roadmap-slug>/<stem>`, `rdm:task/<slug>`) and code
//! references (`rdm:src/<path>[@<rev>][#L<start>[-L<end>]]`), plus
//! [`extract_links`] to find them inside a markdown body.
//!
//! Item-reference syntax is shared with
//! [`ReviewTarget`](crate::model::ReviewTarget) — the same vocabulary used
//! by `Done:` lines and `rdm review --on` — via its `FromStr`/`Display`
//! impls, so links, `Done:` directives, and review targets never drift into
//! three separate parsers.

use std::ops::Range;
use std::str::FromStr;

use pulldown_cmark::{Event, Options, Parser, Tag};
use serde::Serialize;

/// A plan item reference (`roadmap/<slug>`, `phase/<roadmap-slug>/<stem>`,
/// or `task/<slug>`) — syntactically and semantically identical to a
/// [`crate::model::ReviewTarget`], so the two share one type rather than
/// duplicating the grammar.
pub type ItemRef = crate::model::ReviewTarget;

/// A parsed `rdm:` link destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// A reference to a roadmap, phase, or task.
    Item(ItemRef),
    /// A reference to a location in the source repository.
    Code {
        /// Path to the file, relative to the source repository root.
        path: String,
        /// Optional revision (commit SHA, branch, or tag) the path is
        /// resolved against. `None` means the project's configured default
        /// branch.
        rev: Option<String>,
        /// Optional line range within the file: `(start, None)` for a
        /// single line, `(start, Some(end))` for an inclusive range.
        lines: Option<(u32, Option<u32>)>,
    },
}

/// Errors returned by [`parse`] for a malformed `rdm:` URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkParseError {
    /// The URI did not start with the `rdm:` scheme prefix.
    MissingScheme(String),
    /// The URI's first path segment was not `roadmap`, `phase`, `task`, or
    /// `src`.
    UnknownKind {
        /// The full URI that failed to parse.
        uri: String,
        /// The unrecognized first path segment.
        kind: String,
    },
    /// An `rdm:src/` URI had no path after the `src/` prefix.
    EmptyPath(String),
    /// An `rdm:roadmap/`, `rdm:phase/`, or `rdm:task/` URI's remainder did
    /// not match the shared item-reference grammar.
    InvalidItemRef {
        /// The full URI that failed to parse.
        uri: String,
        /// The underlying item-reference parse error.
        source: crate::model::ParseError,
    },
    /// An `rdm:src/` URI's `#`-fragment was not a valid `L<n>` or
    /// `L<n>-L<n>` line range.
    InvalidLineRange {
        /// The full URI that failed to parse.
        uri: String,
        /// The offending fragment (the text after `#`).
        fragment: String,
    },
    /// An `rdm:src/` URI's line range had its end before its start.
    InvertedLineRange {
        /// The full URI that failed to parse.
        uri: String,
        /// The range's start line.
        start: u32,
        /// The range's end line.
        end: u32,
    },
}

impl std::fmt::Display for LinkParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkParseError::MissingScheme(uri) => {
                write!(f, "'{uri}' is not an rdm: link — missing the rdm: scheme")
            }
            LinkParseError::UnknownKind { uri, kind } => {
                write!(
                    f,
                    "unknown link kind '{kind}' in '{uri}' — expected roadmap, phase, task, or src"
                )
            }
            LinkParseError::EmptyPath(uri) => {
                write!(f, "'{uri}' has an empty src path")
            }
            LinkParseError::InvalidItemRef { uri, source } => {
                write!(f, "invalid item reference in '{uri}': {source}")
            }
            LinkParseError::InvalidLineRange { uri, fragment } => {
                write!(
                    f,
                    "invalid line range '{fragment}' in '{uri}' — expected L<n> or L<n>-L<n>"
                )
            }
            LinkParseError::InvertedLineRange { uri, start, end } => {
                write!(
                    f,
                    "line range {start}-{end} in '{uri}' is inverted — end must be >= start"
                )
            }
        }
    }
}

impl std::error::Error for LinkParseError {}

/// Parses an `rdm:` URI into a [`Link`].
///
/// Item form: `rdm:<kind>/<ref>` where `<kind>` is `roadmap`, `phase`, or
/// `task` and `<ref>` follows [`ItemRef`]'s grammar.
///
/// Code form: `rdm:src/<path>[@<rev>][#L<start>[-L<end>]]`. The split
/// assumes a source path never itself contains a literal `@` or `#` — the
/// path is everything up to the first of either character.
///
/// # Errors
///
/// Returns [`LinkParseError::MissingScheme`] if `uri` does not start with
/// `rdm:`, [`LinkParseError::UnknownKind`] if the first path segment is not
/// `roadmap`, `phase`, `task`, or `src`, [`LinkParseError::InvalidItemRef`]
/// if an item form's remainder does not match [`ItemRef`]'s grammar,
/// [`LinkParseError::EmptyPath`] if a `src` form has no path,
/// [`LinkParseError::InvalidLineRange`] if a `src` form's `#`-fragment is
/// not `L<n>` or `L<n>-L<n>`, or [`LinkParseError::InvertedLineRange`] if
/// the range's end is before its start.
pub fn parse(uri: &str) -> Result<Link, LinkParseError> {
    let rest = uri
        .strip_prefix("rdm:")
        .ok_or_else(|| LinkParseError::MissingScheme(uri.to_string()))?;
    let (kind, remainder) = rest.split_once('/').unwrap_or((rest, ""));
    match kind {
        "roadmap" | "phase" | "task" => {
            let item_ref_str = format!("{kind}/{remainder}");
            let item_ref = ItemRef::from_str(&item_ref_str).map_err(|source| {
                LinkParseError::InvalidItemRef {
                    uri: uri.to_string(),
                    source,
                }
            })?;
            Ok(Link::Item(item_ref))
        }
        "src" => parse_code(uri, remainder),
        other => Err(LinkParseError::UnknownKind {
            uri: uri.to_string(),
            kind: other.to_string(),
        }),
    }
}

/// Parses the `<path>[@<rev>][#fragment]` remainder of an `rdm:src/` URI.
fn parse_code(uri: &str, rest: &str) -> Result<Link, LinkParseError> {
    // Path is everything up to the first of `@` or `#`; whichever comes
    // first ends it.
    let split_at = rest.find(['@', '#']).unwrap_or(rest.len());
    let path = &rest[..split_at];
    let after_path = &rest[split_at..];

    if path.is_empty() {
        return Err(LinkParseError::EmptyPath(uri.to_string()));
    }

    let (rev, fragment) = if let Some(after_at) = after_path.strip_prefix('@') {
        match after_at.find('#') {
            Some(hash_idx) => (Some(&after_at[..hash_idx]), Some(&after_at[hash_idx + 1..])),
            None => (Some(after_at), None),
        }
    } else if let Some(after_hash) = after_path.strip_prefix('#') {
        (None, Some(after_hash))
    } else {
        (None, None)
    };

    let lines = match fragment {
        Some(fragment) => Some(parse_line_range(uri, fragment)?),
        None => None,
    };

    Ok(Link::Code {
        path: path.to_string(),
        rev: rev.map(str::to_string),
        lines,
    })
}

/// Parses a `L<n>` or `L<n>-L<n>` line-range fragment (the text after `#`).
fn parse_line_range(uri: &str, fragment: &str) -> Result<(u32, Option<u32>), LinkParseError> {
    let invalid = || LinkParseError::InvalidLineRange {
        uri: uri.to_string(),
        fragment: fragment.to_string(),
    };

    match fragment.split_once('-') {
        None => {
            let start_str = fragment.strip_prefix('L').ok_or_else(invalid)?;
            let start: u32 = start_str.parse().map_err(|_| invalid())?;
            Ok((start, None))
        }
        Some((start_part, end_part)) => {
            let start_str = start_part.strip_prefix('L').ok_or_else(invalid)?;
            let end_str = end_part.strip_prefix('L').ok_or_else(invalid)?;
            let start: u32 = start_str.parse().map_err(|_| invalid())?;
            let end: u32 = end_str.parse().map_err(|_| invalid())?;
            if end < start {
                return Err(LinkParseError::InvertedLineRange {
                    uri: uri.to_string(),
                    start,
                    end,
                });
            }
            Ok((start, Some(end)))
        }
    }
}

impl std::fmt::Display for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Link::Item(item_ref) => write!(f, "rdm:{}", item_ref.label()),
            Link::Code { path, rev, lines } => {
                write!(f, "rdm:src/{path}")?;
                if let Some(rev) = rev {
                    write!(f, "@{rev}")?;
                }
                if let Some((start, end)) = lines {
                    write!(f, "#L{start}")?;
                    if let Some(end) = end {
                        write!(f, "-L{end}")?;
                    }
                }
                Ok(())
            }
        }
    }
}

/// A malformed `rdm:` link destination found while walking a markdown body,
/// kept for a later `link check` feature to surface to the author.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkDiagnostic {
    /// The byte range of the link (start of the markdown link syntax to its
    /// end) within the body.
    pub range: Range<usize>,
    /// The raw, unparsed `rdm:` destination text.
    pub uri: String,
    /// Why the destination failed to parse.
    pub error: LinkParseError,
}

/// The outcome of resolving a [`Link`] against a project's store — what a
/// consumer (CLI/server) needs to act on it.
///
/// Produced by [`crate::ops::links::resolve_link`] and its narrower
/// [`crate::ops::links::resolve_item_link`] /
/// [`crate::ops::links::resolve_code_link`] entry points.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Resolved {
    /// A resolved item reference.
    Item {
        /// The item reference that was resolved.
        target: ItemRef,
        /// Whether the target currently exists in the store. `false` for a
        /// dangling reference (never an error — see
        /// [`crate::ops::links::resolve_item_link`]'s doc comment).
        exists: bool,
    },
    /// A resolved code reference.
    Code {
        /// Path to the file, relative to the source repository root.
        path: String,
        /// The resolved revision, per the precedence documented on
        /// [`crate::ops::links::resolve_code_link`]. `None` when neither an
        /// explicit `@rev` nor a stamped commit was available (the web URL,
        /// if any, then falls back further to the project's default branch).
        rev: Option<String>,
        /// The line range carried over from the link, unvalidated.
        lines: Option<(u32, Option<u32>)>,
        /// The GitHub-style web URL for this reference, or `None` when the
        /// project has no `source` configured.
        web_url: Option<String>,
    },
    /// A link that could not be resolved for a reason other than "the
    /// target doesn't exist" (that case is [`Resolved::Item`] with
    /// `exists: false`). Reserved for resolution-time failures a future
    /// phase may distinguish from plain not-found — nothing in this phase
    /// constructs it.
    Broken {
        /// Why resolution could not proceed.
        reason: String,
    },
}

/// A document that references a [`ItemRef`] target, found by [`crate::ops::links::backlinks`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum DocRef {
    /// A roadmap body.
    Roadmap {
        /// Roadmap slug.
        roadmap: String,
    },
    /// A phase body.
    Phase {
        /// Roadmap the phase belongs to.
        roadmap: String,
        /// Phase file stem.
        stem: String,
    },
    /// A task body.
    Task {
        /// Task slug.
        slug: String,
    },
    /// A review's whole-document summary, or one of its comments.
    Review {
        /// Review id.
        id: String,
        /// Ordinal id of the comment the reference is in, or `None` for the
        /// review's own summary body.
        comment: Option<u32>,
    },
}

/// One reference to a target found while scanning a project for backlinks.
///
/// Ordered (via [`DocRef`]'s derived [`Ord`]) by document kind (roadmap <
/// phase < task < review), then by the document's own identity (slug/id,
/// and comment index within a review), then — as a tiebreaker within the
/// very same document — by [`Self::byte_range`]'s start, so output is
/// deterministic across runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BacklinkEntry {
    /// The document the reference was found in.
    pub document: DocRef,
    /// The byte range of the link within that document's body.
    pub byte_range: Range<usize>,
}

/// The pulldown-cmark options this module parses markdown bodies with.
///
/// Mirrors the GFM flags `rdm-server`'s `markdown` module enables
/// (`ENABLE_TABLES | ENABLE_STRIKETHROUGH | ENABLE_TASKLISTS |
/// ENABLE_GFM`). Duplicated locally rather than shared, since
/// `rdm-server` depends on `rdm-core`, not the reverse.
fn markdown_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
}

/// Walks a markdown body and finds every `rdm:` link destination.
///
/// Non-`rdm:` destinations (`https:`, relative paths, etc.) are silently
/// skipped. Destinations inside inline code spans or fenced/indented code
/// blocks are never visited — pulldown-cmark's tokenizer treats that
/// content as literal text and never emits a link event for it, so no
/// separate exclusion logic is needed here.
///
/// Returns successfully-parsed links (with their byte range in `body`) and
/// diagnostics for `rdm:`-prefixed destinations that failed to parse, kept
/// for a later `link check` feature.
#[must_use]
pub fn extract_links(body: &str) -> (Vec<(Range<usize>, Link)>, Vec<LinkDiagnostic>) {
    let mut links = Vec::new();
    let mut diagnostics = Vec::new();

    let parser = Parser::new_ext(body, markdown_options());
    for (event, range) in parser.into_offset_iter() {
        if let Event::Start(Tag::Link { dest_url, .. }) = event {
            if !dest_url.starts_with("rdm:") {
                continue;
            }
            match parse(&dest_url) {
                Ok(link) => links.push((range, link)),
                Err(error) => diagnostics.push(LinkDiagnostic {
                    range,
                    uri: dest_url.to_string(),
                    error,
                }),
            }
        }
    }

    (links, diagnostics)
}

/// Roadmap slugs reserved for rdm's own use — never a valid roadmap slug.
///
/// `task` is the `Done:`-line/`rdm hook done-line` prefix
/// ([`crate::hook::format_done_directive`]); `src` is the `rdm:src/` link
/// prefix. Both are enforced here as the natural single home for a future
/// consolidation of `hook::format_done_directive`'s separate ad hoc `task`
/// check onto this list — not attempted in this phase.
pub(crate) const RESERVED_ROADMAP_SLUGS: &[&str] = &["task", "src"];

/// Whether `slug` is reserved and therefore invalid as a roadmap slug.
#[must_use]
pub fn is_reserved_roadmap_slug(slug: &str) -> bool {
    RESERVED_ROADMAP_SLUGS.contains(&slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- AC1: item refs parse + Display round-trip ---

    #[test]
    fn parse_roadmap_item_ref() {
        let link = parse("rdm:roadmap/auth").unwrap();
        assert_eq!(
            link,
            Link::Item(ItemRef::Roadmap {
                roadmap: "auth".to_string()
            })
        );
    }

    #[test]
    fn parse_phase_item_ref_with_stem() {
        let link = parse("rdm:phase/auth/phase-1-design").unwrap();
        assert_eq!(
            link,
            Link::Item(ItemRef::Phase {
                roadmap: "auth".to_string(),
                stem: "phase-1-design".to_string(),
            })
        );
    }

    #[test]
    fn parse_task_item_ref() {
        let link = parse("rdm:task/fix-login").unwrap();
        assert_eq!(
            link,
            Link::Item(ItemRef::Task {
                slug: "fix-login".to_string()
            })
        );
    }

    #[test]
    fn display_round_trips_item_refs() {
        for uri in [
            "rdm:roadmap/auth",
            "rdm:phase/auth/phase-1-design",
            "rdm:task/fix-login",
        ] {
            let link = parse(uri).unwrap();
            assert_eq!(link.to_string(), uri);
            assert_eq!(parse(&link.to_string()).unwrap(), link);
        }
    }

    // --- AC2: code refs parse + Display round-trip ---

    #[test]
    fn parse_code_link_bare_path() {
        let link = parse("rdm:src/a/b.rs").unwrap();
        assert_eq!(
            link,
            Link::Code {
                path: "a/b.rs".to_string(),
                rev: None,
                lines: None,
            }
        );
        assert_eq!(parse(&link.to_string()).unwrap(), link);
    }

    #[test]
    fn parse_code_link_with_rev() {
        let link = parse("rdm:src/a/b.rs@abc123").unwrap();
        assert_eq!(
            link,
            Link::Code {
                path: "a/b.rs".to_string(),
                rev: Some("abc123".to_string()),
                lines: None,
            }
        );
        assert_eq!(parse(&link.to_string()).unwrap(), link);
    }

    #[test]
    fn parse_code_link_with_single_line() {
        let link = parse("rdm:src/a/b.rs#L5").unwrap();
        assert_eq!(
            link,
            Link::Code {
                path: "a/b.rs".to_string(),
                rev: None,
                lines: Some((5, None)),
            }
        );
        assert_eq!(parse(&link.to_string()).unwrap(), link);
    }

    #[test]
    fn parse_code_link_with_rev_and_line_range() {
        let link = parse("rdm:src/a/b.rs@abc123#L5-L12").unwrap();
        assert_eq!(
            link,
            Link::Code {
                path: "a/b.rs".to_string(),
                rev: Some("abc123".to_string()),
                lines: Some((5, Some(12))),
            }
        );
        assert_eq!(parse(&link.to_string()).unwrap(), link);
    }

    // --- AC3: malformed URIs -> matchable errors ---

    #[test]
    fn parse_rejects_missing_scheme() {
        let err = parse("roadmap/auth").unwrap_err();
        assert!(matches!(err, LinkParseError::MissingScheme(_)));
        assert!(err.to_string().contains("roadmap/auth"));
    }

    #[test]
    fn parse_rejects_unknown_kind() {
        let err = parse("rdm:foo/bar").unwrap_err();
        assert!(matches!(err, LinkParseError::UnknownKind { .. }));
        let msg = err.to_string();
        assert!(msg.contains("foo"));
        assert!(msg.contains("rdm:foo/bar"));
    }

    #[test]
    fn parse_rejects_empty_src_path() {
        let err = parse("rdm:src/").unwrap_err();
        assert!(matches!(err, LinkParseError::EmptyPath(_)));
        assert!(err.to_string().contains("rdm:src/"));
    }

    #[test]
    fn parse_rejects_malformed_line_range() {
        for uri in ["rdm:src/a.rs#5", "rdm:src/a.rs#Lx"] {
            let err = parse(uri).unwrap_err();
            assert!(
                matches!(err, LinkParseError::InvalidLineRange { .. }),
                "expected InvalidLineRange for {uri}, got {err:?}"
            );
            assert!(err.to_string().contains(uri.rsplit('#').next().unwrap()));
        }
    }

    #[test]
    fn parse_rejects_inverted_line_range() {
        let err = parse("rdm:src/a.rs#L12-L5").unwrap_err();
        assert!(matches!(
            err,
            LinkParseError::InvertedLineRange {
                start: 12,
                end: 5,
                ..
            }
        ));
        let msg = err.to_string();
        assert!(msg.contains("12"));
        assert!(msg.contains("5"));
        assert!(msg.contains("rdm:src/a.rs#L12-L5"));
    }

    // --- AC4: extract_links coverage + exclusions ---

    #[test]
    fn extract_links_in_paragraph() {
        let body = "See [the auth roadmap](rdm:roadmap/auth) for details.";
        let (links, diagnostics) = extract_links(body);
        assert_eq!(links.len(), 1);
        assert!(diagnostics.is_empty());
        assert_eq!(
            links[0].1,
            Link::Item(ItemRef::Roadmap {
                roadmap: "auth".to_string()
            })
        );
        assert_eq!(
            &body[links[0].0.clone()],
            "[the auth roadmap](rdm:roadmap/auth)"
        );
    }

    #[test]
    fn extract_links_in_list_item() {
        let body = "- see [fix](rdm:task/fix-login)\n- other item\n";
        let (links, diagnostics) = extract_links(body);
        assert_eq!(links.len(), 1);
        assert!(diagnostics.is_empty());
        assert_eq!(
            links[0].1,
            Link::Item(ItemRef::Task {
                slug: "fix-login".to_string()
            })
        );
    }

    #[test]
    fn extract_links_in_table_cell() {
        let body = "| item | link |\n| --- | --- |\n| auth | [x](rdm:roadmap/auth) |\n";
        let (links, diagnostics) = extract_links(body);
        assert_eq!(links.len(), 1);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn extract_links_in_review_comment_snippet() {
        let body =
            "Please also see [phase 1](rdm:phase/auth/phase-1-design) before merging.\n\nThanks!";
        let (links, diagnostics) = extract_links(body);
        assert_eq!(links.len(), 1);
        assert!(diagnostics.is_empty());
        assert_eq!(
            links[0].1,
            Link::Item(ItemRef::Phase {
                roadmap: "auth".to_string(),
                stem: "phase-1-design".to_string(),
            })
        );
    }

    #[test]
    fn extract_links_ignores_https_and_relative_destinations() {
        let body = "[external](https://example.com) and [relative](./other.md)";
        let (links, diagnostics) = extract_links(body);
        assert!(links.is_empty());
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn extract_links_ignores_code_span_and_fence() {
        let body = "inline `rdm:task/x` code\n\n```\n[link](rdm:task/x)\n```\n";
        let (links, diagnostics) = extract_links(body);
        assert!(links.is_empty(), "expected no links, got {links:?}");
        assert!(
            diagnostics.is_empty(),
            "expected no diagnostics, got {diagnostics:?}"
        );
    }

    #[test]
    fn extract_links_collects_malformed_rdm_destinations_as_diagnostics() {
        let body = "[bad](rdm:foo/bar)";
        let (links, diagnostics) = extract_links(body);
        assert!(links.is_empty());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].uri, "rdm:foo/bar");
        assert!(matches!(
            diagnostics[0].error,
            LinkParseError::UnknownKind { .. }
        ));
    }

    // --- AC5: reserved roadmap slugs ---

    #[test]
    fn src_is_reserved() {
        assert!(is_reserved_roadmap_slug("src"));
    }

    #[test]
    fn task_is_reserved() {
        assert!(is_reserved_roadmap_slug("task"));
    }

    #[test]
    fn other_slugs_are_not_reserved() {
        assert!(!is_reserved_roadmap_slug("auth"));
    }
}
