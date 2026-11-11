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
//! * four text-level adjustments cover what no option reaches —
//!   [`line_separators_as_breaks`], [`strip_unterminated_block_scalar`],
//!   [`quote_strings_read_as_numbers`] and [`quote_non_finite_scalar`].

use std::borrow::Cow;
use std::fmt;

use serde::de::{DeserializeOwned, Visitor};
use serde::{Deserialize, Deserializer};
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
    let text = match strip_unterminated_block_scalar(&text) {
        Some(stripped) => Cow::Owned(stripped),
        None => text,
    };
    let mut text = match quote_strings_read_as_numbers(&text) {
        Some(quoted) => Cow::Owned(quoted),
        None => text,
    };
    loop {
        match serde_saphyr::from_str_with_options::<T>(&text, options()) {
            Ok(value) => return Ok(value),
            Err(err) => match quote_non_finite_scalar(&text, &err) {
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
        // The default, stated because `quote_non_finite_scalar` relies on
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

/// Quotes each bare scalar the old reader read as a string and the reader
/// takes for a number, returning the rewritten text, or `None` when `text`
/// has no such scalar.
///
/// Where it had to guess, the old reader called a bare scalar a number only
/// if it spelled one its own way, and the old writer quoted a string only
/// when that reader would have misread it. The reader knows more spellings:
/// a radix prefix in capitals (`0X1F`, `-0O17`, `+0B101`), a signed `.nan`,
/// and a decimal float too large to hold (`1e999`, or a commit id made of
/// digits and a single `e`). Read as a number, such a string fails to load
/// where a string is expected and silently becomes a number where anything
/// is.
///
/// The two readers are asked rather than the spellings listed:
/// [`old_reader_read_a_number`] restates the old reader's rule, and
/// [`GuessedString`] puts the scalar to the reader itself. Only a scalar
/// that starts the way a number does — with a digit, a sign or a dot — is
/// asked about; over every other bare scalar the two agree under
/// [`options`]. Quoting a scalar the old reader read as a string never
/// changes what it holds.
fn quote_strings_read_as_numbers(text: &str) -> Option<String> {
    let mut quoted = String::new();
    let mut copied = 0;
    for event in Parser::new_from_str(text) {
        // A text that does not parse is the reader's to report, untouched.
        let (event, span) = event.ok()?;
        // A tag states the type, so nothing is guessed.
        let Event::Scalar(value, ScalarStyle::Plain, _, None) = event else {
            continue;
        };
        if !value.starts_with(|first| matches!(first, '0'..='9' | '+' | '-' | '.'))
            || old_reader_read_a_number(&value)
            || serde_saphyr::from_str_with_options::<GuessedString>(&value, options()).is_ok()
        {
            continue;
        }
        let (start, end) = (span.start.byte_offset()?, span.end.byte_offset()?);
        // A scalar folded over several lines is not its own source text; its
        // value has a space in it, which no number does.
        if text.get(start..end) != Some(&*value) {
            continue;
        }
        quoted.push_str(&text[copied..start]);
        quoted.push('\'');
        quoted.push_str(&value.replace('\'', "''"));
        quoted.push('\'');
        copied = end;
    }
    if copied == 0 {
        return None;
    }
    quoted.push_str(&text[copied..]);
    Some(quoted)
}

/// Whether the old reader, `serde_yaml` 0.9, read the bare scalar `scalar`
/// as an integer or a float where it had to guess a type.
///
/// Its rule, restated. After at most one sign, a number is one of: `0x`,
/// `0o` or `0b` — in lower case only — and digits of that radix that fit in
/// 128 bits; `.inf`, or unsigned `.nan`, in one of three casings; or what
/// Rust parses to a finite `f64`, which covers every decimal integer it
/// took, unless it is all digits behind a redundant leading zero (`0755`).
fn old_reader_read_a_number(scalar: &str) -> bool {
    let (negative, unsigned) = match scalar.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, scalar.strip_prefix('+').unwrap_or(scalar)),
    };
    // Rust's own parsers accept a sign, which here would be a second one.
    let signed_twice = unsigned.starts_with(['+', '-']);
    let leading_zero = unsigned.len() > 1
        && unsigned.starts_with('0')
        && unsigned.bytes().all(|digit| digit.is_ascii_digit());

    let prefixed = [("0x", 16), ("0o", 8), ("0b", 2)]
        .into_iter()
        .find_map(|(prefix, radix)| Some((unsigned.strip_prefix(prefix)?, radix)))
        .is_some_and(|(digits, radix)| {
            !digits.starts_with(['+', '-'])
                && if negative {
                    i128::from_str_radix(&format!("-{digits}"), radix).is_ok()
                } else {
                    u128::from_str_radix(digits, radix).is_ok()
                }
        });
    let non_finite =
        matches!(unsigned, ".inf" | ".Inf" | ".INF") || matches!(scalar, ".nan" | ".NaN" | ".NAN");
    let decimal =
        !signed_twice && !leading_zero && unsigned.parse::<f64>().is_ok_and(f64::is_finite);

    prefixed || non_finite || decimal
}

/// Reads a bare scalar the way the reader does where it has to guess a
/// type, and succeeds only when the guess is a string.
struct GuessedString;

impl<'de> Deserialize<'de> for GuessedString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StringOnly;

        impl Visitor<'_> for StringOnly {
            type Value = GuessedString;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a string")
            }

            fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
                Ok(GuessedString)
            }
        }

        deserializer.deserialize_any(StringOnly)
    }
}

