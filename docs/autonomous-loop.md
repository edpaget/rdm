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

The skill is emitted by `rdm agent-config --skills`, alongside the other
generated skills:

```bash
rdm agent-config claude --skills --project <proj> --out .
rdm agent-config pi     --skills --project <proj> --out .
```

It is invoked with a **required roadmap slug** (from `$ARGUMENTS`), optionally
followed by flags. The loop never roams to another roadmap — choosing *which*
roadmap to advance stays a human decision.

## History: the retired workflow twin

`.claude/workflows/autopilot.js` existed as a parallel Workflow-tool
implementation of this same loop; phase 3 of `prose-autopilot-orchestration`
retired it in favor of this prose skill — see
[`docs/workflow-vs-prose-boundary.md`](./workflow-vs-prose-boundary.md) for why.
The mechanics that twin made explicit still hold for this prose driver:

- **It drives off *persisted* status.** The per-phase unit persists no *terminal*
  phase status — it does stamp the phase (or task) `in-progress`, best-effort,
  before it starts working the item (a `--plan-only`
  run skips that stamp, since it never implements) — and `rdm next` returns
  only `not-started`/`in-progress` phases, so the loop writes the terminal
  status itself: `reviewed` → `rdm phase update --status reviewed` (advance),
  rework-exhausted or `escalated` → `--status blocked --reason "[code|plan] …"`
  (park). `rdm next` reading that status back is what steps the loop forward
  and is the **normal-mode termination oracle** (it eventually returns
  `nothing`). There is no in-memory `seen` Set in normal mode.
- **`--plan-only` uses a re-return guard.** Because a plan-only dispatch never
  advances status, `rdm next` keeps returning the same phase; a `planOnlySeen`
  Set stops the run when a vetted phase comes back, rather than re-vetting it
  forever.
- **The `Done:` line stays with `rdm-review` / landing.** The loop's advance
  step writes only `--status reviewed`; it never emits a `Done:` line, lands, or
  touches `main` — that is left to `rdm-review` and `rdm-land`.

## The per-phase unit: the prose orchestrator

Since `agent-orchestrated-dispatch` phase 6 the per-phase unit is **not** a Workflow. It is
`.claude/skills/rdm-dispatch-phase/SKILL.md`, a prose procedure the driving session loads with
`Skill` and executes in its own turn. `rdm-do` (both modes) is a shim onto it, and
`rdm-autopilot` step 4 enters it the same way. The `rdm-wf-dispatch-phase` engine it replaced
was retired outright by phase 7 of the same roadmap — see
[`docs/workflow-vs-prose-boundary.md`](./workflow-vs-prose-boundary.md) § "Retirement record".

**Why `Skill`, never `Agent`.** An `Agent`-spawned subagent has no `Workflow` tool at all: it is
absent from both its loaded and its deferred tool lists, so the call cannot be formed, and the
same-named Skill route inside a subagent returns only a shim carrying an unsatisfiable
`Invoke: Workflow(...)` directive (`docs/workflow-schemas.md` § "Orchestrator /
Workflow-reachability spike"). The orchestrator makes two Workflow calls, so it can only run in a
session that already holds the tool.

**What is delegated, and what is not.** Only the **planner** and the **implementer** are
dispatched to `Agent` subagents — neither makes a Workflow call, and isolating their contexts is
the whole remaining reason to delegate. Everything else stays in the loaded session: both
Workflow calls, verification, triage, every gate read and every status write. Collapsing the
planner or implementer inline ("inline-collapse") destroys the independent check the lane is
built on; delegating a Workflow call or a status write is simply impossible or unsafe.

**Model sizing.** The worktree/identity step of the sequence below resolves `models.plan` /
`models.implement` from the item's `model` tier (`<rdmBin> model resolve plan --tier <T>` /
`implement --tier <T>`, no `--tier` when the item carries none or is a task) and threads both into
the planner and implementer `Agent` dispatches — the two most expensive judgment sites the sizing
policy (`docs/refuter-model-tiering.md`, `review_floor`, the `rdm-autopilot` estimate pre-pass) exists
to size. The plan-review Workflow call (`rdm-wf-plan-review`) resolves its own
`mechanical`/`review-find`/`review-verify` models internally via its own bootstrap and is
unaffected by this step. The code-review Workflow call (`rdm-wf-review-refute-fix`) does not
self-resolve and is not yet tier-threaded by this step either — it still inherits whatever model
is driving the session; that gap is tracked separately by `task/thread-code-review-judgment-models`,
out of scope here.

**The sequence.**

```
Skill(rdm-dispatch-phase)
  1  resume check: the item's review set (see below), never the global queue
  2  worktree add (idempotent) → review source --on <item>   ← identity pinned ONCE
  3  resolve models.plan / models.implement from the item's tier
  4  phase/task update --status in-progress                  (skipped under --plan-only)
  5  Agent: planner → plan create --implements [--supersedes] + '## Verification command'
  6  Workflow: rdm-wf-plan-review  persist → plan/<slug>      (local copy only; see below)
  7  poll plan show --format json → approved | changes-requested | draft | superseded
  8  Agent: implementer in the pinned worktree → commit
  9  verify run (exit 2 → the plan-recorded command, run in the pinned checkout)
 10  Workflow: rdm-wf-review-refute-fix persist → change/<head>, gate:false
 11  triage every comment: addressed(--applied-commit) | wont-fix, each with a reply
 12  re-review at the post-triage HEAD (a source fix moved it)
 13  phase/task update --status reviewed, source-bound, through the core gate
 14  return OUTCOME + planId + reviewIds
```

