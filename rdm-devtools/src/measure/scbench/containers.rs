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
//!
//! Containers come and go between the listing and the inspection (SCBench's
//! own teardown, a concurrent run). `docker inspect` then exits non-zero but
//! still prints the containers it found, so its partial output is used, and
//! each attributed container is removed with its own `docker rm`, so one
//! that is already gone does not hide the others. `docker ps --filter
//! volume=` cannot narrow the listing: it matches a mount's exact name,
//! source or destination, and SCBench's mount sources are random
//! subdirectories of the scratch directory.

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

/// `docker rm --force <id>`.
pub(super) fn remove(docker: &str, id: &str) -> CommandSpec {
    spec(docker, &["rm", "--force", id], &[])
}

#[derive(Deserialize)]
struct Mount {
    #[serde(rename = "Source", default)]
    source: Option<PathBuf>,
}

/// What [`attributed`] found in `docker inspect`'s output.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Attribution {
    /// The ids of the containers mounting the scratch directory.
    pub(super) ours: Vec<String>,
    /// The 1-based numbers of the lines that were not `<id> <mounts JSON>`
    /// (only numbers, never the text, which is not ours to print).
    pub(super) malformed: Vec<usize>,
}

/// The ids in `inspect_stdout` (as [`inspect`] prints it) whose container has
/// a mount `Source` under `scratch` or its canonical form (on macOS the
/// temp directory `/var/folders/...` is `/private/var/folders/...`). The match
/// is by path component, so `<scratch>x/...` does not match. A line that is
/// not `<id> <mounts JSON>` is skipped and its number recorded, so one bad
/// line never hides the containers on the others.
pub(super) fn attributed(inspect_stdout: &str, scratch: &Path) -> Attribution {
    let canonical = std::fs::canonicalize(scratch).ok();
    let under = |source: &Path| {
        source.starts_with(scratch) || canonical.as_deref().is_some_and(|c| source.starts_with(c))
    };
    let mut found = Attribution::default();
    for (n, line) in inspect_stdout.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parsed = line.split_once(' ').and_then(|(id, mounts)| {
            serde_json::from_str::<Option<Vec<Mount>>>(mounts.trim())
                .ok()
                .map(|m| (id, m.unwrap_or_default()))
        });
        let Some((id, mounts)) = parsed else {
            found.malformed.push(n + 1);
            continue;
        };
        if mounts
            .iter()
            .any(|m| m.source.as_deref().is_some_and(under))
        {
            found.ours.push(id.to_owned());
        }
    }
    found
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
            attributed(&out, scratch),
            Attribution {
                ours: vec!["ours".to_owned(), "second".to_owned()],
                malformed: vec![],
            }
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
        assert_eq!(attributed(&out, &link).ours, ["ours"]);
    }

    #[test]
    fn a_malformed_line_is_skipped_by_number_and_the_rest_still_count() {
        let out = format!(
            "no-space-here\n{}\nsecret-looking-text {{not json",
            line("ours", &["/x/ws".to_owned()])
        );
        assert_eq!(
            attributed(&out, Path::new("/x")),
            Attribution {
                ours: vec!["ours".to_owned()],
                malformed: vec![1, 3],
            }
        );
    }
}
