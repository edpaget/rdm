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
| (b) | A `change/` review with verdict `approve` records `implements` pointing at that plan and matches the observed checkout HEAD | `Error::GateNoApprovedChangeReview` |
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
review afterwards does not un-happen it. When a source checkout is
observable, its full HEAD must equal the review's pinned head. A missing or stale
head cannot approve. The gate searches all approving records for a matching one,
so an older stale review does not hide a later matching review.

Several approved plans can implement one item. (a) is satisfied by any of them;
(b) must find an approving review naming the **same** plan. When it finds none,
the refusal lists every candidate plan it checked (the
`Error::ReviewImplementsAmbiguous` precedent).

### Explicit source binding

`rdm review source --on phase/<roadmap>/<stem>` resolves the existing shared
roadmap checkout without creating one. A task can use its existing task checkout
or explicitly bind a registered shared checkout with `--source <path> --base <rev>`.
The same core selection policy and observed head feed status gates. Phase/task
updates accept `--source`, `--base`, `--expected-head`, `--expected-branch`, and
`--no-code`; explicit requests fail closed if inspection is unavailable, including
non-git builds. Default legacy no-source behavior below remains unchanged.

Source-bound needs-review writes stamp the resolved head/branch even when invoked
from another checkout. `review pending --format json` exposes `review_sha` and
`branch` for readback. Revalidation detects drift at workflow boundaries; git and
the plan Store do not share an atomic transaction.

### Fail-closed probe semantics for (c)

Three distinct cases, and the distinction is load-bearing:

- **No probe supplied** (the HTTP server, a unit test) — (c) is skipped. A
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

A build without rdm-cli's optional `git` feature is the same case for the same
reason: it cannot resolve a worktree at all, so (c) is never applicable there.
`commands::build_gate_probe` is the single place that split is spelled out —
`#[cfg(feature = "git")]` discovery on one side, an unconditional `None` on the
other — so the `phase update` and `task update` arms stay feature-agnostic and
the two builds cannot drift in *when* the gate enforces. That is the contract
the `commands::GateProbe` alias documents, and the CI feature-matrix step
(`cargo check -p rdm-cli --no-default-features …`) is what keeps it honest: the
default-feature `cargo clippy`/`cargo nextest run` gate cannot see a
`git`-only call reached from a feature-agnostic call site.

### Worktrees are per-roadmap

`rdm worktree` keys a worktree to a **roadmap**, shared by all of its phases
(obsolete per-phase checkouts are never preferred or used as a fallback). A sibling phase's
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

**That rule lives in core, once.** `rdm_core::config::resolve_reviewed_gate`
(env value + config → bool) is the whole precedence, and
`rdm_core::config::reviewed_gate_enabled_at` is the plan-root convenience over
it for callers that hold no merged `Config`. `rdm-cli`'s
`paths::resolve_reviewed_gate` and `rdm-server`'s
`state::reviewed_gate_enabled` are both one-line delegations to those. This is
CLAUDE.md's layering contract applied to a config key: two interfaces
re-deriving "is the gate on?" would drift — and the first draft of this phase
proved it, because the server-side copy silently lacked the
`RDM_REVIEWED_GATE` override the CLI had. Any future change to the precedence
now happens in exactly one place.

A missing or malformed `rdm.toml` resolves to `false` rather than erroring: an
unreadable config must never be the thing that *enables* a gate. An invalid
`RDM_REVIEWED_GATE` still errors, on every surface.

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
asserted mechanically by two harnesses, both behind planted-mutation
self-tests:

- `scripts/verify-skill-autopilot.sh` § 5 greps the autopilot surfaces (the
  local skill plus both shipped templates).
- `scripts/verify-workflow-dispatch.sh` § 11 greps the **orchestrator**
  surfaces — and does so by *discovery*, not from a hand-maintained list. It
  walks every agent-facing instruction surface in the repo (`.claude/workflows`
  including `lib/`, `.claude/skills`, `.claude/agents`,
  `rdm-core/src/templates`, and the checked-in `plugins/` tree) and refuses if
  any of them mentions the flag.