/// Quotes the bare scalar `err` rejected as a number that is not finite,
/// returning the patched text, or `None` when `err` is any other failure.
///
/// [`quote_strings_read_as_numbers`] has already quoted the strings that
/// spell such a number. What reaches here is one the old reader took for a
/// number too — `.inf`, `.nan` — in a place where the type is guessed, and
/// none of the reader's policies returns the text. Quoting that one scalar
/// and reading again does.
fn quote_non_finite_scalar(text: &str, err: &serde_saphyr::Error) -> Option<String> {
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
    /// strings, though a YAML 1.1 reader takes them for booleans or numbers
    /// and a YAML 1.2 one for numbers too: a radix prefix in capitals under
    /// each sign, a signed `.nan` in each casing, and a float too large to
    /// hold under each sign and exponent letter.
    const BARE_WORDS: [&str; 38] = [
        "yes", "Yes", "no", "on", "off", "y", "n", "Y", "N", "0755", "-007", "1_000", "0xFF_FF",
        "0X1F", "0XFF", "0Xff", "0X0", "+0X1F", "-0X1F", "0O17", "0O0", "+0O17", "-0O17", "0B101",
        "0B0", "+0B101", "-0B101", "+.nan", "-.nan", "+.NaN", "-.NaN", "+.NAN", "-.NAN", "1e999",
        "1E999", "+1e999", "-1e309", "1.5e999",
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
    fn a_bare_word_stays_a_string_in_an_unrecognised_anchor() {
        // Nothing here expects a string, so a misread word does not fail to
        // load: it would quietly come back as a number.
        for word in BARE_WORDS {
            let yaml = format!("anchor_type: heading-path\nmark: {word}\nmarks:\n- {word}\n");
            let anchor: Anchor = from_str(&yaml).unwrap();
            let Anchor::Unknown { raw, .. } = anchor else {
                panic!("expected Anchor::Unknown, got {anchor:?}");
            };
            assert_eq!(raw.get("mark"), Some(&json!(word)));
            assert_eq!(raw.get("marks"), Some(&json!([word])));
        }
    }

    #[test]
    fn a_bare_word_reads_back_as_the_writer_wrote_it() {
        for word in BARE_WORDS {
            let target = ReviewTarget::Phase {
                roadmap: word.to_string(),
                stem: word.to_string(),
            };
            let anchor = Anchor::TextQuote {
                quote: word.to_string(),
                prefix: word.to_string(),
                suffix: word.to_string(),
            };
            let written = serde_yaml::to_string(&target).unwrap();
            assert_eq!(from_str::<ReviewTarget>(&written).unwrap(), target);
            let written = serde_yaml::to_string(&anchor).unwrap();
            assert_eq!(from_str::<Anchor>(&written).unwrap(), anchor);
        }
    }

    #[test]
    fn a_bare_word_stays_a_string_wherever_it_is_written() {
        let cases = [
            // In a flow sequence, before a comment, as a key, behind an
            // anchor, and after text that is not ASCII.
            ("k: [0X1F, a, -0B1]", json!({"k": ["0X1F", "a", "-0B1"]})),
            ("k: 0X1F  # c\nj: 0O7", json!({"k": "0X1F", "j": "0O7"})),
            ("0X1F: v", json!({"0X1F": "v"})),
            ("k: &a 0X1F\nj: *a", json!({"k": "0X1F", "j": "0X1F"})),
            ("t: é中 x\nk: 0X1F", json!({"t": "é中 x", "k": "0X1F"})),
            (
                "k: 0X1F\r\nj: +.nan\r\n",
                json!({"k": "0X1F", "j": "+.nan"}),
            ),
            // Already a string: quoted, a line of a block scalar, a line of
            // a scalar folded over several.
            ("k: '0X1F'\nj: \"0X1F\"", json!({"k": "0X1F", "j": "0X1F"})),
            ("k: |\n  0X1F\n  x\n", json!({"k": "0X1F\nx\n"})),
            ("k: one\n  0X1F\n  two", json!({"k": "one 0X1F two"})),
            ("k: 0X1F\n  0X1F", json!({"k": "0X1F 0X1F"})),
        ];
        for (text, expected) in cases {
            assert_eq!(from_str::<Value>(text).unwrap(), expected, "{text:?}");
        }
    }

    #[test]
    fn a_number_the_old_reader_read_stays_a_number_where_the_type_is_guessed() {
        let cases = [
            ("0x1F", json!(31)),
            ("+0x1F", json!(31)),
            ("-0x1F", json!(-31)),
            ("0o17", json!(15)),
            ("0b101", json!(5)),
            ("+12", json!(12)),
            ("-12", json!(-12)),
            ("-0", json!(0)),
            ("1e3", json!(1000.0)),
            ("1E3", json!(1000.0)),
            ("+.5", json!(0.5)),
            ("1.", json!(1.0)),
            ("-1.5e-3", json!(-0.0015)),
        ];
        for (text, expected) in cases {
            let anchor: Anchor =
                from_str(&format!("anchor_type: heading-path\nn: {text}\n")).unwrap();
            let Anchor::Unknown { raw, .. } = anchor else {
                panic!("expected Anchor::Unknown, got {anchor:?}");
            };
            assert_eq!(raw.get("n"), Some(&expected), "{text}");
        }
    }

    #[test]
    fn a_problem_after_a_bare_word_keeps_its_position() {
        let err = from_str::<Phase>("phase: 1\ntitle: 0X1F\nstatus: nearly\n").unwrap_err();
        assert_eq!((err.line(), err.column()), (Some(3), Some(9)));
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
