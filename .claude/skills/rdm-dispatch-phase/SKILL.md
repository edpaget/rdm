---
name: rdm-dispatch-phase
description: Drive one rdm phase or task end-to-end from the main session — plan, review the plan, implement, verify, code-review, triage every comment with a reasoned reply, then write the gated terminal status — leaving a persisted plan-and-review trail
allowed-tools:
  - Read
  - Bash
  - Glob
  - Grep
  - Write
  - Edit
  - Agent
  - Workflow
  - Skill
  - EnterPlanMode
  - ExitPlanMode
---

Drive **one** rdm phase (or task) from its plan to a gated `reviewed`, and leave behind the trail
that explains why it ended where it did: an approved `plan/<slug>`, a persisted plan review, a
persisted `change/<sha>` review, and one `addressed`/`wont-fix` resolution — each with a reasoned
reply — for every comment on it. That trail is the archeology. A Workflow journal lives outside the
plan repo and disappears; these records do not, and a human reads and writes them through exactly
the same commands the orchestrator does.

**This procedure replaces the `rdm-wf-dispatch-phase` Workflow.** It is not a wrapper over it and
never invokes it. The operator's principle: a Workflow is for extremely deterministic mechanism —
the find → refute → filter review cycle — and the agent exercises judgment above the review gate.
Planning, implementing, verifying, triaging and deciding are judgment; they run here, in prose. The
engine file was retired in phase 7 of the `agent-orchestrated-dispatch` roadmap and no longer exists
in the tree.

## How this skill must be entered

**You MUST be running this in the session that loaded it with the `Skill` tool** — the main session,
which still holds `Workflow`. **You MUST NOT dispatch this skill to an `Agent` subagent.** An
`Agent`-spawned subagent has no `Workflow` tool at all (it is absent from both its loaded and its
deferred tool lists, so the call cannot even be formed), and the same-named Skill route inside a
subagent returns only a shim carrying an unsatisfiable `Invoke: Workflow(...)` directive. The two
review-engine calls below can therefore only be made by a session that already holds the tool. See
[`docs/workflow-schemas.md`](docs/workflow-schemas.md) § "Orchestrator / Workflow-reachability
spike".

### Delegation boundary — fixed, not a preference

| Role | Where it runs | Why |
| --- | --- | --- |
| Planner | dispatched `Agent` subagent | isolates planning context; makes no Workflow call |
| Implementer | dispatched `Agent` subagent | isolates implementation context; makes no Workflow call |
| Plan review | **this session**, `Workflow` | needs the `Workflow` tool |
| Code review | **this session**, `Workflow` | needs the `Workflow` tool |
| Verification, triage, every status write, every gate read | **this session**, `Bash` | judgment, and the tool is already here |

**You MUST NOT** collapse the planner or the implementer into your own context ("inline-collapse"):
a planner that also reviews its own plan, or an implementer that also decides whether its diff is
acceptable, provides no independent check at all. **You MUST NOT** delegate a Workflow call, a gate
read, or a status write to a subagent.

`Agent` dispatch is **fire-and-forget**: the call returns a background-launch acknowledgement and
the child's result arrives as a later notification. Every dispatch step below MUST drive its
subagent forward on each notification until it converges — never assume the call blocked.

## Contract

**Input** (`$ARGUMENTS`):

- `<roadmap-slug> <phase>` (stem or number) — phase mode; or `--task <slug>` — task mode.
- `--interactive` — the human-in-the-loop mode `rdm-do` (without `--auto`) selects. It changes
  exactly two things, named in step 7 and step 12. Nothing else.
- `--plan-only` — stop after the plan is approved. Stamps nothing, implements nothing, writes no
  status, records no change review.
- `--max-plan-revise N` (default 2) / `--max-code-rework N` (default 2). `0` is legal and distinct
  from unset: terminate on the first blocking review, no revise/rework round at all.
- `--rdm-bin <path>` / `--project <name>` — optional; they resolve the two placeholders below.

Resolve once, in step 1, and use everywhere after:

