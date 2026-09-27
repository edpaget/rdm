//! Serializable views of the working-tree reports `rdm status`, `rdm commit`
//! and `rdm discard` print.
//!
//! Each porcelain builds one report value ([`StatusView`], [`ScopedCommit`],
//! [`ScopedDiscard`]) and renders it either as human text or, through the
//! `to_json` conversions here, as the matching [`rdm_core::json`] type. The
//! conversions live in this crate, next to the types they read, so no
//! interface has to re-derive the JSON shape.

use rdm_core::json::{
    ChangesJson, CommitJson, DiscardJson, FileChangeJson, FileChangeKindJson, MergeStateJson,
    StatusJson, SyncStatusJson,
};

use crate::{FileChange, FileStatus, ScopedCommit, ScopedDiscard, StatusReport, SyncStatus};

impl FileChange {
    /// Returns the JSON change kind for this change.
    pub fn to_json(&self) -> FileChangeKindJson {
        match self {
            FileChange::Added => FileChangeKindJson::Added,
            FileChange::Modified => FileChangeKindJson::Modified,
            FileChange::Deleted => FileChangeKindJson::Deleted,
        }
    }
}

impl FileStatus {
    /// Returns this path and its change kind as JSON.
    pub fn to_json(&self) -> FileChangeJson {
        FileChangeJson {
            path: self.path.clone(),
            change: self.change.to_json(),
        }
    }
}

fn changes_to_json(files: &[FileStatus]) -> Vec<FileChangeJson> {
    files.iter().map(FileStatus::to_json).collect()
}

impl StatusReport {
    /// Returns the three-way partition as the JSON shape `rdm status`,
    /// `rdm commit` and `rdm discard` share.
    pub fn to_json(&self) -> ChangesJson {
        ChangesJson {
            user: changes_to_json(&self.user),
            others: changes_to_json(&self.others),
            unattributed: changes_to_json(&self.unattributed),
        }
    }
}

impl SyncStatus {
    /// Returns this sync state as JSON.
    pub fn to_json(&self) -> SyncStatusJson {
        SyncStatusJson {
            remote: self.remote.clone(),
            branch: self.branch.clone(),
            ahead: self.ahead,
            behind: self.behind,
        }
    }
}

/// A merge in progress in the plan repo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MergeState {
    /// Number of paths still carrying unresolved conflicts.
    pub unmerged: usize,
}

impl MergeState {
    /// Returns this merge state as JSON.
    pub fn to_json(&self) -> MergeStateJson {
        MergeStateJson {
            unmerged: self.unmerged,
        }
    }
}

/// Everything `rdm status` reports, gathered before anything is printed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusView {
    /// The working tree, in the scoped or the whole-tree (`--all`) view.
    pub report: StatusReport,
    /// The merge in progress, or `None` when there is none.
    pub merge: Option<MergeState>,
    /// The sync state against the default remote, or `None` when no default
    /// remote is configured, there is no tracking ref, or the query failed.
    pub sync: Option<SyncStatus>,
    /// Warnings for stderr (a failed fetch, an undeterminable sync state).
    /// Printed in every output format and never part of the JSON document.
    pub warnings: Vec<String>,
}

impl StatusView {
    /// Returns this view as the `rdm status --format json` document.
    pub fn to_json(&self) -> StatusJson {
        StatusJson {
            changes: self.report.to_json(),
            merge: self.merge.as_ref().map(MergeState::to_json),
            sync: self.sync.as_ref().map(SyncStatus::to_json),
        }
    }
}

impl ScopedCommit {
    /// Returns this commit as the `rdm commit --format json` document.
    ///
    /// `advisory` is the harness-continuity advisory the caller computed
    /// (see [`is_continuity_symptom`](Self::is_continuity_symptom)).
    pub fn to_json(&self, advisory: Option<&str>) -> CommitJson {
        CommitJson {
            commit: self.sha.clone(),
            changeset: self.changeset.clone(),
            skipped: self.skipped_missing.clone(),
            advisory: advisory.map(str::to_owned),
            changes: self.report.to_json(),
        }
    }
}

