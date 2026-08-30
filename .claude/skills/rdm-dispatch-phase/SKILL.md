---
name: rdm-dispatch-phase
description: Dispatch a single rdm phase end-to-end in the roadmap's shared worktree on its assigned model tier — plan, independently review the plan, implement, then code-review — and return a structured outcome
allowed-tools:
  - Bash
  - Workflow
---

Run **one** rdm phase (or task) to completion by invoking the **`rdm-wf-dispatch-phase` Workflow** (`.claude/workflows/rdm-wf-dispatch-phase.js`, provisioned automatically by `rdm agent-config claude --skills`). This skill is a **thin shim**: it parses the invocation, hands off to the workflow, and returns the OUTCOME JSON the workflow returns verbatim. All the per-phase work — planning, the independent plan gate, implementation, and code review — happens inside the workflow's own deterministic 4-stage pipeline (Plan → PlanReview → Implement → CodeReview), not in this prose.

## Dispatch contract

**Input** (`$ARGUMENTS`): `<roadmap-slug> <phase>` (stem or number) for phase mode, or `--task <slug>` for task mode. Task mode dispatches a standalone task instead of a phase — no roadmap, no model tier assessed, its own `task/<slug>` worktree.

**Output** — the OUTCOME JSON the workflow returns, printed verbatim as this skill's final result:

```json
{
  "roadmap": "<slug>",
  "phase": "<stem>",
  "outcome": "reviewed | rework | escalated",
  "status": "<the rdm status the canonical review mapped this outcome to>",
  "writesCompletion": "<true only when outcome is reviewed>",
  "summary": "<one-line result>",
  "reason": "<the [plan]/[code]-tagged reason, when parked>",
  "findings": "<plan-review and code-review notes, or the blocker that forced escalation>"
}
```

(Task mode carries a `task` field in place of `roadmap`/`phase`.)

- **reviewed** — code review passed; `status` carries the phase's/task's reviewed status and `writesCompletion` is `true` — `rdm-land` reads that flag and synthesizes the `Done:` trailer at land time.
- **rework** — code review failed after the bounded in-run retry; `status` reflects the item going back to `in-progress`.
- **escalated** — a genuine ambiguity in the AC, or an architectural/design decision with no clear default, blocked progress; `status` is `blocked` and `reason` carries the `[plan]`- or `[code]`-tagged blocker.

## What to do

1. **Parse `$ARGUMENTS`** as `<roadmap-slug> <phase>` (stem or number) for phase mode, or `--task <slug>` for task mode.
2. **Gather the mechanical values yourself and hand them to the workflow.** You are already a running agent with the repo in context; the workflow is not, so anything it has to look up costs it a whole dedicated subagent. Run these yourself and pass the results in the workflow `args` — every one of them is **optional**, and the workflow falls back to its own in-workflow fetch for anything you omit or get wrong, so a partial gather is safe:
   - `./target/debug/rdm phase show <phase> --roadmap <slug> --project rdm --format json` (phase mode) or `./target/debug/rdm task show <slug> --project rdm --format json` (task mode) — parse the JSON.
   - Phase mode only: `./target/debug/rdm roadmap show <slug> --project rdm --format json` — keep its `body` field verbatim as `roadmapBody`.
   - The five per-step model ids. Let `T` be the phase JSON's `model` tier (phase mode only; a task carries no tier). Run `./target/debug/rdm model resolve plan --tier T` and `./target/debug/rdm model resolve implement --tier T` **with** the tier hint when `T` is a non-empty string, without it otherwise; then run `./target/debug/rdm model resolve review-find`, `./target/debug/rdm model resolve review-verify` and `./target/debug/rdm model resolve mechanical` with **no** `--tier` argument, always.
   - Both modes: `./target/debug/rdm config get dispatch.verify --raw` — `--raw` prints the bare value with no `(source: ...)` annotation, and nothing at all when the key is unset. If it prints a non-empty line, keep that line verbatim as `verify`. If it prints nothing or fails, **abandon the hoist entirely** and invoke the Workflow with no `phaseMeta`/`taskMeta` key: only the workflow's own Stage-0 agent knows how to discover a verification command from CI config, `docs/principles.md`, or `CLAUDE.md`/`AGENTS.md`.
   - Assemble `phaseMeta` (phase mode) as `{ roadmap, phase, stem, model, body, roadmapBody, verify, models: { plan, implement, review_find, review_verify, mechanical } }`, or `taskMeta` (task mode) as `{ task, body, verify, models: { … } }`. Copy the `body`, the `roadmapBody` and the resolved model ids **verbatim** — never summarize, paraphrase, or invent one. `roadmapBody` (phase mode only) is the ONE optional field: it carries the parent roadmap's `## Intent` section, which is the plan gate's only intent source on this path, so omitting it silently disables the `intent-alignment` dimension for this dispatch — but a failed roadmap read just drops that one key, it never abandons the hoist. The workflow applies an all-or-nothing guard: a payload missing the body, any one of the five model ids, or — in phase mode — the `model` difficulty tier is rejected outright and the in-workflow fetch runs instead, so a partial payload buys nothing. The tier matters as much as the ids: it is the only source the workflow has for how strictly the code-review gate treats findings, and an absent one silently defaults to `medium`.
   - Unless this is a `--plan-only` invocation, stamp the item in-progress yourself, **before** invoking the workflow: `./target/debug/rdm phase update <phase> --status in-progress --no-edit --roadmap <slug> --project rdm` (or `./target/debug/rdm task update <slug> --status in-progress --no-edit --project rdm`). Pass `alreadyInProgress: true` **only** if that command exited 0. This makes the item observably `in-progress` strictly earlier than the workflow's own stamp would, and lets the workflow skip a subagent. Never pass `alreadyInProgress` for a `--plan-only` run.