**The persisted trail is the archeology.** A Workflow journal lives outside the plan repo and
disappears. What survives is `plan/<slug>`, the review that approved it, the `change/<sha>`
review, and one `addressed`/`wont-fix` resolution with a reasoned reply per comment — written
with the same commands a human uses, which is what makes a human's review and an engine's review
interchangeable.

**The item's review set** is what both the resume check and the triage work list read, and it is
scoped to the item by construction: `plan list --implements <item>` gives the item's plans, `review
list --on <item> --state submitted --verdict request-changes` gives reviews of the item document,
and `plan show <plan-slug> --format json` gives each plan's own `reviews[]` plus the
`change_reviews[]` recorded against it. `rdm review requests` is deliberately *not* used for this:
it is shorthand for `review list --state submitted --verdict request-changes` with no target filter
and no `--on` flag, so it returns every pending review in the project. Triaging off that queue would
dispatch a fix for another roadmap's comment into *this* item's pinned worktree and record an
`--applied-commit` from the wrong branch, and two concurrent dispatch runs would race each other for
the same entries. The completion check is scoped the same way, so another item's open review can
never block this item's terminal write.

**Triage routing** is decided by the comment's own anchor, not by who wrote it. A comment
carrying a `path` is a SOURCE comment: an implementer subagent fixes it in the pinned worktree
and its commit SHA becomes the `--applied-commit`, with a pinned
`rdm:src/<path>@<sha>#L<n>` permalink in the reply. A comment on a plan-repo document goes
through `rdm-revise`, which is the only route that keeps plan-repo `--applied-commit` semantics —
and the only reason SOURCE comments may not go there: that skill edits plan-repo bodies and
cannot carry source-commit provenance. A finding marked `unrefutedReason: budget` was never
graded, so its reply says so explicitly on either branch.

**Refusal as escalation.** The terminal write is source-bound
(`--source`/`--base`/`--expected-head`/`--expected-branch`) and goes through the core
`gates.reviewed` gate, which is enabled for rdm's own plan repo. A refusal is captured verbatim,
parked as `blocked` with that text in the reason, and returned as `outcome: 'escalated'` — except
for one narrow case: a `GateStaleChangeReview` refusal on a delta that changed no executable
behavior since an already-approved review, which the procedure now permits the orchestrator to
override itself, under the four conditions its "terminal write" step spells out. Every other
refusal is still escalated rather than bypassed.

**No downstream divergence.** The distributed `skill-dispatch-phase-cli.md` makes the same
`rdm-wf-plan-review` call as the local copy (since `task/sync-dogfood-and-shipped-skills`).

**How it was accepted.** By dogfooding, and improved iteratively from what a real drive surfaces
(operator, 2026-09-20). There is no smoke-run gate and no harness greps the prose; the real-binary
gates around it (`cargo nextest run`, which includes the Rust `distribution` tests over the
emitted templates and the checked-in plugin tree, and the Rust review workflow tests) are what protect the lane.

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
│                              exhaustion park blocked [code],  │
│                              STOP (escalated)                 │
│      outcome = escalated ──► already blocked, STOP (escalated)│
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
| `rework` | back to `in-progress` (fixable defect) | re-dispatch (new subagent) within the rework-retry budget; on exhaustion park `blocked` with a `[code]` reason and stop the run — see "Budgets and stop conditions" |
| `escalated` | already `blocked` (`[plan]`/`[code]`) | stop the run — see "Budgets and stop conditions" |

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
- **Any park** — an `escalated` OUTCOME, an exhausted rework retry, a
  repeatedly failing advance write, or an unrecognized OUTCOME — parks the
  phase `blocked` exactly as before, and the run now stops immediately with
  `stopReason: escalated`, naming the parked stem. On the shared per-roadmap
  worktree model a parked phase's commits stay on `roadmap/<slug>` underneath
  whatever a later phase would commit on top of them, so continuing past a
  park used to mean a later phase was implemented and reviewed on top of code
  already known to be defective. This was observed on 2026-09-22: a phase
  escalated with real defects and autopilot parked it and dispatched the next
  phase on top of it anyway. A park is now a hard stop, not a skip — see
  plan repo `plan/autopilot-stop-on-escalation` for the decision record.

## Recovering a crashed run

The per-phase unit is prose now, so a crashed *dispatch* is re-entered by
re-running the `rdm-dispatch-phase` skill against the same item; the phase's
persisted status and its plan/review documents are what carry state across the
gap. For the Workflow calls the orchestrator makes (plan review, code review),
the Workflow tool can relaunch a dead or crashed run instead of starting
over: pass the prior run's id back in as `resumeFromRunId`, alongside the
same `scriptPath`:

