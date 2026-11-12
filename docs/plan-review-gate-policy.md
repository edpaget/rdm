# Plan review: the retired tag gate, and the `planned` status decision

This file keeps its path because code and other docs point at the `planned` status
decision below. It used to record the policy of the plan-review tag gate. That gate is
retired, and what is left of it is the first section.

## The `needs-plan-review` tag gate is retired

rdm does not stamp, clear or read a `needs-plan-review` tag.

- **Creation stamps nothing.** `roadmap create`, `phase create` and `task create` write
  exactly the tags passed with `--tags`.
- **Reviewing an item's document is manual, and it leaves a review rather than a
  marker.** A person or a session runs `rdm-plan-review` against a roadmap, a phase or a
  task. The engine (`rdm-wf-plan-review`) returns each unit's `outcome` and a persist
  ladder, and running the ladder records the review. Whether a document was reviewed is
  read from `rdm review list --on <ref>`.
- **The `plan_review` config key and `RDM_PLAN_REVIEW` have no effect.** A config file
  that still sets the key (at the top level of `rdm.toml`, under `[projects.<name>]`, or
  in the global config) loads, and the key is ignored without a warning. The variable is
  never read, whatever its value. `rdm config get plan_review` and `rdm config set
  plan_review <value>` are refused with a message naming the key as retired, and `rdm
  config list` has no row for it. A later `rdm config set` of another key rewrites the
  file without the line, which is what happens to any key rdm does not know.
- **`rdm task create --no-plan-review` is accepted and does nothing**, so a caller that
  still passes it keeps working.
- **An item that already carries the tag keeps it as an ordinary tag.** Nothing strips it
  and there is no migration. It shows in `show`, `list` and `search --tag
  needs-plan-review`, survives every update that does not rewrite the tag list, and goes
  away with `--remove-tag needs-plan-review` or a `--tags` write that omits it.

---

## The `planned` status decision

**Decision: rdm adds NO `planned` status.** Phase statuses stay exactly the
seven they have always been (`not-started`, `in-progress`, `needs-review`,
`reviewed`, `done`, `blocked`, `wont-fix`), and task statuses their seven
(`open` in place of `not-started`).

### What was asked

The `agent-orchestrated-dispatch` roadmap's phase 5 was told to decide, *from
the evidence of running phases 2–4 against a real roadmap*, whether an explicit
`planned` status is needed between `in-progress` and `reviewed` — with a stated
default of no new status, on the grounds that the `reviewed` gate
(`docs/core-enforced-gates.md`) transitively blocks implementation-without-plan
from ever completing.

### The evidence

1. **Plans already carry their own lifecycle.** A plan document has a
   `PlanStatus` of `draft | approved | changes-requested | superseded`, derived
   from reviews rather than set by a flag, and queryable directly:
   `rdm plan list --implements phase/<roadmap>/<stem>`. "This item has been
   planned" is already a fact the plan repo records — on the plan, where it
   belongs.

2. **Phases 2–4 never needed it.** Nothing in running those phases produced a
   moment where "planned but not implemented" was inexpressible. Phase 4
   (`phase-4-change-review-target`) reached `blocked` with its rework budget
   exhausted; that outcome was expressible in the existing vocabulary, and a
   `planned` rung would not have changed what an observer learned from it.

3. **The `reviewed` gate makes the rung unnecessary.** `rdm-core/tests/gate.rs`
   demonstrates that an item with no approved plan *cannot reach `reviewed` at
   all* once `gates.reviewed` is on. Implementation-without-plan is blocked at
   the exit, transitively, so there is nothing left for an intermediate status
   to prevent.

### The counter-argument, and why it does not carry

An observer watching an item sit at `in-progress` cannot tell whether it is
being planned or being implemented. That is a real loss of resolution.

It does not carry, because the signal the observer wants already exists and is
already queryable: the plan document's own `PlanStatus`. Adding a seventh phase
status would make the same fact recoverable from two places that can disagree —
an item stamped `planned` whose plan is still `draft`, or an `in-progress` item
whose plan is `approved`. That is precisely the second-source-of-truth
anti-pattern `PlanStatus`'s own rustdoc rejects for plans ("never set directly
by a status flag: it is **derived from reviews**"). Having rejected it there, it
would be incoherent to introduce it here.

### Enforced by code, not by this prose

Two independent tripwires make reversing this decision a deliberate,
test-visible act rather than a silent one:

- `rdm-core/src/model.rs`'s `phase_status_variants_are_exactly_the_seven_recorded`
  and `task_status_variants_are_exactly_the_seven_recorded` match exhaustively
  over the status enums and assert the recorded seven-element list. A new
  variant fails to compile at the match first, then fails the assertion — and
  both failure messages point back at this section.
- `tests/golden/describe.json` enumerates the same seven `enum_values` for
  `phase.status` and `task.status`, gated by the `golden_json` nextest binary.

(No test asserts on this document's prose. The decision is enforced by the code;
the reasoning is enforced by review.)

### If a later phase reverses it

Should `planned` ever be adopted, these are the touch points — named here so
the contingent half of the question is answered rather than deferred:

- `rdm-core/src/ops/next.rs` — `NextPhase.status` and the actionability
  predicate in `next_actionable` must decide whether a `planned` phase is the
  next actionable one.
- `rdm-core/src/hook.rs` plus `rdm hook post-merge` / `post-commit` — the
  `Done:` directive's terminal write, and whether `planned` is a legal
  predecessor of `done`.
- `rdm-tui` — status rendering and the status filters.
