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
that explains why it ended where it did: an approved `plan/<slug>`, a plan review approving it, a
persisted `change/<sha>` review, and one `addressed`/`wont-fix` resolution — each with a reasoned
reply — for every comment on it. That trail is the archeology, and a human reads and writes it
through exactly the same commands this procedure does.

## How this skill must be entered

**You MUST be running this in the session that loaded it with the `Skill` tool** — the session that
still holds `Workflow`. **You MUST NOT dispatch this skill to an `Agent` subagent.** An
`Agent`-spawned subagent has no `Workflow` tool at all, so the code-review call below cannot even be
formed there; the same-named Skill route inside a subagent returns only a shim carrying an
unsatisfiable `Invoke: Workflow(...)` directive.

### Delegation boundary — fixed, not a preference

| Role | Where it runs | Why |
| --- | --- | --- |
| Planner | dispatched `Agent` subagent | isolates planning context; makes no Workflow call |
| Implementer | dispatched `Agent` subagent | isolates implementation context; makes no Workflow call |
| Code review | **this session**, `Workflow` | needs the `Workflow` tool |
| Verification, triage, every status write, every gate read | **this session**, `Bash` | judgment, and the tool is already here |

**You MUST NOT** collapse the planner or the implementer into your own context ("inline-collapse"): a
planner that also reviews its own plan, or an implementer that also decides whether its diff is
acceptable, provides no independent check at all. **You MUST NOT** delegate a Workflow call, a gate
read, or a status write to a subagent.

`Agent` dispatch is **fire-and-forget**: the call returns a background-launch acknowledgement and the
child's result arrives as a later notification. Every dispatch step below MUST drive its subagent
forward on each notification until it converges — never assume the call blocked.

## Contract

**Input** (`$ARGUMENTS`):

- `<roadmap-slug> <phase>` (stem or number) — phase mode; or `--task <slug>` — task mode.
- `--interactive` — the human-in-the-loop mode `do` (without `--auto`) selects. It changes
  exactly one thing here: step 11 presents each triage decision for confirmation before recording it.
- `--plan-only` — stop once the plan is approved. Stamps nothing, implements nothing, writes no
  status, records no change review.
- `--max-plan-revise N` (default 2) / `--max-code-rework N` (default 2). `0` is legal and distinct
  from unset: terminate on the first blocking review, no revise/rework round at all.
- `rdmBin` — the rdm executable you invoke (e.g. `rdm` on PATH, or a repo-local build path); use the
  same one for every command below and pass it to the Workflow call. **Optional**: omit it and a
  plain `rdm` on `PATH` is used; an explicitly passed value always wins verbatim, and the literal
  `"rdm"` requests `PATH` resolution deliberately. Never probe the filesystem to pick a binary. If a
  "Resolving `rdmBin`" section is appended to this skill, it is the single authoritative resolution
  order — follow it and do not re-derive one here.
- `project` — the project name used in `--project <PROJECT>`. **Optional**, and appended only to
  project-scoped commands; omit it to let rdm's own `RDM_PROJECT`/`default_project` chain apply.

`<item>` below means `phase/<roadmap>/<stem>` or `task/<slug>` — every `--on`/`--implements` ref uses
that exact string.

**Output** — your final message is this object:

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
Its format has exactly one home, `rdm hook done-line`, and `land` is its only writer, at land
time, off `writesCompletion: true`.

## Run state

Keep these for the whole run: `identity` (the pinned checkout: `repository`, `path`, `branch`,
`base`, `head`), `models` (`{ plan, implement }`, the two model ids resolved in step 3 from the
item's `model` tier, carried into the planner/implementer dispatches and never re-resolved
mid-run), `planId` plus any superseded predecessors, `reviewIds` (**additive** — a rework pass
keeps resolving comments on the ids it already has and never starts a fresh review to redo a pass),
`planReviseCount` / `codeReworkCount`, and `verification` (`{ command, exitCode, tail }`).

## Procedure

### 1. Parse `$ARGUMENTS`

Resolve the target, the mode flags, the two budgets, `rdmBin` and `project`. A missing target is a
stop, not a guess: say so and return without invoking anything.

### 2. Resume before planning — never discard work a prior pass recorded