```
Workflow({ scriptPath: '.claude/workflows/rdm-wf-review-refute-fix.js', resumeFromRunId: '<prior runId>' })
```

Any `agent()` call whose `(prompt, opts)` are byte-unchanged from the crashed
run replays its cached result instead of re-dispatching a fresh subagent.
Four caveats govern whether that actually saves anything:

- **Stop the prior run first.** A still-running run cannot be resumed.
- **Same-session only.** `resumeFromRunId` resumes within the Claude Code
  session that produced it — a new session (e.g. tomorrow) cannot resume
  yesterday's `runId`. This is why cross-session resume is explicitly out of
  scope here (see below).
- **A cached result can itself be empty.** If the crashed agent produced
  nothing before dying, resuming replays that emptiness, not a usable
  result — read the run's `journal.jsonl` before assuming there is anything
  worth recovering.
- **Conservative prefix.** Resume replays only the LONGEST UNCHANGED PREFIX
  of the call sequence: the first edited-or-new call, and every call after
  it, all run live — not only the one that crashed. Savings depend entirely
  on where in the sequence the failure or edit sits, and should never be
  planned around as guaranteed.

Why this matters: autopilot run `wf_974e3812-817` died mid-flight against
this repo's own `workflow-token-reduction` roadmap having spent roughly 3.2M
subagent tokens. Its journal recorded 41 `started` entries against 32
`result` entries — 32 completed agents had a cached result available, and
only the 9 API-failed agents lacked one — yet the review was re-run from
scratch because nothing in the lane knew `resumeFromRunId` existed.

(The determinism this depends on — the reason `.claude/workflows/*.js`
scripts never call `Date.now()`/`Math.random()` — is explained in
[`docs/workflow-schemas.md`](workflow-schemas.md) § "The
`.claude/workflows/` convention".)

**Cross-session resume is out of scope here.** *(Historical: while the
per-phase unit was `rdm-wf-dispatch-phase.js`, the authored plan lived in an
`approvedPlanText` JS local that was never written to the plan repo, so it
survived only in that run's journal for the life of one session.)* The prose
orchestrator persists the plan as a real `plan/<slug>` document instead, so a
fresh dispatch tomorrow reads the approved plan back rather than re-authoring
it; what `resumeFromRunId` cannot carry across sessions is the review fan-out's
cached `agent()` results.

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
the user, each tagged `plan` vs `code`. Since a park now stops the run, a
single run carries at most one escalation — the one that stopped it — plus
however many phases completed before it. "Batching" describes not
interrupting the user mid-phase for a routine decision, not accumulating
multiple parks across a run: the run still never raises a question
interactively, but it no longer keeps going past the first park to gather a
whole queue of them in one pass. Review the queue with:

```bash
rdm review blocked --project <proj>
rdm review blocked --project <proj> --format json
```

## Relation to the retired needs-review safety net

Autopilot is the **active driver** — it pushes a roadmap forward phase by phase.
Every Claude workflow lane that can produce a `needs-review` item actively runs the
canonical review (`.claude/workflows/lib/review.mjs`) before that lane's
finalize step returns: the prose orchestrator's code-review stage runs it inline and
returns a `reviewed`/`blocked` status as OUTCOME data — it persists no terminal
status itself (see ["History: the retired workflow twin"](#history-the-retired-workflow-twin)
above) — and autopilot's own advance/park steps persist that status directly
once dispatch-phase returns, and interactive `rdm-do`'s finalize invokes
`rdm-review` (the generated projection of the same canonical source) after the
human confirm gate. With nothing left unreviewed, the once-passive needs-review
Stop hook (Claude Code) and Pi `agent_end` extension — which only re-prompted
when an item was *left* in `needs-review` — have been retired as redundant;
`cli_loops::worktree_review::roadmap_worktrees_scope_pending_and_restamp`
(`rdm-cli/tests/cli_loops/worktree_review.rs`) drives
`rdm review pending` off directly-set rdm state (status/tags), never off a
specific driver, showing the scoping the retired hook depended on is
agnostic to whatever set that state, workflow or skill. The autopilot lane never emits a `Done:` line — the orchestrator's
review is an inline pipeline (not the `rdm-review` skill), and autopilot's
advance step writes only `--status reviewed`. The `Done:` line is supplied later
by `rdm-review` or at landing.

The [Codex phase-one manual lane](codex-support.md) is an explicit exception:
it stops at `needs-review` for independent review handoff and has no stop-hook
reprompt or automatic review driver. The status remains supported; entering
it captures `review_sha` and `review_branch` from the invoking checkout.
Do not treat the absent hook as review approval or permission to land.

See also [`docs/escalation-protocol.md`](./escalation-protocol.md) for the
shared rule on what escalates, what retries, and how parked escalations are
recorded and resumed, and [`docs/landing.md`](./landing.md) for the landing tail
(`rdm-land` + `rdm worktree prune`) that integrates reviewed work into `main`.
