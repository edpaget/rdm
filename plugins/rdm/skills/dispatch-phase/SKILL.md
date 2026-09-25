---
name: dispatch-phase
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

## How this skill must be entered

**You MUST be running this in the session that loaded it with the `Skill` tool** — the main session,
which still holds `Workflow`. **You MUST NOT dispatch this skill to an `Agent` subagent.** An
`Agent`-spawned subagent has no `Workflow` tool at all (it is absent from both its loaded and its
deferred tool lists, so the call cannot even be formed), and the same-named Skill route inside a
subagent returns only a shim carrying an unsatisfiable `Invoke: Workflow(...)` directive. The two
review-engine calls below can therefore only be made by a session that already holds the tool.

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
- `--interactive` — the human-in-the-loop mode `do` (without `--auto`) selects. It changes
  exactly three things, named in step 6, step 7 and step 13 (see "Interactive mode"). Nothing else.
- `--plan-only` — stop after the plan is approved. Stamps nothing, implements nothing, writes no
  status, records no change review.
- `--max-plan-revise N` (default 2) / `--max-code-rework N` (default 2). `0` is legal and distinct
  from unset: terminate on the first blocking review, no revise/rework round at all.
- `--max-refutations N` — optional. The per-run refutation budget both review engines receive: at
  most N consolidated units (distinct defects) per review unit are graded by a refuter; the rest pass
  through un-refuted. Parsed exactly like `--max-plan-revise` — a non-negative integer, `0` legal and
  distinct from unset. When absent, step 4 resolves it from config. The precedence chain is stated
  once, in `docs/file-formats.md` § `rdm.toml`.
- `--run <id>` — optional. The id of an open run record, passed by the autopilot driver. With it, this
  unit writes its `unit-start`/`unit-end` entries into that record and never opens or closes a run of
  its own. Without it (standalone, or entered through the do shim), this unit opens a single-unit run in
  step 1 and closes it in its final step.
- `--rdm-bin <path>` / `--project <name>` — optional; they resolve the two placeholders below.

Resolve once, in step 1, and use everywhere after:

- `<rdmBin>` — the rdm executable every command below invokes, and the `rdmBin` both Workflow calls
  receive. An explicit `--rdm-bin` value wins verbatim; with none given, use `$RDM_BIN`, and a plain
  `rdm` on `PATH` when neither is set. Never probe the filesystem to pick a binary. If a "Resolving
  `rdmBin`" section is appended to this skill, it is the single authoritative resolution order —
  follow it and do not re-derive one here.
- `<proj-flag>` — ` --project <name>` when `--project` was given, otherwise ` --project <PROJECT>`, or
  nothing at all when no project name applies and rdm's own `RDM_PROJECT`/`default_project` chain
  should resolve it (never an empty `--project` value). `<project>` in the Workflow calls is that
  same name, omitted when `<proj-flag>` is empty.
- `<item>` — `phase/<roadmap>/<stem>` or `task/<slug>`. Every `--on`/`--implements` ref uses this
  exact string.

**Output** — your final message is this object, the shape `autopilot`'s advance/park handling
reads:

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
`writesCompletion: true` means `land` will mark the item `done` directly at land time —
no trailer is written by anyone in this flow.

## Run state

Keep these for the whole run: `identity` (the pinned checkout: `repository`, `path`, `branch`,
`base`, `head`), `profiles` (`plan`, `implement`, `reviewFind`, `reviewVerify`, `reviewConsolidate`, each a
`{model, effort}` pair resolved once in step 4 and never re-resolved mid-run), `planId` plus any
superseded predecessors, `reviewIds` (**additive** — a rework pass keeps resolving comments on the
ids it already has and appends any new one; it never starts a fresh review to redo a pass),
`planReviseCount` / `codeReworkCount`, `verification` (`{ command, exitCode, tail }` from step
11), `runId` (the run record this unit writes into: the `--run` value, or the id step 1 recorded), and
`ownsRun` (`true` only when this unit recorded the run itself, i.e. no `--run` was given).

## Procedure

### 1. Parse `$ARGUMENTS` and resolve the placeholders

Resolve `<rdmBin>`, `<proj-flag>`, `<item>`, the mode flags and the two budgets. A missing target is
a stop, not a guess: say so and return without invoking anything.

Then open this unit's run entry, before step 2's resume reads, so they fall inside the unit window:

```bash
# only when no --run was given (ownsRun = true):
<rdmBin> run record --driver dispatch-phase --roadmap <slug><proj-flag>          # task mode: --task <slug>
# always, when runId is known:
<rdmBin> run unit-start <runId> --unit <phase><proj-flag>                          # task mode: --unit <slug>
```

Read the id `run record` prints and keep that literal id in your working context as `runId`; substitute the literal id wherever `<runId>` appears in later commands (shell variables do not survive between Bash calls).

The entry is staged and lands in the **first commit this unit makes**: on a fresh pass that is step
3's `chore(plan): start <item>`, which comes right after. On a pass that skips step 3's commit (under
`--plan-only`, or when a resume jumps straight to triage or code review), it lands in the next commit
this procedure makes or in the closing commit of step 16.

**Run accounting is best-effort.** Every `rdm run …` call and every `chore(plan): close run <id>`
commit is observation only. If one exits nonzero, log a one-line warning naming the command and its
stderr and carry on exactly as if it had succeeded. Never park, stop, retry or change an outcome
because of one. If `run record` itself failed, there is no id: skip every later `rdm run` call and
the close commit for this run, and say so once in the warning.