Read **the item's own review set** (defined once here; step 11 uses this same recipe, for both its
work list and its completion check):

```bash
# (a) the item's plans — the hub every other record hangs off
rdm plan list --implements <item> --project <PROJECT> --format json
# (b) reviews of the item document itself
rdm review list --on <item> --state submitted --verdict request-changes --project <PROJECT> --format json
# (c) per plan slug from (a): its plan reviews (`reviews[]`) and the change
#     reviews implementing it (`change_reviews[]`)
rdm plan show <plan-slug> --project <PROJECT> --no-body --format json
```

**You MUST NOT** substitute `rdm review requests` for this. `review requests` is shorthand for
`review list --state submitted --verdict request-changes` with **no target filter at all** — it has
no `--on` flag and returns every pending review in the whole project, including reviews on other
roadmaps, other tasks and other sessions' items. `review list --on <target>` is the same queue
*scoped*, and (a)+(c) are what enumerate this item's targets — the item document, its `plan/<slug>`
documents, and the `change/<sha>` heads recorded against them. Anything the item's review set does
not contain belongs to another item and is **not yours to triage**: acting on it would dispatch a fix
into this item's worktree for a change that lives somewhere else, record an `--applied-commit` from
the wrong branch, and race any concurrent dispatch run reading the same global queue.

- An `approved` plan for `<item>` **and** an open review **in that set** → **resume at triage (step
  11)** with those ids. Do not re-plan, do not create a second review, and do not re-ask a
  confirmation for a decision already recorded as a reply on a comment.
- An `approved` plan and no open review in that set → resume at step 8 (implement), or step 10 (code
  review) if the implementation is already committed.
- Nothing → continue to step 3.

### 3. Ensure the worktree exists, then pin the checkout identity

`rdm review source` deliberately never creates or changes a worktree, so ensure the checkout exists
first (idempotent — it reuses one already there), then resolve the identity:

```bash
rdm worktree add <slug> --project <PROJECT>          # or: rdm worktree add task/<slug>
rdm review source --on <item> --project <PROJECT> --format json
```

Record `repository`, `path`, `branch`, `base`, `head` as `identity`. One worktree per roadmap: every
phase of a roadmap is implemented in place in the same checkout, so `worktree add` on a later phase
returns the existing path. **You MUST NOT** create a phase-specific branch or fork off `main`.

Then resolve the two dispatch models from the item's tier. Read `model` from `rdm phase show
<phase> --roadmap <slug> --project <PROJECT> --format json` (task form: `rdm task show <slug> --project <PROJECT>
--format json`) and call it `T`.

```bash
# T non-empty (phase mode with a recorded tier):
rdm model resolve plan --tier <T>
rdm model resolve implement --tier <T>
# T empty/missing, or task mode (a task carries no tier at all):
rdm model resolve plan
rdm model resolve implement
```

Record the two resulting ids as `models.plan` / `models.implement`. The code-review Workflow
call's own `findModel`/`verifyModel` gap is out of scope for this phase (tracked by
`task/thread-code-review-judgment-models`).

**Self-check before proceeding:** state the pinned `path`, `branch`, `head`, and the two resolved
`models.plan` / `models.implement` you just read. A failed command is an escalation — never invent
a checkout, and never let a subagent choose one.

### 4. Stamp `in-progress`

```bash
rdm phase update <phase> --status in-progress --no-edit --roadmap <slug> --project <PROJECT>
rdm commit -m "chore(plan): start <item>"
```

(task form: `rdm task update <slug> --status in-progress …`). **Skip this entirely under
`--plan-only`** — a plan-only pass does no implementation, and stamping would misreport work that
never happened.

### 5. Dispatch the planner subagent

**Declare** that you are dispatching the planner on `model: <models.plan>`, then dispatch **one**
`Agent` subagent with `model: <models.plan>`, the item body (`rdm phase show`/`rdm task show`), the
parent roadmap's `## Intent` section verbatim, and the pinned `identity.path` as its working
directory. Require it to write the plan through the CLI:

```bash
rdm plan create <plan-slug> --title "<title>" --implements <item> --body "<plan>" --no-edit --project <PROJECT>
rdm commit -m "docs(plan): plan <item>"
```

