//! Finds and removes the Docker containers one interrupted `slop-code` spawn
//! started, and only those.
//!
//! Each `slop-code run`/`eval` is spawned with `TMPDIR` set to a fresh
//! scratch directory this process created. SCBench bind-mounts two Python
//! `tempfile` directories into every container it creates (the workspace and
//! Claude's home), so every container that spawn started has a mount
//! `Source` under the scratch directory, and no other run's container can.
//! Image names, `docker ps` snapshots and labels cannot tell two concurrent
//! runs of the same checkout apart; the scratch mount can.
//!
//! `docker inspect` is formatted to return only ids and mounts, so no
//! container's environment (which holds the credential) is ever read.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::plan::CommandSpec;

fn spec(docker: &str, args: &[&str], more: &[String]) -> CommandSpec {
    CommandSpec {
        program: docker.to_owned(),
        args: args
            .iter()
            .map(|&a| a.to_owned())
            .chain(more.iter().cloned())
            .collect(),
        cwd: None,
        env: std::collections::BTreeMap::new(),
    }
}

/// `docker ps --all --quiet --no-trunc`: every container's full id, stopped
/// ones included (SCBench may stop a container without removing it).
pub(super) fn list(docker: &str) -> CommandSpec {
    spec(docker, &["ps", "--all", "--quiet", "--no-trunc"], &[])
}

/// `docker inspect --format '{{.Id}} {{json .Mounts}}' <ids...>`: one line
/// per container, its id and its mounts, nothing else.
pub(super) fn inspect(docker: &str, ids: &[String]) -> CommandSpec {
    spec(
        docker,
        &["inspect", "--format", "{{.Id}} {{json .Mounts}}"],
        ids,
    )
}

/// `docker rm --force <ids...>`.
pub(super) fn remove(docker: &str, ids: &[String]) -> CommandSpec {
    spec(docker, &["rm", "--force"], ids)
}

#[derive(Deserialize)]
struct Mount {
    #[serde(rename = "Source", default)]
    source: Option<PathBuf>,
}

/// The ids in `inspect_stdout` (as [`inspect`] prints it) whose container has
/// a mount `Source` under `scratch` or its canonical form (on macOS the
/// temp directory `/var/folders/...` is `/private/var/folders/...`). The match
/// is by path component, so `<scratch>x/...` does not match.
///
/// # Errors
///
/// When a line is not `<id> <mounts JSON>`, naming its line number (never
/// its text).
pub(super) fn attributed(inspect_stdout: &str, scratch: &Path) -> Result<Vec<String>, String> {
    let canonical = std::fs::canonicalize(scratch).ok();
    let under = |source: &Path| {
        source.starts_with(scratch) || canonical.as_deref().is_some_and(|c| source.starts_with(c))
    };
    let mut ids = Vec::new();
    for (n, line) in inspect_stdout.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let malformed = || {
            format!(
                "line {} of the `docker inspect` output is not `<id> <mounts JSON>`",
                n + 1
            )
        };
        let (id, mounts) = line.split_once(' ').ok_or_else(malformed)?;
        let mounts: Option<Vec<Mount>> =
            serde_json::from_str(mounts.trim()).map_err(|_| malformed())?;
        if mounts
            .unwrap_or_default()
            .iter()
            .any(|m| m.source.as_deref().is_some_and(under))
        {
            ids.push(id.to_owned());
        }
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: &str, sources: &[String]) -> String {
        let mounts: Vec<serde_json::Value> = sources
            .iter()
            .map(|s| serde_json::json!({ "Type": "bind", "Source": s, "Destination": "/w" }))
            .collect();
        format!("{id} {}", serde_json::Value::from(mounts))
    }

    #[test]
    fn keeps_only_containers_mounting_the_scratch_directory() {
        let scratch = Path::new("/tmp/scbAAAAAA");
        let out = [
            line("ours", &["/tmp/scbAAAAAA/tmpws".to_owned()]),
            line("foreign", &["/tmp/scbBBBBBB/tmpws".to_owned()]),
            line("sibling", &["/tmp/scbAAAAAAx/ws".to_owned()]),
            "nomounts []".to_owned(),
            "nullmounts null".to_owned(),
            line(
                "second",
                &["/elsewhere".to_owned(), "/tmp/scbAAAAAA/home".to_owned()],
            ),
        ]
        .join("\n");
        assert_eq!(
            attributed(&out, scratch).expect("parses"),
            ["ours", "second"]
        );
    }

    #[test]
    fn matches_the_canonical_form_of_the_scratch_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let link_dir = tempfile::tempdir().expect("tempdir");
        let link = link_dir.path().join("scratch-link");
        std::os::unix::fs::symlink(tmp.path(), &link).expect("symlink");
        let canonical = std::fs::canonicalize(tmp.path()).expect("canonical");
        let out = line("ours", &[canonical.join("ws").display().to_string()]);
        assert_eq!(attributed(&out, &link).expect("parses"), ["ours"]);
    }

    #[test]
    fn a_malformed_line_names_its_number_not_its_text() {
        let out = format!(
            "{}\nsecret-looking-text {{not json",
            line("ours", &["/x".to_owned()])
        );
        let err = attributed(&out, Path::new("/x")).expect_err("malformed");
        assert!(err.contains("line 2"), "{err}");
        assert!(!err.contains("secret-looking-text"), "{err}");
        let err = attributed("no-space-here", Path::new("/x")).expect_err("malformed");
        assert!(err.contains("line 1"), "{err}");
    }
}
