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

Implement a roadmap phase or work on a task. This skill is a **shim**: it picks the target, then
hands the whole run to the **`rdm-dispatch-phase`** orchestrator, which is the single per-phase
procedure for both modes — plan → plan review → implement → verify → code review → triage → the
gated terminal write, with the plan and every review persisted in the plan repo as the record of why
the run ended where it did.

The only difference between the two modes is **who submits the approve review on the plan** — a
human or the plan-review workflow — and whether each triage decision pauses for confirmation. The
orchestrator waits for that approval record either way, so the commands and the resulting records are
identical.

## Run modes

`$ARGUMENTS` may include `--auto`:

- **interactive** (default) → the orchestrator is entered with `--interactive`: it prints the
  `rdm review start --on plan/<slug>` … `submit --verdict approve` commands and waits for a
  human-submitted approve review on the plan, and it presents each triage decision for confirmation
  before recording it.
- **`--auto`** → the orchestrator is entered without `--interactive`: it invokes the plan-review
  workflow itself and applies its triage decisions without pausing.

For unattended runs (no human present to approve permission prompts), launch Claude Code with
`--permission-mode auto` (or `bypassPermissions` in a sandbox) so the orchestrator's Bash commands
and Workflow calls don't block on a prompt.

## Argument forms

- `<roadmap-slug> [phase-number]` → **phase flow**.
- `--task <slug>` → **task flow**.
- empty → **discovery**: list what is in flight and ask the user which to work on.
  ```bash
  rdm search "" --status in-progress --type phase --project rdm
  rdm task list --project rdm
  ```

`--plan-only`, `--max-plan-revise N`, `--max-code-rework N`, `--rdm-bin <path>` and
`--project <name>` are passed straight through — the orchestrator resolves the last two into its own
`rdmBin` and `<proj-flag>` placeholders.

## What to do

1. **Parse `$ARGUMENTS`** and pick the flow (phase, task, or discovery — see above). In discovery,
   ask the user which item to work on before going any further.
2. **Show the target** so the user can see what is about to run:
   - phase: if no phase was given, `rdm phase list --roadmap <slug> --project rdm` and take the first
     `not-started` or `in-progress` phase; then `rdm phase show <phase> --roadmap <slug> --project rdm`.
   - task: `rdm task show <slug> --project rdm`.
3. **Enter the orchestrator in this same session** — with `Skill`, never with `Agent`:

   ```
   Skill({ skill: 'rdm-dispatch-phase',
           args: '<roadmap-slug> <phase>' + (auto ? '' : ' --interactive') })
   ```

   (task form: `args: '--task <slug>'` + the same `--interactive` suffix.) It MUST be `Skill`: an
   `Agent`-spawned subagent has no `Workflow` tool at all, so the orchestrator's two review-engine
   calls can only be made by a session that already holds it
   (`docs/workflow-schemas.md` § "Orchestrator / Workflow-reachability spike"). The orchestrator owns
   everything from there: the worktree, the in-progress stamp, the plan, the plan review and its
   approval wait, the implementer, verification, the persisted `change/<sha>` review, per-comment
   triage with reasoned replies, and the gated `--status reviewed` write.

   The orchestrator runs its code review by invoking the **`rdm-wf-review-refute-fix` Workflow**
   (`.claude/workflows/rdm-wf-review-refute-fix.js`) — the same canonical find → refute → filter
   pipeline `rdm-review` runs — so every finalize is actively reviewed in either mode.
4. **Return the orchestrator's OUTCOME verbatim** as your final message, including its `planId` and
   `reviewIds`. Do not paraphrase it, re-derive a status from it, or add a `Done:` line: the
   orchestrator already performed the gated status write; `writesCompletion: true` means `rdm-land`
   will mark the item `done` directly at land time.

**No completion trailer is written in this flow.** The reviewed work is left on the
`roadmap/<slug>` branch for `rdm-land`, which marks the item `done` directly, with the landed
tip's commit, after fast-forwarding onto `main`.

**Single-item scope.** This entry point runs one item and returns its OUTCOME — it does not loop over
a roadmap. Use `rdm-autopilot` for that, or re-run this skill by hand to take another pass on a
`rework`/`escalated` outcome.

## Side-work

If a plan-review finding cannot be resolved by revising the plan — a genuine ambiguity, or an
architectural decision with no clear default — the orchestrator escalates it rather than dropping
it. If you are the one recording it, file it via the Side-work convention below with `--no-plan-review`:
`rdm task create <slug> --title "..." --tags plan-review --no-plan-review`. That flag keeps the
filed finding from being stamped `needs-plan-review` itself.

If you discover bugs or unrelated improvements while working, do not fix them inline — create a
tagged task instead so the work is findable later:

```bash
rdm task create <slug> --title "Description" --body "Details." --tags <tag1>,<tag2> --no-edit --project rdm
rdm commit -m "chore(plan): file side-work task <slug>"  # land the batch
```

Use lowercase kebab-case tags and prefer ones already present in the project (check with
`rdm search "" --tag <candidate> --project rdm`). When you are inside a roadmap's worktree and the
task body cites a file or behavior that only exists on that unlanded branch, tag it
`depends-unlanded` and say so in the body.