- `<rdmBin>` — the rdm executable every command below invokes. An explicit `--rdm-bin` value wins
  verbatim; with none given, use `$RDM_BIN`
  (this repo's `.mise.toml` sets it to the local development build), and a plain `rdm` on `PATH` when
  neither is set. `docs/workflow-schemas.md` § "Environment args: `rdmBin` and `project`" is the
  canonical order; do not re-derive one here.
- `<proj-flag>` — ` --project <name>` when `--project` was given, otherwise nothing at all (never an
  empty `--project` value).
- `<item>` — `phase/<roadmap>/<stem>` or `task/<slug>`. Every `--on`/`--implements` ref uses this
  exact string.

**Output** — your final message is this object, the same OUTCOME shape the retired engine returned
plus two fields, so `rdm-autopilot`'s advance/park handling needs no change:

```json
{
  "roadmap": "<slug>",
  "phase": "<stem>",
  "outcome": "reviewed | rework | escalated",
  "status": "<the rdm status this outcome maps to>",
  "writesCompletion": "<true only when outcome is reviewed>",
  "summary": "<one-line result>",
  "reason": "<the [plan]/[code]-tagged reason, when parked>",
  "findings": "<the surviving findings, or the blocker that forced escalation>",
  "planId": "plan/<slug>",
  "reviewIds": ["<review id>", "..."]
}
```

(Task mode carries `task` in place of `roadmap`/`phase`.) **You MUST NOT write a `Done:` trailer.**
Its format lives in `rdm_core::hook::format_done_directive` (surfaced as `rdm hook done-line`) and
`rdm-land` is its only writer, at land time, off `writesCompletion: true`.

## Run state

Keep these in your own context for the whole run and carry them into every later step:

- `identity` — the pinned checkout: `repository`, `path`, `branch`, `base`, `head`.
- `models` — `{ plan, implement }`, the two model ids resolved in step 3 ("Ensure the worktree
  exists, then pin the checkout identity") from the item's `model` tier. Carried into the
  planner/implementer dispatches; never re-resolved mid-run.
- `planId` — `plan/<slug>`, and the chain of superseded predecessors on a re-plan.
- `reviewIds` — **additive**. A rework pass keeps resolving comments on the review ids it already
  has and appends any new one; it never starts a fresh review to "redo" a pass.
- `planReviseCount`, `codeReworkCount` — against `--max-plan-revise` / `--max-code-rework`.
- `verification` — `{ command, exitCode, tail }` from step 10.

## Procedure

### 1. Parse `$ARGUMENTS` and resolve the placeholders

Resolve `<rdmBin>`, `<proj-flag>`, `<item>`, the mode flags and the two budgets. A missing target is
a stop, not a guess: say so and return without invoking anything.

### 2. Resume before planning — never discard work a prior pass recorded

Read **the item's own review set** (defined once here; step 12 uses this same recipe, for both its
work list and its completion check) before doing anything else:

```bash
# (a) the item's plans — the hub every other record hangs off
<rdmBin> plan list --implements <item><proj-flag> --format json
# (b) reviews of the item document itself
<rdmBin> review list --on <item> --state submitted --verdict request-changes<proj-flag> --format json
# (c) per plan slug from (a): its plan reviews (`reviews[]`) and the change
#     reviews implementing it (`change_reviews[]`)
<rdmBin> plan show <plan-slug><proj-flag> --no-body --format json
```

**You MUST NOT** substitute `review requests` for this. `review requests` is shorthand for `review
list --state submitted --verdict request-changes` with **no target filter at all** — it has no `--on`
flag and returns every pending review in the whole project, including reviews on other roadmaps,
other tasks and other sessions' items. `review list --on <target>` is the same queue *scoped*, and
(a)+(c) are what enumerate this item's targets — the item document, its `plan/<slug>` documents, and
the `change/<sha>` heads recorded against them. Anything the item's review set does not contain
belongs to another item and is **not yours to triage**: acting on it would dispatch a fix into this
item's worktree for a change that lives somewhere else, record an `--applied-commit` from the wrong
branch, and race any concurrent dispatch run reading the same global queue.

- An `approved` plan for `<item>` **and** an open review **in that set** → **resume at triage (step
  12)** with those ids. Do not re-plan, do not create a second review, and do not re-ask a
  confirmation for a decision already recorded as a reply on a comment.
- An `approved` plan and no open review in that set → resume at step 9 (implement) or, if the
  implementation is already committed, step 11 (code review).
- Nothing → continue to step 3.

This is what makes a parked run resumable from the plan repo alone, with no session context.

### 3. Ensure the worktree exists, then pin the checkout identity

`review source` deliberately never creates or changes a worktree, so ensure the item's checkout
exists first (idempotent — it reuses one that is already there), then resolve the identity:

```bash
<rdmBin> worktree add <slug><proj-flag>          # or: worktree add task/<slug>
<rdmBin> review source --on <item><proj-flag> --format json
```

One worktree per roadmap: every phase of a roadmap is implemented in place in the same checkout, so
`worktree add` on a later phase simply returns the existing path. **You MUST NOT** create a
phase-specific branch or fork off `main`.

Record `repository`, `path`, `branch`, `base`, `head` as `identity`. Every later step uses this
same `path` as its working directory and this same `base`/`head` on the terminal write.

Then resolve the two dispatch models from the item's tier. Read `model` from `phase show <phase>
--roadmap <slug><proj-flag> --format json` (task form: `task show <slug><proj-flag> --format
json`) and call it `T`. **Record that same response's `body`** as `item.body` — steps 5 and 9 hand
it to the planner and the implementer, and this is the read it comes from. Do not issue a second
one.

```bash
# T non-empty (phase mode with a recorded tier):
<rdmBin> model resolve plan --tier <T>
<rdmBin> model resolve implement --tier <T>
# T empty/missing, or task mode (a task carries no tier at all):
<rdmBin> model resolve plan
<rdmBin> model resolve implement
```

Record the two resulting ids as `models.plan` / `models.implement`.

Then read the remaining inputs step 6 hands the plan-review engine, so that engine spawns no
subagent to re-read what this session is already holding:

```bash
<rdmBin> roadmap show <slug><proj-flag> --format json    # record `body` as roadmapBody — SKIP in task mode
<rdmBin> model resolve mechanical                        # untiered: a lane role, not a tier-derived dispatch model
<rdmBin> model resolve review-find
<rdmBin> model resolve review-verify
<rdmBin> task list --tag plan-review --status wont-fix<proj-flag> --format json   # record each result's `title`
```

Record the three ids as `models.mechanical` / `models.reviewFind` / `models.reviewVerify`, the
roadmap body as `roadmapBody`, and the wont-fix titles as `wontFixedTitles`. Three notes on why
these are the commands:

- The three `model resolve` calls take **no `--tier`**. They are review-lane roles, not dispatch
  models, and the engine's own bootstrap resolves them untiered too — resolving them here is
  precisely what stops that `model:mechanical` bootstrap agent from ever firing.
- The roadmap body is read in phase mode only, and step 5 already needs it (it hands the planner
  the roadmap's `## Intent` verbatim), so this makes an existing dependency explicit rather than
  adding a read.
- Use `task list`, **not** `rdm search`, for the wont-fix corpus: `search` truncates at its default
  `--limit 20` while the real corpus is larger, and its JSON carries no `body` field at all.

The code-review Workflow call's own `findModel`/`verifyModel` gap is out of scope for this phase
(tracked by `task/thread-code-review-judgment-models`) and is not touched by this step.

**Self-check before proceeding:** state the pinned `path`, `branch`, `head`, the two resolved
`models.plan` / `models.implement` and the three resolved `models.mechanical` / `models.reviewFind`
/ `models.reviewVerify`, and confirm you captured the item's `body`, the roadmap `body`
(phase mode) and the wont-fix titles. If the worktree or identity command failed, escalate — never
invent a checkout, and never let a subagent choose one. A failed **hoist** read is different and
not fatal, but say which one failed and state the consequence, because it differs per argument and
only one of them still has an engine-side fallback: step 6 reviews the **implementation plan**, and
that branch returns before the engine's fetch block, so no `fetch:*` agent is reachable to re-read
anything you omit.

- the item's `body` is **not** a step-6 argument at all — it feeds the planner (step 5) and the
  implementer (step 9). If it could not be read, escalate rather than dispatching a planner with no
  phase text.
- `roadmapBody` — omitting it loses the recorded `## Intent`, so the engine simply runs no
  intent-alignment dimension. Non-blocking, and the same outcome a roadmap that records no intent
  produces. There is no fallback fetch on this path.
- the model trio — omitting **all three** is the one real fallback: the engine spawns its own
  `model:mechanical` bootstrap to resolve them. Two of three is not legal (see step 6).
- `wontFixedTexts` — omitting it suppresses nothing, and there is no wont-fix search on this path,
  so an already-dismissed finding can resurface and force a revise round. Pass `[]` only when the
  corpus really is empty.

### 4. Stamp `in-progress`

```bash
<rdmBin> phase update <phase> --status in-progress --no-edit --roadmap <slug><proj-flag>
<rdmBin> commit -m "chore(plan): start <item>"
```

(task form: `task update <slug> --status in-progress …`). **Skip this entirely under `--plan-only`**
— a plan-only pass does no implementation and stamping would misreport work that never happened.

### 5. Dispatch the planner subagent

**Declare** that you are dispatching the planner on `model: <models.plan>`, then dispatch **one**
`Agent` subagent with `model: <models.plan>` and:

- the item body (`phase show`/`task show`) and the parent roadmap's `## Intent` section verbatim;
- the pinned `identity.path` as its working directory;
- an explicit instruction to write the plan through the CLI:

  ```bash
  <rdmBin> plan create <plan-slug> --title "<title>" --implements <item> --body "<plan>" --no-edit<proj-flag>
  <rdmBin> commit -m "docs(plan): plan <item>"
  ```

  and, on a re-plan, `--supersedes plan/<previous-slug>` as well — that flip is **load-bearing**,
  not cosmetic: the code-review engine's persist path requires **exactly one** `approved` plan
  implementing the item, and a stale second one makes it throw.
- a required `## Verification command` heading in the plan body holding the project's verification
  command as **a single line**, discovered in this order: `.github/workflows/`, `docs/principles.md`,
  then `CLAUDE.md`/`AGENTS.md`. It goes in the plan, **never** into config — `rdm config set
  dispatch.verify` is an operator act, not something a dispatch writes.

**Self-check before proceeding:** confirm the planner subagent returned and that `plan show
<plan-slug> --format json` reports a real plan whose `implements` is `rdm:<item>`, and **record that
same response's `body` as `planBody`** — step 6 hands it to the plan-review engine as the document
under review, and this is the read it comes from. No new command. If you drafted the plan yourself
instead of dispatching, you have inline-collapsed — stop and dispatch.

### 6. Invoke the plan review — in THIS session

```
Workflow({ scriptPath: '.claude/workflows/rdm-wf-plan-review.js', args: {
  implementationPlan: true,
  planSlug: '<plan-slug>',
  planText: '<planBody verbatim>',
  persist: { on: 'plan/<plan-slug>' },
  roadmapBody: '<roadmapBody verbatim>',          // OMIT in task mode
  mechanicalModel: '<models.mechanical>',
  findModel: '<models.reviewFind>',
  verifyModel: '<models.reviewVerify>',
  wontFixedTexts: [<wontFixedTitles>],
  rdmBin: '<rdmBin>', project: '<project>',
} })
```

The plan document is what is **graded** here, not merely what the verdict is recorded on. The item's
own document is never handed to the engine, so a finding about an inaccuracy in the phase body — one
the plan does not inherit — cannot arise and cannot force a revise round. **You MUST make this call
yourself.** It is the one step a subagent physically cannot perform.

- `planSlug` / `planText` are **not** hoists — they are the document under review. Pass the plan
  body verbatim from step 5's `plan show --format json`. A `planSlug` with no `planText`, a
  `planSlug` on a non-implementation-plan target, and a `persist.on` that is not
  `plan/<plan-slug>` each throw before any agent runs.
- `roadmapBody` — the **parent roadmap's** body, verbatim and unedited. Do not extract the `##
  Intent` section yourself: the engine runs the one canonical extractor over the raw body, which is
  what keeps the hoisted and fetched paths from ever disagreeing. **Omit it in task mode** — a task
  has no parent roadmap. Omitting it is safe: the engine degrades to no intent-alignment dimension,
  never to a block.
- the model trio — **all three or none.** The engine's guard is all-or-nothing, so two out of three
  saves nothing and still spawns the bootstrap.
- `wontFixedTexts` — the wont-fix titles from step 3. An empty array is a legal, meaningful value
  (nothing to suppress) and is **not** the same as omitting the key. Omit it only if the `task
  list` call itself failed.

Because the engine no longer reviews the item document, this call does **not** clear a
`needs-plan-review` tag on the phase. That tag asserts the *item* was plan-reviewed and is cleared
by the standalone `rdm-plan-review` surface or a manual sweep, not here.

### 7. Wait for the plan approval — ONE origin-blind read

```bash
<rdmBin> plan show <plan-slug><proj-flag> --format json
```

Switch **only** on its `status` field:

| `status` | next |
| --- | --- |
| `approved` | proceed to step 8 |
| `changes-requested` | the bounded revise loop below |
| `draft` | keep waiting |
| `superseded` | re-plan (step 5) against the successor |

**You MUST NOT** read the plan-review Workflow's return value to decide approval, and **you MUST
NOT** inspect who authored the review. `rdm-core`'s `ops::reviews` flips plan status through
`ops::plan::set_plan_status` on **any** `review submit` against a `plan/<slug>` target, so a
workflow-persisted approve and a human's approve are the *same write* and this *same read* sees
both. Adding an author or provenance check here would break the human/agent symmetry the whole
roadmap exists for.

Poll with a bounded number of attempts, logging one visible line per attempt. On exhaustion park
`blocked` with reason `[plan] plan approval not recorded on plan/<plan-slug>` — **never** proceed on
an unapproved plan.

While waiting, print the commands a human uses to satisfy this wait (and under `--interactive`,
**pause here** until they have):

```bash
<rdmBin> review start --on plan/<plan-slug> --no-edit<proj-flag>
<rdmBin> review comment <id> --quote "<exact text>" --body "<feedback>" --no-edit<proj-flag>
<rdmBin> review submit <id> --verdict approve --no-edit<proj-flag>
```

**`changes-requested` → revise:** if `planReviseCount < --max-plan-revise`, increment it and run the
`rdm-revise` loop (`Skill({ skill: 'rdm-revise' })`) against the plan review — a plan review's
comments are plan-**document** comments, which is exactly what that skill is for — then re-read this
same `plan show`. On exhaustion park `blocked` with `[plan] plan-revise budget exhausted; open
review(s): <ids>; plan: plan/<plan-slug>`.

### 8. `--plan-only` ends here

Report `outcome: 'reviewed'` with `writesCompletion: false`, having written **no** status and **no**
change review. A plan-only pass that stamps or writes status misreports work that never happened.

### 9. Dispatch the implementer subagent

**Declare** it, then dispatch **one** `Agent` subagent with `model: <models.implement>`, the approved
plan body verbatim, the item body, and `identity.path` as its working directory. Require it to
commit in that worktree and return the commit SHA. Follow the `--permission-mode auto` rules below.
**You MUST NOT** implement inline.

**Self-check before proceeding:** confirm the implementer returned, then re-run `review source --on
<item>` and restate the pinned `path`/`branch`/`base` — any change to `repository`, `path`, `branch`
or `base` is an **escalation**, never a retry. `head` is expected to have moved; record the new one.

### 10. Verify in the pinned checkout

1. `<rdmBin> verify run --item <item><proj-flag> --format json`
   - exit **0** → pass. Record `{ command, exitCode: 0, tail }`.
   - exit **1** → a blocking **rework** finding (not an escalation).
   - exit **2** → `resolved: false`, i.e. no `dispatch.verify` is configured. **This is not a
     failure.** Fall through to 2.
2. Fallback: read the command from the **approved** plan (`plan show --format json` → body → the
   `## Verification command` heading). Refuse a multi-line value the way `verify run` does. Then run
   it directly through Bash with `cd <identity.path>`, and record `{ command, exitCode, tail }` with
   the tail bounded to the last **4000** characters, so both routes report the same shape.
3. Neither source supplies a command → **unresolved escalation**: park `blocked` with `[code] no
   verification command resolved from dispatch.verify or plan/<plan-slug>`. An unverified pass is
   never reported as a pass.

A nonzero exit from either route is a blocking rework finding bounded by `--max-code-rework` — no
new OUTCOME value. Carry `verification` into the next step so it reaches the persisted review.

### 11. Invoke the code review — in THIS session

```
Workflow({ scriptPath: '.claude/workflows/rdm-wf-review-refute-fix.js', args: {
  mode: 'code', roadmap: '<slug>', phase: '<phase>',     // or task: '<slug>'
  persist: true, implements: 'plan/<plan-slug>', gate: false,
  rdmBin: '<rdmBin>', project: '<project>',
} })
```

`persist` records the review on `change/<head>` for the head the engine itself re-resolves;
`gate: false` keeps the status write here, in step 14, where the refusal can be surfaced.

Read the returned object and obey it:

- `outcome: 'escalated'`, or a non-empty `failure` — including `'required review evidence is
  incomplete'` and `'review persisted with unresolved anchor degradation'` — is a **park**. **You
  MUST NOT** write `reviewed` on it.
- `reviewPersistence` carries phase 11's accounting. Log its degradation counts **verbatim**. A
  `rework` outcome keeps its own outcome by design even when anchors degraded; adjudicate those
  comments in triage (step 12).
- Append `reviewId` to `reviewIds`.

### 12. Triage — ONE procedure, whatever the review's origin

Build the work list from **the item's review set** (step 2's recipe (a)+(b)+(c), re-read now)
**plus** the ids the engine returned, deduped by id. Every id on the list must trace back to
`<item>` — its own document, one of its `plan/<slug>` documents, or a `change/<sha>` recorded in one
of their `change_reviews[]`. **You MUST NOT** build this list from `review requests`: that queue is
project-wide and unfiltered, so a literal read of it hands you another item's comments and you would
"fix" them in this item's worktree, under this item's pinned identity, with an `--applied-commit`
that belongs to the wrong change. If an id's target does not resolve to `<item>`, **drop it** — the
failure mode is "triaging another item's review". Then, for each review, read it **once**:

