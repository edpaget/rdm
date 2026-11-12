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
//!   [`quote_scalars_read_differently`].
//!
//! The reader is therefore handed a text that can differ from the one in the
//! file. Each adjustment states its changes as [`Replacement`]s and
//! [`Rewritten`] applies them, so a problem the reader finds is reported
//! where it is in the file rather than in the adjusted text. Each adjustment
//! is one pass over the text and the reader reads it once, so the work
//! grows with the length of the frontmatter and not with how much of it
//! needs adjusting.

use std::borrow::Cow;
use std::fmt;
use std::ops::Range;

use serde::de::{DeserializeOwned, Visitor};
use serde::{Deserialize, Deserializer};
use serde_saphyr::granit_parser::{Event, Parser, ScalarStyle};
use serde_saphyr::{
    DuplicateKeyPolicy, Localizer, Location, MergeKeyPolicy, MessageFormatter,
    NonFiniteFloatPolicy, Options, SnippetMode, Span, UserMessageFormatter,
};

use crate::error::FrontmatterError;

/// Reads `text`, the YAML between a document's `---` delimiters, into `T`.
///
/// # Errors
///
/// Returns a [`FrontmatterError`] carrying the problem and the line and
/// column it was found at, counted within `text` as it was given. It names
/// no document; the caller that knows the document adds it.
pub(crate) fn from_str<T: DeserializeOwned>(text: &str) -> Result<T, FrontmatterError> {
    let mut text = Rewritten::new(text);
    text.apply(line_separators_as_breaks(text.as_str()));
    text.apply(strip_unterminated_block_scalar(text.as_str()));
    text.apply(quote_scalars_read_differently(text.as_str()));
    serde_saphyr::from_str_with_options::<T>(text.as_str(), options())
        .map_err(|err| describe(&err, &text))
}

/// One change an adjustment makes to the text it is given: the bytes at
/// `at` give way to `with`.
struct Replacement {
    at: Range<usize>,
    with: String,
}

/// A [`Replacement`] once applied: `removed` bytes at `at`, an offset in
/// the text before the change, became `inserted` bytes.
struct Edit {
    at: usize,
    removed: usize,
    inserted: usize,
}

/// The text the reader is handed, with the changes that made it out of the
/// original, so a place in it can be traced back.
struct Rewritten<'a> {
    original: &'a str,
    text: Cow<'a, str>,
    /// The edits of each adjustment that changed something, in the order
    /// they ran. An adjustment works on the text the one before it left, so
    /// its offsets index that text, not the original.
    passes: Vec<Vec<Edit>>,
}

impl<'a> Rewritten<'a> {
    fn new(original: &'a str) -> Self {
        Rewritten {
            original,
            text: Cow::Borrowed(original),
            passes: Vec::new(),
        }
    }

    /// The text as it stands.
    fn as_str(&self) -> &str {
        &self.text
    }

    /// Makes one adjustment's `replacements`, which are in the order they
    /// occur in the text as it stands and do not overlap. The text is
    /// copied once however many there are, and not at all when there are
    /// none.
    fn apply(&mut self, replacements: impl IntoIterator<Item = Replacement>) {
        let mut text = String::new();
        let mut edits = Vec::new();
        let mut copied = 0;
        for Replacement { at, with } in replacements {
            text.push_str(&self.text[copied..at.start]);
            text.push_str(&with);
            copied = at.end;
            edits.push(Edit {
                at: at.start,
                removed: at.len(),
                inserted: with.len(),
            });
        }
        if edits.is_empty() {
            return;
        }
        text.push_str(&self.text[copied..]);
        self.text = Cow::Owned(text);
        self.passes.push(edits);
    }

    /// The byte offset in the original of `at`, a byte offset in the text
    /// as it stands. A place inside a replacement is where the text it
    /// replaced began.
    fn original_offset(&self, at: usize) -> usize {
        self.passes.iter().rev().fold(at, |at, edits| {
            // How far the text before this pass and the text after it have
            // each been accounted for.
            let (mut before, mut after) = (0, 0);
            for edit in edits {
                let replacement = after + (edit.at - before);
                if at < replacement {
                    break;
                }
                if at < replacement + edit.inserted {
                    return edit.at;
                }
                before = edit.at + edit.removed;
                after = replacement + edit.inserted;
            }
            before + (at - after)
        })
    }