### 2. Resume before planning — never discard work a prior pass recorded

Read **the item's own review set** (defined once here; step 13 uses this same recipe, for both its
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
  13)** with those ids. Do not re-plan, do not create a second review, and do not re-ask a
  confirmation for a decision already recorded as a reply on a comment.
- An `approved` plan and no open review in that set, with the implementation **not yet committed**
  (e.g. a `--plan-only` pass followed by a real dispatch, or a crash between plan approval and the
  implementer's commit) → resume at **step 9** (record the start commit), then continue to step 10
  (implement) as normal. No implementation commit exists yet, so this is the item's first implementer
  dispatch in every sense that matters — step 9 MUST run here, not be skipped; its own read-back and
  write-once refusal already make running it safe even if some earlier pass somehow recorded a value.
- An `approved` plan and no open review in that set, with the implementation **already committed** →
  resume directly at step 12 (code review). This resume skips step 9: it is not this item's first
  implementer dispatch, and recording a start now would wrongly exclude commits already made. `review
  source` falls back to the merge-base for such an item if it never got a recorded `started_head`,
  which is safe (wider, not narrower).
- Nothing → continue to step 3.

This is what makes a parked run resumable from the plan repo alone, with no session context.

### 3. Ensure the worktree exists, then stamp `in-progress`

```bash
<rdmBin> worktree add <slug><proj-flag>          # or: worktree add task/<slug>
```

Always run this, **including under `--plan-only`** — step 4's identity pin needs the worktree to
exist, whether or not this pass goes on to implement anything. One worktree per roadmap: every phase
of a roadmap is implemented in place in the same checkout, so `worktree add` on a later phase simply
returns the existing path. **You MUST NOT** create a phase-specific branch or fork off `main`.

```bash
<rdmBin> phase update <phase> --status in-progress --no-edit --roadmap <slug><proj-flag>
<rdmBin> commit -m "chore(plan): start <item>"
```

(task form: `task update <slug> --status in-progress …`); the `unit-start` staged in step 1 lands in
this commit. **Skip only these two lines under
`--plan-only`** — a plan-only pass does no implementation, so stamping `in-progress` would misreport
work that never happened. This status stamp records nothing about the item's starting commit — that
is step 9's job, an explicit write independent of this one.

### 4. Pin the checkout identity

`review source` deliberately never creates or changes a worktree — step 3 already ensured it exists
— so this step only resolves the identity. Pass `--no-code`:

```bash
<rdmBin> review source --on <item><proj-flag> --no-code --format json
```

This pin only needs `repository`/`path`/`branch`/`base`/`head`, never the diff itself. Pass
`--no-code` always: a fresh item (no commits yet) has an empty committed range, and without
`--no-code` `review source` refuses with "empty committed range; declare --no-code only for
intentional no-code review" — which would stop the dispatch before planning starts, on exactly the
case AC2 requires to work, for a fresh roadmap or task worktree regardless of whether `started_head`
is recorded. `--no-code` only waives that refusal; it changes nothing about which `base`/`head` get
resolved. Every later `review source` call runs without it, once real commits exist: step 10's
self-check, each reviewer in step 12, and step 14's re-review.

Record `repository`, `path`, `branch`, `base`, `head` as `identity`. Every later step uses this
same `path` as its working directory and this same `base`/`head` on the terminal write.

`identity.base` is the phase's (or task's) own **starting head**: `review source` defaults it to the
recorded `started_head`, not the merge-base with the default branch, once one exists. This matters
because the roadmap worktree is shared — it carries every earlier phase's commits, including a parked
one's — so without this default the review (and the finders/refuters in steps 6 and 12) would re-find
every earlier phase's already-triaged changes and attribute them to this phase. On the item's true
first pass nothing is recorded yet, so this pin falls back to the merge-base instead; step 9 records
the real starting point from this same `identity.head`, immediately before the first implementer
dispatch. Either way the response's `baseNote` field names the fallback, so the gap is visible rather
than silent.

Then resolve the two dispatch **profiles** — a model plus a reasoning effort — from the item's tier.
Read `model` from `phase show <phase> --roadmap <slug><proj-flag> --format json` (task form: `task
show <slug><proj-flag> --format json`) and call it `T`. **Record that same response's `body`** as
`item.body` — steps 5 and 10 hand it to the planner and the implementer, and this is the read it
comes from. Do not issue a second one.

```bash
# T non-empty (phase mode with a recorded tier):
<rdmBin> model resolve plan --tier <T> --format json
<rdmBin> model resolve implement --tier <T> --format json
# T empty/missing, or task mode (a task carries no tier at all):
<rdmBin> model resolve plan --format json
<rdmBin> model resolve implement --format json
```

Each prints `{"step", "host", "tier", "model", "effort"}`. Record the two results as
`profiles.plan` / `profiles.implement` — each a `{model, effort}` pair, read from the one JSON call
(never a second text call).

