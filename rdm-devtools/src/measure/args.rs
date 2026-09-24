//! Shared command-line helpers for `rdm-measure`: value validation that keeps
//! the JavaScript tools' argument rules, and the exit-code mapping.
//!
//! Every value-taking flag rejects a value that is itself a `--flag` (so an
//! omitted value can never silently swallow the next flag), positive-integer
//! flags validate with JavaScript `Number()` semantics, and repeatable list
//! flags split on commas. Argument errors exit 1 (as the JS tools did, not
//! clap's usual 2); `--help` exits 0.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use clap::error::ErrorKind;

use super::jsnum::{is_integer, to_number};

/// A flag value that must not be another flag.
///
/// # Errors
///
/// When the value starts with `--` (the flag's own value was omitted).
pub fn flag_value(raw: &str) -> Result<String, String> {
    if raw.starts_with("--") {
        Err(format!(
            "requires a value, but got the flag \"{raw}\" (was the value omitted?)"
        ))
    } else {
        Ok(raw.to_owned())
    }
}

fn integer(raw: &str, min: f64, what: &str) -> Result<u64, String> {
    flag_value(raw)?;
    let n = to_number(raw);
    if !is_integer(n) || n < min {
        return Err(format!("must be {what}, got \"{raw}\""));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(n as u64)
}

/// A positive integer (`Number.isInteger(n) && n >= 1`).
///
/// # Errors
///
/// When the value is not a positive integer.
pub fn positive_int(raw: &str) -> Result<u64, String> {
    integer(raw, 1.0, "a positive integer")
}

/// An integer `>= 2` (group-size floors).
///
/// # Errors
///
/// When the value is not an integer of at least 2.
pub fn int_at_least_two(raw: &str) -> Result<u64, String> {
    integer(raw, 2.0, "an integer >= 2")
}

/// Splits repeatable comma-separated values, trimming and dropping empties.
pub fn comma_split(values: &[String]) -> Vec<String> {
    values
        .iter()
        .flat_map(|v| v.split(','))
        .map(|s| super::jsnum::js_trim(s).to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Resolves `p` against `root` unless it is absolute (the JS tools resolved
/// corpus/doc/out paths against the checkout, not the working directory).
pub fn resolve_against(root: &Path, p: &str) -> PathBuf {
    root.join(p)
}

/// JavaScript `path.dirname` of a relative or absolute path string.
pub fn dirname(p: &str) -> String {
    match Path::new(p).parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.display().to_string(),
        Some(_) => ".".to_owned(),
        None => p.to_owned(),
    }
}

/// Parses `T` from the process arguments, printing help (exit 0) or an
/// argument error (exit 1) itself.
///
/// # Errors
///
/// The exit code to return when parsing did not produce a `T`.
pub fn parse_or_exit<T: Parser>() -> Result<T, ExitCode> {
    match T::try_parse() {
        Ok(t) => Ok(t),
        Err(e) => {
            let _ = e.print();
            match e.kind() {
                ErrorKind::DisplayHelp
                | ErrorKind::DisplayVersion
                | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => Err(ExitCode::SUCCESS),
                _ => Err(ExitCode::from(1)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_rules_match_the_js_tools() {
        assert!(flag_value("--format").is_err());
        assert_eq!(flag_value("-x"), Ok("-x".to_owned()));
        assert_eq!(positive_int("3"), Ok(3));
        assert_eq!(positive_int("1e1"), Ok(10));
        assert!(positive_int("0").is_err());
        assert!(positive_int("abc").is_err());
        assert!(positive_int("2.5").is_err());
        assert!(positive_int("").is_err());
        assert!(int_at_least_two("1").is_err());
        assert_eq!(int_at_least_two("2"), Ok(2));
        assert_eq!(
            comma_split(&["a, b".to_owned(), ",c".to_owned()]),
            ["a", "b", "c"]
        );
        assert_eq!(dirname("out.json"), ".");
        assert_eq!(dirname("/tmp/x/out.json"), "/tmp/x");
    }
}