impl ScopedDiscard {
    /// Returns this discard as the `rdm discard --format json` document.
    ///
    /// `merge_aborted` says whether the caller aborted an in-progress merge
    /// before discarding.
    pub fn to_json(&self, merge_aborted: bool) -> DiscardJson {
        DiscardJson {
            merge_aborted,
            restored: self.restored.clone(),
            skipped: self.skipped_overwritten.clone(),
            changes: self.report.to_json(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CommitReport;

    fn fs(path: &str, change: FileChange) -> FileStatus {
        FileStatus {
            path: path.to_owned(),
            change,
        }
    }

    fn report() -> StatusReport {
        StatusReport {
            user: vec![
                fs("a.md", FileChange::Added),
                fs("m.md", FileChange::Modified),
                fs("d.md", FileChange::Deleted),
            ],
            others: vec![fs("o.md", FileChange::Modified)],
            unattributed: vec![fs("u.md", FileChange::Added)],
        }
    }

    #[test]
    fn change_kinds_map_one_to_one() {
        assert_eq!(FileChange::Added.to_json(), FileChangeKindJson::Added);
        assert_eq!(FileChange::Modified.to_json(), FileChangeKindJson::Modified);
        assert_eq!(FileChange::Deleted.to_json(), FileChangeKindJson::Deleted);
        let v = serde_json::to_value(FileChangeKindJson::Modified).unwrap();
        assert_eq!(v, "modified");
    }

    #[test]
    fn the_partition_keeps_all_three_buckets() {
        let json = report().to_json();
        let paths = |v: &[FileChangeJson]| v.iter().map(|f| f.path.clone()).collect::<Vec<_>>();
        assert_eq!(paths(&json.user), ["a.md", "m.md", "d.md"]);
        assert_eq!(json.user[2].change, FileChangeKindJson::Deleted);
        assert_eq!(paths(&json.others), ["o.md"]);
        assert_eq!(paths(&json.unattributed), ["u.md"]);
    }

    #[test]
    fn status_view_nulls_absent_merge_and_sync() {
        let view = StatusView {
            report: report(),
            warnings: vec!["warning: fetch failed: x".to_owned()],
            ..StatusView::default()
        };
        let v = serde_json::to_value(view.to_json()).unwrap();
        assert!(v["merge"].is_null());
        assert!(v["sync"].is_null());
        assert!(v.get("warnings").is_none(), "warnings are stderr-only");

        let view = StatusView {
            merge: Some(MergeState { unmerged: 2 }),
            sync: Some(SyncStatus {
                remote: "origin".to_owned(),
                branch: "main".to_owned(),
                ahead: 1,
                behind: 3,
            }),
            ..StatusView::default()
        };
        let v = serde_json::to_value(view.to_json()).unwrap();
        assert_eq!(v["merge"]["unmerged"], 2);
        assert_eq!(v["sync"]["behind"], 3);
    }

    #[test]
    fn a_commit_that_landed_nothing_has_a_null_sha() {
        let commit = ScopedCommit {
            changeset: Some("cs".to_owned()),
            skipped_missing: vec!["gone.md".to_owned()],
            report: report(),
            ..ScopedCommit::default()
        };
        let v = serde_json::to_value(commit.to_json(Some("advice"))).unwrap();
        assert!(v["commit"].is_null());
        assert_eq!(v["changeset"], "cs");
        assert_eq!(v["skipped"][0], "gone.md");
        assert_eq!(v["advisory"], "advice");
        assert_eq!(v["changes"]["unattributed"][0]["path"], "u.md");
    }

    #[test]
    fn a_whole_tree_commit_has_no_changeset() {
        let whole = StatusReport {
            user: vec![fs("a.md", FileChange::Added)],
            ..StatusReport::default()
        };
        let commit = ScopedCommit::whole_tree(
            whole.clone(),
            CommitReport {
                sha: Some("abc".to_owned()),
                committed: vec!["a.md".to_owned()],
                ..CommitReport::default()
            },
        );
        assert_eq!(commit.report, whole);
        let v = serde_json::to_value(commit.to_json(None)).unwrap();
        assert_eq!(v["commit"], "abc");
        assert!(v["changeset"].is_null());
        assert!(v["advisory"].is_null());
    }

    #[test]
    fn only_an_empty_commit_over_a_dirty_tree_is_a_continuity_symptom() {
        let dirty = ScopedCommit {
            report: report(),
            ..ScopedCommit::default()
        };
        assert!(dirty.is_continuity_symptom());
        let clean = ScopedCommit::default();
        assert!(!clean.is_continuity_symptom(), "a clean tree is no symptom");
        let landed = ScopedCommit {
            sha: Some("abc".to_owned()),
            report: report(),
            ..ScopedCommit::default()
        };
        assert!(
            !landed.is_continuity_symptom(),
            "a landed commit is no symptom"
        );
    }

    #[test]
    fn a_whole_tree_discard_restores_every_user_path() {
        let whole = StatusReport {
            user: vec![
                fs("a.md", FileChange::Added),
                fs("m.md", FileChange::Modified),
            ],
            ..StatusReport::default()
        };
        let discard = ScopedDiscard::whole_tree(whole);
        assert_eq!(discard.restored, ["a.md", "m.md"]);
        let v = serde_json::to_value(discard.to_json(true)).unwrap();
        assert_eq!(v["merge_aborted"], true);
        assert_eq!(v["restored"][1], "m.md");
        assert_eq!(v["skipped"], serde_json::json!([]));

        let nothing = ScopedDiscard::whole_tree(StatusReport::default());
        let v = serde_json::to_value(nothing.to_json(false)).unwrap();
        assert_eq!(v["restored"], serde_json::json!([]));
    }

    #[test]
    fn a_scoped_discard_reports_what_it_skipped() {
        let discard = ScopedDiscard {
            report: report(),
            restored: vec!["a.md".to_owned()],
            skipped_overwritten: vec!["m.md".to_owned()],
        };
        let v = serde_json::to_value(discard.to_json(false)).unwrap();
        assert_eq!(v["restored"], serde_json::json!(["a.md"]));
        assert_eq!(v["skipped"], serde_json::json!(["m.md"]));
        assert_eq!(v["changes"]["others"][0]["path"], "o.md");
    }
}