```bash
<rdmBin> review show <id><proj-flag> --format json
```

and drive **only** off its comment array — `anchor`, `path`, `source_link`, `resolution.state`.
**You MUST NOT** add an author or provenance check. A human review created with `review start --on
change/<sha>`, `review comment --path <p> --quote "<exact text>"` and `submit --verdict
request-changes` produces the *same document shape* the engine's persist ladder writes, so there is
no second branch to write. (`--path` is required with `--quote` on a change target, and a path
containing `@` or `#` is refused — the `rdm:src/<path>@<rev>#L<n>` grammar reserves both.)

Per comment, run this numbered checklist:

1. **Declare** the decision, the route, and the reply text you intend to record.
2. **Classify and act:**
   - **SOURCE comment** (carries a `path` — a file-quote anchor into the diff): dispatch an
     implementer `Agent` subagent with `model: <models.implement>` in `identity.path` with the
     comment body and its `source_link` permalink; require a commit and its SHA. **You MUST NOT
     route a source comment to
     `rdm-revise`**: that skill edits plan-repo document bodies and its `--applied-commit` is a
     plan-repo SHA, so it cannot carry source-commit provenance. (This split is the answer to
     `task/plan-dispatch-plan-change-rework-routing`.)
   - **PLAN-DOCUMENT comment** (on a plan-repo document target): run the `rdm-revise` loop, keeping
     its plan-repo `--applied-commit` semantics intact.
   - **`wont-fix`**: a reasoned `--reply` is mandatory, not optional.
   - **Ungraded finding** — the comment's header carries `unrefutedReason: budget`, meaning it was
     never refuted because the per-unit refutation budget ran out: its reply MUST say so explicitly
     and give your own reasoning for the decision taken, on either branch.
   - **Degraded anchor** — a comment that *carries* an anchor which no longer resolves is read as
     whole-document feedback and its reply MUST state that the anchor did not resolve; read the
     reviewed-side body with `--at <created_commit>` before deciding. A comment authored
     deliberately with **no** `--quote` carries no anchor at all: that is a valid whole-document
     comment, **not** degradation, and MUST NOT be reported as anchor loss.
