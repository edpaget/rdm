# Autonomous roadmap loop (`rdm-autopilot`)

`rdm-autopilot` is the capstone of rdm's autonomous-execution skills: it drives
**one named roadmap** from `not-started` to `reviewed` with no per-phase human
approval. It is the *active driver* — it composes the per-phase skills built in
the earlier phases rather than re-implementing them:

- [`rdm next`](#the-loop-driver) — the deterministic selector that picks the
  next actionable phase and doubles as the termination oracle.
- `rdm-estimate` — rates a phase's difficulty and derives its model tier.
- `rdm-dispatch-phase` — runs one phase end-to-end (plan → independent plan
  gate → implement → `rdm-review`) and returns a structured outcome.

`rdm-estimate` and `rdm-dispatch-phase` are invoked **behind an `Agent`
subagent boundary**, not inline: each phase is dispatched as its own subagent
that runs estimation (if needed) and dispatch internally and returns only the
structured outcome. The loop never runs those skills directly with the `Skill`
tool — see [Context isolation](#context-isolation) for why.

The skill is emitted by `rdm agent-config --skills` in both CLI and MCP
variants, alongside the other generated skills:

```bash
rdm agent-config claude --skills --project <proj> --out .
rdm agent-config pi     --skills --project <proj> --out .
```

It is invoked with a **required roadmap slug** (from `$ARGUMENTS`), optionally
followed by flags. The loop never roams to another roadmap — choosing *which*
roadmap to advance stays a human decision.

## Two implementations: the prose skill and the workflow lane

This document describes the **prose** `rdm-autopilot` lane, where the loop is an
LLM following these instructions and dispatching each phase behind an `Agent`
subagent boundary. rdm's own dogfood repo also runs a **workflow** lane —
`.claude/workflows/autopilot.js` (see [`docs/workflow-schemas.md`](./workflow-schemas.md)) —
where the loop, budgets, and stop conditions are real JS control flow calling the
`dispatch-phase` workflow via `workflow()`. The two share this document's model;
the workflow lane makes three mechanics explicit and worth calling out:

- **It drives off *persisted* status.** `dispatch-phase` persists no *terminal*
  phase status — it does stamp the phase (or task) `in-progress`, best-effort,
  right after Stage 0 and before it starts working the item (a `--plan-only`
  run skips that stamp, since it never implements) — and `rdm next` returns
  only `not-started`/`in-progress` phases, so the workflow writes the terminal
  status itself: `reviewed` → `rdm phase update --status reviewed` (advance),
  rework-exhausted or `escalated` → `--status blocked --reason "[code|plan] …"`
  (park). `rdm next` reading that status back is what steps the loop forward
  and is the **normal-mode termination oracle** (it eventually returns
  `nothing`). There is no in-memory `seen` Set in normal mode.
- **`--plan-only` uses a re-return guard.** Because a plan-only dispatch never
  advances status, `rdm next` keeps returning the same phase; a `planOnlySeen`
  Set stops the run when a vetted phase comes back, rather than re-vetting it
  forever.
- **The `Done:` line stays with `rdm-review` / landing.** The workflow's advance
  step writes only `--status reviewed`; it never emits a `Done:` line, lands, or
  touches `main` — exactly as the prose lane leaves that to `rdm-review` and
  `rdm-land`.

## End-to-end flow

```
pick roadmap (human)
      │
      ▼
┌─────────────────────────────────────────────────────────────┐
│  rdm next --roadmap <slug> --format json                     │
│      result = phase ────────────────┐                        │
│      result = nothing ──────────────┼──► stop → summary      │
│      result = blocked-on-deps ──────┘                        │
│                                                              │
│  ┌── Agent subagent (isolated context) ──────────────────┐  │
│  │  difficulty unset? ──► rdm-estimate <slug> <phase>     │  │
│  │  rdm-dispatch-phase <slug> <phase>                     │  │
│  └── returns {roadmap, phase, outcome, summary, findings} ┘  │
│                                                              │
│      outcome = reviewed  ──► advance (next steps past it)    │
│      outcome = rework    ──► retry (new subagent); on budget │
│                              exhaustion park blocked [code]  │
│      outcome = escalated ──► already blocked, continue       │
└─────────────────────────────────────────────────────────────┘
      │ (repeat until a stop condition)
      ▼
   summary  → optionally rdm-land (--land)
```

### The loop driver

Each iteration starts by asking rdm for the next actionable phase:

```bash
rdm next --roadmap <slug> --format json --project <proj>
```

The result discriminates on `result`:

- `phase` — work this phase (the JSON carries `stem`, `number`, `status`, and
  `difficulty`/`model` when assessed).
- `nothing` — no actionable phase remains; stop.
- `blocked-on-dependencies` — the roadmap depends on other roadmaps that are not
  yet complete (`unmet` lists them); stop.

`rdm next` is also the **termination oracle**. It returns the lowest-numbered
`not-started`/`in-progress` phase and **skips** `needs-review`, `reviewed`,
`done`, `blocked`, and `wont-fix`. So a reviewed phase is automatically stepped
over, and a *parked* (`blocked`) phase is stepped over too — which is exactly
how the loop makes progress past work it could not finish. A phase left
`in-progress` (a rework) is returned **again**, so each phase carries a retry
budget and, on exhaustion, is parked `blocked` so the selector moves past it.

### Interpreting a dispatch outcome

Each phase is dispatched inside its own `Agent` subagent (estimation included),
and every rework retry is a **fresh** subagent call — so all the loop ever sees
per phase is the structured outcome the subagent returns, one of three:

| Outcome | Phase state | Autopilot action |
|---------|-------------|------------------|
| `reviewed` | `reviewed`, `Done:` line on the branch | advance |
| `rework` | back to `in-progress` (fixable defect) | re-dispatch (new subagent) within the rework-retry budget; on exhaustion park `blocked` with a `[code]` reason and continue |
| `escalated` | already `blocked` (`[plan]`/`[code]`) | leave it; continue with the remaining actionable phases |

### Context isolation

The loop invokes `rdm-estimate` and `rdm-dispatch-phase` through an `Agent`
subagent boundary rather than inline via the `Skill` tool. The `Skill` tool runs
in the loop's own conversation, so running those skills directly would
accumulate every phase's estimate, plan, plan-review, implementation, and
code-review detail in the loop context — growing it roughly quadratically over a
multi-phase run and diluting attention on later phases, even though the loop only
needs each phase's structured outcome to decide the next step. Dispatching each
phase as its own subagent keeps that detail inside the subagent; only the
`{roadmap, phase, outcome, summary, findings}` JSON crosses back. The loop's
retained state per iteration is therefore bounded: the latest `rdm next` result
and each returned outcome, nothing more. This is the same isolation
`rdm-dispatch-phase` applies one level down, where its planner, plan reviewer,
and implementer are separate subagents — the planner and reviewer seeded with
only the phase body, the implementer with the phase body and the approved plan
document.

## Budgets and stop conditions

The per-phase **rework-retry budget** and what counts as an escalation are
defined once in [`docs/escalation-protocol.md`](./escalation-protocol.md) — the
single shared source the dispatch flow and this loop both apply. Autopilot does
not redefine them; it adds two run-level bounds on top:

- **Global step budget** — a cap on total phase dispatches per run, so a
  pathological roadmap can never loop forever even if every phase keeps
  reworking.
- `--max-phases N` — a user-supplied bound applied the same way.

The loop stops when **any** of these holds:

- `rdm next` returns `nothing` (the roadmap is fully `reviewed`/`done`, or
  everything remaining is parked/terminal).
- `rdm next` returns `blocked-on-dependencies`.
- All remaining work is `blocked`/escalated.
- `--max-phases` or the global step budget is reached.

## Run modes

- `--max-phases N` — bounded run: dispatch at most `N` phases, then stop.
- `--plan-only` — dry-run the planning half: run each phase only through
  `rdm-dispatch-phase`'s plan gate and stop before implementation. Cheap plan
  vetting across a roadmap without writing any code.
- `--land` (opt-in, **default OFF**) — after the roadmap reaches `reviewed`,
  invoke the [`rdm-land`](./landing.md) skill to land the work to `main` with
  linear history. Without it, autopilot **never touches `main`**; it leaves every
  reviewed phase on the `roadmap/<slug>` branch for a human to land.

Because the run is unattended, launch it with `--permission-mode auto` (or
`bypassPermissions` in a sandbox) so worktree edits, commands, and dispatched
subagents don't block on permission prompts.

## Summary and the batch queue

Every run — whatever stopped it — ends with a summary: the phases completed
this run, the tasks filed by the dispatched runs, and the escalations awaiting
the user, each tagged `plan` vs `code`. Batching escalations is the whole point
of autopilot: instead of interrupting the user per phase, it parks decisions and
blockers and surfaces them together at the end. Review the queue with:

```bash
rdm review blocked --project <proj>
rdm review blocked --project <proj> --format json
```

## Relation to the retired needs-review safety net

Autopilot is the **active driver** — it pushes a roadmap forward phase by phase.
Every lane that can produce a `needs-review` item now actively runs the
canonical review (`.claude/workflows/lib/review.mjs`) before that lane's
finalize step returns: `dispatch-phase`'s code-review stage runs it inline and
persists `reviewed`/`blocked` directly, autopilot's advance step relies on that
same dispatch-phase pipeline, and interactive `rdm-do`'s finalize invokes
`rdm-review` (the generated projection of the same canonical source) after the
human confirm gate. With nothing left unreviewed, the once-passive needs-review
Stop hook (Claude Code) and Pi `agent_end` extension — which only re-prompted
when an item was *left* in `needs-review` — have been retired as redundant; see
[`CLAUDE.md`](../CLAUDE.md)'s "Hook reconciliation" note for the harness
evidence. The workflow lane never emits a `Done:` line — `dispatch-phase`'s
review is an inline pipeline (not the `rdm-review` skill), and autopilot's
advance step writes only `--status reviewed`. The `Done:` line is supplied later
by `rdm-review` or at landing.

See also [`docs/escalation-protocol.md`](./escalation-protocol.md) for the
shared rule on what escalates, what retries, and how parked escalations are
recorded and resumed, and [`docs/landing.md`](./landing.md) for the landing tail
(`rdm-land` + `rdm worktree prune`) that integrates reviewed work into `main`.