plus `--supersedes plan/<previous-slug>` on a re-plan — that flip is **load-bearing**, not cosmetic:
the code-review engine's persist path requires **exactly one** `approved` plan implementing the item,
and a stale second one makes it throw. The plan body MUST carry a `## Verification command` heading
holding the project's verification command as **a single line**, discovered in this order:
`.github/workflows/`, `docs/principles.md`, then `CLAUDE.md`/`AGENTS.md`. It goes in the plan,
**never** into config — `rdm config set dispatch.verify` is an operator act, not something a dispatch
writes.

**Self-check before proceeding:** confirm the planner subagent returned and that
`rdm plan show <plan-slug> --project <PROJECT> --format json` reports a real plan whose `implements` field
names the item you are dispatching. If you drafted the plan yourself instead of dispatching, you
have inline-collapsed — stop and dispatch.

### 6. Wait for the plan approval — ONE origin-blind read

```bash
rdm plan show <plan-slug> --project <PROJECT> --format json
```

Switch **only** on its `status` field: `approved` → step 7; `changes-requested` → the bounded revise
loop below; `draft` → keep waiting; `superseded` → re-plan (step 5) against the successor.

Print these commands and wait for a human to run them — a human-submitted approve review on the plan
is the approval route on this surface:

```bash
rdm review start --on plan/<plan-slug> --no-edit --project <PROJECT>
rdm review comment <id> --quote "<exact text>" --body "<feedback>" --no-edit --project <PROJECT>
rdm review submit <id> --verdict approve --no-edit --project <PROJECT>
```

**You MUST NOT** inspect who authored the review: rdm flips plan status on **any** `rdm review
submit` against a `plan/<slug>` target, so a human's approve and a review-tool-recorded approve are
the *same write* and this *same read* sees both. An author or provenance check here would break the
human/agent symmetry this design depends on.

Poll with a bounded number of attempts, logging one visible line per attempt. On exhaustion park
`blocked` with reason `[plan] plan approval not recorded on plan/<plan-slug>` — **never** proceed on
an unapproved plan, and **never** treat `draft` as `changes-requested`.

**`changes-requested` → revise:** if `planReviseCount` is below `--max-plan-revise`, increment it and
run the `revise` loop against the plan review — a plan review's comments are plan-**document**
comments, which is exactly what that skill is for — then re-read this same `rdm plan show`. On
exhaustion park `blocked` with `[plan] plan-revise budget exhausted; open review(s): <ids>; plan:
plan/<plan-slug>`.

### 7. `--plan-only` ends here

Report `outcome: 'reviewed'` with `writesCompletion: false`, having written **no** status and **no**
change review.

### 8. Dispatch the implementer subagent

**Declare** it, then dispatch **one** `Agent` subagent with `model: <models.implement>`, the approved
plan body verbatim, the item body, and `identity.path` as its working directory. Require it to
commit in that worktree and return the commit SHA. Follow the `--permission-mode auto` rules below.
**You MUST NOT** implement inline.

**Self-check before proceeding:** confirm the implementer returned, then re-run `rdm review source
--on <item>` and restate the pinned `path`/`branch`/`base` — any change to `repository`, `path`,
`branch` or `base` is an **escalation**, never a retry. `head` is expected to have moved; record the
new one.

### 9. Verify in the pinned checkout

1. `rdm verify run --item <item> --project <PROJECT> --format json`
   - exit **0** → pass; record `{ command, exitCode: 0, tail }`.
   - exit **1** → a blocking **rework** finding (not an escalation).
   - exit **2** → `resolved: false`, i.e. no `dispatch.verify` is configured. **This is not a
     failure.** Fall through to 2.
2. Fallback: read the command from the **approved** plan (`rdm plan show --format json` → body → the
   `## Verification command` heading). Refuse a multi-line value the way `rdm verify run` does. Then
   run it directly through Bash with `cd <identity.path>`, recording `{ command, exitCode, tail }`
   with the tail bounded to the last **4000** characters, so both routes report the same shape.
3. Neither source supplies a command → **unresolved escalation**: park `blocked` with `[code] no
   verification command resolved from dispatch.verify or plan/<plan-slug>`. An unverified pass is
   never reported as a pass.

