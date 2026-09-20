---
name: rdm-do
description: Implement an rdm roadmap phase or work on an rdm task — plan, execute, then finalize through the canonical code review
allowed-tools:
  - Read
  - Bash
  - Glob
  - Grep
  - Skill
  - EnterPlanMode
  - ExitPlanMode
---

Implement a roadmap phase or work on a task. This skill is a **shim**: it picks the target, then hands
the whole run to the **`rdm-dispatch-phase`** orchestrator, which is the single per-phase procedure for
both modes — plan → plan review → implement → verify → code review → triage → the gated terminal
write, with the plan and every review persisted in the plan repo as the record of why the run ended
where it did.
{principles}
The only difference between the two modes is **who submits the approve review on the plan** and
whether each triage decision pauses for confirmation. The orchestrator waits for that approval record
either way, so the commands and the resulting records are identical.

## Run modes

`$ARGUMENTS` may include `--auto`:

- **interactive** (default) → the orchestrator is entered with `--interactive`: it prints the
  `rdm review start --on plan/<slug>` … `rdm review submit <id> --verdict approve` commands, waits for
  that approve review on the plan, and presents each triage decision for confirmation before
  recording it.
- **`--auto`** → the orchestrator is entered without `--interactive`: it applies its triage decisions
  without pausing.

For unattended runs (no human present to approve permission prompts), launch with
`--permission-mode auto` (or `bypassPermissions` in a sandbox) so the orchestrator's Bash commands and
its Workflow call don't block on a prompt.

## Argument forms

- `<roadmap-slug> [phase-number]` → **phase flow**.
- `--task <slug>` → **task flow**.
- empty → **discovery**: list what is in flight and ask the user which to work on.
  ```bash
  rdm search "" --status in-progress --type phase {proj_flag}
  rdm task list {proj_flag}
  ```

`--plan-only`, `--max-plan-revise N` and `--max-code-rework N` are passed straight through, as are the
`rdmBin` executable and the project name used in `{proj_flag}`.

## What to do

1. **Parse `$ARGUMENTS`** and pick the flow (phase, task, or discovery — see above). In discovery, ask
   the user which item to work on before going any further.
2. **Show the target** so the user can see what is about to run:
   - phase: if no phase was given, `rdm phase list --roadmap <slug> {proj_flag}` and take the first
     `not-started` or `in-progress` phase; then `rdm phase show <phase> --roadmap <slug> {proj_flag}`.
   - task: `rdm task show <slug> {proj_flag}`.
3. **Enter the orchestrator in this same session** — with the `Skill` tool, never with `Agent`:

   ```
   Skill({ skill: 'rdm-dispatch-phase',
           args: '<roadmap-slug> <phase>' + (auto ? '' : ' --interactive') })
   ```

   (task form: `args: '--task <slug>'` plus the same `--interactive` suffix.) It MUST be `Skill`: an
   `Agent`-spawned subagent has no `Workflow` tool at all, so the orchestrator's code-review call
   could not be made there. The orchestrator owns everything from that point: the worktree, the
   in-progress stamp, the plan, the plan approval wait, the implementer, verification, the persisted
   `change/<sha>` review, per-comment triage with reasoned replies, and the gated `--status reviewed`
   write.

   The orchestrator runs its code review by invoking the **`rdm-wf-review-refute-fix` Workflow**
   (`.claude/workflows/rdm-wf-review-refute-fix.js`, provisioned automatically by `rdm agent-config claude --skills`)
   — the same canonical find → refute → filter pipeline `rdm-review` runs — so every finalize is
   actively reviewed in either mode.
4. **Return the orchestrator's OUTCOME verbatim** as your final message, including its `planId` and
   `reviewIds`. Do not paraphrase it, re-derive a status from it, or add a `Done:` line: the
   orchestrator already performed the gated status write, and `rdm-land` is the only writer of the
   completion trailer, at land time, off `writesCompletion: true`.

**Never hand-type the completion trailer.** Its format has exactly one home, `rdm hook done-line`. The
reviewed work is left on the `roadmap/<slug>` branch for `rdm-land`; the merge hook flips `reviewed` →
`done`.

**Single-item scope.** This entry point runs one item and returns its OUTCOME — it does not loop over a
roadmap. Use `rdm-autopilot` for that, or re-run this skill by hand to take another pass on a
`rework`/`escalated` outcome.

## Side-work

If a plan-review finding cannot be resolved by revising the plan — a genuine ambiguity, or an
architectural decision with no clear default — the orchestrator escalates it rather than dropping
it. If you are the one recording it, file it via the Side-work convention below with `--no-plan-review`:
`rdm task create <slug> --title "..." --tags plan-review --no-plan-review`. That flag keeps the
filed finding from being stamped `needs-plan-review` itself.

If you discover bugs or unrelated improvements while working, do not fix them inline — create a tagged
task instead so the work is findable later:

```bash
rdm task create <slug> --title "Description" --body "Details." --tags <tag1>,<tag2> --no-edit {proj_flag}
rdm commit -m "chore(plan): file side-work task <slug>"  # land the batch
```

Use lowercase kebab-case tags and prefer ones already present in the project (check with
`rdm search "" --tag <candidate> {proj_flag}`). When you are inside a roadmap's worktree and the task
body cites a file or behavior that only exists on that unlanded branch, say so in the body.
