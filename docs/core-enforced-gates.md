# Core-enforced gates

Canonical for rdm's **write-time** gates: rules `rdm-core` enforces at the
moment a status is written, rather than rules a prose instruction asks an agent
to follow.

Today there is exactly one: the **`reviewed` transition gate**.

## Why this is in core

`docs/subagent-dispatch-enforcement.md` records the operator's decision. Weaker
models collapse the planner, reviewer and implementer roles inline, and prose
cannot prevent it — a skipped gate leaves no trace, and the phase completes
anyway. If rdm *refuses the terminal write* unless the review records exist, a
model that skipped a gate cannot complete the phase without forging a review,
and the trail shows exactly what happened.

The same move pulls three invariants `.claude/workflows/lib/dispatch-phase.mjs`
enforces fail-closed in JS — the verify gate, the worktree-clean assertion, and
"no `Done:` from the engine" — into nextest-covered Rust that every host
inherits, rather than only the Claude lane. Until the engine is retired the two
enforcements coexist; `parseWorktreeStatus` in that JS file remains the
reference semantics `rdm_core::worktree::parse_porcelain` matches.

## The three preconditions

`rdm phase update --status reviewed` and `rdm task update --status reviewed`
refuse unless all three hold:

| | Precondition | Refusal |
|---|---|---|
| (a) | An `approved` implementation plan `implements` this item | `Error::GateNoApprovedPlan` |
| (b) | A `change/` review with verdict `approve` records `implements` pointing at that plan | `Error::GateNoApprovedChangeReview` |
| (c) | The item's worktree, if `rdm worktree` knows one, has a clean `git status --porcelain` | `Error::GateWorktreeDirty` |

Each refusal names the missing record **and** the command that would create it.

### The evaluation order is fixed

(a) → (b) → (c), always, so the first failure reported is the most specific one
the operator can act on. Reporting (c) before (a) would tell someone to clean a
worktree when the real problem is that no plan exists. `ops::plan::create_plan`
documents its own fixed order for the same reason.

A `superseded` plan is not `approved`, so (a) fails first and the operator is
told to write a new plan rather than chasing a change review on a dead one.

### What counts for (b)

A change review counts when `verdict == Approve` **and** `state != Draft`. A
draft never carries a verdict. An `addressed` or `dismissed` review that *was*
submitted with `approve` still counts — the approval happened, and closing the
review afterwards does not un-happen it.

Several approved plans can implement one item. (a) is satisfied by any of them;
(b) must find an approving review naming the **same** plan. When it finds none,
the refusal lists every candidate plan it checked (the
`Error::ReviewImplementsAmbiguous` precedent).

### Fail-closed probe semantics for (c)

Three distinct cases, and the distinction is load-bearing:

- **No probe supplied** (the HTTP server, MCP, a unit test) — (c) is skipped. A
  request carries no project checkout to inspect.
- **Probe returns `Ok(None)`** — rdm manages no worktree for this item. This is
  the "if `rdm worktree` knows one" escape clause. (c) is skipped.
- **Probe returns `Err`, or returns output rdm cannot parse** —
  `Error::GateWorktreeUnobservable`. **An unobservable worktree is never a
  clean one.**

Running `rdm phase update --status reviewed` from a cwd that is not a distinct
project repo degrades to no probe, and therefore skips (c). That is a
deliberate fail-open on (c) alone — (a) and (b) still apply — recorded here so
it reads as a decision rather than an accident.

### Worktrees are per-roadmap

