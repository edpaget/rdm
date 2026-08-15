---
name: rdm-dispatch-phase
description: Dispatch a single rdm phase end-to-end in the roadmap's shared worktree on its assigned model tier — plan, independently review the plan, implement, then code-review — and return a structured outcome
allowed-tools:
  - Workflow
  - {t_phase_show}
  - {t_phase_update}
  - {t_task_update}
---

Run **one** rdm phase (or task) to completion by invoking the **`rdm-wf-dispatch-phase` Workflow** (`.claude/workflows/rdm-wf-dispatch-phase.js`, provisioned automatically by `rdm agent-config claude --skills`). This skill is a **thin shim**: it parses the invocation, hands off to the workflow, and returns the OUTCOME JSON the workflow returns verbatim. All the per-phase work — planning, the independent plan gate, implementation, and code review — happens inside the workflow's own deterministic 4-stage pipeline (Plan → PlanReview → Implement → CodeReview), not in this prose.
{principles}
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

1. **Parse `$ARGUMENTS`** as `<roadmap-slug> <phase>` (stem or number) for phase mode, or `--task <slug>` for task mode. Optionally sanity-check that the phase exists first with `{t_phase_show}` (`project: {proj_param}, roadmap: "<slug>", phase: "<phase>"`), so a typo'd phase ref fails fast before the workflow spends any tokens.
2. **Stamp the item in-progress yourself** unless this is a `--plan-only` invocation: `{t_phase_update}` (`project: {proj_param}, roadmap: "<slug>", phase: "<phase>", status: "in-progress"`) or `{t_task_update}` (`project: {proj_param}, slug: "<slug>", status: "in-progress"`). Pass `alreadyInProgress: true` in the workflow args **only** if that call succeeded; never pass it for a `--plan-only` run. This makes the item observably `in-progress` strictly earlier than the workflow's own stamp would, and lets the workflow skip a dedicated subagent for it. The flag is **optional** — omit it and the workflow stamps the item itself, exactly as before.

   The CLI variant of this shim additionally hoists the phase/task body plus the five resolved per-step model ids as `phaseMeta`/`taskMeta`. That is **deliberately not done here**: the workflow applies an all-or-nothing guard requiring all five model ids, there is no MCP model-resolve tool, and a partial payload is rejected outright — so this shim omits `phaseMeta`/`taskMeta` entirely and the in-workflow fetch remains the path on MCP.
3. **Invoke the `rdm-wf-dispatch-phase` workflow** via the Workflow tool with `{ roadmap, phase, alreadyInProgress, rdmBin, project: {proj_param} }` (phase mode) or `{ task, alreadyInProgress, rdmBin, project: {proj_param} }` (task mode); pass `args` as a JSON object, never a stringified value. Block for its returned OUTCOME.
   - `rdmBin` — the rdm executable the workflow's own Bash agents invoke (e.g. `rdm` on PATH, or a repo-local build path). Still meaningful on MCP: this shim makes no CLI calls of its own, but the workflow it invokes **does** shell out. **Optional**: omit it and a plain `rdm` on `PATH` is used. An explicitly passed value always wins verbatim, and the literal `"rdm"` requests `PATH` resolution deliberately. Never probe the filesystem to pick a binary. If a "Resolving `rdmBin`" section is appended to this skill, it is the single authoritative resolution order — follow it and do not re-derive one here.
   - `project` — **optional**; appended only to project-scoped commands (`rdm model resolve` never receives it).

   The workflow:
   - reads the phase/task and its model tier, creates or reuses the roadmap's (or task's) shared worktree, and stamps it `in-progress` unless you already did;
   - runs a **planning** stage, then a **separate, independent plan-review** stage — scaled to the phase's difficulty tier — bounded to at most one revise round before escalating;
   - on approval, runs an **implementation** stage inside the worktree;
   - runs the **canonical code review** (the same find → refute → filter → verdict → gate pipeline `rdm-review` runs), bounded to one rework pass before escalating;
   - classifies the result into `reviewed | rework | escalated` and returns the OUTCOME above. It never emits a `Done:` line itself — only `writesCompletion: true`, which is `rdm-land`'s signal to synthesize the trailer at land time.
4. **Return the OUTCOME JSON verbatim** as your final message — do not paraphrase or drop fields.

## Recovering a crashed dispatch

If the `rdm-wf-dispatch-phase` Workflow call in step 3 (`.claude/workflows/rdm-wf-dispatch-phase.js`) crashes mid-run, relaunch the same script with an added `resumeFromRunId: '<prior runId>'` argument instead of invoking it fresh. Any `agent()` call inside that run whose `(prompt, opts)` are byte-unchanged from the crashed attempt replays its cached result instead of re-dispatching. Four caveats apply every time:

- **Stop the prior run first** — a still-running run cannot be resumed.
- **Same-session only** — this only resumes within the current Claude Code session; a later session cannot resume a `runId` from an earlier one.
- **A cached result can be empty** — if the crashed agent produced nothing before dying, the resume replays that emptiness; check the run's `journal.jsonl` before assuming there is something to recover.
- **Conservative prefix** — resume replays only the longest unchanged prefix of the call sequence; the first edited-or-new call, and every call after it, run live. Do not plan around a specific savings figure.

## Safe operations under --permission-mode auto

Unattended autonomous runs (launched with `--permission-mode auto` or equivalent) depend on explicit operational guardrails. The auto-mode permission classifier treats certain operations as **irreversible local destruction** and denies them, blocking a hands-off run indefinitely unless human intervention occurs — defeating the point of unattended execution. This skill delegates all editing to the Workflow's internally-dispatched implementer subagents (this shim itself never edits a file directly), and those subagents must follow these rules:

- **Modify existing files with `Edit`, never `Write`** — a `Write` operation that overwrites an existing tracked file triggers the auto-mode destructive-action classifier and stalls the run. Use the `Edit` tool for surgical changes to files already in the repo, even if you are replacing large sections. Reserve `Write` only for creating new files.
- **Never run `git stash -u`, `git reset --hard`, or `git clean -fdx`** — these are classified as irreversible local destruction and are denied under `--permission-mode auto`. The per-roadmap worktree isolation means destructive whole-tree resets are unnecessary. If work must be set aside, commit a WIP commit (e.g. `git commit -m "wip: <description>"`) on the phase branch instead of stashing untracked files. The worktree can be pruned or reset later when the phase is done or parked.

## Escalation protocol

This skill follows the shared **escalation protocol** (`docs/escalation-protocol.md`) — the single definition the workflow and the autonomous loop both apply. In short:

- **Routine findings never escalate.** Bugs, missing tests, doc gaps — the canonical review fixes them inline or files a task. They never reach the user.
- **Decisions/blockers escalate.** Ambiguous/untestable AC, an architectural decision with no clear default, an exhausted plan-revise or rework budget, or a hard blocker (missing dependency/credential, conflicting requirement).
- **Park, don't interrupt.** The workflow records the escalation by setting the phase/task `blocked` with a stage-tagged reason (`[plan]` or `[code]`) carried in the OUTCOME's `status`/`reason` fields. The user reviews the whole queue at once with the `rdm review blocked` command rather than being interrupted mid-run.
