//! Best-effort diagnostic logger for the post-commit and post-merge hooks.
//!
//! Writes one timestamped line per event to `<git_dir>/rdm-hook.log`. The log
//! lives inside `.git/`, so it is never committed. Logging failures are
//! swallowed so they cannot break the hook.

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use chrono::Utc;

const SIZE_CAP_BYTES: u64 = 256 * 1024;
const TAIL_KEEP_BYTES: usize = 128 * 1024;

/// File-backed logger for hook events. Each call to [`HookLogger::log`]
/// appends a single line. Failures are silently dropped.
pub struct HookLogger {
    path: Option<PathBuf>,
}

impl HookLogger {
    /// Build a logger by discovering the git directory containing `cwd`.
    /// Returns a no-op logger when discovery fails.
    pub fn new(cwd: &Path) -> Self {
        #[cfg(feature = "git")]
        {
            let path = rdm_store_git::discover_git_dir(cwd)
                .ok()
                .map(|git_dir| git_dir.join("rdm-hook.log"));
            Self { path }
        }
        #[cfg(not(feature = "git"))]
        {
            let _ = cwd;
            Self { path: None }
        }
    }

    /// Append one event line. `event` is a short kebab-case identifier;
    /// `kv` is rendered as space-separated `key=value` pairs.
    pub fn log(&self, hook: &str, event: &str, kv: &[(&str, &str)]) {
        let Some(path) = self.path.as_deref() else {
            return;
        };
        let _ = write_line(path, hook, event, kv);
    }
}

fn write_line(path: &Path, hook: &str, event: &str, kv: &[(&str, &str)]) -> std::io::Result<()> {
    let mut line = String::new();
    line.push_str(&Utc::now().to_rfc3339());
    line.push(' ');
    line.push_str(hook);
    line.push(' ');
    line.push_str(event);
    for (key, value) in kv {
        line.push(' ');
        line.push_str(key);
        line.push('=');
        line.push_str(&escape_value(value));
    }
    line.push('\n');

    truncate_if_oversize(path)?;

    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(line.as_bytes())?;
    Ok(())
}

fn escape_value(value: &str) -> String {
    if value
        .chars()
        .any(|c| c.is_whitespace() || c == '"' || c == '\\')
    {
        let escaped: String = value
            .chars()
            .flat_map(|c| match c {
                '"' => vec!['\\', '"'],
                '\\' => vec!['\\', '\\'],
                '\n' => vec!['\\', 'n'],
                '\r' => vec!['\\', 'r'],
                _ => vec![c],
            })
            .collect();
        format!("\"{escaped}\"")
    } else {
        value.to_string()
    }
}

fn truncate_if_oversize(path: &Path) -> std::io::Result<()> {
    let metadata = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if metadata.len() <= SIZE_CAP_BYTES {
        return Ok(());
    }

    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let len = metadata.len();
    let keep = TAIL_KEEP_BYTES as u64;
    let start = len.saturating_sub(keep);
    file.seek(SeekFrom::Start(start))?;
    let mut tail = Vec::with_capacity(TAIL_KEEP_BYTES);
    file.read_to_end(&mut tail)?;

    // Drop a partial leading line so the file always starts at a line boundary.
    if start > 0
        && let Some(pos) = tail.iter().position(|b| *b == b'\n')
    {
        tail.drain(..=pos);
    }

    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&tail)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn escape_value_quotes_whitespace() {
        assert_eq!(escape_value("plain"), "plain");
        assert_eq!(escape_value("two words"), "\"two words\"");
        assert_eq!(escape_value("a\"b"), "\"a\\\"b\"");
    }

    #[test]
    fn truncate_keeps_tail_when_oversize() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log");
        let line = "x".repeat(1024) + "\n";
        let mut f = fs::File::create(&path).unwrap();
        for _ in 0..400 {
            f.write_all(line.as_bytes()).unwrap();
        }
        drop(f);
        assert!(fs::metadata(&path).unwrap().len() > SIZE_CAP_BYTES);

        truncate_if_oversize(&path).unwrap();

        let len = fs::metadata(&path).unwrap().len();
        assert!(
            len <= TAIL_KEEP_BYTES as u64,
            "file should be at most TAIL_KEEP_BYTES after truncate, was {len}"
        );
    }

    #[test]
    fn truncate_noop_when_small() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log");
        fs::write(&path, b"hello\n").unwrap();
        truncate_if_oversize(&path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"hello\n");
    }

    #[test]
    fn truncate_noop_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        truncate_if_oversize(&dir.path().join("absent")).unwrap();
    }
}