3. **Resolve**, with a reply on **both** branches:
   ```bash
   <rdmBin> review update <id> --comment <n> --status addressed --applied-commit <sha> --reply "…"<proj-flag>
   <rdmBin> review update <id> --comment <n> --status wont-fix --reply "<why>"<proj-flag>
   ```
   A SOURCE reply MUST embed the pinned permalink `rdm:src/<path>@<applied-commit>#L<line>`.
4. **Verify**: `review show <id> --format json` shows that comment with a terminal `status` and a
   non-empty `reply`.

Under `--interactive`, **present each decision (1) for confirmation before running (3)**. Under
`--auto`, apply them without pausing. Nothing else differs — **you MUST NOT** fork the triage
procedure, skip a confirmation in `--interactive`, or add one in `--auto`.

Close each review and land the batch:

```bash
<rdmBin> review update <id> --state addressed<proj-flag>
<rdmBin> link check --on <item><proj-flag>
<rdmBin> commit -m "chore(plan): triage review <id>"
```

Then confirm, from the records and not from memory: every comment terminal with a non-empty reply,
each `addressed` one carrying an `applied_commit`, the review `addressed`, `link check` exit 0, and
**the item's review set** (step 2's recipe, re-read) holding **no** submitted request-changes review
— i.e. every `review list --on <target> --state submitted --verdict request-changes` over the item's
own targets comes back empty. A non-empty result is a **refusal to proceed** to the terminal write,
not a warning. Another item's pending review in the project-wide `review requests` queue is **not**
a reason to block this write.

