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

The skill is emitted by `rdm agent-config --skills` in both CLI and MCP
variants, alongside the other six skills:

```bash
rdm agent-config claude --skills --project <proj> --out .
rdm agent-config pi     --skills --project <proj> --out .
```

It is invoked with a **required roadmap slug** (from `$ARGUMENTS`), optionally
followed by flags. The loop never roams to another roadmap — choosing *which*
roadmap to advance stays a human decision.

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
│  difficulty unset? ──► rdm-estimate <slug> <phase>           │
│                                                              │
│  rdm-dispatch-phase <slug> <phase>                           │
│      outcome = reviewed  ──► advance (next steps past it)    │
│      outcome = rework    ──► retry; on budget exhaustion     │
│                              park blocked [code], continue   │
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

`rdm-dispatch-phase` returns one of three outcomes:

| Outcome | Phase state | Autopilot action |
|---------|-------------|------------------|
| `reviewed` | `reviewed`, `Done:` line on the branch | advance |
| `rework` | back to `in-progress` (fixable defect) | re-dispatch within the rework-retry budget; on exhaustion park `blocked` with a `[code]` reason and continue |
| `escalated` | already `blocked` (`[plan]`/`[code]`) | leave it; continue with the remaining actionable phases |

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
  invoke `rdm-land` (the landing skill, phase 7) to land the work. Without it,
  autopilot **never touches `main`**; it leaves every reviewed phase on the
  `roadmap/<slug>` branch for a human to land.

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

## Relation to the passive needs-review safety net

Autopilot is the **active driver** — it pushes a roadmap forward phase by phase.
The needs-review **Stop hook** (Claude Code) and the Pi **`agent_end`
extension** are the **passive safety net**: they only re-prompt when an item is
*left* in `needs-review`, catching a finalize that was never reviewed. The two
are complementary — the driver does the work, the net catches anything dropped
on the floor. Both leave the `Done:` line to `rdm-review`; autopilot writes one
only via the dispatched `rdm-review`, never by hand.

See also [`docs/escalation-protocol.md`](./escalation-protocol.md) for the
shared rule on what escalates, what retries, and how parked escalations are
recorded and resumed.