A nonzero exit from either route is a blocking rework finding bounded by `--max-code-rework` — no new
OUTCOME value. Carry `verification` into the next step so it reaches the persisted review.

### 10. Invoke the code review — in THIS session

Invoke the **`rdm:rdm-wf-review-refute-fix` Workflow**
(`rdm:rdm-wf-review-refute-fix`, installed by the `rdm` plugin)
via the Workflow tool with
`{ mode: 'code', roadmap, phase, persist: true, implements: 'plan/<plan-slug>', gate: false, rdmBin,
project }` (task mode: `task` in place of `roadmap`/`phase`); pass `args` as a JSON object, never a
stringified value, and include the source identity you pinned in step 9 (`source`, `base`,
`expectedHead`, `expectedBranch`) — a path, two SHAs and a branch name, which is everything the
engine needs, because each reviewer runs `rdm review source` itself to reach the diff.

Block for its returned result. **The engine reads nothing and writes nothing**: it dispatches finder
and refuter agents and no others. `persist: true` therefore returns the ladder as `persistCommands` /
`persistScript` rather than writing anything — **you** run that Bash yourself, in one session, and
report its exit status. It prints `reviewId=<id>` on success. If one `review comment` line is refused
for its anchor, re-run that line only with `--path`, `--quote` and `--occurrence` removed; if
`review start` itself is refused, park rather than choosing another target. `gate: false` keeps the
status write here, in step 13, where a refusal can be surfaced.

Optionally add `reviewers: [...]` — **your** judgment about what this diff touches. Omit the key to
run every code reviewer; that is the safe default and the right choice when you are unsure. An
unrecognised name is dropped silently and shows as a gap in `reviewCoverage`; nothing rejects a thin
set, so an under-reviewed diff is a visible choice rather than an error. The code reviewers are:

| reviewer | what it is for | include it when |
|---|---|---|
| `ac` | per-criterion PASS/FAIL/PARTIAL against the item's acceptance criteria | always — it is the structured channel the outcome reads |
| `correctness` | logic bugs, edge cases, error paths | always |
| `tests` | do tests exist and cover the behaviour? | the change adds or alters non-trivial logic |
| `architecture` | does logic live where the layering contract puts it? | the change spans more than one module or layer |
| `api-docs` | do public items carry the required documentation? | the change adds or alters a public API item |
| `changelog` | is there a user-perspective entry in the same commit? | the change is user-facing |
| `security` | can an attacker now do something they should not? | the change touches auth, input parsing, paths, subprocesses, secrets, deserialization, or network code |

Read the returned object and obey it:

- `outcome: 'escalated'`, or a non-empty `failure` — including `'required review evidence is
  incomplete'` — is a **park**. **You
  MUST NOT** write `reviewed` on it.
- The outcome is classified **before** the persist ladder is built and is never recomposed
  afterwards — an anchor that will not land is not a verdict.
- Append the `reviewId` the ladder printed to `reviewIds`.

### 11. Triage — ONE procedure, whatever the review's origin

Build the work list from **the item's review set** (step 2's recipe (a)+(b)+(c), re-read now)
**plus** the ids the review returned, deduped by id. Every id on the list must trace back to
`<item>` — its own document, one of its `plan/<slug>` documents, or a `change/<sha>` recorded in one
of their `change_reviews[]`. **You MUST NOT** build this list from `rdm review requests`: that queue
is project-wide and unfiltered, so a literal read of it hands you another item's comments and you
would "fix" them in this item's worktree, under this item's pinned identity, with an
`--applied-commit` that belongs to the wrong change. If an id's target does not resolve to `<item>`,
**drop it** — the failure mode is "triaging another item's review". Then, per review, read it
**once**:

```bash
rdm review show <id> --project <PROJECT> --format json
```

and drive **only** off its comment array — `anchor`, `path`, `source_link`, `resolution.state`. **You
MUST NOT** add an author or provenance check. A human review created with `rdm review start --on
change/<sha>`, `rdm review comment --path <p> --quote "<exact text>"` and `rdm review submit <id>
--verdict request-changes` produces the *same document shape* the review tool writes, so there is no
second branch. (`--path` is required with `--quote` on a change target, and a path containing `@` or
`#` is refused — the `rdm:src/<path>@<rev>#L<n>` grammar reserves both.)