The discovery shape is the point. A later phase that introduces a new
orchestrator — whatever it is named, and whether it is a workflow script, a
skill, or a shipped template — is covered the moment its files land, with no
edit to the harness and no third copy of the check to write. § 11d proves that
property directly: it plants a deliberately non-existent
`.claude/skills/rdm-orchestrate/SKILL.md` and
`.claude/workflows/rdm-wf-orchestrate.js` in a scratch tree and asserts both are
caught. § 11a holds the floor from the other side, failing if discovery ever
finds implausibly few surfaces or misses any of the four dispatch ones.

Docs and Rust sources are deliberately out of scope — this file documents the
flag and `rdm-cli` implements it. What may never mention it is anything an
autonomous agent reads as instructions.

## Design record: gated sibling entries, not a required parameter

A measurement of the tree — `grep -rn` over the three primitives *and* their
gated siblings, which together are the whole population a required `gate`
parameter would have touched — finds **176 matches = 8 definitions + 168 call
sites across 21 files**, of which only **11 are production** and **157 are
test**. (The 8 "definitions" include `rdm-server`'s own `update_phase` /
`update_task` handler functions, which share the names.) Adding a trailing
`gate` parameter to the three public primitives would therefore have required
168 mechanical edits, 157 of them in test code, producing an unreviewable diff
and a one-shot `clippy -D warnings` / `nextest` cliff across five crates.

**Rejected.** Instead the three public signatures stay byte-identical and gain
gated siblings:

- `ops::phase::update_phase_gated`
- `ops::phase::update_phase_with_estimate_gated`
- `ops::task::update_task_gated`

Each takes the same argument list plus a trailing `gate: &ReviewedGate<'_>`.
Blast radius: **4 production call-site moves, 0 test edits** — confirmed by
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

## Coverage

The gate is enforced on three surfaces, and each is covered where it can
actually fail rather than only where its call site can be grepped:

| Surface | Coverage |
|---|---|
| the rule itself | `rdm-core/tests/gate.rs` — every branch, the fixed order, and the fail-closed probe cases against `MemoryStore` + `MemoryWorktreeProbe` |
| `rdm-cli` | `rdm-cli/tests/cli_gate.rs` — the full ladder end to end through the real binary, against a temp plan repo and a real `rdm worktree add` worktree |
| `rdm-server` | `rdm-server/tests/reviewed_gate.rs` — `PATCH` to `status: reviewed` refused **409** per precondition and allowed once the records exist, for phases and tasks; plus the opt-in and other-transitions-unaffected cases |
| the worktree probe | `rdm-git/src/worktree.rs` tests — the per-phase-beats-roadmap candidate ordering, the task branch, dirty-path reporting, benign misses, and `status_porcelain_at` erroring outside a repo |
| the threading | `scripts/verify-reviewed-gate.sh` § A–C — the static allowlist described above |
| the feature split | `scripts/verify-reviewed-gate.sh` § D — the probe is built in ONE feature-split place (`commands::build_gate_probe`), so neither update arm names `rdm_git::` and both compile with `git` off; CI's feature-matrix step is the dynamic half |

The `rdm-server` row exists because a static call-site grep cannot see a gate
that is wired but not enforcing: a wrong config key, a wrong file, or an error
variant falling through to the wrong HTTP status would all leave the grep
green. It asserts the refusal an HTTP caller actually receives.

## See also

- [`docs/verify-gate.md`](verify-gate.md) — the `dispatch.verify` command and
  the `rdm verify run` / `rdm verify resolve` CLI surface over it.
- [`docs/plan-review-gate-policy.md`](plan-review-gate-policy.md) — the
  plan-review gate, and the recorded `planned`-status decision.
- [`docs/lost-update-evaluation.md`](lost-update-evaluation.md) — the
  content-digest precondition the single-write requirement above protects.
