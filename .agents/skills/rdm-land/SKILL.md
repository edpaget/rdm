---
name: rdm-land
description: Land explicitly authorized, independently reviewed rdm work with checked linear history and verified completion tracking.
---

# Explicit landing

Discovery of this skill is not permission to merge. Require an explicit user request to land the named item/branch. Use the configured `RDM_BIN` (rebuilt development binary in rdm's own repo) or installed `rdm`, carrying the same `RDM_ROOT`, `RDM_PROJECT`, and stable `RDM_SESSION` into each shell call. Do not change permission settings or bypass sandbox approvals.

```bash
"${RDM_BIN:-rdm}" phase show <phase> --roadmap <roadmap> --format json --project rdm
"${RDM_BIN:-rdm}" worktree list --project rdm
```

Use `task show <slug>` for tasks. Require `reviewed` plus independent review evidence covering the actual branch diff. Inspect every unlanded commit on a shared roadmap branch: landing one phase must not sweep in unreviewed phases. If evidence, scope, or branch identity is unclear, stop and ask.

Resolve the configured default branch and its existing worktree with git. Both worktrees must be clean. Never check out the default branch inside the implementation worktree. If the default branch has an upstream, refresh it with `pull --ff-only` in its own worktree; otherwise skip network refresh.

This is the land-time `Done:` writer. Ask `"${RDM_BIN:-rdm}" hook done-line --roadmap <roadmap> --phase <exact-stem>` (or `--task <slug>`) for the exact directive. Add it only to the unlanded reviewed tip if missing. Amending a pushed/shared tip, an unrelated tip, or an unknown review revision requires further direction; do not rewrite it silently. Never emit directives for unreviewed work.

Rebase the implementation branch onto the default branch, then rerun the repository's CI-equivalent checks discovered from CI configuration, principles, or project instructions. If no checks are defined, stop for direction. If conflicts occur, abort the active rebase and report them. If checks fail after a completed rebase, retain the worktree and report the failure; do not reset away work. Material changes during integration require renewed independent review.

Advance the default branch only with `git -C <default-worktree> merge --ff-only <implementation-branch>`. A refusal is not permission for a merge commit or force operation. Verify the landed SHA and read back the item's status. If completion hooks did not run, apply the idempotent fallback only after verifying the source commit is on the default branch:

```bash
"${RDM_BIN:-rdm}" phase update <phase> --roadmap <roadmap> --status done --commit <landed-sha> --no-edit --project rdm
# For a task: task update <slug> --status done --commit <landed-sha> --no-edit
"${RDM_BIN:-rdm}" status
"${RDM_BIN:-rdm}" commit -m "chore(plan): record verified landing"
```

Report the landing and completion state. Remove the exact worktree/branch only when cleanup is authorized, everything it contains is landed, and no remaining roadmap phase needs it. Never broadly prune other worktrees. No force-push, destructive reset, automatic next-item dispatch, or implicit landing is part of this skill.


## Principles

Read `docs/principles.md` before starting. It contains project conventions that should guide your work.