Bounded by `--max-code-rework`; on exhaustion park `blocked` with `[code] rework budget exhausted;
unresolved comments on review <id>` so the review id is in the reason.

### 13. Re-review at the post-triage HEAD

Every source fix moves HEAD, and the core gate requires an **approving** change review recorded at
the HEAD the write observes. Triage only moves a review to `addressed`; it never changes a verdict,
and the persist map sends `rework`/`escalated` to `request-changes`. So after the **last**
source-touching act: re-run `review source --on <item>`, restate the new pinned head, and re-run
step 11. Writing `reviewed` off an `addressed` `request-changes` review is refused as
`GateNoApprovedChangeReview`; writing it off an approve recorded at an older head is refused as
`GateStaleChangeReview`, which names both SHAs.

### 14. The terminal write — through the gate, never around it

```bash
<rdmBin> phase update <phase> --status reviewed \
  --source <identity.path> --base <identity.base> \
  --expected-head <identity.head> --expected-branch <identity.branch> \
  --no-edit --roadmap <slug><proj-flag>
<rdmBin> commit -m "chore(plan): finalize <item>"
```

(task form: `task update <slug> --status reviewed …`.) The explicit `--source`/`--base`/
`--expected-head`/`--expected-branch` binding is required: run from a cwd that is not a distinct
project repo and the worktree probe degrades to *no probe*, silently skipping the cleanliness
precondition.

