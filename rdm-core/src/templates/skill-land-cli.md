---
name: rdm-land
description: Land a reviewed rdm item to `main` with linear history (rebase + merge --ff-only), re-running CI-equivalent checks first, then clean up its worktree — aborting and escalating on conflict or failure instead of force-merging
allowed-tools:
  - Read
  - Bash
  - Glob
  - Grep
  - Agent
---

Land **one** `reviewed` rdm item onto `main` with **linear history** and then clean up after it. This is the landing tail of autonomous execution: `rdm-dispatch-phase` / `rdm-autopilot` drive an item to `reviewed` on its `roadmap/<slug>` (or `task/<slug>`) branch but deliberately **never touch `main`**. This skill performs that final, consequential integration — rebasing the branch onto `main` and fast-forwarding — and then marks the landed item(s) `done` itself, directly.
{principles}
## Contract

**Input** (`$ARGUMENTS`): an **item ref** — `<roadmap>/<phase>`, `task/<slug>`, or a bare `<roadmap>` (lands the whole roadmap's shared branch). This names the single item to land.

This skill is non-interactive. Launch unattended runs with `--permission-mode auto` (or `bypassPermissions` in a sandbox) so the git commands don't block on permission prompts.

## Safety posture

Landing is the one step that **writes to `main`**. It runs **only on explicit invocation** — when you run `/rdm-land`, or when `rdm-autopilot` is given its opt-in `--land` flag. It **never auto-lands**: nothing in the normal review flow reaches `main` on its own. After the fast-forward, this skill marks the landed item(s) `done` itself — no trailer, no hook dependency.

The history guarantee is **linear**: rebase onto `main`, then `git merge --ff-only`. A fast-forward merge creates **no merge commit**. If a fast-forward is not possible, that is a signal to abort — never fall back to a merge commit and never force.

## Preconditions (abort, do not force, if any fail)

Before touching `main`, confirm all of:

1. **The item is `reviewed`.** Read it: `rdm phase show <phase> --roadmap <slug> {proj_flag}` (or `rdm task show <slug> {proj_flag}`). If it is not `reviewed` — e.g. still `needs-review`, `blocked`, or already `done` — stop: only reviewed work lands.
2. **The worktree is clean.** No uncommitted changes (`git status --porcelain` is empty).
3. **The CI-equivalent checks pass on the rebased branch** — see step 4 below. This is checked *after* rebasing, not before.

## Steps

> **Worktree topology.** rdm uses **one worktree per roadmap**: the item's branch (`roadmap/<slug>`, `task/<slug>`, or the phase's branch) is checked out in a *linked* worktree, while `main` stays checked out in the **primary** worktree. git refuses to `git checkout` a branch that is already checked out in another worktree, so **never `git checkout main` from inside the item worktree** — operate on each branch in the worktree that already holds it. Find the primary worktree with `git worktree list` (it is the first entry); call it `<primary>` below.

1. **Read item status** and verify it is `reviewed` (precondition 1). Determine its branch: `roadmap/<slug>`, `task/<slug>`, or the phase's branch. For a bare `<roadmap>` land, also determine the **target set** — every phase that is currently `reviewed`: `rdm phase list --roadmap <slug> --format json {proj_flag}` and keep every element with `"status": "reviewed"`. For a `<roadmap>/<phase>` or `task/<slug>` land, the target set is just that one item.
2. **Update `main`** (in the primary worktree, only if it tracks an upstream): if `git -C <primary> rev-parse --abbrev-ref main@{u}` succeeds, refresh it with `git -C <primary> pull --ff-only`. In a local-only repo with no upstream, **skip this** — `main` is already the rebase base, and `git pull` would error with "no tracking information."
3. **Rebase the item's branch onto `main`:** from inside the item's worktree, `git rebase main` (`main` is a ref readable from any worktree — no checkout needed). On conflict → **abort** (see below).
4. **Re-run the CI-equivalent checks on the rebased branch.** There is no universal command for this — determine it from the consuming repo itself, in order: (a) its CI config (e.g. `.github/workflows/`, `.circleci/config.yml`, `.gitlab-ci.yml`); failing that, (b) `docs/principles.md`; failing that, (c) `CLAUDE.md` / `AGENTS.md` in the project root. Run whatever checks that source names. These mirror the project's CI gate. If any fail → **abort** (see below): the rebase may have surfaced a semantic conflict the checks catch.

   If none of the three sources name any checks, do not skip this step — **abort and escalate** instead (see "Abort / escalation" below): landing without a verified rebase is worse than landing late.

   (For illustration only — not an instruction to run here — this repo's own instance of that rule, discovered from its `.github/workflows/` CI config, is `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo nextest run`; see `docs/landing.md`.)
5. **Fast-forward `main`:** advance `main` from the primary worktree where it is checked out — `git -C <primary> merge --ff-only <branch>` (do **not** `git checkout main` inside the item worktree). Assert this produces **no merge commit** (a true fast-forward). If `--ff-only` is refused, abort — do **not** retry without it.
6. **Mark every item in the target set `done`, directly.** Read the landed tip: `git -C <primary> rev-parse main`. For a single-item land, that tip *is* the item's own last commit — the fast-forward advanced by exactly its un-landed work. For a bare-roadmap land, record the **same** landed tip for every phase in the target set rather than trying to attribute each phase's own distinct commit: a phase's approving change-review head can be wrong when the phase was finalized by a commit made *after* that review, and a phase-start record is not reliably populated for every phase, so neither is a sound general attribution source. Recording the landed tip for every phase gives up per-phase precision but can never cite a wrong, unlanded, or nonexistent commit. For each item in the target set:
   ```bash
   rdm phase update <phase> --status done --commit <tip-sha> --no-edit --roadmap <slug> {proj_flag}
   # or, for a task:
   rdm task update <slug> --status done --commit <tip-sha> --no-edit {proj_flag}
   ```
   This write always happens — it is not a fallback for a missing hook.
7. **Clean up the worktree:** `rdm worktree remove <item> --delete-branch {proj_flag}` removes this item's worktree and its now-merged branch. For batch end-of-run cleanup of *all* already-`done` items at once, use `rdm worktree prune {proj_flag}` (add `--delete-branch` to also drop the merged branches).

## Abort / escalation

On **rebase conflict**, **failing checks**, or **no CI-equivalent checks determinable**:

- `git rebase --abort` (or `git merge --abort` if a merge was in flight) to return the branch to its pre-landing state.
- **Leave the worktree intact** — never `git reset --hard`, force-push, force-merge, or discard the work.
- Surface an **actionable escalation** per `docs/escalation-protocol.md`: state which precondition or step failed, the conflicting files or failing check, and that `main` was left untouched. The item stays `reviewed`, ready for a human to resolve the conflict and re-run landing.

`main` is only ever advanced by a clean fast-forward of fully-checked, reviewed work. Anything less aborts.
