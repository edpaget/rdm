//! The YAML frontmatter reader.
//!
//! Every frontmatter read in rdm-core goes through [`from_str`], and nothing
//! outside this module names the YAML library, so the library can change
//! without touching a caller.
//!
//! The reader is `serde-saphyr`, a YAML 1.2 parser. The files it reads were
//! written by `serde_yaml` (libyaml, YAML 1.1 flavoured), and rdm promises
//! that those files keep loading to the same model. Out of the box the two
//! disagree, so this module does two things on top of the library:
//!
//! * [`options`] turns off the inference and strictness the old reader did
//!   not have;
//! * three text-level adjustments cover what no option reaches —
//!   [`line_separators_as_breaks`], [`strip_unterminated_block_scalar`] and
//!   [`quote_overflowing_scalar`].

use std::borrow::Cow;

use serde::de::DeserializeOwned;
use serde_saphyr::granit_parser::{Event, Parser, ScalarStyle};
use serde_saphyr::{
    DuplicateKeyPolicy, Localizer, Location, MergeKeyPolicy, MessageFormatter,
    NonFiniteFloatPolicy, Options, SnippetMode, UserMessageFormatter,
};

use crate::error::FrontmatterError;

/// Reads `text`, the YAML between a document's `---` delimiters, into `T`.
///
/// # Errors
///
/// Returns a [`FrontmatterError`] carrying the problem and the line and
/// column it was found at, counted within `text`. It names no document;
/// the caller that knows the document adds it.
pub(crate) fn from_str<T: DeserializeOwned>(text: &str) -> Result<T, FrontmatterError> {
    let text = line_separators_as_breaks(text);
    let mut text = match strip_unterminated_block_scalar(&text) {
        Some(stripped) => Cow::Owned(stripped),
        None => text,
    };
    loop {
        match serde_saphyr::from_str_with_options::<T>(&text, options()) {
            Ok(value) => return Ok(value),
            Err(err) => match quote_overflowing_scalar(&text, &err) {
                Some(quoted) => text = Cow::Owned(quoted),
                None => return Err(describe(&err)),
            },
        }
    }
}

/// The reader options under which a file `serde_yaml` wrote, or accepted,
/// reads the way it did.
///
/// All but the last differ from the library's defaults. They matter where
/// serde buffers a value before it knows the target type — an internally
/// tagged enum (`ReviewTarget`), a flattened struct (`Run`), an anchor — and
/// the reader has to guess what a bare scalar is.
fn options() -> Options {
    serde_saphyr::options! {
        // `yes`, `no`, `on`, `off`, `y` and `n` are strings; the old writer
        // left them unquoted.
        strict_booleans: true,
        // So are `0755` and `1_000`.
        leading_zero_numbers_as_strings: true,
        underscored_numbers_as_strings: true,
        // A repeated unknown key is ignored, and a repeated known field is
        // serde's to reject, as before.
        duplicate_keys: DuplicateKeyPolicy::PassThrough,
        // `<<` is an ordinary (unknown, so ignored) key, not a merge.
        merge_keys: MergeKeyPolicy::AsOrdinary,
        // A `!!binary` string is its text, not its base64 decoding.
        ignore_binary_tag_for_string: true,
        // One-line messages: rdm reports the position itself.
        with_snippet: false,
        // The default, stated because `quote_overflowing_scalar` relies on
        // the error it produces.
        non_finite_float_policy: NonFiniteFloatPolicy::Reject,
    }
}

/// Reads U+2028 and U+2029 as line breaks.
///
/// YAML 1.1 counted them as line breaks and YAML 1.2 does not, so the old
/// writer laid a string containing one out across two lines. Read as an
/// ordinary character, the continuation line's indentation leaks into the
/// string, or the scalar swallows the key that follows it. Turning each into
/// a newline gives the reader the line structure the writer produced, so
/// the document loads with every key intact. The separator itself reads
/// back as a line break — a newline in a block scalar, a space in a quoted
/// one — rather than as the character that was written.
fn line_separators_as_breaks(text: &str) -> Cow<'_, str> {
    const SEPARATORS: [char; 2] = ['\u{2028}', '\u{2029}'];
    if text.contains(SEPARATORS) {
        Cow::Owned(text.replace(SEPARATORS, "\n"))
    } else {
        Cow::Borrowed(text)
    }
}

