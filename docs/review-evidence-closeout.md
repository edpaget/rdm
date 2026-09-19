# Review-evidence closeout: `agent-orchestrated-dispatch`

Phase 15 of the `agent-orchestrated-dispatch` roadmap is a bounded evidence
reconciliation, not an implementation phase. Nothing lands here, nothing merges,
and no `Done:` trailer is written. This document is the in-repo narrative of the
closeout so the finding travels with the branch; the machine-readable ledger it
summarises lives outside the repo, under
`rdm-review-artifacts/agent-orchestrated-dispatch-20260915/phase15-closeout/`.

## Why the phase exists

Phases 2–5 and their repair tasks carried review provenance that did not
actually describe the branch under review:

- three review follow-ups (`worktree-tests-leak-temp-dirs`,
  `change-review-path-cwd-relative`, `gate-persist-worktree-ref-seam`) were left
  stamped `review_branch: main` / `review_sha: 00a9aa0…` — a commit on `main`,
  not on the implementation branch, and therefore not evidence of anything about
  the fixes;
- phase 4 is parked `blocked` with `[code] rework budget exhausted`, so its
  contract has never been reviewed as an integrated whole;
- phase 2 advanced with an acceptance-criteria reviewer that exhausted its
  structured-output retries, leaving that dimension ungraded rather than clean.

## The finding that reframes the closeout

**Every fix SHA cited by the phase bodies, by the phase-15 plan and by the
persisted `change/<ref>` reviews is a pre-rebase object, and none of them is
reachable from the current branch head.**

The shared implementation branch was rebased onto a newer `main` after those
reviews were recorded. The commits still exist in the object store, so a naive
`git cat-file -e` check passes and the provenance *looks* sound; but
`git merge-base --is-ancestor <sha> HEAD` fails for all of them. Each fix has an
identical-content equivalent on the current branch — established by `git
patch-id --stable`, or by identical subject where the rebase re-resolved a
`CHANGELOG.md` conflict and changed only that file's context lines:

| Cited (pre-rebase) | Equivalent on the branch | Matched by |
|---|---|---|
| `c647ab0` | `1d5e974` | patch-id |
| `bd88812` | `68e87fa` | subject (CHANGELOG context only) |
| `2c55784` | `cd362a9` | subject (CHANGELOG context only) |
| `95db729` | `649afaf` | patch-id |
| `ba50d8e` | `4bf58a3` | patch-id |
| `fe990ef` | `35cea7f` | subject |
| `3992b05` | `aaba51c` | patch-id |
| `8fa46f8` | `8f2bc7d` | patch-id |

The consequence is not cosmetic. A review is pinned to a `base..head` pair; if
neither endpoint is reachable from the head being shipped, the review does not
cover the shipped content, whatever its verdict says. So **no persisted approval
on this roadmap can be carried forward by SHA**, and the reconciliation has to
re-review the final forms at a freshly frozen head rather than re-stamp the old
verdicts onto it.

A second, independent disqualification applies to the same reviews: every one of
them is authored by the Codex author-check lane (`codex-independent-code-review`,
`codex-independent-plan-review`). The phase is explicit that Codex author checks
do not count as independent review, so those verdicts are recorded as historical
evidence with `independent: false` rather than as approval.

## What the ledger records

`closeout-ledger.json` is the canonical artifact and `closeout-ledger.md` is
rendered from it by `build-ledger.mjs`, so the two cannot drift. It carries:

- the frozen checkout identity (worktree, branch, `BASE`, `HEAD_R`, commit count,
  clean-tree proof) cross-checked through `rdm review source`;
- the pre-rebase → on-branch SHA map above, each entry with its reachability
  verdict and how it was matched;
- one row per historical task: its owning phase, how it was derived, its fix
  commits resolved to full on-branch SHAs, the range actually reviewed, the
  historical review with an explicit `independent` flag, any stale stamp, and a
  computed disposition;
- the phase → commit-range partition of `BASE..HEAD_R`.

Dispositions are computed, never asserted. A row whose only evidence is
`main@00a9aa0` reads `stale_stamp_pending_reconciliation`, and `assert-ledger.mjs`
fails the build if any such row is ever given an approval-shaped disposition.

### Deriving the task set

The nine historical tasks are derived, not assumed: the three named by the
phase-15 body, plus the tasks phases 9–13 each name as their own *Owned task*.
That derivation disagrees with the candidate list in the approved plan, which
guessed `clear-phase-5-stale-blocked-reason` and
`anchor-known-variant-field-preservation`. Those are real open tasks but are not
owned by phases 9–13; phase 13's two tasks
(`change-plan-display-and-server-arms-untested`,
`change-target-doc-enumerations-stale`) are. The count is still nine, the
membership differs, and the discrepancy is recorded in the ledger rather than
resolved by padding or trimming to fit the number.

## What this document is not

It is not an approval. Phase 4 stays `blocked` until its own gate is satisfied,
phase 15 reaches `needs-review` only on independent evidence, and `reviewed` is
reserved for the manual lane. The stale task stamps are corrected only through
supported CLI writes carrying the verified checkout identity, and only after
real evidence exists — never by hand-editing a file under the plan repo, and
never by `--override-gate`.
