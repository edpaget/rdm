//! Looking a commit up across the two repositories a review's fix can land
//! in — the project's source repository and the plan repository.
//!
//! `rdm review update --applied-commit` and the server's comment PATCH both
//! verify the commit they are given before recording it. A fix does not
//! always land in the repository the review targets, so both try the
//! repository the review's target kind implies first and the other one
//! second. That ordered walk is [`resolve_commit_in_candidates`]; discovering
//! the candidates and wording the refusal stay with each caller.

use std::path::PathBuf;

use rdm_core::error::Result;
use rdm_core::model::CommitRepo;
use rdm_core::source::SourceRepo;

use crate::GitSourceRepo;

/// One repository [`resolve_commit_in_candidates`] may look in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitCandidate {
    /// Which of the two repositories this is.
    pub repo: CommitRepo,
    /// Where it is, or why it could not be located at all (the caller's own
    /// discovery failure text).
    pub location: std::result::Result<PathBuf, String>,
}

/// Why a candidate repository did not yield the commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateMiss {
    /// The repository could not be located; the text is the discovery
    /// failure's own.
    Unavailable(String),
    /// The path exists as a candidate but is not inside a git checkout.
    NotAGitCheckout(PathBuf),
    /// The path is a git checkout and the revision does not resolve to a
    /// commit there (unknown, or an ambiguous abbreviation).
    DidNotResolve(PathBuf),
}

/// What [`resolve_commit_in_candidates`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitLookup {
    /// The revision resolved: the full SHA, and the repository it resolved
    /// in.
    Found {
        /// The resolved full commit SHA.
        sha: String,
        /// The repository it resolved in.
        repo: CommitRepo,
    },
    /// The revision resolved nowhere it was looked for.
    NotFound {
        /// What happened at each repository that was looked in, in order.
        misses: Vec<(CommitRepo, CandidateMiss)>,
        /// `true` when `rev` is not commit-shaped (a name such as `HEAD` or
        /// a branch), so only the first candidate was looked in.
        primary_only: bool,
    },
}

/// Whether `rev` is commit-shaped — a hex SHA or a SHA prefix of at least
/// seven characters — rather than a name such as `HEAD`, a branch, or a tag.
///
/// The grammar is the commit link's ([`rdm_core::link::parse`]), so it lives
/// in one place.
#[must_use]
pub fn is_commit_shaped(rev: &str) -> bool {
    matches!(
        rdm_core::link::parse(&format!("rdm:src@{rev}")),
        Ok(rdm_core::link::Link::Commit { .. })
    )
}

