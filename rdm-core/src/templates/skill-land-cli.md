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

Land **one** `reviewed` rdm item onto `main` with **linear history** and then clean up after it. This is the landing tail of autonomous execution: `rdm-dispatch-phase` / `rdm-autopilot` drive an item to `reviewed` on its `roadmap/<slug>` (or `task/<slug>`) branch but deliberately **never touch `main`**. This skill performs that final, consequential integration — rebasing the branch onto `main` and fast-forwarding — so the existing `Done:`-line post-commit hook flips the item `reviewed → done`.
{principles}
## Contract

**Input** (`$ARGUMENTS`): an **item ref** — `<roadmap>/<phase>`, `task/<slug>`, or a bare `<roadmap>` (lands the whole roadmap's shared branch). This names the single item to land.

This skill is non-interactive. Launch unattended runs with `--permission-mode auto` (or `bypassPermissions` in a sandbox) so the git commands don't block on permission prompts.

## Safety posture

Landing is the one step that **writes to `main`**. It runs **only on explicit invocation** — when you run `/rdm-land`, or when `rdm-autopilot` is given its opt-in `--land` flag. It **never auto-lands**: nothing in the normal review flow reaches `main` on its own. The fast-forward is exactly what triggers `rdm hook post-commit` on the default branch to mark the item `done`, so the `Done:` line the branch already carries keeps working unchanged.

The history guarantee is **linear**: rebase onto `main`, then `git merge --ff-only`. A fast-forward merge creates **no merge commit**. If a fast-forward is not possible, that is a signal to abort — never fall back to a merge commit and never force.

## Preconditions (abort, do not force, if any fail)

Before touching `main`, confirm all of:

1. **The item is `reviewed`.** Read it: `rdm phase show <phase> --roadmap <slug> {proj_flag}` (or `rdm task show <slug> {proj_flag}`). If it is not `reviewed` — e.g. still `needs-review`, `blocked`, or already `done` — stop: only reviewed work lands.
2. **The branch carries the `Done:` line — synthesize it if it is missing.** The reviewed commit must include `Done: <roadmap>/<phase>` (or `Done: task/<slug>`); that line is what the post-commit hook reads. Inspect with `git log`.

   The **autonomous** lane deliberately never writes the trailer: `rdm-dispatch-phase` / `rdm-autopilot` return a structured outcome carrying the item's identifiers plus a `writesCompletion` flag, and the workflow scripts are forbidden from emitting the directive. So a missing trailer is **expected**, not an abort condition — this skill is the land-time writer. Ask rdm for the exact line (never hand-type the format) and amend it on:
   ```bash
   rdm hook done-line --roadmap <slug> --phase <stem>   # or: --task <slug>
   git commit --amend -m "$(git log -1 --pretty=%B)

   $(rdm hook done-line --roadmap <slug> --phase <stem>)"
   ```
   Amend **only** when the branch tip is the un-landed reviewed commit — amending rewrites history, so if the tip is already pushed and shared, or is not the reviewed commit, abort and escalate instead. Abort as well when the item's identifiers are unknown, or when `rdm hook done-line` exits non-zero, since a synthesized trailer would then be a guess — never amend an empty trailer.

   **Read the policy off the outcome, do not infer it.** A dispatch/autopilot OUTCOME carries `writesCompletion: true` on `reviewed` (and `false` on `rework`/`escalated`) alongside its identifiers — that flag *is* the instruction that this branch is owed its trailer. Synthesize and amend it **before** the rebase and fast-forward below, so an autonomously produced branch never needs a manual rebase to gain the line.
3. **The worktree is clean.** No uncommitted changes (`git status --porcelain` is empty).
4. **The CI-equivalent checks pass on the rebased branch** — see step 4 below. This is checked *after* rebasing, not before.

## Steps

> **Worktree topology.** rdm uses **one worktree per roadmap**: the item's branch (`roadmap/<slug>`, `task/<slug>`, or the phase's branch) is checked out in a *linked* worktree, while `main` stays checked out in the **primary** worktree. git refuses to `git checkout` a branch that is already checked out in another worktree, so **never `git checkout main` from inside the item worktree** — operate on each branch in the worktree that already holds it. Find the primary worktree with `git worktree list` (it is the first entry); call it `<primary>` below.

1. **Read item status** and verify it is `reviewed` (precondition 1). Determine its branch: `roadmap/<slug>`, `task/<slug>`, or the phase's branch.
2. **Update `main`** (in the primary worktree, only if it tracks an upstream): if `git -C <primary> rev-parse --abbrev-ref main@{u}` succeeds, refresh it with `git -C <primary> pull --ff-only`. In a local-only repo with no upstream, **skip this** — `main` is already the rebase base, and `git pull` would error with "no tracking information."
3. **Rebase the item's branch onto `main`:** from inside the item's worktree, `git rebase main` (`main` is a ref readable from any worktree — no checkout needed). On conflict → **abort** (see below).
4. **Re-run the CI-equivalent checks on the rebased branch.** There is no universal command for this — determine it from the consuming repo itself, in order: (a) its CI config (e.g. `.github/workflows/`, `.circleci/config.yml`, `.gitlab-ci.yml`); failing that, (b) `docs/principles.md`; failing that, (c) `CLAUDE.md` / `AGENTS.md` in the project root. Run whatever checks that source names. These mirror the project's CI gate. If any fail → **abort** (see below): the rebase may have surfaced a semantic conflict the checks catch.

   If none of the three sources name any checks, do not skip this step — **abort and escalate** instead (see "Abort / escalation" below): landing without a verified rebase is worse than landing late.

   (For illustration only — not an instruction to run here — this repo's own instance of that rule, discovered from its `.github/workflows/` CI config, is `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo nextest run`; see `docs/landing.md`.)
5. **Fast-forward `main`:** advance `main` from the primary worktree where it is checked out — `git -C <primary> merge --ff-only <branch>` (do **not** `git checkout main` inside the item worktree). Assert this produces **no merge commit** (a true fast-forward). If `--ff-only` is refused, abort — do **not** retry without it.
6. **Confirm the item flipped `reviewed → done`.** The fast-forward onto the default branch fires `rdm hook post-commit`, which reads the `Done:` line and marks the item done. Verify with `rdm phase show <phase> --roadmap <slug> {proj_flag}`. If the hook did not run (e.g. hooks not installed), apply the idempotent fallback:
   ```bash
   rdm phase update <phase> --status done --commit <sha> --no-edit --roadmap <slug> {proj_flag}
   # or, for a task:
   rdm task update <slug> --status done --commit <sha> --no-edit {proj_flag}
   ```
7. **Clean up the worktree:** `rdm worktree remove <item> --delete-branch {proj_flag}` removes this item's worktree and its now-merged branch. For batch end-of-run cleanup of *all* already-`done` items at once, use `rdm worktree prune {proj_flag}` (add `--delete-branch` to also drop the merged branches).

## Abort / escalation

On **rebase conflict**, **failing checks**, or **no CI-equivalent checks determinable**:

- `git rebase --abort` (or `git merge --abort` if a merge was in flight) to return the branch to its pre-landing state.
- **Leave the worktree intact** — never `git reset --hard`, force-push, force-merge, or discard the work.
- Surface an **actionable escalation** per `docs/escalation-protocol.md`: state which precondition or step failed, the conflicting files or failing check, and that `main` was left untouched. The item stays `reviewed`, ready for a human to resolve the conflict and re-run landing.

`main` is only ever advanced by a clean fast-forward of fully-checked, reviewed work. Anything less aborts.
