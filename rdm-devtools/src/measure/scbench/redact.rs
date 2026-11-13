//! Scrubs the provider's credential from the files a `slop-code` call left
//! behind.
//!
//! SlopCodeBench logs the full `docker exec ... --env
//! CLAUDE_CODE_OAUTH_TOKEN=<value> ...` line into each problem's `infer.log`,
//! and the pinned checkout is not patched, so after every `slop-code` call
//! the runner rewrites every regular file under that call's output directory
//! ([`scrub_tree`]), replacing each occurrence of the credential's value:
//!
//! - a gzip stream is decoded, scrubbed and re-encoded;
//! - a tar archive is masked at the value's length ([`PLACEHOLDER`] cycled or
//!   truncated), so member sizes and headers stay valid and it still extracts;
//! - any other file has each occurrence replaced with [`PLACEHOLDER`].
//!
//! Values inside zlib-compressed git objects within a tar are not decoded.
//! An unset or empty credential is a no-op, and the scrub never fails the
//! run: a file it cannot rewrite is named on stderr.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;

use super::ladder;

/// What each occurrence of the credential is replaced with.
pub const PLACEHOLDER: &[u8] = b"***redacted***";

/// A credential's bytes. Its `Debug` never prints the value, and it has no
/// `Display`, `Serialize`, `Clone` or string accessor.
pub struct Secret(Vec<u8>);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl Secret {
    /// The value of `provider`'s credential variable
    /// ([`ladder::credential_variable`]), or `None` for an unknown provider
    /// or an unset or empty variable.
    #[must_use]
    pub fn for_provider(provider: &str) -> Option<Secret> {
        let variable = ladder::credential_variable(provider)?;
        Self::from_value(std::env::var_os(variable))
    }

    fn from_value(value: Option<OsString>) -> Option<Secret> {
        use std::os::unix::ffi::OsStringExt;
        let bytes = value?.into_vec();
        (!bytes.is_empty()).then_some(Secret(bytes))
    }
}

/// [`PLACEHOLDER`] cycled or truncated to exactly `len` bytes.
fn mask(len: usize) -> Vec<u8> {
    PLACEHOLDER.iter().copied().cycle().take(len).collect()
}

/// `data` with every non-overlapping occurrence of `needle` (scanned left to
/// right) replaced by `with`, or `None` when there is none.
fn replace(data: &[u8], needle: &[u8], with: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    let mut found = false;
    while i + needle.len() <= data.len() {
        if &data[i..i + needle.len()] == needle {
            out.extend_from_slice(&data[start..i]);
            out.extend_from_slice(with);
            i += needle.len();
            start = i;
            found = true;
        } else {
            i += 1;
        }
    }
    if !found {
        return None;
    }
    out.extend_from_slice(&data[start..]);
    Some(out)
}

fn is_tar(data: &[u8]) -> bool {
    data.get(257..262) == Some(b"ustar")
}

/// `data` with `secret` scrubbed out, or `None` when it holds no copy: gzip
/// is decoded, scrubbed and re-encoded; a tar is masked at the same length;
/// anything else has each occurrence replaced with [`PLACEHOLDER`].
#[must_use]
pub fn scrub_bytes(data: &[u8], secret: &Secret) -> Option<Vec<u8>> {
    if data.starts_with(&[0x1f, 0x8b]) {
        let mut decoded = Vec::new();
        if MultiGzDecoder::new(data).read_to_end(&mut decoded).is_ok() {
            let scrubbed = scrub_bytes(&decoded, secret)?;
            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            if encoder.write_all(&scrubbed).is_ok()
                && let Ok(encoded) = encoder.finish()
            {
                return Some(encoded);
            }
        }
    }
    let needle = secret.0.as_slice();
    if is_tar(data) {
        replace(data, needle, &mask(needle.len()))
    } else {
        replace(data, needle, PLACEHOLDER)
    }
}

/// What [`scrub_tree`] did.
#[derive(Debug, Default)]
pub struct ScrubReport {
    /// Files rewritten without the credential.
    pub scrubbed: Vec<PathBuf>,
    /// Files or directories that could not be read or rewritten; they may
    /// still hold the credential.
    pub failed: Vec<PathBuf>,
}

fn cannot(who: &str, path: &Path, e: &std::io::Error, stderr: &mut dyn Write) {
    let _ = writeln!(
        stderr,
        "{who}: cannot scrub the credential from {} ({:?}); it may still hold the credential",
        path.display(),
        e.kind()
    );
}

fn walk(dir: &Path, secret: &Secret, who: &str, stderr: &mut dyn Write, report: &mut ScrubReport) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            cannot(who, dir, &e, stderr);
            report.failed.push(dir.to_path_buf());
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                cannot(who, dir, &e, stderr);
                report.failed.push(dir.to_path_buf());
                continue;
            }
        };
        let path = entry.path();
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => walk(&path, secret, who, stderr, report),
            Ok(meta) if meta.is_file() => scrub_file(&path, secret, who, stderr, report),
            Ok(_) => {}
            Err(e) => {
                cannot(who, &path, &e, stderr);
                report.failed.push(path);
            }
        }
    }
}