**You MUST NOT bypass the gate.** rdm has an operator-only flag that waives the gate's *record*
preconditions; it is refused outright unless the gate is enforcing, and this procedure has no case for
it. The named failure mode is **forging the terminal write**: a bypass here would record a `reviewed`
the records do not support, which is precisely what the gate exists to prevent. This prose
deliberately does not spell that flag, so a mechanical scan over every agent-facing surface can keep
asserting that no surface emits it — do not "helpfully" add the literal back.

On a nonzero exit, capture stderr **verbatim** — `GateNoApprovedPlan`, `GateNoApprovedChangeReview`,
`GateStaleChangeReview`, `GateWorktreeDirty`, `GateWorktreeUnobservable` each already name the
missing record *and* the command that would create it — then:

```bash
<rdmBin> phase update <phase> --status blocked --reason "[code] reviewed-gate refused: <verbatim refusal>" --no-edit --roadmap <slug><proj-flag>
```

and return `outcome: 'escalated'` with that same text in `reason`. Surface a `GateWorktreeDirty`
refusal verbatim rather than cleaning the tree: worktrees are per-roadmap, so a sibling phase's
uncommitted edit trips it even when this item is clean, and `git reset --hard`/`git clean -fdx`/
`git stash -u` are denied under `--permission-mode auto` anyway.