Per comment, run this numbered checklist:

1. **Declare** the decision, the route, and the reply text you intend to record.
2. **Classify and act:**
   - **SOURCE comment** (carries a `path` — a file-quote anchor into the diff): dispatch an
     implementer `Agent` subagent with `model: <models.implement>` in `identity.path` with the
     comment body and its `source_link` permalink; require a commit and its SHA. **You MUST NOT
     route a source comment to `revise`**: that skill edits plan-repo document bodies and its
     `--applied-commit` is a plan-repo SHA, so it cannot carry source-commit provenance.
   - **PLAN-DOCUMENT comment** (on a plan-repo document target): run the `revise` loop, keeping
     its plan-repo `--applied-commit` semantics intact.
   - **`wont-fix`**: a reasoned `--reply` is mandatory, not optional.
   - **Ungraded finding** — the comment's header carries `unrefutedReason: budget`, meaning it was
     never refuted because the per-unit refutation budget ran out: its reply MUST say so explicitly
     and give your own reasoning for the decision taken, on either branch.
   - **Degraded anchor** — a comment that *carries* an anchor which no longer resolves is read as
     whole-document feedback and its reply MUST state that the anchor did not resolve; read the
     reviewed-side body with `--at <created_commit>` before deciding. A comment authored deliberately
     with **no** `--quote` carries no anchor at all: that is a valid whole-document comment, **not**
     degradation, and MUST NOT be reported as anchor loss.
3. **Resolve**, with a reply on **both** branches:
   ```bash
   rdm review update <id> --comment <n> --status addressed --applied-commit <sha> --reply "…" --project <PROJECT>
   rdm review update <id> --comment <n> --status wont-fix --reply "<why>" --project <PROJECT>
   ```
   A SOURCE reply MUST embed the pinned permalink `rdm:src/<path>@<applied-commit>#L<line>`.
4. **Verify**: `rdm review show <id> --format json` shows that comment with a terminal `status` and a
   non-empty `reply`.

Under `--interactive`, **present each decision (1) for confirmation before running (3)**; otherwise
apply them without pausing. Nothing else differs — **you MUST NOT** fork the triage procedure, skip a
confirmation in `--interactive`, or add one without it.

Close each review and land the batch:

```bash
rdm review update <id> --state addressed --project <PROJECT>
rdm link check --on <item> --project <PROJECT>
rdm commit -m "chore(plan): triage review <id>"
```

Then confirm, from the records and not from memory: every comment terminal with a non-empty reply,
each `addressed` one carrying an `applied_commit`, the review `addressed`, `rdm link check` exit 0,
and **the item's review set** (step 2's recipe, re-read) holding **no** submitted request-changes
review — i.e. every `rdm review list --on <target> --state submitted --verdict request-changes` over
the item's own targets comes back empty. A non-empty result is a **refusal to proceed** to the
terminal write, not a warning. Another item's pending review in the project-wide `rdm review
requests` queue is **not** a reason to block this write.

Bounded by `--max-code-rework`; on exhaustion park `blocked` with `[code] rework budget exhausted;
unresolved comments on review <id>` so the review id is in the reason.

### 12. Re-review at the post-triage HEAD

Every source fix moves HEAD, and the core gate requires an **approving** change review recorded at
the HEAD the write observes. Triage only moves a review to `addressed`; it never changes a verdict.
So after the **last** source-touching act: re-run `rdm review source --on <item>`, restate the new
pinned head, and re-run step 10. Writing `reviewed` off an `addressed` request-changes review is
refused for want of an approving change review; writing it off an approve recorded at an older head
is refused as stale, and that refusal names both SHAs.

### 13. The terminal write — through the gate, never around it

```bash
rdm phase update <phase> --status reviewed \
  --source <identity.path> --base <identity.base> \
  --expected-head <identity.head> --expected-branch <identity.branch> \
  --no-edit --roadmap <slug> --project <PROJECT>