/// Rewrites a block scalar that runs into the end of `text` so it ends
/// without a line break, returning the rewritten text, or `None` when
/// `text` has no such scalar.
///
/// Frontmatter reaches the reader without the line break before the closing
/// `---`. When the last thing in it is a block scalar (`body: |`), the old
/// reader found no final line break to keep and returned the text without
/// one; `serde-saphyr` supplies one. That last scalar is real data — the
/// body or reply of a review's last comment — so the header's chomping
/// indicator is replaced with `-` (strip), which says what the old reader
/// did.
fn strip_unterminated_block_scalar(text: &str) -> Option<String> {
    // Cheap pre-filter: only a text that stops in the middle of an indented
    // line of content, and has a block-scalar indicator somewhere, can be
    // affected. A text that ends with a line break — a file with CRLF line
    // endings still carries the `\r` — has the break and keeps it.
    let last_line = text.rsplit(['\n', '\r']).next().unwrap_or(text);
    if !last_line.starts_with([' ', '\t'])
        || last_line.trim().is_empty()
        || !text.contains(['|', '>'])
    {
        return None;
    }

    // Find the last node. It must be a block scalar that ends at the end of
    // the text with a line break the text does not have.
    let mut last_scalar = None;
    let mut previous_end = 0;
    for event in Parser::new_from_str(text) {
        let (event, span) = event.ok()?;
        let (start, end) = (span.start.byte_offset()?, span.end.byte_offset()?);
        match event {
            Event::Scalar(value, style, ..) => {
                let block = matches!(style, ScalarStyle::Literal | ScalarStyle::Folded);
                last_scalar = Some((start, end, previous_end, block && value.ends_with('\n')));
                previous_end = end;
            }
            // Closing events follow the last node without being one.
            Event::StreamStart
            | Event::StreamEnd
            | Event::DocumentStart(..)
            | Event::DocumentEnd
            | Event::MappingEnd
            | Event::SequenceEnd => {}
            _ => {
                last_scalar = None;
                previous_end = end;
            }
        }
    }
    let (content_start, end, previous_end, true) = last_scalar? else {
        return None;
    };
    if end != text.len() {
        return None;
    }

    // The header is the nearest non-blank line above the scalar's content.
    let mut header_end = text[..content_start].rfind('\n')?;
    let header_start = loop {
        let start = text[..header_end].rfind('\n').map_or(0, |at| at + 1);
        if start == 0 || !text[start..header_end].trim().is_empty() {
            break start;
        }
        header_end = start - 1;
    };
    // Look only past the node before it (the key, say), which may itself
    // contain a `|` or `>`.
    let from = header_start.max(previous_end.min(header_end));
    let header = &text[from..header_end];

    // The indicator is the `|` or `>` followed by nothing but an indentation
    // digit, a chomping sign, blanks and a comment.
    header.char_indices().find_map(|(at, ch)| {
        if ch != '|' && ch != '>' {
            return None;
        }
        let rest = &header[at + 1..];
        let signs = rest
            .chars()
            .take_while(|c| matches!(c, '+' | '-' | '1'..='9'))
            .count();
        let (signs, tail) = rest.split_at(signs);
        let ends_the_line = tail.trim().is_empty()
            || (tail.starts_with([' ', '\t']) && tail.trim_start().starts_with('#'));
        if signs.len() > 2 || !ends_the_line {
            return None;
        }
        let indentation: String = signs.chars().filter(char::is_ascii_digit).collect();
        let signs_at = from + at + 1;
        Some(format!(
            "{}{indentation}-{}",
            &text[..signs_at],
            &text[signs_at + signs.len()..]
        ))
    })
}

/// Quotes the bare scalar `err` rejected as a float too large to hold,
/// returning the patched text, or `None` when `err` is any other failure.
///
/// The old writer left strings such as `1e999`, or a commit id made of
/// digits and a single `e`, unquoted, and the old reader gave them back as
/// strings. Where the reader has to guess a type it sees a float that
/// overflows, and none of its policies returns the text. Quoting that one
/// scalar and reading again does.
fn quote_overflowing_scalar(text: &str, err: &serde_saphyr::Error) -> Option<String> {
    let serde_saphyr::Error::NonFiniteFloat { value, location } = err.without_snippet() else {
        return None;
    };
    let start = usize::try_from(location.span().byte_offset()?).ok()?;
    let end = start.checked_add(value.len())?;
    if text.get(start..end)? != value {
        return None;
    }
    Some(format!("{}'{value}'{}", &text[..start], &text[end..]))
}

/// Words a reader failure for someone editing a plan file: the library's
/// user-facing message, with the position left out of the text because
/// [`FrontmatterError`] carries it.
struct ProblemText;

impl Localizer for ProblemText {
    fn attach_location<'a>(&self, base: Cow<'a, str>, _: Location) -> Cow<'a, str> {
        base
    }

    fn alias_defined_at(&self, _: Location) -> String {
        String::new()
    }

    fn alias_used_at(&self, _: Location) -> String {
        String::new()
    }
}