fn scrub_file(
    path: &Path,
    secret: &Secret,
    who: &str,
    stderr: &mut dyn Write,
    report: &mut ScrubReport,
) {
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(e) => {
            cannot(who, path, &e, stderr);
            report.failed.push(path.to_path_buf());
            return;
        }
    };
    let Some(scrubbed) = scrub_bytes(&data, secret) else {
        return;
    };
    // In place, so the file keeps its mode, owner and hard links.
    let written = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .and_then(|mut f| f.write_all(&scrubbed));
    match written {
        Ok(()) => report.scrubbed.push(path.to_path_buf()),
        Err(e) => {
            cannot(who, path, &e, stderr);
            report.failed.push(path.to_path_buf());
        }
    }
}

/// Rewrites every regular file under `root` without `secret`, never
/// following a symlink. `None` or a missing `root` touches nothing. A file
/// or directory that cannot be read or rewritten is named on `stderr`
/// (path and error kind only, never the value) and recorded in
/// [`ScrubReport::failed`]; when any file changed, one
/// `<who>: scrubbed the credential from <n> file(s) under <root>` line
/// follows. It never fails.
pub fn scrub_tree(
    root: &Path,
    secret: Option<&Secret>,
    who: &str,
    stderr: &mut dyn Write,
) -> ScrubReport {
    let mut report = ScrubReport::default();
    let Some(secret) = secret else {
        return report;
    };
    match std::fs::symlink_metadata(root) {
        Ok(meta) if meta.is_dir() => walk(root, secret, who, stderr, &mut report),
        Ok(meta) if meta.is_file() => scrub_file(root, secret, who, stderr, &mut report),
        _ => return report,
    }
    if !report.scrubbed.is_empty() {
        let _ = writeln!(
            stderr,
            "{who}: scrubbed the credential from {} file(s) under {}",
            report.scrubbed.len(),
            root.display()
        );
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALUE: &[u8] = b"sk-ant-oat01-unit-SECRET-42";

    fn secret() -> Secret {
        Secret(VALUE.to_vec())
    }

    fn holds(data: &[u8], needle: &[u8]) -> bool {
        data.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn plain_replacement_keeps_surrounding_bytes() {
        let data = [b"--env T=".as_slice(), VALUE, b" image\n"].concat();
        let want = [b"--env T=".as_slice(), PLACEHOLDER, b" image\n"].concat();
        assert_eq!(scrub_bytes(&data, &secret()), Some(want));
    }

    #[test]
    fn every_occurrence_is_replaced() {
        let data = [VALUE, b"|", VALUE].concat();
        let want = [PLACEHOLDER, b"|", PLACEHOLDER].concat();
        assert_eq!(scrub_bytes(&data, &secret()), Some(want));
    }

    #[test]
    fn no_occurrence_is_unchanged() {
        assert_eq!(scrub_bytes(b"nothing here", &secret()), None);
    }

    #[test]
    fn gzip_round_trip_decodes_to_scrubbed_text() {
        let text = [b"line ".as_slice(), VALUE, b" end\n"].concat();
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&text).expect("encode");
        let gz = enc.finish().expect("finish");
        let out = scrub_bytes(&gz, &secret()).expect("changed");
        let mut decoded = Vec::new();
        MultiGzDecoder::new(out.as_slice())
            .read_to_end(&mut decoded)
            .expect("decode");
        assert_eq!(
            decoded,
            [b"line ".as_slice(), PLACEHOLDER, b" end\n"].concat()
        );
    }

    #[test]
    fn tar_member_is_masked_at_the_same_length() {
        let member = [b"token=".as_slice(), VALUE, b"\n"].concat();
        let mut tar = vec![0u8; 512];
        tar[..8].copy_from_slice(b"leak.txt");
        tar[257..263].copy_from_slice(b"ustar\0");
        let mut body = member.clone();
        body.resize(512, 0);
        tar.extend_from_slice(&body);
        tar.extend_from_slice(&[0u8; 1024]);
        let out = scrub_bytes(&tar, &secret()).expect("changed");
        assert_eq!(out.len(), tar.len());
        assert!(!holds(&out, VALUE));
        let masked = [b"token=".as_slice(), &mask(VALUE.len()), b"\n"].concat();
        assert_eq!(&out[512..512 + member.len()], masked.as_slice());
        assert_eq!(&out[..512], &tar[..512]);
    }

    #[test]
    fn debug_hides_the_value() {
        let shown = format!("{:?}", secret());
        assert!(!holds(shown.as_bytes(), VALUE), "{shown}");
    }

    #[test]
    fn mask_cycles_or_truncates_to_length() {
        assert_eq!(mask(3), b"***");
        assert_eq!(mask(PLACEHOLDER.len()), PLACEHOLDER);
        let long = mask(PLACEHOLDER.len() + 4);
        assert_eq!(long.len(), PLACEHOLDER.len() + 4);
        assert_eq!(&long[..PLACEHOLDER.len()], PLACEHOLDER);
        assert_eq!(&long[PLACEHOLDER.len()..], &PLACEHOLDER[..4]);
    }

    #[test]
    fn for_provider_skips_empty_and_unknown() {
        assert!(Secret::for_provider("no-such-provider").is_none());
        assert!(Secret::from_value(None).is_none());
        assert!(Secret::from_value(Some(OsString::new())).is_none());
        assert!(Secret::from_value(Some(OsString::from("v"))).is_some());
    }
}
