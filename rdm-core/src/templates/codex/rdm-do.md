---
name: rdm-do
description: Implement one authorized rdm phase or task manually in its worktree, verify it, and hand it off for independent review without landing.
---

# Manual implementation lane

This lane uses ordinary shell, file-reading, and editing tools. It does not invoke a workflow engine, dispatch/autopilot, or an automated review skill. Follow repository instructions for building `RDM_BIN`; otherwise use installed `rdm`. Carry the same `RDM_ROOT`, `RDM_PROJECT`, and stable `RDM_SESSION` into every shell call. Replace placeholders before execution.

Read the item and its parent roadmap, relevant documentation, and current worktree state:

```bash
"${RDM_BIN:-rdm}" phase show <phase> --roadmap <roadmap> --format json {proj_flag}
"${RDM_BIN:-rdm}" roadmap show <roadmap> {proj_flag}
"${RDM_BIN:-rdm}" worktree list {proj_flag}
```

For a task, use `task show <slug>` instead. Confirm user authorization, prerequisites, and independent plan-review evidence. If `needs-plan-review` remains on the selected phase/task, or a parent-wide concern applies, stop for independent plan review by a human or working review host. A parent's tag may remain for other phases; do not clear it globally when only this phase passed. No available review host means a pending gate, never a pass.

Use one shared worktree per roadmap:

```bash
"${RDM_BIN:-rdm}" worktree add <roadmap> --format json {proj_flag}
# For a task: worktree add task/<slug>
"${RDM_BIN:-rdm}" phase update <phase> --roadmap <roadmap> --status in-progress --no-edit {proj_flag}
```

Inspect the returned path, branch, existing changes, and git worktree list before editing. An existing empty phase worktree is not proof that implementation is absent: inspect the shared roadmap worktree and its branch. Do not overwrite or move unrelated work. Use explicit working directories on subsequent calls, and rebuild/rebind the development binary in the selected checkout when applicable.

Implement only the selected scope, using repository testing and development policies. Run focused tests and the appropriate broader checks; report failures without claiming completion. Rebuild rdm after changing its source before using it. Record unrelated findings separately only if authorized; do not silently expand the implementation.

After verification, commit only your source changes with a conventional message. Do not add `Done:` or merge to the default branch. Record the source SHA and test evidence in the item's full preserved body, including a `Key code` section with pinned `rdm:src/<path>@<source-sha>` links. Run link checks before committing plan changes.

```bash
"${RDM_BIN:-rdm}" phase update <phase> --roadmap <roadmap> --status needs-review --no-edit {proj_flag}
"${RDM_BIN:-rdm}" status
"${RDM_BIN:-rdm}" commit -m "chore(plan): hand implementation to independent review"
```

Run that status update from the implementation worktree after confirming its HEAD and branch: rdm automatically stamps internal `review_sha` and `review_branch` metadata from the current checkout. Read the status back and run `"${RDM_BIN:-rdm}" review pending --format json {proj_flag}` from the same checkout; confirm the item appears with the intended `branch`. The ordinary item JSON does not expose the internal review SHA, so retain the explicit source SHA and pinned links in the handoff. Use `task update <slug>` for tasks. `--commit` is the terminal completion commit, not the review revision. Plan commits and source commits are different repositories; do not confuse their SHAs. `status` and `commit` take no project flag. Never use `--all` to compensate for a lost session.

Handoff must name the item, worktree, branch, source SHA, diff base, checks, and any limitations. Implementation ends at `needs-review`. An independent human or working code-review host must review the actual implementation diff before advancing to `reviewed`; self-checks and a missing review runtime are not approval. Landing is a separate, explicitly authorized operation.

{principles}