On success, read the status back (`phase show --format json` → `status == "reviewed"`) and treat a
mismatch as an escalation, not a success.

### 15. Return the OUTCOME

Produce the object from the Contract above as your final message, `planId` and `reviewIds` included.
This is the in-session result of a loaded skill rather than a Workflow return value; the fields are
unchanged, so `rdm-autopilot`'s advance/park handling reads it exactly as before.

## Interactive mode (`rdm-do` without `--auto`)

Identical procedure, two differences only:

1. **Step 7** prints the human plan-review commands and waits for a human-submitted approve review
   instead of proceeding on the workflow-persisted one. An `--interactive` run abandoned at that
   wait MUST park with a durable reason rather than spin, and the poll MUST NOT treat `draft` as
   `changes-requested`.
2. **Step 12** presents each triage decision for confirmation before replying.

Both modes run the same commands and produce the same review-record shape.

## Safe operations under --permission-mode auto

Unattended runs (launched with `--permission-mode auto`, or `bypassPermissions` in a sandbox) stall
forever if they trip the auto-mode destructive-action classifier. This procedure and every subagent
it dispatches MUST follow these rules:

- **Modify existing files with `Edit`, never `Write`.** A `Write` over an existing tracked file
  trips the classifier. Reserve `Write` for new files.