**How a profile reaches the planner and the implementer.** The `Agent` tool's `model` parameter
carries the model, but it has no effort parameter; a custom agent definition's `effort:` frontmatter
does. rdm ships one role-agnostic definition per effort level — `rdm:rdm-effort-low`,
`rdm:rdm-effort-medium`, `rdm:rdm-effort-high`, `rdm:rdm-effort-xhigh`, `rdm:rdm-effort-max` — each a
general-purpose agent with the full toolset and no model of its own (`rdm agent-config claude
--skills` writes them to `.claude/agents/`; the `rdm` plugin carries them in its `agents/`
directory). So the planner and the implementer are dispatched with
`subagent_type: rdm:rdm-effort-<profile effort>` **and** `model: <profile model>`; the per-call model
wins over the definition. **Fallback:** if that `Agent` call fails with "Agent type … not found"
(for example, a session that started before the agent definitions were installed — Claude Code only
watches agent directories that existed at session start), re-dispatch that one role as
`general-purpose` with the model only, and **name the effort gap in the run report** — never
silently.

Then read what this session — and only this session — can supply. **The plan-review engine reads
nothing:** every reviewer fetches the document it needs from the command its prompt names. What is
left to gather is the three judgment-site profiles, which are yours to resolve, and the wont-fix
corpus, which no document records.

```bash
<rdmBin> roadmap show <slug><proj-flag> --format json    # for STEP 5's planner — SKIP in task mode
<rdmBin> model resolve review-find --format json
<rdmBin> model resolve review-verify --format json
<rdmBin> model resolve review-consolidate --format json
<rdmBin> config get max_refutations --raw     # SKIP when --max-refutations was given
<rdmBin> task list --tag plan-review --status wont-fix<proj-flag> --format json   # record each result's `title`
```

Record the three results as `profiles.reviewFind` / `profiles.reviewVerify` /
`profiles.reviewConsolidate` (each `{model, effort}`)
and the wont-fix titles as `wontFixedTitles`:

- `maxRefutations` — the `--max-refutations` flag when given. Otherwise the trimmed output of
  `config get max_refutations --raw`, which already applies `RDM_MAX_REFUTATIONS` over the repo and
  then global `max_refutations` key, so this one call covers both layers — do not read the env var
  separately. Empty output means unset: OMIT the `maxRefutations` key from both engine calls (steps 6
  and 12) entirely, never forward a default the engine applies itself. Pass a value through as-is;
  the engine validates it and throws before any agent runs on a malformed one.

- The three `model resolve` calls take **no `--tier`** — review-lane roles, not dispatch models. There
  is no mechanical model left to resolve and no bootstrap agent to pre-empt: each model and effort is
  independently optional, and an omitted one makes that judgment agent inherit the session's model
  or effort.
- The roadmap read is **step 5's**, listed here only because this is where reads are gathered: the
  planner gets `## Intent` verbatim, while step 6's `intent-alignment` reviewer reads that section
  itself, from the roadmap slug alone.
- Use `task list`, **not** `rdm search`, for the wont-fix corpus: `search` truncates at its default
  `--limit 20` and its JSON carries no `body` field.

**Self-check before proceeding:** state the pinned `path`, `branch`, `head`, the two resolved
`profiles.plan` / `profiles.implement` and the three resolved `profiles.reviewFind` /
`profiles.reviewVerify` / `profiles.reviewConsolidate` (model and effort each) and the resolved
`maxRefutations` (or "unset"), and confirm you captured the item's `body`, the
roadmap `body` (phase mode) and the wont-fix titles. If the worktree or identity command failed,
escalate — never invent a checkout, and never let a subagent choose one. A failed **read** is
different and not fatal, but say which one failed and state the consequence — it differs per value,
and **no engine-side fallback exists for any of them** since the engine dispatches finder,
consolidator and refuter agents only, with nothing reachable to re-read what you omit:

- item `body` feeds the planner (step 5) and implementer (step 10), never step 6 directly — if
  unreadable, escalate rather than dispatching a planner with no phase text.
- roadmap `body` likewise feeds only step 5's planner; if unreadable, the planner loses `## Intent`,
  but `intent-alignment` is unaffected since it reads that section itself from the `roadmap` slug.
- `findModel` / `verifyModel` / `consolidateModel` and `findEffort` / `verifyEffort` /
  `consolidateEffort` — each independently optional; an
  omitted model makes that agent inherit the session model, an omitted effort its effort. One
  resolved value plus one omitted one is perfectly legal.
- `wontFixedTexts` — omitting it suppresses nothing, and there is no wont-fix search on this path,
  so an already-dismissed finding can resurface and force a revise round. Pass `[]` only when the
  corpus really is empty.

### 5. Dispatch the planner subagent