/// Resolves `rev` to a full commit SHA in the first of `candidates` that has
/// it, reporting which repository that was.
///
/// `candidates` are tried in order, so the caller puts the repository the
/// review's target kind implies first. A candidate that is unavailable or is
/// not a git checkout is recorded as a miss and the walk moves on — it is
/// not fatal on its own.
///
/// Only a commit-shaped `rev` ([`is_commit_shaped`]) is looked for beyond the
/// first candidate. A name such as `HEAD` or a branch means something
/// different in each repository, so it resolves in the first candidate or
/// not at all: letting it fall through would record the other repository's
/// unrelated `HEAD` as the fix whenever the first one is missing. A later
/// candidate's hit also counts only when the resolved SHA begins with `rev`,
/// so a ref that happens to be named like a hex prefix is never mistaken for
/// the commit.
///
/// # Errors
///
/// Returns an error when git itself fails in a candidate that is a git
/// checkout, or when `rev` is not a legal revision argument (it starts with
/// `-`). A revision that simply resolves nowhere is
/// [`CommitLookup::NotFound`], not an error.
pub fn resolve_commit_in_candidates(
    candidates: &[CommitCandidate],
    rev: &str,
) -> Result<CommitLookup> {
    let primary_only = !is_commit_shaped(rev);
    let tried = if primary_only {
        &candidates[..candidates.len().min(1)]
    } else {
        candidates
    };
    let mut misses = Vec::new();
    for (index, candidate) in tried.iter().enumerate() {
        let path = match &candidate.location {
            Ok(path) => path,
            Err(reason) => {
                misses.push((candidate.repo, CandidateMiss::Unavailable(reason.clone())));
                continue;
            }
        };
        if crate::discover_git_dir(path).is_err() {
            misses.push((candidate.repo, CandidateMiss::NotAGitCheckout(path.clone())));
            continue;
        }
        match GitSourceRepo::new(path).rev_parse(rev)? {
            Some(sha) if index == 0 || sha.starts_with(&rev.to_ascii_lowercase()) => {
                return Ok(CommitLookup::Found {
                    sha,
                    repo: candidate.repo,
                });
            }
            _ => misses.push((candidate.repo, CandidateMiss::DidNotResolve(path.clone()))),
        }
    }
    Ok(CommitLookup::NotFound {
        misses,
        primary_only,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_test_support::git;
    use std::path::Path;
    use tempfile::TempDir;

    /// A git repository with one commit; returns it and that commit's SHA.
    fn repo_with_commit(file: &str) -> (TempDir, String) {
        let dir = TempDir::new().unwrap();
        let p = dir.path();
        git(p, &["init", "-b", "main"]);
        std::fs::write(p.join(file), file).unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-m", file]);
        let sha = GitSourceRepo::new(p).rev_parse("HEAD").unwrap().unwrap();
        (dir, sha)
    }

    fn at(repo: CommitRepo, path: &Path) -> CommitCandidate {
        CommitCandidate {
            repo,
            location: Ok(path.to_path_buf()),
        }
    }

    #[test]
    fn a_sha_resolves_in_the_first_candidate_that_has_it() {
        let (source, source_sha) = repo_with_commit("source.txt");
        let (plan, plan_sha) = repo_with_commit("plan.txt");
        let candidates = [
            at(CommitRepo::Source, source.path()),
            at(CommitRepo::Plan, plan.path()),
        ];
        assert_eq!(
            resolve_commit_in_candidates(&candidates, &source_sha).unwrap(),
            CommitLookup::Found {
                sha: source_sha,
                repo: CommitRepo::Source
            }
        );
        // An abbreviated SHA found only in the second candidate is expanded.
        assert_eq!(
            resolve_commit_in_candidates(&candidates, &plan_sha[..10]).unwrap(),
            CommitLookup::Found {
                sha: plan_sha,
                repo: CommitRepo::Plan
            }
        );
    }

    #[test]
    fn an_unavailable_first_candidate_does_not_stop_a_sha() {
        let (plan, plan_sha) = repo_with_commit("plan.txt");
        let not_git = TempDir::new().unwrap();
        for first in [
            at(CommitRepo::Source, not_git.path()),
            CommitCandidate {
                repo: CommitRepo::Source,
                location: Err("no source configured".to_string()),
            },
        ] {
            let candidates = [first, at(CommitRepo::Plan, plan.path())];
            assert_eq!(
                resolve_commit_in_candidates(&candidates, &plan_sha).unwrap(),
                CommitLookup::Found {
                    sha: plan_sha.clone(),
                    repo: CommitRepo::Plan
                }
            );
        }
    }

    #[test]
    fn a_sha_in_neither_reports_every_candidate_in_order() {
        let (source, _) = repo_with_commit("source.txt");
        let not_git = TempDir::new().unwrap();
        let candidates = [
            at(CommitRepo::Source, source.path()),
            at(CommitRepo::Plan, not_git.path()),
        ];
        let missing = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            resolve_commit_in_candidates(&candidates, missing).unwrap(),
            CommitLookup::NotFound {
                misses: vec![
                    (
                        CommitRepo::Source,
                        CandidateMiss::DidNotResolve(source.path().to_path_buf())
                    ),
                    (
                        CommitRepo::Plan,
                        CandidateMiss::NotAGitCheckout(not_git.path().to_path_buf())
                    ),
                ],
                primary_only: false,
            }
        );
    }

    /// `HEAD` exists in every repository, so it must never fall through to
    /// the second candidate: that would record an unrelated commit.
    #[test]
    fn a_symbolic_rev_resolves_only_in_the_first_candidate() {
        let (source, source_sha) = repo_with_commit("source.txt");
        let (plan, _) = repo_with_commit("plan.txt");
        let not_git = TempDir::new().unwrap();

        let both = [
            at(CommitRepo::Source, source.path()),
            at(CommitRepo::Plan, plan.path()),
        ];
        assert_eq!(
            resolve_commit_in_candidates(&both, "HEAD").unwrap(),
            CommitLookup::Found {
                sha: source_sha,
                repo: CommitRepo::Source
            }
        );

        let primary_missing = [
            at(CommitRepo::Source, not_git.path()),
            at(CommitRepo::Plan, plan.path()),
        ];
        for rev in ["HEAD", "main"] {
            assert_eq!(
                resolve_commit_in_candidates(&primary_missing, rev).unwrap(),
                CommitLookup::NotFound {
                    misses: vec![(
                        CommitRepo::Source,
                        CandidateMiss::NotAGitCheckout(not_git.path().to_path_buf())
                    )],
                    primary_only: true,
                },
                "{rev}"
            );
        }
    }

    /// A branch named like a hex prefix in the second repository is a name,
    /// not the commit that was asked for.
    #[test]
    fn a_hex_named_ref_in_a_later_candidate_is_not_a_hit() {
        let (source, _) = repo_with_commit("source.txt");
        let (plan, plan_sha) = repo_with_commit("plan.txt");
        let name = if plan_sha.starts_with("deadbeef") {
            "cafebabe"
        } else {
            "deadbeef"
        };
        assert!(is_commit_shaped(name));
        git(plan.path(), &["branch", name]);
        let candidates = [
            at(CommitRepo::Source, source.path()),
            at(CommitRepo::Plan, plan.path()),
        ];
        assert!(matches!(
            resolve_commit_in_candidates(&candidates, name).unwrap(),
            CommitLookup::NotFound {
                primary_only: false,
                ..
            }
        ));
    }
}