3. **Invoke the `rdm-wf-dispatch-phase` workflow** via the Workflow tool with `{ roadmap, phase, phaseMeta, alreadyInProgress, rdmBin: "./target/debug/rdm", project: "rdm" }` (phase mode) or `{ task, taskMeta, alreadyInProgress, rdmBin: "./target/debug/rdm", project: "rdm" }` (task mode); pass `args` as a JSON object, never a stringified value. Block for its returned OUTCOME.
   - `rdmBin` is **optional** — the rdm executable every command the workflow emits will invoke, defaulting to a plain `rdm` on `PATH` when omitted; an explicitly passed value always wins verbatim. In this repo always pass `./target/debug/rdm` (the development-build rule), which `.mise.toml` also exports as `RDM_BIN`; never rely on the bare-`rdm` default here. See `docs/workflow-schemas.md` § "Environment args: `rdmBin` and `project`" for the canonical resolution order.
   - `project` is **optional** — the project name appended as ` --project <name>` to project-scoped commands only (`rdm model resolve` never receives it). Omit it to let rdm's own `RDM_PROJECT`/`default_project` chain apply.

   The workflow:
   - reads the phase/task and its model tier (or uses the `phaseMeta`/`taskMeta` you supplied), creates or reuses the roadmap's (or task's) shared worktree, and stamps it `in-progress` unless you already did;
   - runs a **planning** stage, then a **separate, independent plan-review** stage — scaled to the phase's difficulty tier — bounded to at most one revise round before escalating;
   - on approval, runs an **implementation** stage inside the worktree;
   - runs the **canonical code review** (the same find → refute → filter → verdict → gate pipeline `rdm-review` runs), bounded to one rework pass before escalating;
   - classifies the result into `reviewed | rework | escalated` and returns the OUTCOME above. It never emits a `Done:` line itself — only `writesCompletion: true`, which is `rdm-land`'s signal to synthesize the trailer at land time.
4. **Return the OUTCOME JSON verbatim** as your final message — do not paraphrase or drop fields.

## Recovering a crashed dispatch

If the `rdm-wf-dispatch-phase` Workflow call in step 3 crashes mid-run, relaunch it with `resumeFromRunId` instead of invoking it fresh:

```
Workflow({ scriptPath: '.claude/workflows/rdm-wf-dispatch-phase.js', resumeFromRunId: '<prior runId>' })
```

Any `agent()` call inside that run whose `(prompt, opts)` are byte-unchanged from the crashed attempt replays its cached result instead of re-dispatching. Four caveats apply every time:

- **Stop the prior run first** — a still-running run cannot be resumed.
- **Same-session only** — this only resumes within the current Claude Code session; a later session cannot resume a `runId` from an earlier one.
- **A cached result can be empty** — if the crashed agent produced nothing before dying, the resume replays that emptiness; check the run's `journal.jsonl` before assuming there is something to recover.
- **Conservative prefix** — resume replays only the longest unchanged prefix of the call sequence; the first edited-or-new call, and every call after it, run live. Do not plan around a specific savings figure.

See [`docs/autonomous-loop.md`](docs/autonomous-loop.md) § "Recovering a crashed run" for the full contract and the measured evidence motivating it.

## Safe operations under --permission-mode auto

Unattended autonomous runs (launched with `--permission-mode auto` or equivalent) depend on explicit operational guardrails. The auto-mode permission classifier treats certain operations as **irreversible local destruction** and denies them, blocking a hands-off run indefinitely unless human intervention occurs — defeating the point of unattended execution. This skill delegates all editing to the Workflow's internally-dispatched implementer subagents (this shim itself never edits a file directly), and those subagents must follow these rules:

- **Modify existing files with `Edit`, never `Write`** — a `Write` operation that overwrites an existing tracked file triggers the auto-mode destructive-action classifier and stalls the run. Use the `Edit` tool for surgical changes to files already in the repo, even if you are replacing large sections. Reserve `Write` only for creating new files.
- **Never run `git stash -u`, `git reset --hard`, or `git clean -fdx`** — these are classified as irreversible local destruction and are denied under `--permission-mode auto`. The per-roadmap worktree isolation means destructive whole-tree resets are unnecessary. If work must be set aside, commit a WIP commit (e.g. `git commit -m "wip: <description>"`) on the phase branch instead of stashing untracked files. The worktree can be pruned or reset later when the phase is done or parked.

## Escalation protocol

This skill follows the shared **escalation protocol** (`docs/escalation-protocol.md`) — the single definition the workflow and the autonomous loop both apply. In short:

- **Routine findings never escalate.** Bugs, missing tests, doc gaps — the canonical review fixes them inline or files a task. They never reach the user.
- **Decisions/blockers escalate.** Ambiguous/untestable AC, an architectural decision with no clear default, an exhausted plan-revise or rework budget, or a hard blocker (missing dependency/credential, conflicting requirement).
- **Park, don't interrupt.** The workflow records the escalation by setting the phase/task `blocked` with a stage-tagged reason (`[plan]` or `[code]`) carried in the OUTCOME's `status`/`reason` fields. The user reviews the whole queue at once with `rdm review blocked --project rdm` rather than being interrupted mid-run.