- **Never run `git stash -u`, `git reset --hard`, or `git clean -fdx`.** They are classified as
  irreversible local destruction and denied. Per-roadmap worktree isolation makes whole-tree resets
  unnecessary; commit a `wip:` commit on the branch instead.
- **Never run `rdm discard --force` in the shared plan repo.** It resets other sessions' uncommitted
  work. Commit each batch immediately instead — the plan repo is shared, and any `rdm commit` sweeps
  only your own session's changeset.

## Escalation protocol

This skill follows the shared escalation protocol (`docs/escalation-protocol.md`):

- **Routine findings never escalate.** Bugs, missing tests, doc gaps are fixed in triage or filed as
  a task; they never reach the user mid-run.
- **Decisions and blockers escalate.** Ambiguous or untestable acceptance criteria, an architectural
  decision with no clear default, an exhausted plan-revise or rework budget, an unresolved
  verification command, a gate refusal, or a changed checkout identity.
- **Park, don't interrupt.** Record the escalation by setting the item `blocked` with a stage-tagged
  reason (`[plan]` or `[code]`) that names the open review ids and the plan, so a later session
  reconstructs the pending work from the item's review set (step 2's recipe) alone. The user reviews
  the whole cross-item queue at once with `<rdmBin> review blocked<proj-flag>`.

## How this procedure is validated

By **dogfooding**, and improved iteratively from what a real drive surfaces — not by a harness that
greps this file. A test asserting that particular strings are present in static prose is not
evidence that the procedure behaves, so `scripts/verify-skill-dispatch.sh` is deliberately **not**
written (operator, 2026-09-20), and there is no live-smoke-run gate. What does gate this lane is the
real-binary machinery it stands on: `scripts/verify-agent-config-distribution.sh` and
`scripts/verify-plugin-install.sh` over the emitted templates, `scripts/verify-workflow-review.sh`
over the two review engines, and `cargo nextest run` over the gate, plan and review surfaces this
prose drives. See [`docs/workflow-vs-prose-boundary.md`](docs/workflow-vs-prose-boundary.md) and
[`docs/autonomous-loop.md`](docs/autonomous-loop.md).