    /// The 1-based line and column in the original of `location`, a place
    /// the reader found in the text as it stands.
    ///
    /// A text no adjustment changed keeps the reader's own count. Otherwise
    /// the place is traced back through the edits and counted again in the
    /// original, the way the reader counts.
    fn original_position(&self, location: &Location) -> (u64, u64) {
        let span = location.span();
        if self.passes.is_empty() || span == Span::UNKNOWN {
            return (location.line(), location.column());
        }
        // The reader states every place as a count of characters, and only
        // some as a count of bytes: a syntax error has none.
        let at = usize::try_from(span.offset())
            .ok()
            .and_then(|characters| self.text.char_indices().nth(characters))
            .map_or(self.text.len(), |(at, _)| at);
        position_at(self.original, self.original_offset(at))
    }
}

/// The 1-based line and column of the byte offset `at` in `text`, counted
/// as the reader counts them: a line ends at a line feed, a carriage return
/// or the pair of them, and every character on it is one column, whatever
/// its width in bytes — a tab and a U+2028 included, a byte order mark that
/// opens the line excepted.
fn position_at(text: &str, at: usize) -> (u64, u64) {
    let (mut line, mut column) = (1, 1);
    let mut after_carriage_return = false;
    for (_, ch) in text.char_indices().take_while(|&(start, _)| start < at) {
        match ch {
            // The second half of a CRLF, which has already ended the line.
            '\n' if after_carriage_return => {}
            '\n' | '\r' => (line, column) = (line + 1, 1),
            '\u{feff}' if column == 1 => {}
            _ => column += 1,
        }
        after_carriage_return = ch == '\r';
    }
    (line, column)
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
        // The default, stated because `quote_scalars_read_differently`
        // tells a number that is not finite by the error it produces.
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
fn line_separators_as_breaks(text: &str) -> Vec<Replacement> {
    const SEPARATORS: [char; 2] = ['\u{2028}', '\u{2029}'];
    text.match_indices(SEPARATORS)
        .map(|(at, separator)| Replacement {
            at: at..at + separator.len(),
            with: "\n".to_string(),
        })
        .collect()
}

/// Rewrites a block scalar that runs into the end of `text` so it ends
/// without a line break, returning the change to its header, or `None`
/// when `text` has no such scalar.
///
/// Frontmatter reaches the reader without the line break before the closing
/// `---`. When the last thing in it is a block scalar (`body: |`), the old
/// reader found no final line break to keep and returned the text without
/// one; `serde-saphyr` supplies one. That last scalar is real data — the
/// body or reply of a review's last comment — so the header's chomping
/// indicator is replaced with `-` (strip), which says what the old reader
/// did.
fn strip_unterminated_block_scalar(text: &str) -> Option<Replacement> {
    // Cheap pre-filter: only a text that stops in the middle of an indented
    // line, and has a block-scalar indicator somewhere, can be affected. A
    // text that ends with a line break — a file with CRLF line endings still
    // carries the `\r` — has the break and keeps it.
    let last_line = text.rsplit(['\n', '\r']).next().unwrap_or(text);
    if !last_line.starts_with([' ', '\t']) || !text.contains(['|', '>']) {
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

    // The last line is content only where it reaches past the scalar's
    // indentation, which is the column the content starts at. Spaces that
    // stop short of it are an empty line: the break before them is in the
    // text and stays. Past it, anything is content — a tab, further spaces,
    // a space that is not ASCII — as YAML counts only the space as
    // indentation.
    let content_line = text[..content_start].rfind(['\n', '\r'])? + 1;
    if last_line.len() <= content_start - content_line {
        return None;
    }

    // The header is the nearest non-blank line above the scalar's content.
    // Blank is as YAML has it, spaces and tabs, with the `\r` a CRLF line
    // ending leaves behind.
    let mut header_end = text[..content_start].rfind('\n')?;
    let header_start = loop {
        let start = text[..header_end].rfind('\n').map_or(0, |at| at + 1);
        let line = &text[start..header_end];
        if start == 0 || !line.trim_matches([' ', '\t', '\r']).is_empty() {
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
        Some(Replacement {
            at: signs_at..signs_at + signs.len(),
            with: format!("{indentation}-"),
        })
    })
}

/// Quotes each bare scalar that has to reach rdm as its text and that the
/// reader, where it guesses the type, would not read as text, returning
/// one replacement for each, in the order they occur in `text`.
///
/// There are two kinds. The first is a string the reader takes for a
/// number. Where it had to guess, the old reader called a bare scalar a
/// number only if it spelled one its own way, and the old writer quoted a
/// string only when that reader would have misread it. The reader knows
/// more spellings: a radix prefix in capitals (`0X1F`, `-0O17`, `+0B101`), a
/// signed `.nan`, and a decimal float too large to hold (`1e999`, or a
/// commit id made of digits and a single `e`). Read as a number, such a
/// string fails to load where a string is expected and silently becomes a
/// number where anything is.
///
/// The second is a number that is not finite — `.inf`, `.nan` — which the
/// old reader took for a number too. Where the type is guessed the reader
/// refuses it, and none of its policies returns the text as written.
/// Quoted, it reads as that text there, and still as the number where a
/// float is asked for.
///
/// The two readers are asked rather than the spellings listed:
/// [`old_reader_read_a_number`] restates the old reader's rule, and
/// [`GuessedString`] puts the scalar to the reader itself. Only a scalar
/// that starts the way a number does — with a digit, a sign or a dot — is
/// asked about; over every other bare scalar the two agree under
/// [`options`]. Quoting a scalar the old reader read as a string never
/// changes what it holds.
///
/// Every such scalar is found in this one walk, so the text is read once
/// however many there are.
fn quote_scalars_read_differently(text: &str) -> Vec<Replacement> {
    // Each replacement, and whether the reader refuses the scalar.
    let mut quoted = Vec::new();
    for event in Parser::new_from_str(text) {
        // A text that does not parse is the reader's to report, as it was
        // written — but for the scalars it refuses. It reads as it goes, so
        // it would stop at one of those before it got to the problem.
        let Ok((event, span)) = event else {
            quoted.retain(|&(_, refused)| refused);
            break;
        };
        // A tag states the type, so nothing is guessed.
        let Event::Scalar(value, ScalarStyle::Plain, _, None) = event else {
            continue;
        };
        if !value.starts_with(|first| matches!(first, '0'..='9' | '+' | '-' | '.')) {
            continue;
        }
        // To the reader it is a string already, as it was to the old one.
        let Err(guess) = serde_saphyr::from_str_with_options::<GuessedString>(&value, options())
        else {
            continue;
        };
        // A number the reader refuses for not being finite is quoted
        // whatever the old reader made of it. Any other is left bare when
        // it was a number to the old reader too.
        let refused = matches!(
            guess.without_snippet(),
            serde_saphyr::Error::NonFiniteFloat { .. }
        );
        if !refused && old_reader_read_a_number(&value) {
            continue;
        }
        let (Some(start), Some(end)) = (span.start.byte_offset(), span.end.byte_offset()) else {
            continue;
        };
        // A scalar folded over several lines is not its own source text; its
        // value has a space in it, which no number does.
        if text.get(start..end) != Some(&*value) {
            continue;
        }
        let replacement = Replacement {
            at: start..end,
            with: format!("'{}'", value.replace('\'', "''")),
        };
        quoted.push((replacement, refused));
    }
    quoted
        .into_iter()
        .map(|(replacement, _)| replacement)
        .collect()
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

/// Converts the reader's error over `text` into rdm-core's own, placed in
/// the original of `text`.
fn describe(err: &serde_saphyr::Error, text: &Rewritten<'_>) -> FrontmatterError {
    let problem = err.render_with_options(serde_saphyr::render_options! {
        formatter: &ProblemText,
        snippets: SnippetMode::Off,
    });
    let position = err
        .location()
        .map(|location| text.original_position(&location));
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
        // ends of the range do, so two scalars have to be quoted.
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

    // -- numbers that are not finite --

    /// The spellings the old reader read as a number that is not finite.
    const NON_FINITE: [&str; 9] = [
        ".inf", ".Inf", ".INF", "+.inf", "-.inf", "-.INF", ".nan", ".NaN", ".NAN",
    ];

    #[test]
    fn thousands_of_non_finite_numbers_under_an_unknown_key_leave_the_known_fields_readable() {
        let junk: String = NON_FINITE
            .iter()
            .cycle()
            .take(6000)
            .map(|number| format!("- {number}\n"))
            .collect();
        let yaml = format!("{}junk:\n{junk}tags:\n- z\n", task_titled("T"));
        let task: Task = from_str(&yaml).unwrap();
        assert_eq!(task.project, "p");
        assert_eq!(task.title, "T");
        assert_eq!(task.status, crate::model::TaskStatus::Open);
        assert_eq!(task.priority, crate::model::Priority::High);
        assert_eq!(task.tags, Some(vec!["z".to_string()]));
    }

    #[test]
    fn thousands_of_non_finite_numbers_are_each_kept_as_their_text_where_the_type_is_guessed() {
        let marks: Vec<&str> = NON_FINITE.iter().copied().cycle().take(6000).collect();
        let yaml = format!(
            "anchor_type: heading-path\nmarks: [{}]\nafter: 3\n",
            marks.join(", ")
        );
        let anchor: Anchor = from_str(&yaml).unwrap();
        let Anchor::Unknown { raw, .. } = anchor else {
            panic!("expected Anchor::Unknown, got {anchor:?}");
        };
        assert_eq!(raw.get("marks"), Some(&json!(marks)));
        assert_eq!(raw.get("after"), Some(&json!(3)));
    }

    #[test]
    fn a_non_finite_number_is_still_a_number_where_a_float_is_asked_for() {
        #[derive(Deserialize)]
        struct Weighted {
            weight: f64,
            limits: Vec<f64>,
        }

        let weighted: Weighted = from_str("weight: -.inf\nlimits: [.inf, .NaN]\n").unwrap();
        assert_eq!(weighted.weight, f64::NEG_INFINITY);
        assert_eq!(weighted.limits[0], f64::INFINITY);
        assert!(weighted.limits[1].is_nan());
    }

    #[test]
    fn a_non_finite_number_does_not_hide_a_syntax_error_after_it() {
        // The reader reads as it goes and, where the type is guessed, it
        // refuses such a number: left bare, the number would be the problem
        // reported, though it is the open quote that keeps the file from
        // loading.
        let broken = "phase: 1\ntitle: \"T\nstatus: done\n";
        let err = from_str::<Phase>(&format!("junk: [.inf, .nan, 1e999]\n{broken}")).unwrap_err();
        let twin = from_str::<Phase>(&format!("junk: [abcd, abcd, abcde]\n{broken}")).unwrap_err();
        assert_eq!(err, twin);
        assert_eq!(err.line(), Some(4));
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
            // A blank line under the header of a file with CRLF line endings.
            ("k: |\r\n\r\n  foo", json!({"k": "\nfoo"})),
            // A last line that looks blank and is content: a space that is
            // not ASCII, a tab, and spaces past the indentation.
            ("k: |\n  \u{3000}", json!({"k": "\u{3000}"})),
            ("k: |\n  foo\n  \u{3000}", json!({"k": "foo\n\u{3000}"})),
            ("k: |+\n  foo\n  \u{a0}", json!({"k": "foo\n\u{a0}"})),
            ("k: >\n  foo\n  \u{a0}", json!({"k": "foo \u{a0}"})),
            ("k: |\n  foo\n  \t", json!({"k": "foo\n\t"})),
            ("k: |\n  foo\n   ", json!({"k": "foo\n "})),
            ("k: |2\n    foo\n   ", json!({"k": "  foo\n "})),
            ("k: |2\n   ", json!({"k": " "})),
        ];
        for (text, expected) in cases {
            assert_eq!(from_str::<Value>(text).unwrap(), expected, "{text:?}");
            let old = serde_yaml::from_str::<Value>(text).unwrap();
            assert_eq!(old, expected, "the old reader, {text:?}");
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
            // The last line is spaces within the indentation: an empty
            // line, with the break before it in the text.
            ("k: |\n  foo\n  ", json!({"k": "foo\n"})),
            ("k: |\n  foo\n ", json!({"k": "foo\n"})),
            ("k: |+\n  foo\n\n  ", json!({"k": "foo\n\n"})),
            ("k: |2\n    foo\n  ", json!({"k": "  foo\n"})),
        ];
        for (text, expected) in cases {
            assert_eq!(from_str::<Value>(text).unwrap(), expected, "{text:?}");
            let old = serde_yaml::from_str::<Value>(text).unwrap();
            assert_eq!(old, expected, "the old reader, {text:?}");
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

    // -- a problem is placed in the text as it was given --

    /// The line and column of the problem reading `text` as a phase.
    fn position(text: &str) -> (u64, u64) {
        let err = from_str::<Phase>(text).unwrap_err();
        (err.line().unwrap(), err.column().unwrap())
    }

    /// Asserts that the problem in `text` is reported at `expected`, and
    /// that so is the one in `twin`: the same text, character for character,
    /// but for holding nothing the adapter rewrites, so the position is the
    /// reader's own.
    fn assert_placed_like_its_twin(text: &str, twin: &str, expected: (u64, u64)) {
        assert_eq!(position(twin), expected, "the twin {twin:?}");
        assert_eq!(position(text), expected, "{text:?}");
    }

    #[test]
    fn a_problem_after_a_line_separator_keeps_its_line() {
        for separator in ['\u{2028}', '\u{2029}'] {
            for eol in ["\n", "\r\n", "\r"] {
                assert_placed_like_its_twin(
                    &format!("phase: 1{eol}title: 'a{separator}  b'{eol}status: nearly{eol}"),
                    &format!("phase: 1{eol}title: 'a   b'{eol}status: nearly{eol}"),
                    (3, 9),
                );
            }
            // Three of them, in two scalars.
            assert_placed_like_its_twin(
                &format!(
                    "phase: 1\ntitle: 'a{separator}  b{separator}  c'\n\
                     commit: \"d{separator}  e\"\nstatus: nearly\n"
                ),
                "phase: 1\ntitle: 'a   b   c'\ncommit: \"d   e\"\nstatus: nearly\n",
                (4, 9),
            );
        }
    }

    #[test]
    fn a_problem_on_the_line_of_a_bare_word_keeps_its_column() {
        let cases = [
            // After one quoted word, after two, and after two numbers that
            // are not finite.
            (
                "{phase: 1, title: 0X1F, status: nearly}\n",
                "{phase: 1, title: abcd, status: nearly}\n",
                (1, 33),
            ),
            (
                "{title: 0X1F, commit: -0B1, phase: 1, status: nearly}\n",
                "{title: abcd, commit: abcd, phase: 1, status: nearly}\n",
                (1, 47),
            ),
            (
                "{junk: .inf, more: .NaN, phase: 1, title: T, status: nearly}\n",
                "{junk: abcd, more: abcd, phase: 1, title: T, status: nearly}\n",
                (1, 54),
            ),
            // A byte order mark that opens the line is not a column.
            (
                "\u{feff}{phase: 1, title: 0X1F, status: nearly}\n",
                "\u{feff}{phase: 1, title: abcd, status: nearly}\n",
                (1, 33),
            ),
        ];
        for (text, twin, expected) in cases {
            assert_placed_like_its_twin(text, twin, expected);
        }
    }

    #[test]
    fn a_problem_on_a_line_that_is_not_ascii_keeps_its_column() {
        // A column is a character, whatever its width in bytes.
        assert_placed_like_its_twin(
            "{title: é中文, commit: 0X1F, phase: 1, status: nearly}\n",
            "{title: é中文, commit: abcd, phase: 1, status: nearly}\n",
            (1, 46),
        );
        // A separator, a bare word and a number that is not finite, all
        // ahead of the problem on its line.
        assert_placed_like_its_twin(
            "{title: 'é\u{2028} 中', commit: 0X1F, junk: .inf, phase: 1, status: nearly}\n",
            "{title: 'é  中', commit: abcd, junk: abcd, phase: 1, status: nearly}\n",
            (1, 61),
        );
    }

    #[test]
    fn a_problem_after_every_kind_of_rewrite_keeps_its_position() {
        // Two numbers that are not finite, a bare word, two separators and
        // a block scalar that runs into the end of the text.
        assert_placed_like_its_twin(
            "junk: [.inf, .nan]\ncommit: 0X1F\ntitle: 'a\u{2028}  b\u{2029}  c'\nphase: 1\n\
             status: |\n  nearly",
            "junk: [abcd, abcd]\ncommit: abcd\ntitle: 'a   b   c'\nphase: 1\n\
             status: |\n  nearly\n",
            (6, 3),
        );
    }

    #[test]
    fn a_problem_in_a_block_scalar_at_the_end_of_the_text_keeps_its_position() {
        // The header gains a character, loses none, or swaps one.
        for header in ["|", ">", "|+", "|2", "|+2"] {
            let text = format!("phase: 1\ntitle: T\nstatus: {header}\n  nearly");
            assert_placed_like_its_twin(&text, &format!("{text}\n"), (4, 3));
        }
    }

    #[test]
    fn a_problem_at_a_rewritten_word_or_separator_is_placed_there() {
        // The quoted word is itself the problem.
        assert_eq!(position("phase: 1\ntitle: T\nstatus: 0X1F\n"), (3, 9));
        assert_eq!(
            position("{title: 0X1F, phase: 1, status: +.nan}\n"),
            (1, 33)
        );
        // A separator is a character of its line, so what follows it is
        // further along that line, not on the next one.
        assert_eq!(
            position("phase: 1\ntitle: T\u{2028}status: nearly\n"),
            (2, 18)
        );
        assert_eq!(
            position("phase: 1\ntitle: \"T\u{2028}status: done\n"),
            (2, 11)
        );
    }

    #[test]
    fn a_syntax_error_after_a_rewrite_keeps_its_position() {
        let cases = [
            (
                "title: 'a\u{2028}  b'\nphase: 1\nstatus: \"done\n",
                "title: 'a   b'\nphase: 1\nstatus: \"done\n",
                (4, 1),
            ),
            (
                "junk: .inf\ncommit: 0X1F\nphase: 1\ntitle: T\nstatus: [done\n",
                "junk: abcd\ncommit: abcd\nphase: 1\ntitle: T\nstatus: [done\n",
                (5, 9),
            ),
            (
                "{junk: .inf, commit: 0X1F, phase: 1, title: \"T}\n",
                "{junk: abcd, commit: abcd, phase: 1, title: \"T}\n",
                (1, 45),
            ),
        ];
        for (text, twin, expected) in cases {
            assert_placed_like_its_twin(text, twin, expected);
        }
    }

    #[test]
    fn a_place_is_traced_back_through_every_pass() {
        let mut text = Rewritten::new("ab\u{2028}cd0X1Fef");
        text.apply(line_separators_as_breaks(text.as_str()));
        text.apply([Replacement {
            at: 5..9,
            with: "'0X1F'".to_string(),
        }]);
        assert_eq!(text.as_str(), "ab\ncd'0X1F'ef");

        // Before a replacement, on it, inside it, and after it. The
        // separator is three bytes of the original.
        let traced: Vec<usize> = (0..=text.as_str().len())
            .map(|at| text.original_offset(at))
            .collect();
        assert_eq!(traced, [0, 1, 2, 5, 6, 7, 7, 7, 7, 7, 7, 11, 12, 13]);
    }
}