rdm commit -m "chore(plan): finalize <item>"
```

(task form: `rdm task update <slug> --status reviewed …`.) The explicit `--source`/`--base`/
`--expected-head`/`--expected-branch` binding is required: run from a cwd that is not a distinct
project repo and the worktree probe degrades to *no probe*, silently skipping the cleanliness
precondition.

**You MUST NOT bypass the gate.** rdm has an operator-only flag that waives the gate's *record*
preconditions; it is refused outright unless the gate is enforcing, and this procedure has no case for
it. The named failure mode is **forging the terminal write**: a bypass here would record a `reviewed`
the records do not support, which is precisely what the gate exists to prevent. This prose
deliberately does not spell that flag, so a mechanical scan over every agent-facing surface can keep
asserting that no surface emits it.

On a nonzero exit, capture stderr **verbatim** — each refusal already names the missing record *and*
the command that would create it — then park:

```bash
rdm phase update <phase> --status blocked --reason "[code] reviewed-gate refused: <verbatim refusal>" --no-edit --roadmap <slug> --project <PROJECT>
```

and return `outcome: 'escalated'` with that same text in `reason`. Surface a dirty-worktree refusal
verbatim rather than cleaning the tree: worktrees are per-roadmap, so a sibling phase's uncommitted
edit trips it even when this item is clean, and `git reset --hard`/`git clean -fdx`/`git stash -u`
are denied under `--permission-mode auto` anyway.

On success, read the status back (`rdm phase show --format json` → `status` is the reviewed status)
and treat a mismatch as an escalation, not a success.

### 14. Return the OUTCOME

Produce the object from the Contract above as your final message, `planId` and `reviewIds` included.

## Why there is no plan-review Workflow call here

This surface's plan gate is the **human-submitted approve review** of step 6, not a workflow. The
plan-review engine (`rdm-wf-plan-review`) is deliberately **not** emitted by `rdm agent-config claude
--skills`: the only engine that ships is the `rdm:rdm-wf-review-refute-fix` one named above, so a skill
that instructed invoking it would reference a file absent from its own tree and fail at the exact
point its contract depends on. This follows the precedent already set for the
`rdm-wf-estimate` pre-pass — see
`docs/workflow-vs-prose-boundary.md`. Shipping that engine downstream is separate work; until it
lands, step 6's human approve review is the whole gate, and it satisfies the same single
`rdm plan show` read a workflow-recorded approve would.

## Safe operations under --permission-mode auto

Unattended runs (launched with `--permission-mode auto`, or `bypassPermissions` in a sandbox) stall
forever if they trip the auto-mode destructive-action classifier. This procedure and every subagent
it dispatches MUST follow these rules:

- **Modify existing files with `Edit`, never `Write`.** A `Write` over an existing tracked file trips
  the classifier. Reserve `Write` for new files.
- **Never run `git stash -u`, `git reset --hard`, or `git clean -fdx`.** They are classified as
  irreversible local destruction and denied. Per-roadmap worktree isolation makes whole-tree resets
  unnecessary; commit a `wip:` commit on the branch instead.
- **Never run `rdm discard --force` in a shared plan repo.** It resets other sessions' uncommitted
  work. Commit each batch immediately instead.

## Escalation protocol

- **Routine findings never escalate.** Bugs, missing tests, doc gaps are fixed in triage or filed as
  a task; they never reach the user mid-run.
- **Decisions and blockers escalate.** Ambiguous or untestable acceptance criteria, an architectural
  decision with no clear default, an exhausted plan-revise or rework budget, an unresolved
  verification command, a gate refusal, or a changed checkout identity.
- **Park, don't interrupt.** Record the escalation by setting the item `blocked` with a stage-tagged
  reason (`[plan]` or `[code]`) naming the open review ids and the plan, so a later session
  reconstructs the pending work from the item's review set (step 2's recipe) alone. The user reviews
  the whole cross-item queue at once with `rdm review blocked --project <PROJECT>`.

## Resolving `rdmBin` (plugin install)

This skill was installed from the `rdm` plugin, so there is no repo-local build path to assume. Resolve the `rdmBin` argument in this order and use the first that exists:

1. an explicitly supplied `--rdm-bin <path>`;
2. the `RDM_BIN` environment variable;
3. a plain `rdm` on `PATH`.

If none resolves, stop and report: `rdm binary not found. Install rdm, then set RDM_BIN=/path/to/rdm, put rdm on your PATH, or pass --rdm-bin /path/to/rdm.` Never guess a path, and never invoke a workflow without one.