**Declare** that you are dispatching the planner on `profiles.plan`, then dispatch **one** `Agent`
subagent with `subagent_type: rdm:rdm-effort-<profiles.plan.effort>` and `model: <profiles.plan.model>`
(see step 4 for the not-found fallback) and:

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
<plan-slug> --format json` reports a real plan whose `implements` link names `<item>`. No new command
beyond that confirmation — step 6 never needs the plan body handed to it: it passes the slug, and
each reviewer runs `plan show` itself. If you drafted the plan yourself instead of dispatching, you
have inline-collapsed — stop and dispatch.

### 6. Invoke the plan review — in THIS session

**Skipped under `--interactive`** — go straight to step 7, whose approve must then come from a
human. Otherwise:

Invoke the **`rdm:rdm-wf-plan-review` Workflow** (`rdm:rdm-wf-plan-review`, installed by the `rdm` plugin)
via the Workflow tool, passing `args` as a JSON object (never a stringified value):

```js
{
  implementationPlan: true,
  planSlug: '<plan-slug>',
  persist: { on: 'plan/<plan-slug>' },
  reviewers: ['coherence', 'architectural-fit', 'restraint'],  // + 'intent-alignment' when the roadmap records `## Intent`
  roadmap: '<slug>', phase: '<phase>',            // task mode: task: '<slug>' instead, and OMIT roadmap/phase
  source: '<identity.path>', base: '<identity.base>',
  expectedHead: '<identity.head>', expectedBranch: '<identity.branch>',
  findModel: '<profiles.reviewFind.model>',
  verifyModel: '<profiles.reviewVerify.model>',
  findEffort: '<profiles.reviewFind.effort>',
  verifyEffort: '<profiles.reviewVerify.effort>',
  consolidateModel: '<profiles.reviewConsolidate.model>',
  consolidateEffort: '<profiles.reviewConsolidate.effort>',
  wontFixedTexts: [<wontFixedTitles>],
  maxRefutations: <maxRefutations>,               // OMIT the key when step 4 resolved nothing
  rdmBin: '<rdmBin>', project: '<project>',
}
```

The plan document is what is **graded** here, not merely what the verdict is recorded on. The item's
own document is never handed to the engine, so a finding about an inaccuracy in the phase body — one
the plan does not inherit — cannot arise and cannot force a revise round. **You MUST make this call
yourself.** It is the one step a subagent physically cannot perform.

- `planSlug` names the document under review, and it is the ONLY way to name it: each reviewer is
  told the `plan show <plan-slug> --format json` command and runs it itself, reading the body once
  in its own context. There is no `planText`, no second verifying read, and nothing transcribed into
  this call's arguments. A `planSlug` on a non-implementation-plan target, and a `persist.on` that is
  not `plan/<plan-slug>`, each throw before any agent runs.
- `roadmap` names the parent roadmap so the `intent-alignment` reviewer knows which document to read
  its `## Intent` from. Omit it in task mode.
- `source` / `base` / `expectedHead` / `expectedBranch` — the SAME pinned checkout identity you
  recorded in step 4, in the SAME flat shape step 12's code-review call takes. Every finder and
  refuter runs `rdm review source --on <item> --source ... --no-code --format json` and verifies the
  checkout has not moved before reading any file the plan cites, out of that pinned `path` at that
  pinned `head` — never out of whatever checkout the dispatching session happens to be sitting in,
  so a plan targeting an unlanded worktree is never graded against `main`. `phase` (or `task` in
  task mode) is **required alongside this pin** — the engine derives the pinned `--on <item>` from
  it. **Omitting all four is legal**: the engine then reads from the invoking session's own working
  directory — the correct behavior for the standalone `plan-review` surface run outside a
  dispatch worktree.
- `reviewers` — **the reviewer set you are selecting.** Omitting the key entirely runs every plan
  reviewer, which is the safe default; naming a set runs exactly those. An unrecognised name is
  dropped silently and shows as a gap in the unit's `coverage.selected`/`coverage.ran` — nothing
  rejects a thin set, so under-review is your visible choice, not an error. A set in which **no**
  name resolves is the one exception: it throws before any agent runs, rather than reporting a clean
  review over an empty fleet. The plan reviewers are:

  | reviewer | what it is for | include it when |
  |---|---|---|
  | `coherence` | is the plan internally consistent, concrete, actionable? | always |
  | `architectural-fit` | does any step violate a stated project constraint? | always |
  | `restraint` | has the plan over-specified what could be left to the implementer? | always |
  | `unit-of-work` | is this independently deliverable and testable? | **only on a phase** — never on an implementation plan, a task, or a roadmap body |
  | `intent-alignment` | does the plan serve the operator's recorded `## Intent`? | when the parent roadmap records one; it reads that section itself |

  Here the target is an **implementation plan**, so `unit-of-work` is omitted: sizing was settled
  when the phase was created. Add `'intent-alignment'` when the parent roadmap records an `##
  Intent` section — the reviewer reads it out of the roadmap itself, so nothing is transcribed into
  this call. In task mode there is no parent roadmap, so omit it.
- `findModel` / `verifyModel` / `consolidateModel` and `findEffort` / `verifyEffort` /
  `consolidateEffort` — the three judgment-site profiles'
  `model` and `effort`, each independently optional. An omitted model makes that agent inherit the
  session model, an omitted effort its effort. An effort the engine does not accept is refused
  before any agent runs.
- `wontFixedTexts` — the wont-fix titles from step 4. An empty array is a legal, meaningful value
  (nothing to suppress) and is **not** the same as omitting the key. Omit it only if the `task
  list` call itself failed.

**The plan engine reads nothing and writes nothing either** — it dispatches finder, consolidator
and refuter agents only. When it returns `persistCommands` / `persistScript`, **you** run that ladder in Bash,
in one session, and report the exit status; it records the plan review on `plan/<plan-slug>` exactly
as the code ladder records the change review.

Because the engine does not review the item document, this call does **not** clear a
`needs-plan-review` tag on the phase. That tag asserts the *item* was plan-reviewed and is cleared
by the standalone `plan-review` surface or a manual sweep, not here.

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
NOT** inspect who authored the review. rdm flips plan status on **any** `review submit` against a
`plan/<slug>` target, so a workflow-persisted approve and a human's approve are the *same write* and
this *same read* sees both. An author or provenance check here would break the human/agent symmetry
this design depends on.

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
`revise` loop (`Skill({ skill: 'revise' })`) against the plan review — a plan review's
comments are plan-**document** comments, which is exactly what that skill is for — then re-read this
same `plan show`. On exhaustion park `blocked` with `[plan] plan-revise budget exhausted; open
review(s): <ids>; plan: plan/<plan-slug>`.