impl MessageFormatter for ProblemText {
    fn localizer(&self) -> &dyn Localizer {
        self
    }

    fn format_message<'a>(&self, err: &'a serde_saphyr::Error) -> Cow<'a, str> {
        UserMessageFormatter.format_message(err)
    }
}

/// Converts the reader's error into rdm-core's own.
fn describe(err: &serde_saphyr::Error) -> FrontmatterError {
    let problem = err.render_with_options(serde_saphyr::render_options! {
        formatter: &ProblemText,
        snippets: SnippetMode::Off,
    });
    let position = err
        .location()
        .map(|location| (location.line(), location.column()));
    FrontmatterError::new(problem, position)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::model::{Anchor, Phase, PhaseStatus, ReviewTarget, Run, RunTarget, Task};

    /// Scalars the old writer left bare and the old reader gave back as
    /// strings, though a YAML 1.1 reader takes them for booleans or numbers.
    const BARE_WORDS: [&str; 13] = [
        "yes", "Yes", "no", "on", "off", "y", "n", "Y", "N", "0755", "-007", "1_000", "0xFF_FF",
    ];

    fn task_titled(title_yaml: &str) -> String {
        format!(
            "project: p\ntitle: {title_yaml}\nstatus: open\npriority: high\ncreated: 2026-03-14\n"
        )
    }

    fn run_yaml(id: &str, target: &str, args: &str) -> String {
        format!(
            "id: {id}\nproject: rdm\ndriver: autopilot\nroadmap: {target}\nargs: {args}\n\
             status: open\nstarted: 2026-09-24T15:30:12.345Z\nstop_reason: {args}\nunits: []\n"
        )
    }

    // -- scalars keep the type the old reader gave them --

    #[test]
    fn a_bare_word_stays_a_string_in_a_typed_field() {
        for word in BARE_WORDS {
            let yaml = format!("{}tags:\n- {word}\n- z\n", task_titled(word));
            let task: Task = from_str(&yaml).unwrap();
            assert_eq!(task.title, word);
            assert_eq!(task.tags, Some(vec![word.to_string(), "z".to_string()]));
        }
    }

    #[test]
    fn a_bare_word_stays_a_string_in_a_review_target() {
        for word in BARE_WORDS {
            let yaml = format!("kind: phase\nroadmap: {word}\nstem: {word}\n");
            assert_eq!(
                from_str::<ReviewTarget>(&yaml).unwrap(),
                ReviewTarget::Phase {
                    roadmap: word.to_string(),
                    stem: word.to_string(),
                }
            );
        }
    }

    #[test]
    fn a_bare_word_stays_a_string_in_a_run() {
        for word in BARE_WORDS {
            let run: Run = from_str(&run_yaml(word, word, word)).unwrap();
            assert_eq!(run.id, word);
            assert_eq!(run.target, RunTarget::Roadmap(word.to_string()));
            assert_eq!(run.args.as_deref(), Some(word));
            assert_eq!(run.stop_reason.as_deref(), Some(word));
        }
    }

    #[test]
    fn a_bare_word_stays_a_string_in_an_anchor() {
        for word in BARE_WORDS {
            let yaml =
                format!("anchor_type: text-quote\nquote: {word}\nprefix: {word}\nsuffix: {word}\n");
            assert_eq!(
                from_str::<Anchor>(&yaml).unwrap(),
                Anchor::TextQuote {
                    quote: word.to_string(),
                    prefix: word.to_string(),
                    suffix: word.to_string(),
                }
            );
        }
    }

    #[test]
    fn a_commit_id_of_digits_and_one_e_reads_back_as_itself() {
        // Forty characters that spell a float far too large to hold. Both
        // ends of the range do, so the reader has to recover twice.
        let head = "12345e6789012345678901234567890123456789";
        let base = "9e87654321098765432109876543210987654321";
        let yaml = format!("kind: change\nhead: {head}\nbase: {base}\n");
        assert_eq!(
            from_str::<ReviewTarget>(&yaml).unwrap(),
            ReviewTarget::Change {
                head: head.to_string(),
                base: Some(base.to_string()),
            }
        );
    }

    #[test]
    fn an_overflowing_exponent_reads_back_as_itself_in_a_run() {
        let run: Run = from_str(&run_yaml("1e999", "auth", "1e999")).unwrap();
        assert_eq!(run.id, "1e999");
        assert_eq!(run.args.as_deref(), Some("1e999"));
        assert_eq!(run.stop_reason.as_deref(), Some("1e999"));
    }

    #[test]
    fn an_unrecognised_anchor_keeps_an_overflowing_scalar_as_its_text() {
        let anchor: Anchor =
            from_str("anchor_type: heading-path\nweight: 1e999\nlimit: .inf\n").unwrap();
        let Anchor::Unknown { raw, .. } = anchor else {
            panic!("expected Anchor::Unknown, got {anchor:?}");
        };
        assert_eq!(raw.get("weight"), Some(&json!("1e999")));
        assert_eq!(raw.get("limit"), Some(&json!(".inf")));
    }

    #[test]
    fn a_number_stays_a_number_where_the_type_is_guessed() {
        let anchor: Anchor =
            from_str("anchor_type: heading-path\ndepth: 3\nweight: 1.5\nhex: 0x1F\n").unwrap();
        let Anchor::Unknown { raw, .. } = anchor else {
            panic!("expected Anchor::Unknown, got {anchor:?}");
        };
        assert_eq!(raw.get("depth"), Some(&json!(3)));
        assert_eq!(raw.get("weight"), Some(&json!(1.5)));
        assert_eq!(raw.get("hex"), Some(&json!(31)));
    }

    // -- mappings keep the old reader's rules --

    const PHASE: &str = "phase: 1\ntitle: T\nstatus: done\n";

    #[test]
    fn a_merge_key_is_not_expanded() {
        let phase: Phase = from_str(&format!("{PHASE}<<: {{commit: abc}}\n")).unwrap();
        assert_eq!(phase.commit, None);
    }

    #[test]
    fn a_repeated_unknown_key_is_ignored() {
        let phase: Phase = from_str(&format!("{PHASE}bogus: 1\nbogus: 2\n")).unwrap();
        assert_eq!(phase.title, "T");
        assert_eq!(phase.status, PhaseStatus::Done);
    }

    #[test]
    fn a_repeated_known_field_is_an_error() {
        let err = from_str::<Phase>(&format!("{PHASE}title: U\n")).unwrap_err();
        assert!(err.problem().contains("duplicate field `title`"), "{err:?}");
    }

    #[test]
    fn a_binary_tagged_string_is_its_text() {
        let task: Task = from_str(&task_titled("!!binary aGk=")).unwrap();
        assert_eq!(task.title, "aGk=");
    }

    // -- a block scalar that runs into the end of the text --

    #[test]
    fn a_block_scalar_at_the_end_of_the_text_has_no_final_line_break() {
        let cases = [
            // Clip, keep and folded all lose the break the text never had.
            ("k: |\n  foo", json!({"k": "foo"})),
            ("k: |+\n  foo", json!({"k": "foo"})),
            ("k: >\n  foo\n  bar", json!({"k": "foo bar"})),
            ("k: >+\n  foo\n  bar", json!({"k": "foo bar"})),
            ("k: |\r\n  foo", json!({"k": "foo"})),
            // Interior breaks are untouched.
            ("k: |\n\n  foo", json!({"k": "\nfoo"})),
            ("k: |\n  foo\n\n  bar", json!({"k": "foo\n\nbar"})),
            // As a sequence item and nested.
            ("- a\n- |\n  foo\n  bar", json!(["a", "foo\nbar"])),
            ("k:\n  j: |\n    foo", json!({"k": {"j": "foo"}})),
            ("k:\n- j: |\n    foo", json!({"k": [{"j": "foo"}]})),
            // With a header comment, a tag and an indentation indicator.
            ("k: | # c\n  foo", json!({"k": "foo"})),
            ("k: !!str |\n  foo", json!({"k": "foo"})),
            ("k: |2\n    foo", json!({"k": "  foo"})),
            ("k: |+2\n    foo", json!({"k": "  foo"})),
            ("k: >2-\n    foo", json!({"k": "  foo"})),
            // A key that itself contains an indicator.
            ("'x|y': >\n  foo", json!({"x|y": "foo"})),
            // A block scalar's own text that looks like a header.
            ("k: |\n  a: |\n    nested", json!({"k": "a: |\n  nested"})),
        ];
        for (text, expected) in cases {
            assert_eq!(from_str::<Value>(text).unwrap(), expected, "{text:?}");
        }
    }

    #[test]
    fn a_block_scalar_that_is_terminated_keeps_its_line_breaks() {
        let cases = [
            // Strip never had one to lose.
            ("k: |-\n  foo", json!({"k": "foo"})),
            // The text ends with a line break.
            ("k: |\n  foo\n", json!({"k": "foo\n"})),
            ("k: |+\n  foo\n\n", json!({"k": "foo\n\n"})),
            // A file with CRLF line endings: what is left of the last line's
            // ending is still a line break.
            ("k: |\r\n  foo\r", json!({"k": "foo\n"})),
            (
                "a: 1\r\nk: >\r\n  foo\r\n  bar\r",
                json!({"a": 1, "k": "foo bar\n"}),
            ),
            // The block scalar is not the last node.
            ("a: |\n  x\nk: plain", json!({"a": "x\n", "k": "plain"})),
            ("k: |\n  x\nz: {}", json!({"k": "x\n", "z": {}})),
            // Only the last of two is unterminated.
            ("a: |\n  x\nk: |\n  y", json!({"a": "x\n", "k": "y"})),
        ];
        for (text, expected) in cases {
            assert_eq!(from_str::<Value>(text).unwrap(), expected, "{text:?}");
        }
    }

    // -- U+2028 and U+2029 --

    #[test]
    fn a_line_separator_in_a_quoted_scalar_reads_as_a_break_and_keeps_the_next_key() {
        for separator in ['\u{2028}', '\u{2029}'] {
            // What the old writer produced for the title `a<separator>b`.
            let task: Task = from_str(&task_titled(&format!("'a{separator}  b'"))).unwrap();
            assert_eq!(task.title, "a b");
            assert_eq!(task.project, "p");
            assert_eq!(task.status, crate::model::TaskStatus::Open);
        }
    }

    #[test]
    fn a_line_separator_in_a_block_scalar_reads_as_a_break_and_keeps_the_next_key() {
        // What the old writer produced for `one\ntwo<LS>three\n`.
        let task: Task = from_str(&task_titled("|\n  one\n  two\u{2028}  three")).unwrap();
        assert_eq!(task.title, "one\ntwo\nthree\n");
        assert_eq!(task.status, crate::model::TaskStatus::Open);
    }

    #[test]
    fn a_line_separator_ending_a_block_scalar_keeps_the_next_key() {
        // What the old writer produced for `one\ntwo<LS>`: the separator is
        // the only thing between the scalar and the key after it.
        let yaml = "project: p\ntitle: |\n  one\n  two\u{2028}status: open\npriority: high\n\
                    created: 2026-03-14\n";
        let task: Task = from_str(yaml).unwrap();
        assert_eq!(task.title, "one\ntwo\n");
        assert_eq!(task.status, crate::model::TaskStatus::Open);
        assert_eq!(task.priority, crate::model::Priority::High);
    }

    // -- errors --

    #[test]
    fn a_bare_null_for_a_required_string_is_an_error_with_its_position() {
        for null in ["", " ~", " null"] {
            let err =
                from_str::<Phase>(&format!("phase: 1\ntitle:{null}\nstatus: done\n")).unwrap_err();
            assert_eq!(err.line(), Some(2), "{err:?}");
            assert!(err.column().is_some(), "{err:?}");
            assert!(!err.problem().is_empty(), "{err:?}");
            assert_eq!(err.document(), None);
        }
    }

    #[test]
    fn an_optional_field_left_empty_is_absent() {
        let phase: Phase = from_str(&format!("{PHASE}commit:\ntags: ~\n")).unwrap();
        assert_eq!(phase.commit, None);
        assert_eq!(phase.tags, None);
    }

    #[test]
    fn a_problem_is_one_line_of_text_without_its_position() {
        let cases = [
            // A type error, a syntax error and a structural one.
            ("phase: 1\ntitle: T\nstatus: nearly\n", 3),
            ("phase: 1\ntitle: \"T\nstatus: done\n", 3),
            ("phase: 1\ntitle: T\nstatus: done\ntags: nope\n", 4),
        ];
        for (text, line) in cases {
            let err = from_str::<Phase>(text).unwrap_err();
            assert_eq!(err.line(), Some(line), "{err:?}");
            assert!(!err.problem().is_empty(), "{err:?}");
            assert!(!err.problem().contains('\n'), "{err:?}");
            assert!(!err.problem().contains(" at line "), "{err:?}");
            assert_eq!(
                err.to_string(),
                format!(
                    "{} at line {line}, column {}",
                    err.problem(),
                    err.column().unwrap()
                )
            );
        }
    }

    #[test]
    fn a_mistyped_value_says_what_was_found() {
        let err = from_str::<Phase>("phase: 1\ntitle: T\nstatus: nearly\n").unwrap_err();
        assert!(
            err.problem().contains("unknown variant `nearly`"),
            "{err:?}"
        );
        assert_eq!((err.line(), err.column()), (Some(3), Some(9)));
    }
}
