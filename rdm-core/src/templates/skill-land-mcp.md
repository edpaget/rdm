---
name: rdm-land
description: Land a reviewed rdm item to `main` with linear history (rebase + merge --ff-only), re-running CI-equivalent checks first, then clean up its worktree — aborting and escalating on conflict or failure instead of force-merging
allowed-tools:
  - Read
  - Glob
  - Grep
  - Agent
  - {t_phase_show}
  - {t_phase_update}
  - {t_task_show}
  - {t_task_update}
  - {t_worktree_current}
  - {t_worktree_remove}
---

Land **one** `reviewed` rdm item onto `main` with **linear history** and then clean up after it. This is the landing tail of autonomous execution: `rdm-dispatch-phase` / `rdm-autopilot` drive an item to `reviewed` on its `roadmap/<slug>` (or `task/<slug>`) branch but deliberately **never touch `main`**. This skill performs that final, consequential integration — rebasing the branch onto `main` and fast-forwarding — so the existing `Done:`-line post-commit hook flips the item `reviewed → done`.
{principles}
## Contract

**Input** (`$ARGUMENTS`): an **item ref** — `<roadmap>/<phase>`, `task/<slug>`, or a bare `<roadmap>` (lands the whole roadmap's shared branch). This names the single item to land.

This skill is non-interactive. Launch unattended runs with `bypassPermissions` (or `--permission-mode auto`) so the rdm tool calls and the dispatched subagent don't block on permission prompts.

## Safety posture

Landing is the one step that **writes to `main`**. It runs **only on explicit invocation** — when you run `/rdm-land`, or when `rdm-autopilot` is given its opt-in `--land` flag. It **never auto-lands**: nothing in the normal review flow reaches `main` on its own. The fast-forward is exactly what triggers `rdm hook post-commit` on the default branch to mark the item `done`, so the `Done:` line the branch already carries keeps working unchanged.

The history guarantee is **linear**: rebase onto `main`, then `git merge --ff-only`. A fast-forward merge creates **no merge commit**. If a fast-forward is not possible, that is a signal to abort — never fall back to a merge commit and never force.

## MCP tools vs. delegated git

This variant drives the **status reads/updates and single-worktree cleanup** through the rdm MCP tools:

- `{t_phase_show}` / `{t_task_show}` (with `project: {proj_param}`) to read item status.
- `{t_phase_update}` / `{t_task_update}` for the idempotent `done` fallback.
- `{t_worktree_current}` to confirm context and `{t_worktree_remove}` to clean up the landed worktree.

The **git landing itself (steps 2–5) and the batch prune have no MCP equivalent** — rebase and `merge --ff-only` are inherently shell work, and `rdm worktree prune` ships as a CLI command only this phase. **Delegate that git work to a Bash-capable subagent via the `Agent` tool**, passing it the branch name and the abort-and-escalate contract below. This skill's own frontmatter is Bash-free.

## Preconditions (abort, do not force, if any fail)

Before touching `main`, confirm all of:

1. **The item is `reviewed`.** Read it with `{t_phase_show}` (`project: {proj_param}, roadmap: "<slug>", phase: "<phase>"`) or `{t_task_show}`. If it is not `reviewed` — e.g. still `needs-review`, `blocked`, or already `done` — stop: only reviewed work lands.
2. **The branch carries the `Done:` line — synthesize it if it is missing.** The reviewed commit must include `Done: <roadmap>/<phase>` (or `Done: task/<slug>`); that line is what the post-commit hook reads. The subagent inspects it with `git log`.

   The **autonomous** lane deliberately never writes the trailer: `rdm-dispatch-phase` / `rdm-autopilot` return a structured outcome carrying the item's identifiers plus a `writesCompletion` flag, and the workflow scripts are forbidden from emitting the directive. A missing trailer is therefore **expected**, not an abort condition — this skill is the land-time writer. There is no MCP equivalent for the amend, so instruct the Bash-capable subagent to source the exact line from rdm (never hand-typing the format) and amend it onto the branch tip before the rebase:
   ```bash
   git commit --amend -m "$(git log -1 --pretty=%B)

   $(rdm hook done-line --roadmap <slug> --phase <stem>)"   # or: --task <slug>
   ```
   Amend **only** when the branch tip is the un-landed reviewed commit — amending rewrites history, so if the tip is already pushed and shared, or is not the reviewed commit, abort and escalate instead. Abort as well when the item's identifiers are unknown, or when `rdm hook done-line` exits non-zero, since a synthesized trailer would then be a guess — never amend an empty trailer.

   **Read the policy off the outcome, do not infer it.** A dispatch/autopilot OUTCOME carries `writesCompletion: true` on `reviewed` (and `false` on `rework`/`escalated`) alongside its identifiers — that flag *is* the instruction that this branch is owed its trailer. Synthesize and amend it **before** the rebase and fast-forward below, so an autonomously produced branch never needs a manual rebase to gain the line.
3. **The worktree is clean** (no uncommitted changes — the subagent checks `git status --porcelain`).
4. **The CI-equivalent checks pass on the rebased branch** — see below. Checked *after* rebasing, not before.

## Steps

1. **Read item status** via `{t_phase_show}` / `{t_task_show}` and verify it is `reviewed` (precondition 1). Determine its branch.
2–5. **Delegate the git landing to a Bash-capable subagent** (the `Agent` tool). rdm uses **one worktree per roadmap**: the item's branch is checked out in a *linked* worktree while `main` stays in the **primary** worktree, so `git checkout main` from the item worktree is refused by git. Instruct the subagent to find the primary worktree (`git worktree list`, first entry — call it `<primary>`) and then:
   1. Update `main` in the primary worktree, only if it tracks an upstream: if `git -C <primary> rev-parse --abbrev-ref main@{u}` succeeds, run `git -C <primary> pull --ff-only`; in a local-only repo with no upstream, skip it (`main` is already the rebase base; `git pull` would error "no tracking information").
   2. From inside the item's worktree, `git rebase main` (no checkout of `main` needed — it is a readable ref). On conflict → `git rebase --abort` and report failure (do not continue).
   3. Re-run the CI-equivalent checks on the rebased branch. There is no universal command for this — determine it from the consuming repo itself, in order: (a) its CI config (e.g. `.github/workflows/`, `.circleci/config.yml`, `.gitlab-ci.yml`); failing that, (b) `docs/principles.md`; failing that, (c) `CLAUDE.md` / `AGENTS.md` in the project root. Run whatever checks that source names. If any fail → `git rebase --abort` and report failure.

      If none of the three sources name any checks, do not skip this step — report failure and let the calling skill abort and escalate: landing without a verified rebase is worse than landing late.

      (For illustration only — not an instruction to run here — this repo's own instance of that rule, discovered from its `.github/workflows/` CI config, is `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo nextest run`; see `docs/landing.md`.)
   4. Fast-forward `main` from the primary worktree: `git -C <primary> merge --ff-only <branch>`, asserting **no merge commit**. If `--ff-only` is refused → report failure (never retry without it).
   The subagent reports back the merged commit SHA on success, or which step failed (with conflicting files / failing check) on failure.
6. **Confirm the item flipped `reviewed → done`.** The fast-forward onto the default branch fires `rdm hook post-commit`, which reads the `Done:` line and marks the item done. Verify with `{t_phase_show}` / `{t_task_show}`. If the hook did not run, apply the idempotent fallback via `{t_phase_update}` (`project: {proj_param}, roadmap: "<slug>", phase: "<phase>", status: "done", commit: "<sha>"`) or `{t_task_update}` (`status: "done", commit: "<sha>"`).
7. **Clean up the worktree** via `{t_worktree_remove}` (`project: {proj_param}, target: "<item>", delete_branch: true`) to remove this item's worktree and its now-merged branch. For batch end-of-run cleanup of *all* already-`done` items at once, have the subagent run `rdm worktree prune` (add `--delete-branch` to also drop merged branches) — prune has no MCP tool this phase.

## Abort / escalation

On **rebase conflict**, **failing checks**, or **no CI-equivalent checks determinable**, the subagent must:

- `git rebase --abort` (or `git merge --abort` if a merge was in flight) to restore the branch's pre-landing state.
- **Leave the worktree intact** — never `git reset --hard`, force-push, force-merge, or discard the work.

Then surface an **actionable escalation** per `docs/escalation-protocol.md`: state which precondition or step failed, the conflicting files or failing check, and that `main` was left untouched. The item stays `reviewed`, ready for a human to resolve and re-run landing.

`main` is only ever advanced by a clean fast-forward of fully-checked, reviewed work. Anything less aborts.