`rdm worktree` keys a worktree to a **roadmap**, shared by all of its phases
(with a per-phase worktree preferred when one exists). A sibling phase's
uncommitted edit therefore blocks this phase's `reviewed` transition. That
matches `docs/verify-gate.md` § 8 ("pre-existing dirt the dispatch did not
create will now force rework"); the refusal names the dirty paths so an
operator can tell instantly that the dirt was not theirs.

## Opt-in: `gates.reviewed`

The gate is **off by default** and enabled per plan repo:

```bash
rdm config set gates.reviewed true     # repo-only; --global is refused
```

Resolution: `RDM_REVIEWED_GATE` env → `gates.reviewed` in `rdm.toml` → `false`.
The env var is a loud override, not a fuzzy boolean: `"1"`, `"yes"`, `"True"`
and `""` all error rather than silently resolving to `false` and quietly
disabling the gate — mirroring `RDM_PLAN_REVIEW`.

Defaulting off is not timidity. It is what lets the gate ship without changing
behavior for a single existing plan repo, and what keeps rdm's own hermetic
harnesses (`verify-skill-autopilot.sh`, `verify-workflow-do-auto.sh` and
`-task.sh`) green with comment-only edits: their seeded repos set no
`gates.reviewed` key, so the gate is `NotApplicable` there.

## The operator override

```bash
rdm phase update <stem> --status reviewed --override-gate "<reason>" --roadmap <r>
```

- It waives **(a) and (b) only**. Precondition (c) still applies — a dirty
  worktree means the reviewed code is not the committed code, which no operator
  intent can make untrue. The `GateWorktreeDirty` message says so explicitly,
  or an operator reads the refusal as a bug in the override.
- An empty or whitespace-only reason is refused: an audit trail that explains
  nothing is worse than none.
- It requires `--status reviewed`. Passing it with any other status is
  **rejected**, not silently ignored — otherwise an operator records a bypass
  on an item that was never gated.
- It requires the gate to be **enforcing**. Passing it while `gates.reviewed`
  resolves to `false` (the shipped default) is likewise **rejected**
  (`GateOverrideGateDisabled`), for the same reason: there is nothing to
  bypass, so honoring the request would silently discard the reason and actor
  the operator supplied and leave `rdm phase show` disagreeing with what they
  asked for. The refusal names both remedies — drop the flag, or
  `rdm config set gates.reviewed true`. This is the one thing a *disabled* gate
  still does; a write with no override is unaffected. The alternative — stamping
  a `gate_override` block on an item that was never gated — was rejected because
  the record would then mean "a bypass happened" on a write that bypassed
  nothing.
- The reason, the actor (resolved exactly as a review's author is:
  `RDM_REVIEW_AUTHOR` → `$USER`/`$USERNAME`) and the date are recorded in a
  `gate_override` frontmatter block, surfaced by `rdm phase show` /
  `rdm task show` in both text and JSON.
- It is **cleared whenever the item leaves `reviewed`**, including through the
  ungated primitives, so a stale override can never authorize a later
  `reviewed` write. A *satisfied* gate also clears it: once real records exist,
  the bypass is no longer what authorizes the status and must stop claiming
  otherwise.

**The override is for humans.** No rdm skill or workflow emits it, and that is
asserted mechanically: `scripts/verify-skill-autopilot.sh` § 5 and
`scripts/verify-workflow-dispatch.sh` § 11 each grep their surfaces for the
flag, behind a planted-mutation self-test.

## Design record: gated sibling entries, not a required parameter

A measurement of the tree (`grep -rn "update_phase(\|update_phase_with_estimate(\|update_task("`)
found **160 matches = 5 definitions + 155 call sites across 20 files**, of
which only **13 are production** and **142 are test**. Adding a trailing `gate`
parameter to the three public primitives would therefore have required 155
mechanical edits, 142 of them in test code, producing an unreviewable diff and
a one-shot `clippy -D warnings` / `nextest` cliff across six crates.

**Rejected.** Instead the three public signatures stay byte-identical and gain
gated siblings:

- `ops::phase::update_phase_gated`
- `ops::phase::update_phase_with_estimate_gated`
- `ops::task::update_task_gated`

Each takes the same argument list plus a trailing `gate: &ReviewedGate<'_>`.
Blast radius: **6 production call-site moves, 0 test edits** — confirmed by
`cargo build --workspace` being green after the core change and *before* any
caller was touched.

`update_phase_with_estimate` does **not** delegate to `update_phase` (both call
`apply_phase_update` independently), so each needs its own gated sibling.

### Exactly one store write per call

Each gated entry is a thin shell over a shared private `*_inner(…,
gate_override: GateOverrideUpdate)`. A wrapper that called the ungated
primitive and then wrote `gate_override` separately would journal the path
twice and trip `rdm-store-fs`'s content-digest optimistic-concurrency
precondition (`docs/lost-update-evaluation.md`). Single write is a hard
requirement, not a style preference.

The gate is evaluated **before** the load+write, so a refusal leaves the file
untouched.

### The ungated allowlist

Seven production call sites stay on the unchecked primitives, deliberately.
None can ever write `reviewed`:

| Site | Status written | Why |
|---|---|---|
| `rdm-cli/src/commands/review.rs` ×2 | `NeedsReview` | the `rdm review restamp` path; gating it would be inert and would put `scripts/verify-worktree-review-loop.sh` at risk |
| `rdm-cli/src/commands/mod.rs` ×2 | `Done` | the `Done:` post-merge/post-commit hook path, contractually exit-0 and bounded — it must never acquire a failure mode |
| `rdm-core/src/ops/task.rs` ×3 | `Done` / `WontFix` / `None` | `consolidate_task_into_roadmap` and `merge_tasks`, core-internal |

`scripts/verify-reviewed-gate.sh` is what holds this boundary: it asserts every
non-test call to an ungated primitive is on the allowlist (with an exact count,
so a new one cannot hide), that every user-facing status-write surface uses a
`_gated` entry, and that each refusal names a remediation — each half behind a
planted-mutation self-test, and the two interlocking so a downgraded `_gated`
call trips both.

**Follow-up (deferred):** rename the ungated primitives to `*_unchecked` once
the 142 test call sites can be swept as a standalone mechanical commit rather
than inside a behavioral phase. The allowlist holds the boundary until then.

## Non-goals

- **`--status done` is not gated.** `done` is written by the `Done:` hook on
  the merge path, where a refusal would violate the hook's exit-0 contract, and
  by the time an item lands its `reviewed` transition has already been gated.
- **The primitives themselves are not gated.** They are deliberately
  unchecked; the allowlist harness, not a runtime check, is what bounds their
  callers. `rdm-core/tests/gate.rs`'s
  `gated_entry_refuses_where_ungated_primitive_writes` asserts the split is
  intentional and visible rather than an oversight.
- **The gate never writes a `Done:` trailer.** The gate checks records and
  nothing else. The trailer's format lives in
  `rdm_core::hook::format_done_directive` (surfaced as `rdm hook done-line`)
  and `rdm-land` remains its only writer.

## See also

- [`docs/verify-gate.md`](verify-gate.md) — the `dispatch.verify` command and
  the `rdm verify run` / `rdm verify resolve` CLI surface over it.
- [`docs/plan-review-gate-policy.md`](plan-review-gate-policy.md) — the
  plan-review gate, and the recorded `planned`-status decision.
- [`docs/lost-update-evaluation.md`](lost-update-evaluation.md) — the
  content-digest precondition the single-write requirement above protects.