### 8. `--plan-only` ends here

Report `outcome: 'reviewed'` with `writesCompletion: false`, having written **no** status and **no**
change review. A plan-only pass that stamps or writes status misreports work that never happened.

### 9. Record the item's starting commit

**Skip this entire step under `--plan-only`** — a plan-only pass does no implementation, so there is
nothing yet to scope a review to.

Read the item back (`<rdmBin> phase show <phase> --roadmap <slug><proj-flag> --format json` / task
form: `<rdmBin> task show <slug><proj-flag> --format json`). Skip the write below when either is
true: `started_head` is already recorded (a resumed dispatch, or an item predating this field), or
this run's own state shows this is a resume with prior implementation commits already on this item,
not a fresh first pass (recording now would wrongly include that earlier pass's commits in the
item's "starting" point). Otherwise:

```bash
<rdmBin> phase update <phase> --no-edit --start-commit <identity.head> --roadmap <slug><proj-flag>
<rdmBin> commit -m "chore(plan): record start commit for <item>"
```

(task form: `<rdmBin> task update <slug> --no-edit --start-commit <identity.head><proj-flag>`), using
the head pinned in step 4 (`identity.head`) — the shared roadmap worktree's HEAD immediately before
this item's first implementer dispatch, so it excludes every earlier phase's commits already on the
branch. `--start-commit` is write-once and refuses to overwrite an existing value; a refusal here
means it was already recorded by someone else between your read above and this write — re-read
`started_head` rather than treating the refusal as a failure.

When you DO record it, also set `identity.base` in your own run state to `identity.head` — the change
step 4 anticipated, moving `base` off its merge-base fallback onto the item's real starting point, so
step 10's self-check compares against the updated value and does not escalate. When you skip the
write (already recorded, or a resumed pass with prior commits), `identity.base` stays exactly as step
4 (or an earlier pass) left it.

### 10. Dispatch the implementer subagent

**Declare** it, then dispatch **one** `Agent` subagent with
`subagent_type: rdm:rdm-effort-<profiles.implement.effort>` and `model: <profiles.implement.model>` (see
step 4 for the not-found fallback), the approved plan body verbatim, the item body, and
`identity.path` as its working directory. Require it to commit in that worktree and return the
commit SHA. Follow the `--permission-mode auto` rules below. **You MUST NOT** implement inline.

**Self-check before proceeding:** confirm the implementer returned, then re-run `review source --on
<item>` and restate the pinned `path`/`branch`/`base`, comparing `base` against the **current**
`identity.base` in your run state — the value step 9 just set, or left unchanged — not the value step
4 originally read. Any change to `repository`, `path`, `branch`, or a `base` that no longer matches
that current `identity.base`, is an **escalation**, never a retry. `head` is expected to have moved;
record the new one.

### 11. Verify in the pinned checkout

1. `<rdmBin> verify run --item <item><proj-flag> --format json`
   - exit **0** → pass. Record `{ command, exitCode: 0, tail }`.
   - exit **2** → `resolved: false`, i.e. no `dispatch.verify` is configured. **This is not a
     failure.** Fall through to 2.
   - exit **3** with the JSON payload showing `resolved: true` and `exit: null` → the item did not
     resolve to a worktree at all — the command never ran. This is an **escalation**, not a rework
     finding and not the exit-2 fallback: park `blocked` naming the resolution error, and do not
     attempt the plan-recorded-command fallback (the command IS configured; only the checkout is
     unknown).
   - any other nonzero exit — including a command that legitimately exits 3 itself, reported with a
     non-null `exit` in the payload — → a blocking **rework** finding (not an escalation). Key off the
     payload, not the process exit code, to tell this apart from the escalation above.
2. Fallback: read the command from the **approved** plan (`plan show --format json` → body → the
   `## Verification command` heading). Refuse a multi-line value the way `verify run` does. Then run
   it directly through Bash with `cd <identity.path>`, and record `{ command, exitCode, tail }` with
   the tail bounded to the last **4000** characters, so both routes report the same shape.
3. Neither source supplies a command → **unresolved escalation**: park `blocked` with `[code] no
   verification command resolved from dispatch.verify or plan/<plan-slug>`. An unverified pass is
   never reported as a pass.

A nonzero exit from the fallback route, or any other nonzero exit from `verify run` (including the
command's own exit code), is a blocking rework finding bounded by `--max-code-rework` — no new
OUTCOME value. The one exception is exit 3 from `verify run` whose JSON payload shows `resolved: true`
and `exit: null`: that is the escalation above, never rework. Carry `verification` into the next step
so it reaches the persisted review.

### 12. Invoke the code review — in THIS session

Invoke the **`rdm:rdm-wf-review-refute-fix` Workflow** (`rdm:rdm-wf-review-refute-fix`, installed by the `rdm` plugin)
via the Workflow tool, passing `args` as a JSON object (never a stringified value):

```js
{
  mode: 'code', roadmap: '<slug>', phase: '<phase>',     // or task: '<slug>'
  persist: true, implements: 'plan/<plan-slug>', gate: false,
  reviewers: [<the set you selected — see below>],
  source: '<identity.path>', base: '<identity.base>',
  expectedHead: '<identity.head>', expectedBranch: '<identity.branch>',
  findModel: '<profiles.reviewFind.model>',
  verifyModel: '<profiles.reviewVerify.model>',
  findEffort: '<profiles.reviewFind.effort>',
  verifyEffort: '<profiles.reviewVerify.effort>',
  consolidateModel: '<profiles.reviewConsolidate.model>',
  consolidateEffort: '<profiles.reviewConsolidate.effort>',
  maxRefutations: <maxRefutations>,               // OMIT the key when step 4 resolved nothing
  rdmBin: '<rdmBin>', project: '<project>',
}
```

**The engine reads nothing and writes nothing.** You pass the identity you pinned in step 10 — a
path, two SHAs and a branch name, nothing more — and each reviewer runs `rdm review source` itself
to reach the diff. The engine dispatches finder, consolidator and refuter agents and no others — the
consolidator (`consolidate:<mode>`, plus one `:retry`) only on a unit with two or more candidates.

- `findModel` / `verifyModel` / `consolidateModel` and `findEffort` / `verifyEffort` /
  `consolidateEffort` — the same three resolved `profiles.reviewFind` / `profiles.reviewVerify` /
  `profiles.reviewConsolidate` from step 4, mirroring step 6's plan-review call.
  Each is independently optional; an omitted model makes that judgment agent inherit the session
  model, an omitted effort its effort.

`persist: true` therefore does **not** write a review. It returns the ladder as `persistCommands` /
`persistScript`: ready-to-run Bash that records the review on `change/<head>`. **You run it**, in
one Bash call, and report the exit status:

```bash
<paste result.persistScript verbatim>
```

It prints `reviewId=<id>`, then `anchorsDegraded=<all|partial|none>`, then `anchorsParkRequired=<yes|no>`
on success — append the id to `reviewIds`. A path-anchored comment (one carrying both `--path` and
`--quote`) that the real binary refuses at run time is retried **mechanically, by the ladder itself**:
it lands whole-document, header-marked `anchor: degraded`, and is counted into the printed
`anchorsDegraded=` line. There is nothing for you to re-run by hand for that case. If `review start`
itself is refused, **park** — never invent a different target. A nonzero exit anywhere else is a park.

**Check the ladder's own printed `anchorsParkRequired=<yes|no>` line before treating the run as
ordinary persistence** — not `anchorsDegraded`, and not `result.persistDegraded` (a build-time-only
preview that can under-report a run whose anchors degraded at run time). When `anchorsParkRequired=yes`
— an anchor was lost for a systemic cause, or a `blocking` finding lost its anchor for any reason,
including the benign one below — **park** `blocked` with `[code] a comment anchor was lost for a
systemic cause, or a blocking finding lost its anchor; see the review's own note comment and each
comment's \`anchor\` header`, even though the ladder itself exited 0. Whenever any anchor degrades, the
persist ladder appends one whole-document note comment stating how many of how many requested anchors
could not be placed, so the review can never read as clean persistence merely because
`outcome`/`classifyOutcome` stay independent of anchor plumbing (see `docs/workflow-schemas.md` §
"Persisting a review"). Checking `anchorsParkRequired` is a **separate step you take yourself** after
running the ladder — the write it gates has already happened by the time you read it.

**Benign vs. systemic, for a run-time refusal of a `--path`+`--quote` line:** a quote sitting on a
line the diff did not touch, or naming a real, in-range file the diff never modifies at all, is
BENIGN — "you missed an edit here" (or "you missed editing this file entirely") is as legitimate a
finding as one about a touched line, and the ladder's whole-document fallback loses nothing. A path
that does not exist at the reviewed head at all, a quote that does not exist anywhere in the document,
or an ambiguous quote (occurs more than once) is SYSTEMIC — the finder was probably not looking at the
right code. `anchorsDegraded` stays purely informational — the total whole-document-fallback volume,
worth noting in a reply, but never a park signal by itself, including at `partial`. A non-`blocking`
finding (a suggestion or concern) refused only for the benign cause above reads `anchorsDegraded=all`
or `partial` but `anchorsParkRequired=no`: goes to ordinary triage, and its reply notes the anchor did
not resolve.

`gate: false` keeps the status write here, in step 15, where the refusal can be surfaced.

`reviewers` is **your** judgment about what this diff touches. Omit the key to run every code
reviewer — the safe default, and the right choice when you are unsure. An unrecognised name is
dropped silently and shows as a gap in `reviewCoverage`; nothing rejects a thin set, so an
under-reviewed diff is your visible choice rather than an error. The code reviewers are:

| reviewer | what it is for | include it when |
|---|---|---|
| `ac` | per-criterion PASS/FAIL/PARTIAL against the item's acceptance criteria | always — it is the structured channel the outcome reads |
| `correctness` | logic bugs, edge cases, error paths | always |
| `tests` | do tests exist and cover the behaviour? | the change adds or alters non-trivial logic |
| `architecture` | does logic live where the layering contract puts it? | the change spans more than one module or layer |
| `api-docs` | do public items carry the required documentation? | the change adds or alters a public API item |
| `changelog` | is there a user-perspective entry in the same commit? | the change is user-facing |
| `security` | can an attacker now do something they should not? | the change touches auth, input parsing, paths, subprocesses, secrets, deserialization, or network code |

**Scale the set to the delta, not to the fleet.** A change confined to documentation, comments, or
CHANGELOG prose does not need all seven reviewers; a substantive logic change does. There is no
line-count or file-count threshold here — judge what the diff actually touches. Whatever you choose
is recorded in `reviewCoverage`, so a thin set is a visible choice, not a silent gap.

Read the returned object and obey it:

- `outcome: 'escalated'`, or a non-empty `failure` — including `'required review evidence is
  incomplete'` — is a **park**. **You
  MUST NOT** write `reviewed` on it.
- The outcome is classified **before** the persist ladder is even built, and nothing recomposes it
  afterwards. An anchor that would not land is not a verdict: fix it by re-running the one refused
  line whole-document, or park, per the persist block above.

### 13. Triage — ONE procedure, whatever the review's origin

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
     implementer `Agent` subagent with `subagent_type: rdm:rdm-effort-<profiles.implement.effort>` and
     `model: <profiles.implement.model>` in `identity.path` with the comment body and its
     `source_link` permalink; require a commit and its SHA. **You MUST NOT route a source comment to
     `revise`**: that skill edits plan-repo document bodies and its `--applied-commit` is a
     plan-repo SHA, so it cannot carry source-commit provenance.
   - **PLAN-DOCUMENT comment** (on a plan-repo document target): run the `revise` loop, keeping
     its plan-repo `--applied-commit` semantics intact.
   - **`wont-fix`**: a reasoned `--reply` is mandatory, not optional.
   - **Ungraded finding** — the comment's header carries `unrefutedReason: budget`, meaning it was
     never refuted because the per-unit refutation budget ran out: its reply MUST say so explicitly
     and give your own reasoning for the decision taken, on either branch.
   - **Out-of-scope finding** — the comment's header carries `inScope: false`, meaning a refuter
     confirmed the finding but graded it outside the approved plan's scope (behaviour the plan did
     not change, or whose only adequate fix reaches outside what the plan changed). This does not
     gate and MUST NOT be routed as though it were an ordinary defect requiring a fix in this
     dispatch. Route it `wont-fix` with a `--reply` that states it is deferred as future work —
     optionally filing a task for it — never a silent drop. This is a confirmed defect, not an
     unverified observation, so the reply must not read as a dismissal of the finding itself.
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

Under `--interactive`, **present each decision (1) for confirmation before running (3)**. Otherwise,
apply them without pausing. Nothing else differs — **you MUST NOT** fork the triage procedure, skip a
confirmation in `--interactive`, or add one without it.

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

### 14. Re-review at the post-triage HEAD

Every source fix moves HEAD, and the core gate requires an **approving** change review recorded at
the HEAD the write observes. Triage only moves a review to `addressed`; it never changes a verdict,
and the persist map sends `rework`/`escalated` to `request-changes`. So after the **last**
source-touching act: re-run `review source --on <item>`, restate the new pinned head, and re-run
step 12. Writing `reviewed` off an `addressed` `request-changes` review is refused as
`GateNoApprovedChangeReview`; writing it off an approve recorded at an older head is refused as
`GateStaleChangeReview`, which names both SHAs.

Apply the same proportionate-reviewer judgment on this re-run: a re-review of a delta that changed
nothing but documentation, comments, or CHANGELOG prose does not need the full fleet either.

### 15. The terminal write — through the gate, never around it

```bash
<rdmBin> phase update <phase> --status reviewed \
  --source <identity.path> --base <identity.base> \
  --expected-head <identity.head> --expected-branch <identity.branch> \
  --no-edit --roadmap <slug><proj-flag>
# read-back — the staged status is readable before it is committed:
<rdmBin> phase show <phase> --roadmap <slug><proj-flag> --format json   # status == "reviewed"
# only when the write exited 0 AND the read-back confirmed "reviewed":
<rdmBin> run unit-end <runId> --outcome reviewed<proj-flag>
<rdmBin> commit -m "chore(plan): finalize <item>"
```

(task form: `task update <slug> --status reviewed …`.) The explicit `--source`/`--base`/
`--expected-head`/`--expected-branch` binding is required: run from a cwd that is not a distinct
project repo and the worktree probe degrades to *no probe*, silently skipping the cleanliness
precondition.

**The gate may be overridden, but only for one refusal and only within narrow conditions.** You —
the `dispatch-phase` orchestrator — are authorized to invoke rdm's `--override-gate "<reason>"`
flag for exactly one case: a `GateStaleChangeReview` refusal, when, in your judgment, all four
conditions below hold. A human operator may invoke the same flag more broadly; either way it waives
the gate's *record* preconditions and is refused outright unless the gate is enforcing.

- the change review at the reviewed HEAD returned verdict `approve` and is `addressed`, with every
  comment terminal and replied; and
- every commit between that HEAD and the current HEAD came from triaging comments on that same
  review; and
- that delta changes **no executable behavior** — documentation, comments, CHANGELOG, test names,
  and nothing else; and
- `<reason>` names the review id, both HEADs (the reviewed HEAD and the current HEAD), the comments
  addressed, and cites this authorization by section name — e.g. "per dispatch-phase § 'The
  terminal write' stale-review waiver" — so the citation stays correct however this section is later
  renumbered.

**You judge the "no executable behavior changed" condition yourself**, from the diff and the review
already in your context. rdm does not classify it, and nothing here re-derives eligibility by
parsing the persisted comment header — judge it from the review record. A misjudgment is recorded in
the audited reason string, which is the point of overriding rather than fabricating a verdict.

Still forbidden, unconditionally: overriding a missing or `request-changes` review
(`GateNoApprovedChangeReview`), a missing approved plan (`GateNoApprovedPlan`), or a dirty worktree
(`GateWorktreeDirty`); overriding to skip a re-review after a genuine code fix; and creating your own
approve review to satisfy the gate. The waiver above covers **staleness only** — never reach for it
on any other refusal.

When the four conditions hold:

```bash
<rdmBin> phase update <phase> --status reviewed --override-gate "<reason>" \
  --source <identity.path> --base <identity.base> \
  --expected-head <identity.head> --expected-branch <identity.branch> \
  --no-edit --roadmap <slug><proj-flag>
# read-back — the staged status is readable before it is committed:
<rdmBin> phase show <phase> --roadmap <slug><proj-flag> --format json   # status == "reviewed"
# only when the write exited 0 AND the read-back confirmed "reviewed":
<rdmBin> run unit-end <runId> --outcome reviewed<proj-flag>
<rdmBin> commit -m "chore(plan): finalize <item>"
```

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
mismatch as an escalation, not a success. In both blocks above that read-back runs immediately after
the status write and **before** the finalize commit, and the `reviewed` unit-end is written only when
the write exited 0 **and** the read-back confirmed `reviewed`, so it lands in the finalize commit. On
a gate refusal or a read-back mismatch write no `reviewed` unit-end: that path escalates, and step 16
ends the unit with the final `escalated` outcome, so the record never says `reviewed` for a parked
unit.

### 16. Return the OUTCOME

Every return passes through here. That covers step 15's reviewed return (already ended there), the
`--plan-only` return (step 8), and every park: plan approval or revise exhausted, verification
unresolved or unresolvable, code-review escalation or `anchorsParkRequired=yes`, rework exhausted,
gate refusal, and a changed checkout identity. It also covers a `rework` return. If the unit entry is
not yet ended, end it now with the OUTCOME's `outcome` field:

```bash
<rdmBin> run unit-end <runId> --outcome <OUTCOME.outcome><proj-flag>
```

For a `--plan-only` return, pass `--outcome plan-only-vetted`, which keeps it distinct from a real
`reviewed`.

**When `ownsRun`** (no `--run` was given), close the single-unit run and commit:

```bash
<rdmBin> run close <runId> --stop-reason "<OUTCOME.outcome>"<proj-flag>
<rdmBin> commit -m "chore(plan): close run <runId>"
```

That commit also lands any park status this procedure staged. **Under `--run`**, do not close or
commit here. The entry stays staged and lands in the caller's next commit: the next dispatch's start
commit, or autopilot's `chore(plan): close run <id>` commit, which always follows a park or stop.

Produce the object from the Contract above as your final message, `planId` and `reviewIds` included.

## Interactive mode (`do` without `--auto`)

Identical procedure, three differences only:

1. **Step 6** is skipped: no plan-review Workflow runs, so no workflow-persisted approve exists.
2. **Step 7** prints the human plan-review commands and waits for a human-submitted approve review
   (step 6 recorded none). An `--interactive` run abandoned at that
   wait MUST park with a durable reason rather than spin, and the poll MUST NOT treat `draft` as
   `changes-requested`.
3. **Step 13** presents each triage decision for confirmation before replying.

Apart from step 6's skipped plan-review Workflow, both modes run the same commands and produce the
same review-record shape.

## Safe operations under --permission-mode auto

Unattended runs (launched with `--permission-mode auto`, or `bypassPermissions` in a sandbox) stall
forever if they trip the auto-mode destructive-action classifier. This procedure and every subagent
it dispatches MUST follow these rules:

- **Modify existing files with `Edit`, never `Write`.** A `Write` over an existing tracked file
  trips the classifier. Reserve `Write` for new files.
- **Never run `git stash -u`, `git reset --hard`, or `git clean -fdx`.** They are classified as
  irreversible local destruction and denied. Per-roadmap worktree isolation makes whole-tree resets
  unnecessary; commit a `wip:` commit on the branch instead.
- **Never run `rdm discard --force` in a shared plan repo.** It resets other sessions' uncommitted
  work. Commit each batch immediately instead — any `rdm commit` sweeps only your own session's
  changeset.

## Escalation protocol

- **Routine findings never escalate.** Bugs, missing tests, doc gaps are fixed in triage or filed as
  a task; they never reach the user mid-run.
- **Decisions and blockers escalate.** Ambiguous or untestable acceptance criteria, an architectural
  decision with no clear default, an exhausted plan-revise or rework budget, an unresolved
  verification command, a gate refusal, or a changed checkout identity.
- **Park, don't interrupt.** Record the escalation by setting the item `blocked` with a stage-tagged
  reason (`[plan]` or `[code]`) that names the open review ids and the plan, so a later session
  reconstructs the pending work from the item's review set (step 2's recipe) alone. The user reviews
  the whole cross-item queue at once with `<rdmBin> review blocked<proj-flag>`.

## Resolving `rdmBin` (plugin install)

This skill was installed from the `rdm` plugin, so there is no repo-local build path to assume. Resolve the `rdmBin` argument in this order and use the first that exists:

1. an explicitly supplied `--rdm-bin <path>`;
2. the `RDM_BIN` environment variable;
3. a plain `rdm` on `PATH`.

If none resolves, stop and report: `rdm binary not found. Install rdm, then set RDM_BIN=/path/to/rdm, put rdm on your PATH, or pass --rdm-bin /path/to/rdm.` Never guess a path, and never invoke a workflow without one.
