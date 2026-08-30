#!/bin/sh
# Hermetic regression for the dispatch-phase keystone workflow.
#
# dispatch-phase (`.claude/workflows/rdm-wf-dispatch-phase.js`) is the unit of
# autonomous execution for phases and tasks: a deterministic 4-stage pipeline
#   Plan → PlanReview → Implement → CodeReview → OUTCOME
# that returns (phase mode) { roadmap, phase, outcome, status, writesCompletion,
# summary, reason, findings } or (task mode) { task, outcome, status,
# writesCompletion, summary, reason, findings } with outcome ∈ { reviewed, rework,
# escalated } and NEVER emits a land-time completion directive. Its pure decision core lives once in
# `.claude/workflows/lib/dispatch-phase.mjs` and is copied BYTE-IDENTICAL into the
# workflow script (the Workflow runtime cannot import a helper module — see
# docs/workflow-schemas.md § "Import spike"). This harness gates three things:
#
#   1. BEHAVIOR — the pure decision logic, driven in Node with fabricated ranked
#                 finding arrays (zero LLM calls): all three outcome branches
#                 (reviewed / rework / escalated), tier-scaling, determinism, and
#                 that no OUTCOME ever carries a `Done:` directive. Section 1c
#                 additionally DRIVES the two budget-bounded gates
#                 (runPlanGate / runCodeGate) under injected fakes: per-budget
#                 agent-call counts, budget independence, the null-plan and
#                 null-revise short-circuits, budget validation, and the
#                 boundedness proof (a never-clean stage terminates at exactly
#                 `budget + 1` reviews).
#   2. BLOCK DRIFT — the `dispatch-outcome` region is byte-identical between the
#                 lib source of truth and the stamped workflow script (with a
#                 planted-mutation self-test proving the gate is not a no-op).
#   3. STATIC INVARIANTS — grep-based assertions on the workflow source for the
#                 ACs that are structural rather than runtime (no `Done:` line;
#                 no `isolation:` worktree flag but a `worktree add` prompt; both
#                 stamped review markers present; no import/require/nested
#                 workflow() call; distinct planner/implementer/fetch agent labels;
#                 the implementer prompt seeded from phase body + plan doc only,
#                 never the plan-review findings; the marker-scoped loop rules
#                 — no `while` in the driver region, `for` only from an exact
#                 allowlist, "bounded" itself being proven semantically in 1c;
#                 the code gate is the sole, signals-fed canonical review
#                 construction site (AC-CANONICAL-REVIEW); the plan gate is
#                 the sole canonical review construction site, bound as
#                 runPlanGate's only `review:` callback, with its fail-open
#                 no-signals comment intact and no `signals` threaded into it
#                 (AC-CANONICAL-PLAN-REVIEW); and AC-STAMP — the best-effort
#                 in-progress stamp fires right after Stage 0 and before the
#                 plan gate, is guarded by `if (!planOnly)`, and cannot
#                 early-return between its call site and the plan gate).
#   5. DOC AGREEMENT — docs/escalation-protocol.md § Budgets names all four
#                 budgets with exactly the values the two libs declare.
#
# NOTE ON THE DETERMINISTIC MODEL: the pipeline cannot classify a code finding's
# *nature* (the FINDING schema has severity but no fixable/decision flag), so a
# code defect that survives the bounded reworks resolves to `rework`, and
# genuine decisions surface earlier at the plan gate as `escalated`. That is why
# the code stage yields only reviewed|rework here.
#
# Node is used only as a host to unit-test the pure module; it is stdlib-only
# (node:assert), with no package.json / node_modules / third-party packages.
# node is pinned in .mise.toml.
#
# Requires: node (via PATH or `mise exec node --`).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)

LIB="$REPO_ROOT/.claude/workflows/lib/dispatch-phase.mjs"
WF="$REPO_ROOT/.claude/workflows/rdm-wf-dispatch-phase.js"

# Clear rdm-related env vars inherited from the caller's shell for hermeticity.
unset RDM_ROOT RDM_PROJECT RDM_STAGE RDM_FORMAT RDM_PLAN_REPO RDM_PLAN_REPO_TOKEN RDM_PLAN_REPO_PATH 2>/dev/null || true

say() { printf '\n\033[1;34m==>\033[0m %s\n' "$*"; }
fail() {
    printf '\n\033[1;31m[FAIL]\033[0m %s\n' "$*" >&2
    exit 1
}
pass() { printf '\033[1;32m[ok]\033[0m %s\n' "$*"; }

[ -f "$LIB" ] || fail "source module not found: $LIB"
[ -f "$WF" ] || fail "workflow script not found: $WF"

# Resolve a node command: prefer PATH, fall back to the mise-pinned toolchain.
# Fail hard if node is genuinely unavailable (matches the sibling harnesses'
# tool-guard convention — a silent skip would turn this gate into a no-op).
NODE_VIA_MISE=0
if command -v node >/dev/null 2>&1; then
    NODE_VIA_MISE=0
elif command -v mise >/dev/null 2>&1 && mise exec node -- node --version >/dev/null 2>&1; then
    NODE_VIA_MISE=1
else
    fail "node not found on PATH or via 'mise exec node --'. node is pinned in .mise.toml; run 'mise install'."
fi

run_node() {
    if [ "$NODE_VIA_MISE" -eq 1 ]; then
        mise exec node -- node "$@"
    else
        node "$@"
    fi
}

# Parse a workflow script under MODULE semantics and fail on a SyntaxError.
#
# A naive `node --check "$WF"` does NOT work: the file mixes `export` with a
# top-level `return`, so plain --check would false-fail a VALID workflow on the
# legal top-level return (and, on the broken file, does not reliably surface the
# redeclaration). This transform strips the leading `export` and wraps the body
# in an async function so top-level `return`/`await` are legal, while keeping the
# top-level `const meta`/`let meta` in ONE shared scope so a redeclaration is
# still a SyntaxError. Empirically: exit 1 on a duplicate top-level `meta`, exit
# 0 on the valid file.
parse_workflow() {
    {
        echo '(async function(){'
        sed 's/^export //' "$1"
        echo '})'
    } |
        run_node --check --input-type=module -
}

# Distinct `phase: '<Name>',` literals the workflow actually emits (driver agent()
# calls + the inlined review block), one per line, sorted-unique. Real
# agent-option phase names are TitleCase with a trailing comma, which excludes
# prose like the `phase: '<stem-or-number>'` placeholder in the module's doc
# comment AND the review block's lowercase STATUS_MAPPING rows
# (`phase: 'reviewed'`, `phase: 'blocked'`), which are rdm statuses, not
# workflow phases.
emitted_phases() {
    grep -oE "phase: '[A-Z][A-Za-z]*'," "$1" | sed "s/phase: '//;s/',//" | sort -u
}

# Distinct `{ title: '<name>' }` entries declared in the `meta.phases` array. The
# awk window is scoped to the meta.phases array (from `phases: [` to its closing
# `],`) so DIMENSIONS' `title:` literals in the stamped review block are excluded.
declared_phases() {
    awk '/phases: \[/{p=1} p{print} p&&/^  \],?$/{exit}' "$1" |
        grep -oE "title: '[^']+'" | sed "s/title: '//;s/'\$//" | sort -u
}

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

# --- 1. BEHAVIOR -------------------------------------------------------------
say "1. Behavior: all three outcome branches, tier-scaling, bounded loops, determinism"

cat >"$TMP/test.mjs" <<'NODE_TEST'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const libPath = process.argv[2];
const mod = await import(pathToFileURL(libPath).href);
const { hasBlocking, summarizeFindings, classifyOutcome, buildOutcome } = mod;

// Fabricated ranked findings (already in most-severe-first order per severity).
const B = (id) => ({ id, concern: 'x', severity: 'blocking', confidence: 90, what_fails: id });
const C = (id) => ({ id, concern: 'x', severity: 'concern', confidence: 90, what_fails: id });

const SHAPE = ['findings', 'outcome', 'phase', 'reason', 'reviewBudget', 'reviewCoverage', 'roadmap', 'status', 'summary', 'writesCompletion'];

// ============================================================================
// Happy path FIRST — clean plan + clean code → reviewed, full OUTCOME shape.
// ============================================================================
const rev = buildOutcome({
  roadmap: 'rm',
  phase: 'phase-1-foo',
  planFindings: [],
  codeFindings: [],
  codeFindingsAfterRework: [],
  tier: 'medium',
});
assert.equal(rev.outcome, 'reviewed', 'clean plan + clean code → reviewed');
assert.equal(rev.roadmap, 'rm', 'roadmap echoed into OUTCOME');
assert.equal(rev.phase, 'phase-1-foo', 'phase echoed into OUTCOME');
assert.ok(Array.isArray(rev.findings), 'findings is an array');
assert.equal(typeof rev.summary, 'string', 'summary is a string');
assert.deepEqual(
  Object.keys(rev).sort(),
  SHAPE,
  'OUTCOME is exactly {roadmap,phase,outcome,status,writesCompletion,summary,reason,findings}'
);

// ============================================================================
// Failure-branch fixtures.
// ============================================================================
// Blocking code fixed by the ONE bounded rework → reviewed.
const revRework = buildOutcome({
  roadmap: 'rm',
  phase: 'p',
  planFindings: [],
  codeFindings: [B('bug')],
  codeFindingsAfterRework: [],
  tier: 'medium',
});
assert.equal(revRework.outcome, 'reviewed', 'blocking code fixed by the one rework → reviewed');
// Pin the ternary's TRUE side: a cleared rework must surface the POST-rework
// findings, never the stale pre-rework blockers. Without this, flipping
// `hasBlocking(...) ? codeFindingsAfterRework : codeFindings` survives the suite.
assert.deepEqual(revRework.findings, [], 'reviewed-after-rework surfaces the post-rework findings');
assert.ok(
  !revRework.summary.includes('blocking'),
  'a reviewed summary never names a stale pre-rework blocker'
);

// Blocking code surviving the one rework → rework (budget exhausted).
const rw = buildOutcome({
  roadmap: 'rm',
  phase: 'p',
  planFindings: [],
  codeFindings: [B('bug')],
  codeFindingsAfterRework: [B('bug')],
  tier: 'medium',
});
assert.equal(rw.outcome, 'rework', 'blocking code surviving rework → rework');
assert.deepEqual(rw.findings.map((f) => f.id), ['bug'], 'rework surfaces the post-rework code findings');

// Blocking plan → escalated regardless of the code arrays (never implements).
const esc = buildOutcome({
  roadmap: 'rm',
  phase: 'p',
  planFindings: [B('vague')],
  codeFindings: [B('bug')],
  codeFindingsAfterRework: [],
  tier: 'medium',
});
assert.equal(esc.outcome, 'escalated', 'blocking plan → escalated regardless of code');
assert.deepEqual(esc.findings.map((f) => f.id), ['vague'], 'escalated surfaces the plan findings');

// fetchError short-circuits to escalated.
const fe = buildOutcome({ roadmap: 'rm', phase: 'p', fetchError: true });
assert.equal(fe.outcome, 'escalated', 'fetchError → escalated');
assert.equal(fe.summary, 'phase fetch failed', 'fetchError summary is fixed');
assert.deepEqual(fe.findings, [], 'fetchError findings empty');
assert.deepEqual(Object.keys(fe).sort(), SHAPE, 'fetchError OUTCOME keeps the full shape');

// ============================================================================
// classifyOutcome directly — bounded loops reach a terminal (AC-4). The
// classifier consumes ONLY first-pass + one-rework arrays, so no state it sees
// can loop; every combination lands on a terminal value.
// ============================================================================
assert.equal(classifyOutcome({ planFindings: [], codeFindings: [], codeFindingsAfterRework: [], tier: 'medium' }), 'reviewed');
assert.equal(classifyOutcome({ planFindings: [], codeFindings: [B('x')], codeFindingsAfterRework: [], tier: 'medium' }), 'reviewed');
assert.equal(classifyOutcome({ planFindings: [], codeFindings: [B('x')], codeFindingsAfterRework: [B('x')], tier: 'medium' }), 'rework');
assert.equal(classifyOutcome({ planFindings: [B('x')], codeFindings: [], codeFindingsAfterRework: [], tier: 'medium' }), 'escalated');
for (const cf of [[], [B('a')]]) {
  for (const cr of [[], [B('b')]]) {
    const o = classifyOutcome({ planFindings: [], codeFindings: cf, codeFindingsAfterRework: cr, tier: 'medium' });
    assert.ok(['reviewed', 'rework'].includes(o), 'code stage is terminal — only reviewed|rework');
  }
}

// ============================================================================
// Tier-scaling — a surviving `concern` is blocking at `large`, not below it.
// ============================================================================
assert.equal(hasBlocking([C('c')], 'large'), true, 'large: concern counts as blocking');
assert.equal(hasBlocking([C('c')], 'medium'), false, 'medium: concern does not block');
assert.equal(hasBlocking([B('b')], 'medium'), true, 'medium: blocking still blocks');
assert.equal(
  classifyOutcome({ planFindings: [C('c')], codeFindings: [], codeFindingsAfterRework: [], tier: 'large' }),
  'escalated',
  'plan concern escalates at large'
);
assert.notEqual(
  classifyOutcome({ planFindings: [C('c')], codeFindings: [], codeFindingsAfterRework: [], tier: 'medium' }),
  'escalated',
  'plan concern does NOT escalate at medium (one-directional tightening)'
);

// ============================================================================
// Determinism — two buildOutcome calls on the same input are byte-identical.
// ============================================================================
const detInput = {
  roadmap: 'rm',
  phase: 'p',
  planFindings: [],
  codeFindings: [B('a'), C('b')],
  codeFindingsAfterRework: [B('a')],
  tier: 'medium',
};
assert.equal(
  JSON.stringify(buildOutcome(detInput)),
  JSON.stringify(buildOutcome(detInput)),
  'buildOutcome is deterministic across runs'
);

// summarizeFindings is a deterministic one-liner off the top-ranked finding.
assert.equal(summarizeFindings([]), 'no surviving findings');
assert.ok(summarizeFindings([B('bug')]).includes('blocking'), 'summary names the top severity');

// ============================================================================
// buildTaskOutcome — the task-shaped OUTCOME contract. Keyed by `task` slug
// instead of roadmap/phase; reuses the same pure classification core.
// ============================================================================
const { buildTaskOutcome } = mod;
assert.equal(typeof buildTaskOutcome, 'function', 'buildTaskOutcome is exported from the lib');

const TASK_SHAPE = ['findings', 'outcome', 'reason', 'reviewBudget', 'reviewCoverage', 'status', 'summary', 'task', 'writesCompletion'];

// reviewed — clean code review on the first pass.
const tRev = buildTaskOutcome({
  task: 'my-task',
  planFindings: [],
  codeFindings: [],
  codeFindingsAfterRework: [],
  tier: 'medium',
});
assert.equal(tRev.outcome, 'reviewed', 'clean task review → reviewed');
assert.equal(tRev.task, 'my-task', 'task OUTCOME is keyed by slug');
assert.deepEqual(Object.keys(tRev).sort(), TASK_SHAPE, 'task OUTCOME shape is task-keyed');
assert.ok(!('roadmap' in tRev) && !('phase' in tRev), 'task OUTCOME carries no roadmap/phase keys');

// rework — a blocking code finding survives the one bounded rework.
const tRw = buildTaskOutcome({
  task: 'my-task',
  planFindings: [],
  codeFindings: [B('leak')],
  codeFindingsAfterRework: [B('leak')],
  tier: 'medium',
});
assert.equal(tRw.outcome, 'rework', 'unresolved blocking code finding → rework');
assert.deepEqual(tRw.findings.map((f) => f.id), ['leak'], 'rework surfaces post-rework findings');

// escalated — a blocking plan finding short-circuits before implementation.
const tEsc = buildTaskOutcome({
  task: 'my-task',
  planFindings: [B('vague')],
  codeFindings: [],
  codeFindingsAfterRework: [],
  tier: 'medium',
});
assert.equal(tEsc.outcome, 'escalated', 'blocking plan finding → escalated');
assert.deepEqual(tEsc.findings.map((f) => f.id), ['vague'], 'escalated surfaces plan findings');

// fetchError short-circuits to escalated with the task-shaped identifier.
const tFe = buildTaskOutcome({ task: 'my-task', fetchError: true });
assert.equal(tFe.outcome, 'escalated', 'task fetchError → escalated');
assert.equal(tFe.summary, 'task fetch failed', 'task fetchError summary is fixed');
assert.deepEqual(tFe.findings, [], 'task fetchError findings empty');
assert.deepEqual(Object.keys(tFe).sort(), TASK_SHAPE, 'task fetchError keeps the task shape');

// Tasks always run at the fixed `medium` tier, so the `large` tightening (which
// promotes a surviving `concern` to blocking) must NOT apply to them.
const tConcern = buildTaskOutcome({
  task: 'my-task',
  planFindings: [],
  codeFindings: [C('nit')],
  codeFindingsAfterRework: [],
  tier: 'medium',
});
assert.equal(tConcern.outcome, 'reviewed', 'a concern alone never blocks a medium-tier task');

// The task twin of revRework: a blocking finding on the first pass, cleared by
// the one bounded rework, must land `reviewed` surfacing the POST-rework array.
const tRevRework = buildTaskOutcome({
  task: 'my-task',
  planFindings: [],
  codeFindings: [B('sqli')],
  codeFindingsAfterRework: [],
  tier: 'medium',
});
assert.equal(tRevRework.outcome, 'reviewed', 'task: blocking first pass cleared by rework → reviewed');
assert.deepEqual(tRevRework.findings, [], 'task reviewed-after-rework surfaces post-rework findings');
assert.ok(
  !tRevRework.summary.includes('blocking'),
  'a reviewed task summary never names a stale pre-rework blocker'
);

// Determinism — two calls on the same input are byte-identical.
const tDet = {
  task: 'my-task',
  planFindings: [],
  codeFindings: [B('a'), C('b')],
  codeFindingsAfterRework: [B('a')],
  tier: 'medium',
};
assert.equal(
  JSON.stringify(buildTaskOutcome(tDet)),
  JSON.stringify(buildTaskOutcome(tDet)),
  'buildTaskOutcome is deterministic across runs'
);

// ============================================================================
// AC-1 (positive half): the OUTCOME carries the canonical gate/completion policy.
//
// `status` must agree with the canonical statusFor(outcome, kind) and
// `writesCompletion` with the canonical writesCompletion(outcome) — for all
// three outcomes x both item kinds. This is what lets rdm-land know, without
// restating any map, that a `reviewed` branch is owed its land-time trailer.
// ============================================================================
const { statusFor, writesCompletion, outcomePolicy, OUTCOME_REASON_PREFIX } = mod;

for (const [o, kind] of [
  [rev, 'phase'], [revRework, 'phase'], [rw, 'phase'], [esc, 'phase'], [fe, 'phase'],
  [tRev, 'task'], [tRw, 'task'], [tEsc, 'task'], [tFe, 'task'],
]) {
  assert.equal(o.status, statusFor(o.outcome, kind), o.outcome + '/' + kind + ' status matches statusFor');
  assert.equal(o.writesCompletion, writesCompletion(o.outcome), o.outcome + ' writesCompletion matches the canonical policy');
  assert.equal(typeof o.writesCompletion, 'boolean', 'writesCompletion is a boolean, never a trailer literal');
}

// The three outcomes, pinned explicitly (a policy flip must fail loudly).
assert.equal(rev.status, 'reviewed', 'a clean review yields status reviewed');
assert.equal(rev.writesCompletion, true, 'a clean review is owed the land-time completion trailer');
assert.equal(rev.reason, '', 'a clean review carries no park reason');
assert.equal(rw.status, 'in-progress', 'rework yields status in-progress');
assert.equal(rw.writesCompletion, false, 'rework is NOT owed the completion trailer');
assert.ok(rw.reason.startsWith('[code]'), 'a rework reason is tagged [code]');
assert.equal(esc.status, 'blocked', 'escalated yields status blocked');
assert.equal(esc.writesCompletion, false, 'escalated is NOT owed the completion trailer');
assert.ok(esc.reason.startsWith('[plan]'), 'a dispatch escalation is tagged [plan] (it comes out of the plan gate)');
// Task-mode escalated maps to the `blocked` TASK status — never downgraded.
assert.equal(tEsc.status, 'blocked', 'an escalated TASK is blocked, not downgraded to in-progress');
assert.equal(tRev.writesCompletion, true, 'a clean task review is owed the completion trailer');
assert.equal(OUTCOME_REASON_PREFIX.escalated, '[plan]', 'dispatch tags escalations [plan]');
assert.equal(OUTCOME_REASON_PREFIX.rework, '[code]', 'dispatch tags reworks [code]');
assert.equal(outcomePolicy('reviewed', 'phase', 's').reason, '', 'outcomePolicy leaves a clean reason empty');

// Budget-0 must not be recomputed from a findings array: derive from the
// classifier. A blocking first pass with maxRework 0 is `rework`, never clean.
const b0 = buildOutcome({ roadmap: 'rm', phase: 'p', planFindings: [], codeFindings: [B('bug')], maxRework: 0, tier: 'medium' });
assert.equal(b0.outcome, 'rework', 'budget-0 with a blocking first pass is rework');
assert.equal(b0.status, 'in-progress', 'budget-0 rework status derives from the classifier');
assert.equal(b0.writesCompletion, false, 'budget-0 rework is not owed the completion trailer');

// ============================================================================
// No OUTCOME ever carries a land-time completion (`Done:`) directive.
// The completion POLICY is carried as the boolean above; the literal is written
// only at land time by rdm-land, from `rdm hook done-line`.
// ============================================================================
for (const o of [rev, revRework, rw, esc, fe, tRev, tRw, tEsc, tFe, b0]) {
  assert.ok(!JSON.stringify(o).includes('Done:'), 'OUTCOME never contains a Done: directive');
}

console.log('all dispatch-phase behavior assertions passed');
NODE_TEST

if run_node "$TMP/test.mjs" "$LIB"; then
    pass "outcome classification verified (reviewed/rework/escalated, tier-scaling, bounded, deterministic)"
else
    fail "dispatch-phase behavior assertions failed"
fi

# --- 1c. DRIVEN GATES --------------------------------------------------------
say "1c. Driven gates: budget-bounded plan/code loops under injected fakes"

cat >"$TMP/gates.mjs" <<'NODE_TEST'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const libPath = process.argv[2];
const mod = await import(pathToFileURL(libPath).href);
const {
  DEFAULT_MAX_PLAN_REVISE,
  DEFAULT_MAX_CODE_REWORK,
  parseBudget,
  parseDispatchArgs,
  runPlanGate,
  runCodeGate,
  classifyOutcome,
  buildOutcome,
  buildTaskOutcome,
} = mod;

const B = (id) => ({ id, concern: 'x', severity: 'blocking', confidence: 90, what_fails: id });

// The declared defaults are the values the docs and the escalation protocol
// quote; pin them so a silent change to either has to update both.
assert.equal(DEFAULT_MAX_PLAN_REVISE, 2, 'plan-revise budget defaults to 2');
assert.equal(DEFAULT_MAX_CODE_REWORK, 2, 'code-rework budget defaults to 2');

// Fakes: every stage records into a shared callLog and the reviewer is scripted.
// `reviewScript` is consumed per review call, repeating its last entry.
function makePlanFakes(o) {
  const opts = o || {};
  const callLog = [];
  let reviewIdx = 0;
  const script = opts.reviewScript || [[B('never-clean')]];
  return {
    callLog,
    deps: {
      plan: async () => {
        callLog.push('plan');
        return opts.planNull ? null : { doc: 0 };
      },
      revise: async (doc, findings) => {
        callLog.push('revise');
        assert.ok(Array.isArray(findings), 'revise receives the findings it must address');
        if (opts.reviseNull) return null;
        return { doc: (doc.doc || 0) + 1 };
      },
      // runPlanReview (a `runReview` from the canonical review source) resolves
      // { survivors, acTable }; plan mode always sets acTable to null.
      review: async (doc) => {
        assert.ok(doc !== null && doc !== undefined, 'a null plan doc must NEVER reach the reviewer');
        callLog.push('review');
        const r = script[Math.min(reviewIdx, script.length - 1)];
        reviewIdx++;
        return { survivors: r, acTable: null };
      },
    },
  };
}

// acScript (optional) is a PARALLEL per-round array of AC tables (or null),
// consumed alongside `reviewScript`'s per-round findings — mirrors runReview's
// { survivors, acTable } shape for the code-mode gate.
function makeCodeFakes(o) {
  const opts = o || {};
  const callLog = [];
  let reviewIdx = 0;
  const script = opts.reviewScript || [[B('never-clean')]];
  const acScript = opts.acScript || null;
  const deps = {
    implement: async (notes) => {
      callLog.push(notes == null ? 'implement' : 'rework');
    },
    review: async () => {
      callLog.push('review');
      const idx = Math.min(reviewIdx, script.length - 1);
      const r = script[idx];
      const acTable = acScript ? acScript[Math.min(reviewIdx, acScript.length - 1)] : null;
      reviewIdx++;
      return { survivors: r, acTable: acTable };
    },
  };
  // `act` is OPTIONAL on the real deps contract — only wire it in when the
  // caller supplied one, so the "no act dep" no-op path stays exercised too.
  if (opts.act) {
    deps.act = async (findings) => {
      callLog.push('act');
      return opts.act(findings);
    };
  }
  return { callLog, deps };
}

const count = (log, kind) => log.filter((c) => c === kind).length;

// ============================================================================
// AC1 — agent-call COUNTS per budget, not just the terminal outcome.
// ============================================================================
for (const b of [0, 1, 2]) {
  const h = makePlanFakes({});
  const res = await runPlanGate({ maxRevise: b, tier: 'medium' }, h.deps);
  assert.equal(count(h.callLog, 'plan'), 1, 'budget ' + b + ': the plan is authored exactly once');
  assert.equal(count(h.callLog, 'revise'), b, 'budget ' + b + ': exactly ' + b + ' revise agent calls');
  assert.equal(res.reviseCount, b, 'budget ' + b + ': reviseCount matches the budget');
  assert.equal(res.fetchError, false, 'budget ' + b + ': a never-clean plan is not a fetchError');
  assert.equal(
    classifyOutcome({ planFindings: res.findings, tier: 'medium' }),
    'escalated',
    'budget ' + b + ': a plan that never clears escalates'
  );

  const c = makeCodeFakes({});
  const cres = await runCodeGate({ maxRework: b, tier: 'medium' }, c.deps);
  assert.equal(count(c.callLog, 'implement'), 1, 'budget ' + b + ': the first implement runs exactly once');
  assert.equal(count(c.callLog, 'rework'), b, 'budget ' + b + ': exactly ' + b + ' rework implement calls');
  assert.equal(cres.reworkCount, b, 'budget ' + b + ': reworkCount matches the budget');
  assert.equal(
    classifyOutcome({ planFindings: [], codeReviews: cres.rounds, maxRework: b, tier: 'medium' }),
    'rework',
    'budget ' + b + ': code that never clears is rework'
  );
}

// Budget 0 explicitly: NO revise / NO rework agent call at all.
{
  const h = makePlanFakes({});
  await runPlanGate({ maxRevise: 0, tier: 'medium' }, h.deps);
  assert.equal(count(h.callLog, 'revise'), 0, 'budget 0: zero revise calls (zero tokens burned on revision)');
  const c = makeCodeFakes({});
  await runCodeGate({ maxRework: 0, tier: 'medium' }, c.deps);
  assert.equal(count(c.callLog, 'rework'), 0, 'budget 0: zero rework implement calls');
}

// Early break: a clean first review stops the loop below its budget.
{
  const h = makePlanFakes({ reviewScript: [[]] });
  const res = await runPlanGate({ maxRevise: 2, tier: 'medium' }, h.deps);
  assert.equal(count(h.callLog, 'revise'), 0, 'a clean first plan review never revises');
  assert.equal(res.reviewCount, 1, 'a clean first plan review runs exactly one review');
}
{
  const h = makePlanFakes({ reviewScript: [[B('x')], []] });
  const res = await runPlanGate({ maxRevise: 2, tier: 'medium' }, h.deps);
  assert.equal(res.reviseCount, 1, 'a plan cleared by revision 1 does not spend revision 2');
  assert.equal(res.reviewCount, 2, 'and runs exactly two reviews');
  assert.equal(classifyOutcome({ planFindings: res.findings, tier: 'medium' }), 'reviewed');
}

// null-plan short-circuit: no review, no revise, fetchError at stage 'plan'.
{
  const h = makePlanFakes({ planNull: true });
  const res = await runPlanGate({ maxRevise: 2, tier: 'medium' }, h.deps);
  assert.equal(res.fetchError, true, 'a null plan doc short-circuits to fetchError');
  assert.equal(res.stage, 'plan', 'the null is attributed to the plan stage');
  assert.equal(count(h.callLog, 'review'), 0, 'a null plan is NEVER reviewed as an empty plan');
  assert.equal(count(h.callLog, 'revise'), 0, 'a null plan is never revised');
  assert.equal(buildOutcome({ roadmap: 'rm', phase: 'p', fetchError: res.fetchError }).outcome, 'escalated');
}

// null-revise short-circuit: the guard runs BEFORE the reassignment and BEFORE
// the next review, so the null neither reaches the reviewer nor clobbers the
// last good plan doc.
{
  const h = makePlanFakes({ reviseNull: true });
  const res = await runPlanGate({ maxRevise: 2, tier: 'medium' }, h.deps);
  assert.equal(res.fetchError, true, 'a null revise result short-circuits to fetchError');
  assert.equal(res.stage, 'revise', 'the null is attributed to the revise stage');
  assert.equal(res.reviseCount, 1, 'reviseCount counts the attempt that failed');
  assert.equal(count(h.callLog, 'review'), 1, 'the null revised doc is NEVER reviewed');
  assert.deepEqual(res.planDoc, { doc: 0 }, 'the last good plan doc survives a null revise');
  assert.equal(buildOutcome({ roadmap: 'rm', phase: 'p', fetchError: true }).outcome, 'escalated');
}

// ============================================================================
// AC2 — the budget-0 classifier hole: a blocking FIRST-pass code review must be
// `rework`, never `reviewed`. Before the fix the empty codeFindingsAfterRework
// slot marked a failing review clean.
// ============================================================================
assert.equal(
  classifyOutcome({ planFindings: [], codeFindings: [B('bug')], codeFindingsAfterRework: [], maxRework: 0, tier: 'medium' }),
  'rework',
  'blocking first-pass code review at budget 0 must NOT classify reviewed'
);
{
  const o = buildOutcome({
    roadmap: 'rm',
    phase: 'p',
    planFindings: [],
    codeFindings: [B('bug')],
    codeFindingsAfterRework: [],
    maxRework: 0,
    tier: 'medium',
  });
  assert.equal(o.outcome, 'rework', 'buildOutcome: budget-0 blocking review → rework');
  assert.deepEqual(o.findings.map((f) => f.id), ['bug'], 'budget-0 rework surfaces the failing review findings');
  assert.ok(o.summary.startsWith('code rework unresolved: '), 'budget-0 rework keeps the summary prefix');
  const t = buildTaskOutcome({
    task: 't',
    planFindings: [],
    codeFindings: [B('bug')],
    codeFindingsAfterRework: [],
    maxRework: 0,
    tier: 'medium',
  });
  assert.equal(t.outcome, 'rework', 'buildTaskOutcome: budget-0 blocking review → rework');
  assert.ok(!('roadmap' in t) && !('phase' in t), 'task-shaped OUTCOME carries no roadmap/phase keys');
}
// The same shape at the DEFAULT budget still means "fixed by the rework" —
// the fix must not invert the legacy fixture.
assert.equal(
  classifyOutcome({ planFindings: [], codeFindings: [B('bug')], codeFindingsAfterRework: [], tier: 'medium' }),
  'reviewed',
  'legacy two-slot input under the default budget still classifies reviewed'
);
// And the driven equivalent: budget 0 + a blocking review → rework via rounds.
{
  const c = makeCodeFakes({ reviewScript: [[B('bug')]] });
  const cres = await runCodeGate({ maxRework: 0, tier: 'medium' }, c.deps);
  assert.equal(
    buildOutcome({ roadmap: 'rm', phase: 'p', planFindings: [], codeReviews: cres.rounds, maxRework: 0, tier: 'medium' }).outcome,
    'rework',
    'driven budget-0 code gate yields rework'
  );
}

// ============================================================================
// AC3 — the two budgets are independent: exhausting one leaves the other whole.
// ============================================================================
{
  const p = makePlanFakes({});
  const c = makeCodeFakes({ reviewScript: [[]] });
  const pres = await runPlanGate({ maxRevise: 2, tier: 'medium' }, p.deps);
  const cres = await runCodeGate({ maxRework: 2, tier: 'medium' }, c.deps);
  assert.equal(pres.reviseCount, 2, 'the plan budget is fully spent');
  assert.equal(cres.reworkCount, 0, 'a spent plan budget consumes NO code-rework budget');
  assert.equal(cres.reviewCount, 1, 'the code stage still reviews exactly once');
}
{
  const p = makePlanFakes({ reviewScript: [[]] });
  const c = makeCodeFakes({});
  const pres = await runPlanGate({ maxRevise: 2, tier: 'medium' }, p.deps);
  const cres = await runCodeGate({ maxRework: 2, tier: 'medium' }, c.deps);
  assert.equal(pres.reviseCount, 0, 'a clean plan spends no plan budget');
  assert.equal(pres.reviewCount, 1, 'and reviews exactly once');
  assert.equal(cres.reworkCount, 2, 'while the code budget is independently exhausted');
  assert.ok(!('reworkCount' in pres), 'the plan gate exposes only its own counters');
  assert.ok(!('reviseCount' in cres), 'the code gate exposes only its own counters');
}

// ============================================================================
// AC4 — boundedness proof: a stage that never comes back clean terminates at
// EXACTLY budget + 1 review calls (assert.equal, not <=). A hung loop fails the
// harness by never resolving.
// ============================================================================
for (const b of [0, 1, 2, 5]) {
  const p = makePlanFakes({});
  const pres = await runPlanGate({ maxRevise: b, tier: 'medium' }, p.deps);
  assert.equal(pres.reviewCount, b + 1, 'plan gate: never-clean terminates at exactly budget + 1 reviews');
  assert.equal(count(p.callLog, 'review'), b + 1, 'plan gate: review call count matches reviewCount');

  const c = makeCodeFakes({});
  const cres = await runCodeGate({ maxRework: b, tier: 'medium' }, c.deps);
  assert.equal(cres.rounds.length, b + 1, 'code gate: never-clean terminates at exactly budget + 1 rounds');
  assert.equal(cres.reviewCount, b + 1, 'code gate: reviewCount == rounds.length');
}
// The null-revise early exit terminates BELOW budget + 1 — an exit, not a hang.
{
  const h = makePlanFakes({ reviseNull: true });
  const res = await runPlanGate({ maxRevise: 2, tier: 'medium' }, h.deps);
  assert.ok(res.reviewCount < 3, 'a null revise exits strictly below budget + 1 reviews');
}

// The `large` tightening applies inside the loops: a surviving concern blocks.
{
  const C = (id) => ({ id, concern: 'x', severity: 'concern', confidence: 90, what_fails: id });
  const h = makePlanFakes({ reviewScript: [[C('nit')]] });
  const res = await runPlanGate({ maxRevise: 2, tier: 'large' }, h.deps);
  assert.equal(res.reviseCount, 2, 'large tier: a surviving concern exhausts the plan budget');
  const m = makePlanFakes({ reviewScript: [[C('nit')]] });
  const mres = await runPlanGate({ maxRevise: 2, tier: 'medium' }, m.deps);
  assert.equal(mres.reviseCount, 0, 'medium tier: a concern alone never spends budget');
}

// ============================================================================
// AC8 — budget validation: 0 accepted, negatives/non-integers rejected with an
// actionable message naming the flag.
// ============================================================================
assert.equal(parseBudget(0, 'maxCodeRework', 2), 0, '0 is a meaningful budget, never "unset"');
assert.equal(parseBudget('0', 'maxCodeRework', 2), 0, "'0' parses to 0, not the default");
assert.equal(parseBudget(undefined, 'maxPlanRevise', 2), 2, 'unset falls back to the default');
assert.equal(parseBudget(null, 'maxPlanRevise', 2), 2, 'null falls back to the default');
assert.equal(parseBudget('', 'maxPlanRevise', 2), 2, 'empty string falls back to the default');
assert.equal(parseBudget('3', 'maxPlanRevise', 2), 3, 'a numeric string parses');
for (const bad of [-1, '-1', 1.5, '1.5', 'abc', '2abc', NaN, Infinity, true, [], {}, -0]) {
  assert.throws(
    () => parseBudget(bad, 'maxPlanRevise', 2),
    /maxPlanRevise must be a non-negative integer/,
    'rejected: ' + String(bad)
  );
}
try {
  parseBudget('2abc', 'maxCodeRework', 2);
  assert.fail('parseInt coercion trap: "2abc" must be rejected, not read as 2');
} catch (e) {
  assert.match(e.message, /maxCodeRework/, 'the message names the offending flag');
  assert.match(e.message, /non-negative integer/, 'the message states what a valid value is');
  assert.match(e.message, /0 means no reworks/, 'the message explains that 0 is legal');
}

// parseDispatchArgs — the same validation at the args boundary, plus coercion.
{
  const d = parseDispatchArgs({ roadmap: 'r', phase: 'p', rdmBin: 'rdm' });
  assert.equal(d.maxPlanRevise, DEFAULT_MAX_PLAN_REVISE, 'unset args take the default plan budget');
  assert.equal(d.maxCodeRework, DEFAULT_MAX_CODE_REWORK, 'unset args take the default code budget');
  assert.equal(d.planOnly, false, 'planOnly defaults false');
  const z = parseDispatchArgs({ roadmap: 'r', phase: 'p', rdmBin: 'rdm', maxCodeRework: 0, maxPlanRevise: 0 });
  assert.equal(z.maxCodeRework, 0, 'an explicit 0 survives the args boundary');
  assert.equal(z.maxPlanRevise, 0, 'an explicit 0 survives the args boundary (plan)');
  assert.throws(
    () => parseDispatchArgs({ roadmap: 'r', phase: 'p', rdmBin: 'rdm', maxCodeRework: -1 }),
    /maxCodeRework must be a non-negative integer/,
    'an invalid budget is rejected at parse time, before any agent() call'
  );
  // A stringified payload is coerced ONCE and its budgets are still validated.
  const s = parseDispatchArgs(JSON.stringify({ roadmap: 'r', phase: 'p', rdmBin: 'rdm', maxPlanRevise: 3 }));
  assert.equal(s.roadmap, 'r', 'a stringified payload is coerced');
  assert.equal(s.maxPlanRevise, 3, 'a budget inside a stringified payload is read');
  assert.throws(
    () => parseDispatchArgs(JSON.stringify({ roadmap: 'r', rdmBin: 'rdm', maxPlanRevise: 'nope' })),
    /maxPlanRevise must be a non-negative integer/,
    'a budget inside a stringified payload is still validated'
  );
  const t = parseDispatchArgs({ task: 'my-task', rdmBin: 'rdm' });
  assert.equal(t.task, 'my-task', 'task mode survives the parse');
  assert.equal(t.roadmap, '', 'task mode carries no roadmap');
}

// ============================================================================
// AC-2 — the code gate's review is SIGNALS-FED and RE-DERIVES per rework round.
//
// The driver's `review` closure fetches a diff and threads
// deriveSignals({targetType, changedFiles, diffText}) into the canonical
// pipeline. Here that closure's contract is driven with a fake review dep and a
// mutable diff, so the three load-bearing properties are pinned:
//   1. an exported-symbol + no-tests diff turns `api-docs` and `tests` ON;
//   2. round 2 re-derives from the POST-rework tree (a fix that newly adds an
//      exported symbol must turn `api-docs` on for round 2);
//   3. an empty/failed diff omits the `signals` KEY ENTIRELY (fail-open), never
//      passing `{}` — selectDimensions treats those two cases differently.
// ============================================================================
{
  const { deriveSignals } = mod;

  // A SYNTHETIC non-rdm project: the signal comes from diff CONTENT, not paths.
  const EXPORTED_API = ['src/api/index.ts'];
  const s1 = deriveSignals({ targetType: 'phase', changedFiles: EXPORTED_API, diffText: '+export function foo() {}\n' });
  assert.equal(s1.publicApiChanged, true, 'an added exported symbol turns publicApiChanged on');
  assert.equal(s1.missingTests, true, 'a code-only diff with no test file turns missingTests on');
  assert.equal(s1.changesLogic, true, 'a .ts change turns changesLogic on');
  assert.equal(s1.targetType, 'phase', 'the target type rides along for the plan-mode trigger');

  // Round 1 is docs-only; round 2's fix adds an export. Re-derivation must notice.
  const roundDiffs = [
    { changedFiles: ['docs/workflow-schemas.md'], diffText: '+prose\n' },
    { changedFiles: EXPORTED_API, diffText: '+export function bar() {}\n' },
  ];
  let roundIdx = 0;
  const seen = [];
  const codeDeps = {
    implement: async () => {},
    review: async () => {
      // Mirrors the driver closure: fetch the diff INSIDE review, per round.
      const d = roundDiffs[Math.min(roundIdx, roundDiffs.length - 1)];
      roundIdx++;
      const files = (d.changedFiles || []).filter(Boolean);
      if (files.length === 0) {
        seen.push({ hasSignalsKey: false });
        return { survivors: [B('still-broken')], acTable: null };
      }
      const signals = deriveSignals({ targetType: 'phase', changedFiles: files, diffText: d.diffText });
      seen.push({ hasSignalsKey: true, signals });
      return { survivors: roundIdx >= 2 ? [] : [B('round-1-blocker')], acTable: null };
    },
  };
  const gate = await runCodeGate({ maxRework: 1, tier: 'medium' }, codeDeps);
  assert.equal(gate.reviewCount, 2, 'a blocking round 1 triggers a second, re-derived review');
  assert.equal(seen[0].signals.publicApiChanged, false, 'round 1 (docs-only) leaves publicApiChanged off');
  assert.equal(seen[1].signals.publicApiChanged, true, 'round 2 RE-DERIVES and turns publicApiChanged on');
  assert.equal(gate.findings.length, 0, 'the rework cleared the gate');

  // Fail-open: an empty diff must yield NO signals key at all.
  let failOpenCall = null;
  const failOpenDeps = {
    implement: async () => {},
    review: async () => {
      const files = [];
      const call = { target: 'rm/phase-1-x' };
      if (files.length > 0) call.signals = deriveSignals({ targetType: 'phase', changedFiles: files });
      failOpenCall = call;
      return { survivors: [], acTable: null };
    },
  };
  await runCodeGate({ maxRework: 0, tier: 'medium' }, failOpenDeps);
  assert.ok(
    !Object.prototype.hasOwnProperty.call(failOpenCall, 'signals'),
    'an empty diff omits the signals KEY entirely (fail-open) — never passes {}'
  );
}

// ============================================================================
// AC-TABLE CHANNEL + ACT STEP (classify-outcome-ac-table-channel)
// ============================================================================

// (a) An AC-only gap (no blocking finding at all) must still consume the
// rework budget — the loop-continuation fix. Round 1: zero findings but a FAIL
// acTable; round 2: clean on both axes.
{
  const c = makeCodeFakes({ reviewScript: [[], []], acScript: [[{ criterion: 'x', status: 'FAIL', evidence: 'y' }], null] });
  const cres = await runCodeGate({ maxRework: 2, tier: 'medium' }, c.deps);
  assert.equal(count(c.callLog, 'rework'), 1, 'an AC-only gap on round 1 consumes a rework attempt');
  assert.equal(cres.reworkCount, 1, 'reworkCount reflects the AC-only-gap rework');
  assert.equal(cres.acRounds.length, 2, 'acRounds records one entry per round');
  assert.deepEqual(cres.acRounds[0], [{ criterion: 'x', status: 'FAIL', evidence: 'y' }], 'round 1 AC table is recorded');
  assert.equal(cres.acRounds[1], null, 'round 2 AC table (clean) is recorded as null');
  const o = buildOutcome({ roadmap: 'rm', phase: 'p', planFindings: [], codeReviews: cres.rounds, acRounds: cres.acRounds, maxRework: 2, tier: 'medium' });
  assert.equal(o.outcome, 'reviewed', 'a round-2 clean AC table (and no findings) yields reviewed');
}

// (a2) An AC-only gap that NEVER resolves still reports `rework` once the
// rework budget is exhausted — cap-exhaustion surfaces correctly even though
// codeReviewRounds' findings-only view sees an empty last round.
{
  const c = makeCodeFakes({ reviewScript: [[], [], []], acScript: [[{ criterion: 'x', status: 'FAIL', evidence: 'y' }]] });
  const cres = await runCodeGate({ maxRework: 2, tier: 'medium' }, c.deps);
  assert.equal(cres.reworkCount, 2, 'the AC-only gap exhausts the full rework budget');
  assert.equal(cres.rounds.length, 3, 'three rounds ran (original + 2 reworks)');
  const o = buildOutcome({ roadmap: 'rm', phase: 'p', planFindings: [], codeReviews: cres.rounds, acRounds: cres.acRounds, maxRework: 2, tier: 'medium' });
  assert.equal(o.outcome, 'rework', 'cap-exhaustion on an AC-only gap still reports rework, never reviewed');
  assert.equal(o.summary, 'code rework unresolved: unmet acceptance criteria in AC table', 'the summary names the real cause instead of "no surviving findings"');
  // maxRework: 0 is legal and meaningful — a round-1 AC gap must still classify
  // rework even though runCodeGate never attempts a second round.
  const c0 = makeCodeFakes({ reviewScript: [[]], acScript: [[{ criterion: 'x', status: 'FAIL', evidence: 'y' }]] });
  const cres0 = await runCodeGate({ maxRework: 0, tier: 'medium' }, c0.deps);
  assert.equal(cres0.reworkCount, 0, 'budget 0: zero rework attempts');
  const o0 = buildOutcome({ roadmap: 'rm', phase: 'p', planFindings: [], codeReviews: cres0.rounds, acRounds: cres0.acRounds, maxRework: 0, tier: 'medium' });
  assert.equal(o0.outcome, 'rework', 'budget-0 AC-only gap still classifies rework, never a laundered reviewed');
}

// (b) Act is invoked EXACTLY once on a clean final round with a surviving
// (non-blocking) finding, and the OUTCOME is `reviewed` with the finding
// annotated by the Act step's disposition.
{
  const concernFinding = { id: 'nit', concern: 'style', severity: 'concern', confidence: 80, what_fails: 'a nit' };
  const suggestionFinding = { id: 'unaddressed', concern: 'style', severity: 'suggestion', confidence: 75, what_fails: 'a nit too' };
  let actArg = null;
  const c = makeCodeFakes({
    reviewScript: [[concernFinding, suggestionFinding]],
    act: (findings) => {
      actArg = findings;
      // Only reports a disposition for one of the two findings it was given —
      // the other must default to 'unhandled', not be silently dropped.
      return { handled: [{ id: 'nit', action: 'fixed-inline' }] };
    },
  });
  const cres = await runCodeGate({ maxRework: 2, tier: 'medium' }, c.deps);
  assert.equal(count(c.callLog, 'act'), 1, 'act is invoked exactly once on a clean round with surviving findings');
  assert.deepEqual(actArg.map((f) => f.id), ['nit', 'unaddressed'], 'act receives every surviving finding from the clean round');
  assert.deepEqual(
    cres.actResult,
    { handled: [{ id: 'nit', action: 'fixed-inline' }] },
    'runCodeGate reports the act result'
  );
  const o = buildOutcome({ roadmap: 'rm', phase: 'p', planFindings: [], codeReviews: cres.rounds, acRounds: cres.acRounds, maxRework: 2, tier: 'medium', actResult: cres.actResult });
  assert.equal(o.outcome, 'reviewed', 'surviving concern/suggestion findings alone (medium tier) still yield reviewed');
  assert.equal(o.findings.find((f) => f.id === 'nit').handled, 'fixed-inline', 'the finding is annotated with the act disposition');
  assert.equal(
    o.findings.find((f) => f.id === 'unaddressed').handled,
    'unhandled',
    'a finding the act step did not report on defaults to unhandled, not silently dropped'
  );
}

// (c) Act is NEVER invoked on a still-blocking round (budget exhausted, still
// non-clean).
{
  let actCalled = false;
  const c = makeCodeFakes({ reviewScript: [[B('still-broken')]], act: () => { actCalled = true; return { handled: [] }; } });
  const cres = await runCodeGate({ maxRework: 0, tier: 'medium' }, c.deps);
  assert.equal(count(c.callLog, 'act'), 0, 'act is never invoked on a still-blocking round');
  assert.equal(actCalled, false, 'the act callback itself never ran');
  assert.equal(cres.actResult, null, 'actResult stays null when act never ran');
  const o = buildOutcome({ roadmap: 'rm', phase: 'p', planFindings: [], codeReviews: cres.rounds, acRounds: cres.acRounds, maxRework: 0, tier: 'medium', actResult: cres.actResult });
  assert.equal(o.outcome, 'rework', 'a still-blocking round yields rework, never invoking act');
}

// (d) An act dep that THROWS never affects the outcome — reviewed stays
// reviewed, and runCodeGate does not itself throw.
{
  const concernFinding = { id: 'nit2', concern: 'style', severity: 'concern', confidence: 80, what_fails: 'a nit' };
  const c = makeCodeFakes({
    reviewScript: [[concernFinding]],
    act: () => {
      throw new Error('boom act');
    },
  });
  const cres = await runCodeGate({ maxRework: 2, tier: 'medium' }, c.deps);
  assert.equal(cres.actResult, null, 'a thrown act call resolves to a null actResult, not a rejection');
  const o = buildOutcome({ roadmap: 'rm', phase: 'p', planFindings: [], codeReviews: cres.rounds, acRounds: cres.acRounds, maxRework: 2, tier: 'medium', actResult: cres.actResult });
  assert.equal(o.outcome, 'reviewed', 'a thrown act call never downgrades a clean outcome');
  assert.equal(
    o.findings.find((f) => f.id === 'nit2').handled,
    undefined,
    'a null actResult (act threw) leaves the finding unannotated — there is no disposition to report, so nothing is guessed'
  );
}

// (f) buildTaskOutcome mirrors of (a)/(a2)/(b): the AC-table-gate and
// Act-annotation logic is duplicated (not shared) between buildOutcome and
// buildTaskOutcome, so the task-shaped path needs its OWN direct coverage —
// a divergence here (typo, inverted condition, dropped annotateHandled call)
// would otherwise go undetected.
{
  // (f-a) An AC-only gap forces a task to `rework` even with zero findings.
  const c = makeCodeFakes({ reviewScript: [[], []], acScript: [[{ criterion: 'x', status: 'FAIL', evidence: 'y' }], null] });
  const cres = await runCodeGate({ maxRework: 2, tier: 'medium' }, c.deps);
  assert.equal(cres.reworkCount, 1, 'task: an AC-only gap on round 1 consumes a rework attempt');
  const tClean = buildTaskOutcome({ task: 'my-task', planFindings: [], codeReviews: cres.rounds, acRounds: cres.acRounds, maxRework: 2, tier: 'medium' });
  assert.equal(tClean.outcome, 'reviewed', 'task: a round-2 clean AC table (and no findings) yields reviewed');

  // (f-a2) A never-resolved AC-only gap still reports `rework` for a task once
  // the budget is exhausted, with the AC-aware summary (not "no surviving
  // findings").
  const c2 = makeCodeFakes({ reviewScript: [[], [], []], acScript: [[{ criterion: 'x', status: 'FAIL', evidence: 'y' }]] });
  const cres2 = await runCodeGate({ maxRework: 2, tier: 'medium' }, c2.deps);
  const tRw = buildTaskOutcome({ task: 'my-task', planFindings: [], codeReviews: cres2.rounds, acRounds: cres2.acRounds, maxRework: 2, tier: 'medium' });
  assert.equal(tRw.outcome, 'rework', 'task: cap-exhaustion on an AC-only gap still reports rework, never reviewed');
  assert.equal(
    tRw.summary,
    'code rework unresolved: unmet acceptance criteria in AC table',
    'task: the summary names the real cause instead of "no surviving findings"'
  );

  // (f-b) Act is invoked once on a clean round with a surviving finding, and
  // the task OUTCOME carries the handled annotation.
  const concernFinding = { id: 'tnit', concern: 'style', severity: 'concern', confidence: 80, what_fails: 'a nit' };
  const c3 = makeCodeFakes({
    reviewScript: [[concernFinding]],
    act: () => ({ handled: [{ id: 'tnit', action: 'filed-as-task', taskSlug: 'follow-up' }] }),
  });
  const cres3 = await runCodeGate({ maxRework: 2, tier: 'medium' }, c3.deps);
  assert.equal(count(c3.callLog, 'act'), 1, 'task: act is invoked exactly once on a clean round with surviving findings');
  const tAct = buildTaskOutcome({
    task: 'my-task',
    planFindings: [],
    codeReviews: cres3.rounds,
    acRounds: cres3.acRounds,
    maxRework: 2,
    tier: 'medium',
    actResult: cres3.actResult,
  });
  assert.equal(tAct.outcome, 'reviewed', 'task: surviving concern alone (medium tier) still yields reviewed');
  assert.equal(
    tAct.findings.find((f) => f.id === 'tnit').handled,
    'filed-as-task',
    'task: the finding is annotated with the act disposition'
  );
}

// (e) runPlanGate regression: the plan gate must correctly unwrap the
// { survivors, acTable } shape rather than misreading it as a non-array — a
// blocking plan finding must still trigger the revise loop / escalation.
{
  const blockingPlan = [B('plan-defect')];
  const h = makePlanFakes({ reviewScript: [blockingPlan, blockingPlan, blockingPlan] });
  const res = await runPlanGate({ maxRevise: 2, tier: 'medium' }, h.deps);
  assert.equal(count(h.callLog, 'revise'), 2, 'a blocking plan finding still drives the full revise loop');
  assert.ok(mod.hasBlocking(res.findings, 'medium'), 'hasBlocking still detects the unwrapped survivors, not a wrapper object');
  assert.equal(classifyOutcome({ planFindings: res.findings, tier: 'medium' }), 'escalated', 'the plan gate still escalates after the {survivors,acTable} shape change');
}

// (f) REFUTATION BUDGET threading (bound-review-fan-out phase 4). The review
// pipeline now resolves a third field, `budget`; both gates must capture it per
// round and the OUTCOME must surface it — with a VISIBLE summary clause only
// when a round actually hit its bound.
{
  const BUDGET_HIT = { max: 5, produced: 13, gating: 13, graded: 5, passedThroughNonGating: 0, passedThroughBudget: 8, refuterErrors: 0, hit: true };
  const BUDGET_CLEAN = { max: 5, produced: 2, gating: 2, graded: 2, passedThroughNonGating: 0, passedThroughBudget: 0, refuterErrors: 0, hit: false };

  // Two code rounds: round 1 hits its bound, round 2 does not.
  let round = 0;
  const codeGate = await runCodeGate(
    { maxRework: 2, tier: 'medium' },
    {
      implement: async () => null,
      review: async () => {
        const r = round++;
        return r === 0
          ? { survivors: [B('bug')], acTable: null, budget: BUDGET_HIT }
          : { survivors: [], acTable: null, budget: BUDGET_CLEAN };
      },
    }
  );
  assert.equal(codeGate.budgetRounds.length, 2, 'runCodeGate records one budget entry PER ROUND');
  assert.equal(codeGate.budgetRounds[0].hit, true, 'round 1 hit its bound');
  assert.equal(codeGate.budgetRounds[1].hit, false, 'round 2 did not');

  const out = buildOutcome({
    roadmap: 'rm',
    phase: 'p',
    planFindings: [],
    codeReviews: codeGate.rounds,
    acRounds: codeGate.acRounds,
    budgetRounds: codeGate.budgetRounds,
    maxRework: 2,
    tier: 'medium',
  });
  assert.equal(out.outcome, 'reviewed', 'the rework resolved the blocker');
  assert.equal(out.reviewBudget.everHit, true, 'everHit is true when ANY round hit, even if the last did not');
  assert.equal(out.reviewBudget.rounds, 2, 'reviewBudget reports how many rounds were budgeted');
  assert.equal(out.reviewBudget.max, 5, 'reviewBudget carries the LAST round\'s cap');
  assert.equal(out.reviewBudget.produced, 2, 'reviewBudget carries the LAST round\'s produced count');
  assert.equal(out.reviewBudget.hit.produced, 13, 'reviewBudget.hit points at the round that actually hit');
  assert.ok(
    out.summary.includes('[review budget hit: 13 produced, 5 graded, 8 ungraded]'),
    'a budget-hit unit is VISIBLY distinguishable in the OUTCOME summary'
  );

  // No round hit -> the summary clause is ABSENT and the summary is unchanged.
  const clean = buildOutcome({
    roadmap: 'rm',
    phase: 'p',
    planFindings: [],
    codeReviews: [[]],
    acRounds: [null],
    budgetRounds: [BUDGET_CLEAN],
    tier: 'medium',
  });
  assert.equal(clean.reviewBudget.everHit, false, 'no round hit');
  assert.ok(!clean.summary.includes('review budget hit'), 'an unbounded run carries NO summary clause');
  assert.equal(clean.summary, 'phase reviewed clean: no surviving findings', 'the unbounded summary is byte-unchanged');

  // An older caller that reports no budget at all still gets a well-formed
  // OUTCOME with reviewBudget === null.
  const legacy = buildOutcome({ roadmap: 'rm', phase: 'p', planFindings: [], codeFindings: [], tier: 'medium' });
  assert.equal(legacy.reviewBudget, null, 'no budget reported -> reviewBudget is null, never undefined');
  assert.ok(!legacy.summary.includes('review budget hit'), 'a budget-less OUTCOME carries no clause');

  // The PLAN gate captures its own budget, counted independently, and a
  // plan-gate hit surfaces on an escalated OUTCOME's summary and reason.
  const ph = makePlanFakes({ reviewScript: [[B('plan-defect')]] });
  const planReviewWithBudget = ph.deps.review;
  ph.deps.review = async (doc) => ({ ...(await planReviewWithBudget(doc)), budget: BUDGET_HIT });
  const planGate = await runPlanGate({ maxRevise: 0, tier: 'medium' }, ph.deps);
  assert.equal(planGate.budgetRounds.length, 1, 'runPlanGate records a budget per review round');
  assert.equal(planGate.budget.hit, true, 'runPlanGate returns the last round\'s budget');
  const esc = buildOutcome({
    roadmap: 'rm',
    phase: 'p',
    planFindings: planGate.findings,
    planBudget: planGate.budget,
    tier: 'medium',
  });
  assert.equal(esc.outcome, 'escalated', 'a blocking plan finding still escalates');
  assert.equal(esc.reviewBudget.everHit, true, 'a plan-gate budget hit is visible on the OUTCOME');
  assert.ok(esc.summary.includes('[review budget hit:'), 'the escalated summary carries the clause');
  assert.ok(esc.reason.includes('[review budget hit:'), 'the persisted reason carries it too (outcomePolicy derives reason from summary)');

  // MULTI-ROUND plan gate: a plan round that hit its bound and was then
  // RESOLVED by a later revision must still set everHit. This is the plan-side
  // analogue of the two-code-round case above; passing only the last plan round
  // would silently report complete coverage for a run that passed over gating
  // findings ungraded on round 1.
  {
    let pr = 0;
    const mh = makePlanFakes({ reviewScript: [[B('plan-defect')], []] });
    const baseReview = mh.deps.review;
    mh.deps.review = async (doc) => ({ ...(await baseReview(doc)), budget: pr++ === 0 ? BUDGET_HIT : BUDGET_CLEAN });
    const mpg = await runPlanGate({ maxRevise: 1, tier: 'medium' }, mh.deps);
    assert.equal(mpg.budgetRounds.length, 2, 'runPlanGate records a budget entry PER plan-revise round');
    assert.equal(mpg.budget.hit, false, 'the LAST plan round did not hit — the single-object shape would hide the round-1 hit');
    const mOut = buildOutcome({
      roadmap: 'rm',
      phase: 'p',
      planFindings: mpg.findings,
      planBudget: mpg.budgetRounds,
      codeReviews: [[]],
      acRounds: [null],
      budgetRounds: [BUDGET_CLEAN],
      tier: 'medium',
    });
    assert.equal(mOut.outcome, 'reviewed', 'the plan revision resolved the blocker');
    assert.equal(mOut.reviewBudget.planRounds, 2, 'reviewBudget reports how many PLAN rounds were budgeted');
    assert.equal(
      mOut.reviewBudget.everHit,
      true,
      'a plan round-1 budget hit resolved by round 2 is STILL visible on the OUTCOME'
    );
    assert.equal(mOut.reviewBudget.hit.produced, 13, 'the reported hit is the plan round that actually hit');
    assert.ok(mOut.summary.includes('[review budget hit:'), 'the resolved plan-gate hit still carries the summary clause');
  }

  // ORDERING: when BOTH gates hit, `hit` must report the CHRONOLOGICALLY last
  // one — the code round, which runs strictly after the plan gate completes.
  {
    const PLAN_HIT = { ...BUDGET_HIT, produced: 9, graded: 5, passedThroughBudget: 4 };
    const both = buildOutcome({
      roadmap: 'rm',
      phase: 'p',
      planFindings: [],
      planBudget: [PLAN_HIT],
      codeReviews: [[]],
      acRounds: [null],
      budgetRounds: [BUDGET_HIT],
      tier: 'medium',
    });
    assert.equal(both.reviewBudget.everHit, true, 'both gates hit');
    assert.equal(both.reviewBudget.hit.produced, 13, 'the CODE round (chronologically later) is the reported hit, not the plan gate');
    assert.ok(
      both.summary.includes('[review budget hit: 13 produced, 5 graded, 8 ungraded]'),
      'the summary clause reports the later code round, not the earlier plan round'
    );
  }

  // Backward compatibility: a caller still passing a single last-round plan
  // budget object (rather than the array) keeps working.
  const legacyPlan = buildOutcome({ roadmap: 'rm', phase: 'p', planFindings: [], planBudget: BUDGET_HIT, tier: 'medium' });
  assert.equal(legacyPlan.reviewBudget.everHit, true, 'a single plan-budget OBJECT is still accepted');
  assert.equal(legacyPlan.reviewBudget.planRounds, 1, 'a single plan-budget object counts as one plan round');

  // The task-shaped OUTCOME behaves identically.
  const tOut = buildTaskOutcome({
    task: 't',
    planFindings: [],
    codeReviews: [[]],
    acRounds: [null],
    budgetRounds: [BUDGET_HIT],
    tier: 'medium',
  });
  assert.equal(tOut.reviewBudget.everHit, true, 'task OUTCOME carries reviewBudget');
  assert.ok(tOut.summary.includes('[review budget hit:'), 'task OUTCOME summary carries the clause');
}


// (h) DIMENSION COVERAGE — the sibling per-round accounting. A gate whose review
// round lost a dimension must surface it on the OUTCOME (`reviewCoverage`) AND,
// visibly, in the `summary` string; a complete run must leave the summary
// BYTE-UNCHANGED so no existing assertion is disturbed.
{
  const BUDGET_HIT = { max: 5, produced: 13, gating: 13, graded: 5, passedThroughNonGating: 0, passedThroughBudget: 8, refuterErrors: 0, hit: true };
  const COV_PARTIAL = {
    mode: 'code',
    total: 7,
    selected: ['ac', 'correctness', 'tests', 'architecture', 'api-docs', 'changelog', 'security'],
    ran: ['tests', 'api-docs', 'changelog'],
    failed: ['ac', 'correctness', 'architecture', 'security'],
    retried: ['ac', 'correctness', 'architecture', 'security'],
    complete: false,
    acDimensionRan: false,
    acTableAbsent: true,
  };
  const COV_FULL = {
    mode: 'code',
    total: 7,
    selected: ['ac', 'correctness', 'tests', 'architecture', 'api-docs', 'changelog', 'security'],
    ran: ['ac', 'correctness', 'tests', 'architecture', 'api-docs', 'changelog', 'security'],
    failed: [],
    retried: [],
    complete: true,
    acDimensionRan: true,
    acTableAbsent: false,
  };

  // runCodeGate collects one coverage entry PER ROUND, exactly like budgetRounds.
  const h = makeCodeFakes({ reviewScript: [[B('bug')], []] });
  const inner = h.deps.review;
  let round = 0;
  h.deps.review = async () => {
    const r = await inner();
    round++;
    return { ...r, coverage: round === 1 ? COV_PARTIAL : COV_FULL };
  };
  const codeGate = await runCodeGate({ maxRework: 2, tier: 'medium' }, h.deps);
  assert.equal(codeGate.coverageRounds.length, 2, 'runCodeGate records one coverage entry PER ROUND');
  assert.equal(codeGate.coverageRounds[0].complete, false, 'round 1 lost four dimensions');
  assert.equal(codeGate.coverageRounds[1].complete, true, 'round 2 ran them all');

  const out = buildOutcome({
    roadmap: 'rm',
    phase: 'p',
    planFindings: [],
    codeReviews: codeGate.rounds,
    acRounds: codeGate.acRounds,
    budgetRounds: codeGate.budgetRounds,
    coverageRounds: codeGate.coverageRounds,
    maxRework: 2,
    tier: 'medium',
  });
  assert.equal(out.outcome, 'reviewed', 'coverage does NOT gate — the outcome is unchanged');
  assert.equal(out.reviewCoverage.complete, false, 'a round-1 gap stays visible even after a clean round 2');
  assert.deepEqual(
    out.reviewCoverage.failed,
    ['ac', 'correctness', 'architecture', 'security'],
    'a caller can tell 3-of-7 coverage from 7-of-7'
  );
  assert.equal(out.reviewCoverage.total, 7, 'and how many dimensions were selected');
  assert.equal(out.reviewCoverage.acTableAbsent, true, 'an absent AC table is recorded distinctly from a clean one');
  assert.ok(
    out.summary.includes('[review coverage: 3/7 dimensions ran; failed: ac,correctness,architecture,security; NO AC TABLE]'),
    'reduced coverage is VISIBLE in the OUTCOME summary, not just in the machine-readable key'
  );

  // A complete run: no clause at all, summary byte-unchanged.
  const clean = buildOutcome({
    roadmap: 'rm',
    phase: 'p',
    planFindings: [],
    codeReviews: [[]],
    acRounds: [null],
    coverageRounds: [COV_FULL],
    tier: 'medium',
  });
  assert.equal(clean.reviewCoverage.complete, true, 'a complete run reports complete coverage');
  assert.equal(clean.summary, 'phase reviewed clean: no surviving findings', 'and its summary is BYTE-UNCHANGED');

  // An older caller reporting no coverage at all.
  const legacy = buildOutcome({ roadmap: 'rm', phase: 'p', planFindings: [], codeFindings: [], tier: 'medium' });
  assert.equal(legacy.reviewCoverage, null, 'no coverage reported -> null, never a fabricated complete object');
  assert.ok(!legacy.summary.includes('review coverage'), 'and no clause');

  // fetchError never ran a review: it must NOT read as full coverage.
  const fe = buildOutcome({ roadmap: 'rm', phase: 'p', fetchError: true });
  assert.equal(fe.reviewCoverage, null, 'a fetch failure reports null coverage, never a complete one');

  // ORDERING is fixed and deterministic: budget clause first, coverage second.
  const both = buildOutcome({
    roadmap: 'rm',
    phase: 'p',
    planFindings: [],
    codeReviews: [[]],
    acRounds: [null],
    budgetRounds: [BUDGET_HIT],
    coverageRounds: [COV_PARTIAL],
    tier: 'medium',
  });
  assert.ok(
    both.summary.indexOf('[review budget hit:') < both.summary.indexOf('[review coverage:'),
    'the budget clause always precedes the coverage clause'
  );

  // The task shape carries it too.
  const tOut = buildTaskOutcome({
    task: 'slug',
    planFindings: [],
    codeReviews: [[]],
    acRounds: [null],
    coverageRounds: [COV_PARTIAL],
    tier: 'medium',
  });
  assert.equal(tOut.reviewCoverage.complete, false, 'task OUTCOME carries reviewCoverage');
  assert.ok(tOut.summary.includes('[review coverage:'), 'task OUTCOME summary carries the clause');

  // The PLAN gate's own coverage is counted independently and still surfaces.
  const ph = makePlanFakes({ reviewScript: [[]] });
  const planReviewInner = ph.deps.review;
  ph.deps.review = async (doc) => ({ ...(await planReviewInner(doc)), coverage: { ...COV_PARTIAL, mode: 'plan', acTableAbsent: false } });
  const planGate = await runPlanGate({ maxRevise: 0, tier: 'medium' }, ph.deps);
  assert.equal(planGate.coverageRounds.length, 1, 'runPlanGate records a coverage entry per review round');
  const withPlan = buildOutcome({
    roadmap: 'rm',
    phase: 'p',
    planFindings: [],
    codeReviews: [[]],
    acRounds: [null],
    planCoverage: planGate.coverageRounds,
    tier: 'medium',
  });
  assert.equal(withPlan.reviewCoverage.planRounds, 1, 'the plan gate is counted separately');
  assert.ok(withPlan.summary.includes('[review coverage:'), 'a plan-gate coverage gap is visible on the OUTCOME');
  assert.ok(!withPlan.summary.includes('NO AC TABLE'), 'plan mode never claims an absent AC table');
}

// (g) parseDispatchArgs validates maxRefutations at PARSE time, before any agent
// call, exactly like the two retry budgets.
assert.equal(parseDispatchArgs({ rdmBin: 'rdm' }).maxRefutations, 5, 'maxRefutations defaults to the review core\'s 5');
assert.equal(parseDispatchArgs({ rdmBin: 'rdm', maxRefutations: 0 }).maxRefutations, 0, '0 is legal and distinct from unset');
assert.equal(parseDispatchArgs({ rdmBin: 'rdm', maxRefutations: '3' }).maxRefutations, 3, 'an integer-only string is accepted');
assert.throws(
  () => parseDispatchArgs({ rdmBin: 'rdm', maxRefutations: 'x' }),
  /maxRefutations must be a non-negative integer/,
  'an invalid maxRefutations throws at parse time'
);
assert.throws(() => parseDispatchArgs({ rdmBin: 'rdm', maxRefutations: '5abc' }), /maxRefutations/, "'5abc' is rejected, not coerced to 5");
assert.throws(() => parseDispatchArgs({ rdmBin: 'rdm', maxRefutations: -1 }), /maxRefutations/, 'a negative budget is rejected');

// ============================================================================
// AC8 — a caller with NO ROADMAP IN HAND never produces a blocking
// intent-alignment finding, and therefore never exhausts maxPlanRevise.
//
// This is the failure mode the whole non-blocking missing-intent policy exists
// to prevent: a blocking finding on a document nobody can revise would burn
// every plan-revise round and escalate the phase — "noise parks phases blocked
// and halts autopilot". The gate is driven over the REAL plan pipeline (not a
// scripted reviewer), with NO `intent` and NO `signals`, so selection fail-opens
// and the intent-alignment finder is actually dispatched and must honor its own
// backstop.
// ============================================================================
{
  const reviewMod = await import(new URL('./review.mjs', pathToFileURL(libPath)).href);
  const refParallel = (thunks) =>
    Promise.all(
      thunks.map(async (t) => {
        try {
          return await t();
        } catch {
          return null;
        }
      })
    );
  const refPipeline = async (items, ...stages) =>
    Promise.all(
      items.map(async (item, i) => {
        let acc = item;
        for (const stage of stages) {
          try {
            acc = await stage(acc, item, i);
          } catch {
            return null;
          }
        }
        return acc;
      })
    );
  const finderCalls = [];
  const fakeAgent = async (prompt, opts) => {
    const label = (opts && opts.label) || '';
    if (label.indexOf('find:') === 0) {
      finderCalls.push({ label, prompt });
      // Every finder is clean. The intent-alignment finder in particular honors
      // the backstop prose: with no recorded intent in the material it was
      // given, it returns an empty findings array rather than manufacturing one.
      return { findings: [] };
    }
    if (label.indexOf('refute:') === 0) return { refuted: false, confidence: 90 };
    throw new Error('unexpected agent label: ' + label);
  };
  const realPlanReview = reviewMod.buildReviewPipeline('plan', {
    agent: fakeAgent,
    pipeline: refPipeline,
    parallel: refParallel,
    log: () => {},
  });

  let planCount = 0;
  let reviseCount = 0;
  let reviewCount = 0;
  const res = await runPlanGate(
    { maxRevise: 2, tier: 'medium' },
    {
      plan: async () => {
        planCount++;
        return { doc: 'a plan document with no recorded intent behind it' };
      },
      revise: async () => {
        reviseCount++;
        return { doc: 'revised' };
      },
      // Exactly the shape the workflow's own plan-gate callback has: a target,
      // an `intent` value that is null here, and NO `signals` key.
      review: async (doc) => {
        reviewCount++;
        return await realPlanReview({ target: String(doc.doc), intent: null, maxRefutations: 5 });
      },
    }
  );

  // The intent-alignment finder DID run (selection fail-opens with no signals),
  // proving the backstop prose is what kept the gate quiet — not a lucky skip.
  assert.ok(
    finderCalls.some((c) => c.label === 'find:plan:intent-alignment'),
    'AC8: with no signals, selection fail-opens and the intent-alignment finder IS dispatched'
  );
  assert.equal(planCount, 1, 'AC8: the plan is authored exactly once');
  assert.equal(reviewCount, 1, 'AC8: exactly ONE review round — no blocking finding forced another');
  assert.equal(reviseCount, 0, 'AC8: maxPlanRevise is never consumed by a missing intent');
  assert.equal(reviseCount < 2, true, 'AC8: the plan-revise budget is not exhausted');
  assert.equal(reviewMod.hasBlocking(res.findings), false, 'AC8: no blocking finding reaches the gate');
  // The absence IS reported — as a non-gating suggestion, on the same array.
  assert.ok(
    res.findings.some((f) => f.concern === 'intent-alignment' && f.severity === 'suggestion'),
    'AC8: the missing input is reported as a non-blocking suggestion, not silently skipped'
  );
}
console.log('AC8: a caller with no roadmap in hand never blocks and never exhausts maxPlanRevise');

console.log('all dispatch-phase gate assertions passed');
NODE_TEST

if run_node "$TMP/gates.mjs" "$LIB"; then
    pass "budget-bounded gates verified (per-budget call counts, budget-0 regression, independence, boundedness, validation)"
else
    fail "dispatch-phase gate assertions failed"
fi

# --- 1d. VERIFY GATE ----------------------------------------------------------
# The phase-time verification gate (docs/verify-gate.md): one project-supplied
# command, run once per implementation attempt, whose exit code decides whether
# the item can be reported reviewed. Driven in Node against injected fakes, so
# every assertion below is about the SHIPPED logic and costs zero LLM calls.
say "1d. Verify gate: one command per implementation attempt, rework on failure, escalate when unresolvable"

cat >"$TMP/verify-gate.mjs" <<'NODE_VERIFY'
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const libPath = process.argv[2];
const wfPath = process.argv[3];
const mod = await import(pathToFileURL(libPath).href);
const {
  runCodeGate,
  buildOutcome,
  buildTaskOutcome,
  statusFor,
  classifyOutcome,
  GATE_POLICY,
  truncateVerifyOutput,
  VERIFY_OUTPUT_CAP,
  extractVerifyCommand,
  verifyToolingLine,
  verifyFailureClause,
  verifyFailureFinding,
  normalizeVerifyResult,
  VERIFY_UNRESOLVED_SUMMARY,
} = mod;

const CMD = 'sh scripts/verify-all.sh';

// A counting fake code gate. `verifyScript` is consumed per verify call,
// repeating its last entry; `mode` selects ok / non-zero / throw / null.
function makeFakes(o) {
  const opts = o || {};
  const log = [];
  let vIdx = 0;
  const script = opts.verifyScript || ['ok'];
  const deps = {
    implement: async (notes) => {
      log.push(notes == null ? 'implement' : 'rework');
    },
    review: async () => {
      log.push('review');
      return { survivors: opts.survivors || [], acTable: null };
    },
    verify: async (command) => {
      log.push('verify');
      assert.equal(command, CMD, 'the verify dep receives the resolved command verbatim');
      const kind = script[Math.min(vIdx, script.length - 1)];
      vIdx++;
      if (kind === 'throw') throw new Error('agent blew up');
      if (kind === 'null') return null;
      if (kind === 'ok') return { exitCode: 0, output: 'all good' };
      return { exitCode: 1, output: 'harness red' };
    },
  };
  if (opts.noVerifyDep) delete deps.verify;
  // The terminal tail's two deps. Both are OPTIONAL and are wired only when the
  // scenario asks for them, so every pre-existing scenario above keeps its exact
  // call counts and its `reviewed` outcome.
  if (opts.act !== undefined) {
    deps.act = async (findings) => {
      log.push('act');
      if (opts.act === 'throw') throw new Error('act blew up');
      return typeof opts.act === 'function' ? opts.act(findings) : opts.act;
    };
  }
  if (opts.clean !== undefined) {
    deps.clean = async () => {
      log.push('clean');
      if (opts.clean === 'throw') throw new Error('clean probe blew up');
      return opts.clean;
    };
  }
  return { log, deps };
}
const count = (log, kind) => log.filter((c) => c === kind).length;
const lastIndexOfCall = (log, kind) => log.lastIndexOf(kind);
// A single non-gating survivor: at tier `medium` only `blocking` gates, so the
// round is CLEAN and the act step runs. This is how every act-path scenario
// below reaches the terminal tail without first burning the rework budget.
const NON_GATING = [{ id: 'f1', severity: 'suggestion', confidence: 90, what_fails: 'x', unrefuted: true }];

// ===========================================================================
// (a) Pure helpers.
// ===========================================================================
assert.equal(extractVerifyCommand({}), '', 'an absent verify field yields the empty string');
assert.equal(extractVerifyCommand({ verify: '   ' }), '', 'a whitespace-only value yields the empty string');
assert.equal(extractVerifyCommand({ verify: '  ' + CMD + ' ' }), CMD, 'a real command is trimmed and returned');
assert.equal(extractVerifyCommand({ verify: 42 }), '', 'a non-string value yields the empty string rather than throwing');
assert.equal(extractVerifyCommand({ verify: 'a\nb' }), '', 'a multi-line command is refused, not run');
assert.equal(extractVerifyCommand(null), '', 'a null meta yields the empty string');
// `rdm config get` WITHOUT --raw prints `<value>  (source: <where>)`. The
// resolution prompt asks for --raw so that never reaches us, but the value
// crosses an LLM: an annotated line must resolve to the command it names, not
// to a bash syntax error blamed on the project.
for (const src of ['repo config', 'global config', 'environment variable']) {
  assert.equal(
    extractVerifyCommand({ verify: CMD + '  (source: ' + src + ')' }),
    CMD,
    'a `config get` source annotation (' + src + ') is stripped, not run'
  );
}
assert.equal(extractVerifyCommand({ verify: '  (source: repo config)' }), '', 'an annotation with no value ahead of it yields the empty string');
assert.equal(extractVerifyCommand({ verify: 'sh x.sh (source of truth)' }), 'sh x.sh (source of truth)', 'only the trailing `(source: ...)` annotation is stripped — ordinary parentheses survive');

const longOut = 'x'.repeat(VERIFY_OUTPUT_CAP + 500) + 'TAIL-MARKER';
const truncated = truncateVerifyOutput(longOut);
assert.ok(truncated.length < longOut.length, 'an oversized output is truncated');
assert.ok(truncated.endsWith('TAIL-MARKER'), 'truncation keeps the TAIL — failures print last');
assert.equal(truncateVerifyOutput('short'), 'short', 'a short output is returned unchanged');
assert.equal(truncateVerifyOutput(null), '', 'a non-string output yields the empty string');

assert.ok(verifyToolingLine(CMD).includes(CMD), 'the tooling line names the command');
assert.equal(verifyToolingLine(''), '', 'an empty command renders no tooling line');
assert.equal(verifyFailureClause({ failed: false }), '', 'a passing verify renders no failure clause');
assert.equal(verifyFailureClause(null), '', 'an absent verify result renders no failure clause');
const clause = verifyFailureClause({ command: CMD, failed: true, exitCode: 7, output: 'OUT-SENTINEL' });
assert.ok(clause.includes(CMD) && clause.includes('7') && clause.includes('OUT-SENTINEL'), 'the failure clause carries command, exit code, and output tail');

// Fail-closed normalization: a throw, a null, and a missing exit code are all failures.
for (const [name, raw, threw] of [
  ['throw', null, true],
  ['null resolution', null, false],
  ['no exit code', { output: 'x' }, false],
]) {
  const n = normalizeVerifyResult(CMD, raw, threw);
  assert.equal(n.failed, true, name + ' must be treated as a FAILURE (fail-closed)');
  assert.equal(n.ran, false, name + ' did not observe a real exit status');
}
assert.equal(normalizeVerifyResult(CMD, { exitCode: 0, output: '' }, false).failed, false, 'exit 0 passes');
assert.equal(normalizeVerifyResult(CMD, { exitCode: 3, output: '' }, false).failed, true, 'a non-zero exit fails');

const finding = verifyFailureFinding({ command: CMD, exitCode: 1, output: 'harness red' });
assert.equal(finding.severity, 'blocking', 'the synthesized finding is blocking');
assert.ok(finding.what_fails.includes(CMD), 'the command lives in what_fails — the field summarizeFindings renders');

// ===========================================================================
// (b) AC2 — a non-zero exit yields rework whose reason names the command, and
//     the phase never reaches reviewed. A clean review is the ONLY other input,
//     so the verify failure is provably the sole cause.
// ===========================================================================
const seededOutcomes = new Set();
{
  const f = makeFakes({ verifyScript: ['fail'] });
  const gate = await runCodeGate({ maxRework: 0, tier: 'medium', verifyCommand: CMD }, f.deps);
  const out = buildOutcome({
    roadmap: 'rm',
    phase: '1',
    codeReviews: gate.rounds,
    acRounds: gate.acRounds,
    maxRework: 0,
    tier: 'medium',
  });
  seededOutcomes.add(out.outcome);
  assert.equal(out.outcome, 'rework', 'a failing verify with an otherwise-clean review yields rework');
  assert.notEqual(out.outcome, 'reviewed', 'a failing verify can never report reviewed');
  assert.equal(out.status, statusFor('rework', 'phase'), 'the status comes from the untouched statusFor table');
  assert.ok(out.summary.includes(CMD), 'the OUTCOME summary names the failing command');
  assert.ok(out.reason.includes(CMD), 'the OUTCOME reason names the failing command');
}
{
  const f = makeFakes({ verifyScript: ['ok'] });
  const gate = await runCodeGate({ maxRework: 0, tier: 'medium', verifyCommand: CMD }, f.deps);
  const out = buildOutcome({ roadmap: 'rm', phase: '1', codeReviews: gate.rounds, acRounds: gate.acRounds, maxRework: 0, tier: 'medium' });
  seededOutcomes.add(out.outcome);
  assert.equal(out.outcome, 'reviewed', 'exit 0 with a clean review still reports reviewed — the gate is not a blanket rejecter');
}
for (const kind of ['throw', 'null']) {
  const f = makeFakes({ verifyScript: [kind] });
  const gate = await runCodeGate({ maxRework: 0, tier: 'medium', verifyCommand: CMD }, f.deps);
  const out = buildOutcome({ roadmap: 'rm', phase: '1', codeReviews: gate.rounds, acRounds: gate.acRounds, maxRework: 0, tier: 'medium' });
  seededOutcomes.add(out.outcome);
  assert.equal(out.outcome, 'rework', 'a verify dep that ' + kind + 's is FAIL-CLOSED, never a pass');
}
console.log('1d(b) OK: a non-zero exit (and a thrown/null verify) yields rework naming the command; exit 0 still reviews clean');

// ===========================================================================
// (c) AC3 — the unresolved-verify escalation, for both item shapes.
// ===========================================================================
for (const [kind, build, ident] of [
  ['phase', buildOutcome, { roadmap: 'rm', phase: '1' }],
  ['task', buildTaskOutcome, { task: 'my-task' }],
]) {
  const out = build({ ...ident, verifyUnresolved: true });
  seededOutcomes.add(out.outcome);
  assert.equal(out.outcome, 'escalated', kind + ': an unresolvable verify command escalates');
  assert.notEqual(out.outcome, 'reviewed', kind + ': an unresolvable verify command never reports reviewed');
  assert.equal(out.status, statusFor('escalated', kind), kind + ': the status is the untouched escalated mapping (blocked)');
  assert.equal(out.writesCompletion, false, kind + ': an escalated unit never writes a completion trailer');
  for (const src of ['dispatch.verify', '.github/workflows/', 'principles.md', 'CLAUDE.md', 'AGENTS.md']) {
    assert.ok(out.summary.includes(src), kind + ': the escalation summary must name ' + src);
  }
  assert.ok(out.reason.includes(VERIFY_UNRESOLVED_SUMMARY), kind + ': the reason carries the summary');
}
console.log('1d(c) OK: an unresolvable verify command escalates for both phases and tasks, naming every discovery source');

// ===========================================================================
// (d) AC4/AC6 — one call per implementation attempt, bounded by maxCodeRework
//     alone. Four scenarios, exact integers, verify count == implement count.
// ===========================================================================
for (const [name, script, budget, expected, expectedOutcome] of [
  ['clean first pass', ['ok'], 2, 1, 'reviewed'],
  ['fail then pass', ['fail', 'ok'], 2, 2, 'reviewed'],
  ['always fail, budget 2', ['fail'], 2, 3, 'rework'],
  ['always fail, budget 0', ['fail'], 0, 1, 'rework'],
]) {
  const f = makeFakes({ verifyScript: script });
  const gate = await runCodeGate({ maxRework: budget, tier: 'medium', verifyCommand: CMD }, f.deps);
  const implementCalls = count(f.log, 'implement') + count(f.log, 'rework');
  assert.equal(gate.verifyCalls, expected, name + ': exactly ' + expected + ' verify call(s)');
  assert.equal(implementCalls, expected, name + ': verify count equals implement count');
  assert.equal(count(f.log, 'verify'), expected, name + ": the fake's own call count agrees");
  assert.equal(gate.rounds.length, expected, name + ': one review round per implementation attempt');
  const out = buildOutcome({ roadmap: 'rm', phase: '1', codeReviews: gate.rounds, acRounds: gate.acRounds, maxRework: budget, tier: 'medium' });
  seededOutcomes.add(out.outcome);
  assert.equal(out.outcome, expectedOutcome, name + ': outcome is ' + expectedOutcome);
  assert.equal(out.status, statusFor(expectedOutcome, 'phase'), name + ': status read from the untouched table');
}
console.log('1d(d) OK: the command runs exactly once per implementation attempt (1/2/3/1) and is bounded by maxCodeRework alone');

// A gate with NO verify dep, or an empty command, calls nothing and stays clean.
{
  const f = makeFakes({ noVerifyDep: true });
  const gate = await runCodeGate({ maxRework: 2, tier: 'medium', verifyCommand: CMD }, f.deps);
  assert.equal(gate.verifyCalls, 0, 'a gate with no verify dep runs no verification');
  assert.equal(count(f.log, 'verify'), 0, 'and calls nothing');
  const f2 = makeFakes({ verifyScript: ['fail'] });
  const gate2 = await runCodeGate({ maxRework: 2, tier: 'medium', verifyCommand: '' }, f2.deps);
  assert.equal(gate2.verifyCalls, 0, 'an empty resolved command runs no verification');
}

// The rework implementer is handed the PRIOR round's failing verify result.
{
  const notes = [];
  const f = makeFakes({ verifyScript: ['fail', 'ok'] });
  const inner = f.deps.implement;
  f.deps.implement = async (n) => {
    notes.push(n);
    return inner(n);
  };
  await runCodeGate({ maxRework: 2, tier: 'medium', verifyCommand: CMD }, f.deps);
  assert.equal(notes.length, 2, 'one first pass plus one rework');
  assert.equal(notes[0], null, 'the first pass gets null rework notes');
  assert.ok(notes[1] && notes[1].verify, 'the rework notes carry the verify result');
  assert.equal(notes[1].verify.failed, true, "and it is the PRIOR round's FAILING result");
  assert.ok(verifyFailureClause(notes[1].verify).includes('harness red'), 'whose output reaches the rework clause');
}
console.log('1d(d2) OK: an absent dep/command runs nothing, and the rework notes carry the prior failing result');

// ===========================================================================
// (e) AC5 — the OUTCOME vocabulary is exactly reviewed|rework|escalated.
// ===========================================================================
assert.deepEqual([...seededOutcomes].sort(), ['escalated', 'reviewed', 'rework'], 'every seeded verify scenario lands in the canonical vocabulary');
assert.deepEqual(Object.keys(GATE_POLICY.code).sort(), ['escalated', 'reviewed', 'rework'], 'GATE_POLICY.code was not widened');
assert.deepEqual(Object.keys(GATE_POLICY.plan).sort(), ['escalated', 'reviewed', 'rework'], 'GATE_POLICY.plan was not widened');
console.log('1d(e) OK: the OUTCOME value set is exactly reviewed|rework|escalated and GATE_POLICY is untouched');

// Determinism: identical inputs produce identical outcomes.
{
  const run = async () => {
    const f = makeFakes({ verifyScript: ['fail'] });
    const g = await runCodeGate({ maxRework: 1, tier: 'medium', verifyCommand: CMD }, f.deps);
    return buildOutcome({ roadmap: 'rm', phase: '1', codeReviews: g.rounds, acRounds: g.acRounds, maxRework: 1, tier: 'medium' });
  };
  assert.deepEqual(await run(), await run(), 'the verify gate is deterministic across identical runs');
}

// ===========================================================================
// (f) AC7 — the resolved command reaches BOTH implementer prompts. The real
//     builder is extracted from the shipped workflow file.
// ===========================================================================
const wfSrc = fs.readFileSync(wfPath, 'utf8').replace(/^export /m, '');
const shimPath = path.join(os.tmpdir(), 'verify-gate-implement-prompt-' + process.pid + '.mjs');
fs.writeFileSync(
  shimPath,
  'export default async function(args, agent, pipeline, parallel, log) {\n' +
    wfSrc.replace(
      /^\/\/ --- Driver ---/m,
      'return { buildImplementPrompt: buildImplementPrompt, buildFetchPrompt: buildFetchPrompt, buildTaskFetchPrompt: buildTaskFetchPrompt }\n// --- Driver ---'
    ) +
    '\n}\n'
);
const shim = (await import('file://' + shimPath + '?t=' + process.pid)).default;
const exported = await shim({}, async () => null, null, null, () => {});
const buildImplementPrompt = exported.buildImplementPrompt;
assert.equal(typeof buildImplementPrompt, 'function', 'buildImplementPrompt was extracted from the shipped workflow');

const SENTINEL = 'CMD-SENTINEL-123';
const cfg = { rdmBin: '/fake/bin/rdm', project: 'demo' };
const firstPass = buildImplementPrompt('rm', 'BODY', 'PLAN', null, cfg, SENTINEL);
assert.ok(firstPass.includes(SENTINEL), 'the FIRST-PASS implementer prompt names the resolved command');
const reworkPrompt = buildImplementPrompt('rm', 'BODY', 'PLAN', {
  findings: [{ id: 'f1', severity: 'blocking', concern: 'x', confidence: 90, what_fails: 'y' }],
  acTable: null,
  verify: { command: SENTINEL, failed: true, exitCode: 9, output: 'OUT-SENTINEL' },
}, cfg, SENTINEL);
assert.ok(reworkPrompt.includes(SENTINEL), 'the REWORK implementer prompt names the resolved command');
assert.ok(reworkPrompt.includes('OUT-SENTINEL'), 'the rework prompt carries the failing output tail');
assert.ok(reworkPrompt.includes('9'), 'the rework prompt carries the failing exit code');
console.log('1d(f) OK: the resolved command reaches both the first-pass and rework implementer prompts');

// ===========================================================================
// (f2) PROJECT DIRECTIVES reach BOTH implementer prompts, and an ABSENT set
//      changes nothing. (dispatch-dev-discipline phase 3, AC1 + AC5.)
//      scripts/verify-project-directives.sh owns the full selection/rendering
//      contract; this section owns the WIRING — that the shipped builder
//      actually threads the block onto the shared path, where a refactor that
//      moved it into one branch would go unnoticed.
// ===========================================================================
{
  const DIR_SENTINEL = 'DIRECTIVE-SENTINEL-XYZ';
  const block = '--- PROJECT DIRECTIVE: .claude/rules/x.md ---\n' + DIR_SENTINEL + '\n--- END PROJECT DIRECTIVE ---';
  const first = buildImplementPrompt('rm', 'BODY', 'PLAN', null, cfg, SENTINEL, block);
  assert.ok(first.includes(DIR_SENTINEL), '(f2) the FIRST-PASS implementer prompt carries the directive block');
  const re = buildImplementPrompt('rm', 'BODY', 'PLAN', {
    findings: [{ id: 'f1', severity: 'blocking', concern: 'x', confidence: 90, what_fails: 'y' }],
    acTable: null,
  }, cfg, SENTINEL, block);
  assert.ok(re.includes(DIR_SENTINEL), '(f2) the REWORK implementer prompt carries it too — the push is on the SHARED path');
  // The verify tooling line must still come first: phase 1's positional greps
  // and its escalation prose depend on that ordering staying stable.
  assert.ok(first.indexOf(SENTINEL) < first.indexOf(DIR_SENTINEL), '(f2) the verify tooling line still precedes the directive block');

  // ABSENT ⇒ byte-identical to a directives-free prompt. `join('\n')` turns a
  // pushed '' into a real blank line, so this only holds because the push is
  // emptiness-guarded — assert it rather than assume it.
  assert.equal(
    buildImplementPrompt('rm', 'BODY', 'PLAN', null, cfg, SENTINEL, ''),
    buildImplementPrompt('rm', 'BODY', 'PLAN', null, cfg, SENTINEL),
    '(f2) an EMPTY directive block leaves the implementer prompt byte-identical'
  );
  assert.ok(
    !buildImplementPrompt('rm', 'BODY', 'PLAN', null, cfg, SENTINEL, '').includes('PROJECT DIRECTIVE'),
    '(f2) ...and carries no directive fence at all'
  );
  console.log('1d(f2) OK: the directive block reaches both implementer branches, after the tooling line, and is inert when absent');
}

// ===========================================================================
// (g) AC3's OTHER half — the Stage-0 RESOLUTION prompt itself. 1d(c) and 6h
//     prove what the driver does with a resolved (or unresolvable) command,
//     but every driven fake answers `fetch:*` with a canned `verify` field
//     regardless of the prompt it was handed. Nothing there observes whether
//     the real agent is ever TOLD how to resolve one. This asserts the shipped
//     prompt text: the declared key first, then the three discovery sources in
//     order. The self-test below strips the concat and requires this to fail.
// ===========================================================================
{
  const buildFetchPrompt = exported.buildFetchPrompt;
  const buildTaskFetchPrompt = exported.buildTaskFetchPrompt;
  assert.equal(typeof buildFetchPrompt, 'function', 'buildFetchPrompt was extracted from the shipped workflow');
  assert.equal(typeof buildTaskFetchPrompt, 'function', 'buildTaskFetchPrompt was extracted from the shipped workflow');

  // The declared-key read, exactly as the CLI must be invoked for it: `--raw`
  // is load-bearing, because a bare `config get` prints
  // `<value>  (source: repo config)` and the agent is told to return the line
  // VERBATIM.
  const DECLARED = '/fake/bin/rdm config get dispatch.verify --raw';
  // Precedence, in order. The prompt is a single string, so ORDER is asserted
  // by index rather than by mere presence.
  const ORDERED = ['dispatch.verify', '.github/workflows/', 'docs/principles.md', 'CLAUDE.md'];

  for (const [name, prompt] of [
    ['phase', buildFetchPrompt('rm', '1', cfg)],
    ['task', buildTaskFetchPrompt('my-task', cfg)],
  ]) {
    assert.ok(prompt.includes(DECLARED), name + ': the Stage-0 prompt tells the agent to read the declared key with --raw');
    assert.ok(/--raw/.test(prompt) && /source:/.test(prompt), name + ': ... and explains what --raw suppresses, so an annotated line is never returned verbatim');
    assert.ok(prompt.includes('AGENTS.md'), name + ': the prompt names AGENTS.md alongside CLAUDE.md');
    let cursor = -1;
    for (const src of ORDERED) {
      const at = prompt.indexOf(src);
      assert.ok(at > -1, name + ': the resolution paragraph must name ' + src);
      assert.ok(at > cursor, name + ': ' + src + ' must appear in precedence order');
      cursor = at;
    }
    assert.ok(/EMPTY STRING/.test(prompt), name + ': ... and the prompt says to return an empty string when nothing resolves, so the driver can escalate');

    // PROJECT DIRECTIVES ride on the SAME Stage-0 agent, concatenated AFTER the
    // verify paragraph so the positional greps above stay stable.
    const DIRECTIVES_CMD = '/fake/bin/rdm dispatch directives --format json';
    assert.ok(prompt.includes(DIRECTIVES_CMD), name + ': the Stage-0 prompt tells the agent to resolve project directives');
    assert.ok(
      prompt.indexOf(DECLARED) < prompt.indexOf(DIRECTIVES_CMD),
      name + ': ... and does so AFTER the verify resolution paragraph, so phase 1\'s ordering greps stay stable'
    );
    assert.ok(/CHARACTER FOR CHARACTER/.test(prompt), name + ': ... instructing a verbatim copy, since a paraphrase is exactly what must not reach the implementer');
    assert.ok(/directivesSkipped/.test(prompt), name + ': ... and asking for the skipped list, so a withheld rule stays observable');
    // The failure direction is OPPOSITE to the verify command's: an absent set is
    // normal and must produce NOTHING, never an invented directive.
    assert.ok(/Absent directives are NORMAL/.test(prompt), name + ': ... and says absent directives are normal rather than an error');
  }
  console.log('1d(g) OK: both Stage-0 fetch prompts carry the declared-key-then-discovery resolution paragraph, in precedence order');
}

// ===========================================================================
// (h) A FIX APPLIED AFTER THE CHECK RUN RE-TRIGGERS THE CHECKS.
//     (dispatch-dev-discipline phase 2 AC3.) The act step now commits its own
//     inline fix, which means code can change AFTER the round's verify ran.
//     The terminal tail therefore re-runs the SAME single call site, and the
//     ordering is asserted by log INDEX, not by a bare count.
// ===========================================================================
{
  // (h-a) Act fixes inline, the re-verify passes: exactly one extra run, it is
  //       the LAST call in the log, and it happens AFTER the act step.
  const f = makeFakes({
    verifyScript: ['ok'],
    survivors: NON_GATING,
    act: { handled: [{ id: 'f1', action: 'fixed-inline', commit: 'abc1234' }] },
  });
  const gate = await runCodeGate({ maxRework: 2, tier: 'medium', verifyCommand: CMD }, f.deps);
  const implementCalls = count(f.log, 'implement') + count(f.log, 'rework');
  assert.equal(implementCalls, 1, '(h-a) one implementation attempt');
  assert.equal(gate.verifyCalls, implementCalls + 1, '(h-a) the checks run once more after the act step changed code');
  assert.equal(count(f.log, 'verify'), 2, "(h-a) the fake's own verify count agrees");
  assert.equal(f.log[f.log.length - 1], 'verify', '(h-a) the extra check run is the LAST thing that happens');
  assert.ok(
    lastIndexOfCall(f.log, 'verify') > lastIndexOfCall(f.log, 'act'),
    '(h-a) ... and it happens AFTER the act step, not before it — ordering, not merely count'
  );
  assert.ok(gate.postActVerify && gate.postActVerify.failed === false, '(h-a) the post-act verify result is reported out');
  const out = buildOutcome({
    roadmap: 'rm',
    phase: '1',
    codeReviews: gate.rounds,
    acRounds: gate.acRounds,
    maxRework: 2,
    tier: 'medium',
    actResult: gate.actResult,
  });
  seededOutcomes.add(out.outcome);
  assert.equal(out.outcome, 'reviewed', '(h-a) a passing re-verify still reviews clean');
}
{
  // (h-b) Act fixes inline and the re-verify FAILS. The failure folds into the
  //       FINAL round: rework, naming the command, with NO phantom review round
  //       and NO second rework budget.
  const f = makeFakes({
    verifyScript: ['ok', 'fail'],
    survivors: NON_GATING,
    act: { handled: [{ id: 'f1', action: 'fixed-inline', commit: 'abc1234' }] },
  });
  const gate = await runCodeGate({ maxRework: 2, tier: 'medium', verifyCommand: CMD }, f.deps);
  assert.equal(gate.verifyCalls, 2, '(h-b) the checks ran again after the fix');
  assert.equal(gate.rounds.length, 1, '(h-b) the failure folds into the FINAL round — no phantom review round');
  assert.equal(gate.reviewCount, 1, '(h-b) ... so reviewCount stays honest');
  assert.equal(gate.reworkCount, 0, '(h-b) ... and the rework budget is NOT re-entered');
  assert.equal(gate.acRounds.length, gate.rounds.length, '(h-b) acRounds stays index-parallel to rounds');
  assert.equal(gate.budgetRounds.length, gate.rounds.length, '(h-b) budgetRounds stays index-parallel to rounds');
  assert.equal(gate.coverageRounds.length, gate.rounds.length, '(h-b) coverageRounds stays index-parallel to rounds');
  const out = buildOutcome({
    roadmap: 'rm',
    phase: '1',
    codeReviews: gate.rounds,
    acRounds: gate.acRounds,
    maxRework: 2,
    tier: 'medium',
    actResult: gate.actResult,
  });
  seededOutcomes.add(out.outcome);
  assert.equal(out.outcome, 'rework', '(h-b) a fix that breaks the checks cannot ship as reviewed');
  assert.ok(out.summary.includes(CMD), '(h-b) the summary names the failing command');
  // The commit provenance for the finding the act step DID close survives onto
  // the rework branch — it matters most exactly here.
  assert.equal(out.findings.find((x) => x.id === 'f1').handledCommit, 'abc1234', '(h-b) handledCommit survives a post-act rework');
}
{
  // (h-c) FAIL-CLOSED: act ran but its result is unusable. An unknown act
  //       outcome must re-verify — never assume nothing changed.
  for (const [name, actValue] of [
    ['null resolution', null],
    ['thrown agent', 'throw'],
    ['no handled array', { ok: true }],
  ]) {
    const f = makeFakes({ verifyScript: ['ok'], survivors: NON_GATING, act: actValue });
    const gate = await runCodeGate({ maxRework: 2, tier: 'medium', verifyCommand: CMD }, f.deps);
    const implementCalls = count(f.log, 'implement') + count(f.log, 'rework');
    assert.equal(gate.verifyCalls, implementCalls + 1, '(h-c) ' + name + ': the checks re-run anyway (fail-closed)');
  }
}
{
  // (h-d) NEGATIVE CONTROL. An act step that changed NO code re-runs nothing,
  //       and neither does a clean round with no survivors at all — which is
  //       why (d)'s exact 1/2/3/1 counts above stay green.
  for (const [name, handled] of [
    ['all filed-as-task', [{ id: 'f1', action: 'filed-as-task', taskSlug: 't' }]],
    ['all skipped', [{ id: 'f1', action: 'skipped', reason: 'r' }]],
  ]) {
    const f = makeFakes({ verifyScript: ['ok'], survivors: NON_GATING, act: { handled: handled } });
    const gate = await runCodeGate({ maxRework: 2, tier: 'medium', verifyCommand: CMD }, f.deps);
    assert.equal(gate.verifyCalls, 1, '(h-d) ' + name + ': no code changed, so no extra check run');
  }
  const f = makeFakes({ verifyScript: ['ok'], act: { handled: [] } });
  const gate = await runCodeGate({ maxRework: 2, tier: 'medium', verifyCommand: CMD }, f.deps);
  assert.equal(count(f.log, 'act'), 0, '(h-d) a clean round with no survivors never invokes the act step');
  assert.equal(gate.verifyCalls, 1, '(h-d) ... and runs no extra check');
}
console.log('1d(h) OK: a fix applied after the check run re-triggers the checks, ordered after the act step, fail-closed on an unusable act result');

console.log('all verify-gate assertions passed');
NODE_VERIFY

if run_node "$TMP/verify-gate.mjs" "$LIB" "$WF"; then
    pass "1d: verify gate — one run per attempt, rework naming the command, fail-closed, escalate-on-unresolvable, both prompts"
else
    fail "1d: verify-gate assertions failed"
fi

# Planted-mutation self-test: strip the unconditional tooling push from a scratch
# copy of the workflow and require 1d(f) to turn red. Without this the
# both-prompts assertion could pass vacuously on a builder that never renders it.
sed 's/^  lines.push(verifyToolingLine(verifyCommand))$/  \/\/ tooling push removed by the self-test/' "$WF" >"$TMP/wf-no-tooling.js"
if cmp -s "$WF" "$TMP/wf-no-tooling.js"; then
    fail "1d self-test: the planted mutation was a no-op — the tooling push was not found"
fi
if run_node "$TMP/verify-gate.mjs" "$LIB" "$TMP/wf-no-tooling.js" >/dev/null 2>&1; then
    fail "1d self-test: the assertions PASSED against a workflow with no tooling push — 1d(f) is vacuous"
fi
pass "1d self-test: removing the implementer tooling push turns 1d red"

# Planted-mutation self-test for 1d(g): drop the resolution paragraph from BOTH
# Stage-0 fetch prompts. Without 1d(g) this mutation was invisible — every
# driven fake answers `fetch:*` with a canned verify field whatever the prompt
# says — so a refactor could ship an agent with no resolution instructions at
# all and stay green.
sed 's/^    \.concat(verifyResolutionLines(bin))$/    \.concat([])/' "$WF" >"$TMP/wf-no-resolution.js"
if cmp -s "$WF" "$TMP/wf-no-resolution.js"; then
    fail "1d self-test: the planted mutation was a no-op — no .concat(verifyResolutionLines(bin)) call site was found"
fi
if [ "$(grep -c '\.concat(\[\])' "$TMP/wf-no-resolution.js")" -ne 2 ]; then
    fail "1d self-test: expected BOTH fetch-prompt call sites to be mutated (phase + task)"
fi
if run_node "$TMP/verify-gate.mjs" "$LIB" "$TMP/wf-no-resolution.js" >/dev/null 2>&1; then
    fail "1d self-test: the assertions PASSED against fetch prompts with no resolution paragraph — 1d(g) is vacuous"
fi
pass "1d self-test: stripping the resolution paragraph from the Stage-0 fetch prompts turns 1d red"

# ... and a narrower one: keep the paragraph but drop `--raw` from the declared-key
# read. A bare `rdm config get` prints `<value>  (source: repo config)`, which the
# prompt then tells the agent to return VERBATIM — the exact mismatch that would
# hand the verify agent an unrunnable line on every declared-key dispatch.
sed "s/ config get dispatch.verify --raw'/ config get dispatch.verify'/" "$WF" >"$TMP/wf-no-raw.js"
if cmp -s "$WF" "$TMP/wf-no-raw.js"; then
    fail "1d self-test: the planted mutation was a no-op — the --raw declared-key read was not found"
fi
if run_node "$TMP/verify-gate.mjs" "$LIB" "$TMP/wf-no-raw.js" >/dev/null 2>&1; then
    fail "1d self-test: the assertions PASSED against a bare (annotated) 'config get' read — 1d(g) does not pin --raw"
fi
pass "1d self-test: dropping --raw from the declared-key read turns 1d red"

# --- 1e. CLEAN-WORKTREE GATE ---------------------------------------------------
# `reviewed` must mean landable. A dispatch that reports an item reviewed while
# work it produced is still uncommitted is a false green: `rdm-land` rebases
# before merging, so that work never ships and the review finding it satisfied
# is satisfied by nothing. Two complementary mechanisms are gated here (see
# docs/verify-gate.md § 8):
#
#   (i)  the Act step COMMITS its own inline fix, with a `Review-Finding: <id>`
#        message trailer and the short sha reported back on the `handled` entry;
#   (ii) a TERMINAL cleanliness assertion runs `git status --porcelain` in the
#        item's worktree and folds a non-empty result into the FINAL round as a
#        mechanical blocking finding, so the untouched classifier says `rework`.
#
# Driven in Node against the real lib (zero LLM calls), and — for the parser —
# against a REAL seeded dirty git tree, so the porcelain shapes are git's own
# rather than a fixture author's guess.
say "1e. Clean-worktree gate: reviewed implies an empty git status, the act step commits its fix"

# A REAL dirty worktree: a tracked-and-modified file, an untracked file, and a
# renamed file (so the `old -> new` porcelain shape is git's, not a guess).
DIRTY_REPO="$TMP/dirty-worktree"
mkdir -p "$DIRTY_REPO"
(
    cd "$DIRTY_REPO" || exit 1
    git init -q -b main .
    git config user.email harness@example.invalid
    git config user.name Harness
    printf 'one\n' >tracked.rs
    printf 'two\n' >"old name.rs"
    git add -A
    git commit -qm seed
    printf 'one\nchanged\n' >>tracked.rs
    printf 'three\n' >untracked.rs
    git mv "old name.rs" "new name.rs"
) >/dev/null 2>&1 || fail "1e: could not seed the real dirty git worktree"
(cd "$DIRTY_REPO" && git status --porcelain) >"$TMP/real-porcelain.txt" ||
    fail "1e: could not capture git status --porcelain from the seeded tree"
[ -s "$TMP/real-porcelain.txt" ] || fail "1e: the seeded worktree came back CLEAN — the fixture proves nothing"

cat >"$TMP/clean-gate.mjs" <<'NODE_CLEAN'
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { pathToFileURL } from 'node:url';

const libPath = process.argv[2];
const realPorcelainPath = process.argv[3];
const mod = await import(pathToFileURL(libPath).href);
const {
  runCodeGate,
  buildOutcome,
  buildTaskOutcome,
  statusFor,
  parseWorktreeStatus,
  dirtyWorktreeFinding,
  actChangedCode,
  buildCleanCheckPrompt,
  WORKTREE_CLEAN_SCHEMA,
  WORKTREE_PATH_CAP,
} = mod;

const CMD = 'sh scripts/verify-all.sh';
// Non-gating at tier `medium` (only `blocking` gates there), so the round is
// CLEAN and the terminal tail is reached.
const NON_GATING = [{ id: 'f1', severity: 'suggestion', confidence: 90, what_fails: 'x', unrefuted: true }];

function makeDeps(o) {
  const opts = o || {};
  const deps = {
    implement: async () => {},
    review: async () => ({ survivors: opts.survivors || NON_GATING, acTable: null }),
    verify: async () => ({ exitCode: 0, output: '' }),
  };
  if (opts.act !== undefined) deps.act = async () => opts.act;
  if (opts.clean !== undefined) {
    deps.clean = async () => {
      if (opts.clean === 'throw') throw new Error('probe blew up');
      return opts.clean;
    };
  }
  return deps;
}

// ===========================================================================
// (a) parseWorktreeStatus — fail-closed, and correct on git's own shapes.
// ===========================================================================
assert.deepEqual(parseWorktreeStatus({ porcelain: '' }), { ran: true, clean: true, paths: [], truncated: 0 }, 'an empty porcelain is clean');
assert.deepEqual(parseWorktreeStatus({ porcelain: '\n' }), { ran: true, clean: true, paths: [], truncated: 0 }, 'a trailing newline alone is clean — no phantom path');
for (const [name, raw] of [
  ['null', null],
  ['undefined', undefined],
  ['a non-object', 'clean'],
  ['a missing porcelain field', {}],
  ['a non-string porcelain field', { porcelain: 42 }],
  ['text that parses to no path', { porcelain: '   ' }],
]) {
  const r = parseWorktreeStatus(raw);
  assert.equal(r.ran, false, name + ' did not observe the worktree');
  assert.equal(r.clean, false, name + ' must be FAIL-CLOSED — an unobservable worktree is never clean');
}
{
  const r = parseWorktreeStatus({ porcelain: ' M a.rs\n?? b.rs\n' });
  assert.equal(r.clean, false, 'a modified + untracked pair is dirty');
  assert.deepEqual(r.paths, ['a.rs', 'b.rs'], 'the XY status prefix is stripped and both paths survive');
}
assert.deepEqual(
  parseWorktreeStatus({ porcelain: 'R  old.rs -> new.rs\n' }).paths,
  ['new.rs'],
  'a rename entry keeps the DESTINATION path'
);
assert.deepEqual(
  parseWorktreeStatus({ porcelain: '?? a file with spaces.rs\n' }).paths,
  ['a file with spaces.rs'],
  'a path containing spaces is never split on whitespace'
);
{
  const many = [];
  for (let i = 0; i < WORKTREE_PATH_CAP + 7; i++) many.push('?? f' + i + '.rs');
  const r = parseWorktreeStatus({ porcelain: many.join('\n') });
  assert.equal(r.paths.length, WORKTREE_PATH_CAP, 'a wholesale-dirty tree is capped at WORKTREE_PATH_CAP paths');
  assert.equal(r.truncated, 7, '... and the overflow is reported as a count');
  assert.ok(dirtyWorktreeFinding(r).what_fails.includes('and 7 more'), '... which the finding text names');
}

// ===========================================================================
// (b) actChangedCode — must the checks run again?
// ===========================================================================
assert.equal(actChangedCode(null, false), false, 'act never invoked: nothing changed');
assert.equal(actChangedCode({ handled: [{ id: 'a', action: 'fixed-inline' }] }, false), false, 'not invoked wins over the payload');
assert.equal(actChangedCode({ handled: [{ id: 'a', action: 'filed-as-task' }] }, true), false, 'filing a task changes no code');
assert.equal(actChangedCode({ handled: [{ id: 'a', action: 'skipped' }] }, true), false, 'a skip changes no code');
assert.equal(actChangedCode({ handled: [{ id: 'a', action: 'skipped' }, { id: 'b', action: 'fixed-inline' }] }, true), true, 'ONE inline fix is enough');
for (const [name, r] of [
  ['a null result', null],
  ['a non-object result', 'ok'],
  ['no handled array', { ok: true }],
  ['a non-array handled', { handled: 'x' }],
]) {
  assert.equal(actChangedCode(r, true), true, name + ' is FAIL-CLOSED — an unknown act outcome must re-verify');
}

// ===========================================================================
// (c) AC1 — a dirty worktree can never reach `reviewed`, and a clean one still
//     can (the probe is not a blanket rejecter). Both item shapes.
// ===========================================================================
const seen = new Set();
for (const [kind, build, ident] of [
  ['phase', buildOutcome, { roadmap: 'rm', phase: '1' }],
  ['task', buildTaskOutcome, { task: 'my-task' }],
]) {
  const dirty = await runCodeGate(
    { maxRework: 2, tier: 'medium', verifyCommand: CMD },
    makeDeps({ clean: { porcelain: ' M a.rs\n?? b.rs\n' } })
  );
  const out = build({ ...ident, codeReviews: dirty.rounds, acRounds: dirty.acRounds, maxRework: 2, tier: 'medium', actResult: dirty.actResult });
  seen.add(out.outcome);
  assert.equal(out.outcome, 'rework', kind + ': a dirty worktree yields rework, never reviewed');
  assert.equal(out.status, statusFor('rework', kind), kind + ': the status comes from the untouched statusFor table');
  assert.equal(out.writesCompletion, false, kind + ': ... and it writes no completion trailer');
  assert.ok(out.summary.includes('a.rs') && out.summary.includes('b.rs'), kind + ': the summary names the uncommitted paths');
  assert.ok(out.reason.includes('a.rs') && out.reason.includes('b.rs'), kind + ': the reason names them too, so the parked queue shows them');
  assert.equal(dirty.rounds.length, 1, kind + ': the dirty finding folds into the FINAL round — no phantom review round');

  const clean = await runCodeGate({ maxRework: 2, tier: 'medium', verifyCommand: CMD }, makeDeps({ clean: { porcelain: '' } }));
  const okOut = build({ ...ident, codeReviews: clean.rounds, acRounds: clean.acRounds, maxRework: 2, tier: 'medium', actResult: clean.actResult });
  seen.add(okOut.outcome);
  assert.equal(okOut.outcome, 'reviewed', kind + ': an empty porcelain still reviews clean — the probe discriminates');
  assert.equal(clean.cleanResult.clean, true, kind + ': ... and the probe result is reported out');
}
// Fail-closed rows through the FULL gate, not just the parser.
for (const [name, cleanValue] of [
  ['a thrown probe', 'throw'],
  ['a null resolution', null],
  ['a schema-violating payload', { porcelain: 42 }],
]) {
  const gate = await runCodeGate({ maxRework: 2, tier: 'medium', verifyCommand: CMD }, makeDeps({ clean: cleanValue }));
  const out = buildOutcome({ roadmap: 'rm', phase: '1', codeReviews: gate.rounds, acRounds: gate.acRounds, maxRework: 2, tier: 'medium' });
  seen.add(out.outcome);
  assert.equal(out.outcome, 'rework', name + ' is FAIL-CLOSED — an unobservable worktree never reports reviewed');
}
// An ABSENT dep is a SKIP, not a failure: every pre-existing fake-driven
// scenario omits it, and fail-closing on absence would flip them all to rework.
// Non-vacuity is bought by § 3-clean's static gate on the shipped driver.
{
  const gate = await runCodeGate({ maxRework: 2, tier: 'medium', verifyCommand: CMD }, makeDeps({}));
  assert.equal(gate.cleanResult, null, 'an absent clean dep records no probe result');
  const out = buildOutcome({ roadmap: 'rm', phase: '1', codeReviews: gate.rounds, acRounds: gate.acRounds, maxRework: 2, tier: 'medium' });
  assert.equal(out.outcome, 'reviewed', 'an absent clean dep is a SKIP, not a failure');
}
assert.deepEqual([...seen].sort(), ['reviewed', 'rework'], 'the cleanliness gate adds no new OUTCOME value');

// ===========================================================================
// (d) AC2 — the finding and the commit that closed it are BOTH recoverable.
// ===========================================================================
{
  const gate = await runCodeGate(
    { maxRework: 2, tier: 'medium', verifyCommand: CMD },
    makeDeps({ act: { handled: [{ id: 'f1', action: 'fixed-inline', commit: 'abc1234' }] }, clean: { porcelain: '' } })
  );
  const out = buildOutcome({
    roadmap: 'rm',
    phase: '1',
    codeReviews: gate.rounds,
    acRounds: gate.acRounds,
    maxRework: 2,
    tier: 'medium',
    actResult: gate.actResult,
  });
  assert.equal(out.outcome, 'reviewed', '(d) a committed inline fix leaves the tree clean and reviews clean');
  const f1 = out.findings.find((x) => x.id === 'f1');
  assert.equal(f1.handled, 'fixed-inline', '(d) the finding records HOW it was handled');
  assert.equal(f1.handledCommit, 'abc1234', '(d) ... and WHICH commit closed it');
}
{
  // The same annotation must survive onto the rework branch — a post-act dirty
  // tree is exactly when the provenance matters most.
  const gate = await runCodeGate(
    { maxRework: 2, tier: 'medium', verifyCommand: CMD },
    makeDeps({ act: { handled: [{ id: 'f1', action: 'fixed-inline', commit: 'abc1234' }] }, clean: { porcelain: '?? leftover.rs\n' } })
  );
  const out = buildOutcome({
    roadmap: 'rm',
    phase: '1',
    codeReviews: gate.rounds,
    acRounds: gate.acRounds,
    maxRework: 2,
    tier: 'medium',
    actResult: gate.actResult,
  });
  assert.equal(out.outcome, 'rework', '(d) a post-act dirty tree is rework');
  assert.equal(out.findings.find((x) => x.id === 'f1').handledCommit, 'abc1234', '(d) the commit provenance survives the rework branch');
  assert.ok(out.summary.includes('leftover.rs'), '(d) ... and the summary names what was left uncommitted');
}
// A fixer that reports no sha still records its disposition.
{
  const annotated = mod.annotateHandled([{ id: 'f1' }], { handled: [{ id: 'f1', action: 'fixed-inline' }] });
  assert.equal(annotated[0].handled, 'fixed-inline', 'a missing sha does not lose the disposition');
  assert.equal(annotated[0].handledCommit, '', '... and the absent sha reads as an empty string');
}

// ===========================================================================
// (e) The REAL seeded dirty tree: git's own porcelain output, verbatim.
// ===========================================================================
{
  const real = fs.readFileSync(realPorcelainPath, 'utf8');
  const parsed = parseWorktreeStatus({ porcelain: real });
  assert.equal(parsed.ran, true, '(e) git’s own porcelain output parses');
  assert.equal(parsed.clean, false, '(e) ... and a genuinely dirty tree reads dirty');
  // git quotes a path containing a space (`R  "old name.rs" -> "new name.rs"`),
  // so the destination arrives quoted. That is git's own representation and it
  // is kept VERBATIM — a substring match is the honest assertion here.
  for (const needle of ['tracked.rs', 'untracked.rs', 'new name.rs']) {
    assert.ok(
      parsed.paths.some((x) => x.includes(needle)),
      '(e) the real path ' + needle + ' survives parsing (paths: ' + JSON.stringify(parsed.paths) + ')'
    );
  }
  assert.ok(!parsed.paths.some((x) => x.includes('old name.rs')), '(e) a rename reports the DESTINATION, not the source');
  assert.ok(!parsed.paths.some((x) => x === ''), '(e) no phantom empty path from the trailing newline');
  const gate = await runCodeGate({ maxRework: 2, tier: 'medium', verifyCommand: CMD }, makeDeps({ clean: { porcelain: real } }));
  const out = buildOutcome({ roadmap: 'rm', phase: '1', codeReviews: gate.rounds, acRounds: gate.acRounds, maxRework: 2, tier: 'medium' });
  assert.equal(out.outcome, 'rework', '(e) a REAL dirty worktree drives the whole gate to rework');
  assert.ok(out.summary.includes('untracked.rs'), '(e) ... naming a real uncommitted path');
}

// ===========================================================================
// (f) The probe prompt observes and does not repair.
// ===========================================================================
{
  const prompt = buildCleanCheckPrompt('roadmap/rm', { rdmBin: '/fake/bin/rdm', project: 'demo' });
  assert.ok(prompt.includes('git status --porcelain'), 'the probe runs the porcelain status command');
  assert.ok(prompt.includes('/fake/bin/rdm worktree add roadmap/rm --project demo'), 'the probe enters the item worktree via the parameterized binary');
  for (const forbidden of ['git add', 'git commit', 'git stash', 'git reset', 'git clean']) {
    assert.ok(prompt.includes(forbidden), 'the probe explicitly forbids `' + forbidden + '` — a destructive shortcut would be worse than no probe');
  }
  assert.ok(prompt.includes('Edit no files'), 'the probe edits nothing');
  assert.deepEqual(WORKTREE_CLEAN_SCHEMA.required, ['porcelain'], 'the WORKTREE_CLEAN schema requires the porcelain field');
  assert.equal(WORKTREE_CLEAN_SCHEMA.properties.porcelain.type, 'string', '... as a string');
  assert.equal(WORKTREE_CLEAN_SCHEMA.additionalProperties, false, '... and admits nothing else');
}

console.log('all clean-worktree assertions passed');
NODE_CLEAN

if run_node "$TMP/clean-gate.mjs" "$LIB" "$TMP/real-porcelain.txt"; then
    pass "1e: clean-worktree gate — reviewed implies an empty status, fail-closed probe, real-git porcelain, commit provenance"
else
    fail "1e: clean-worktree assertions failed"
fi

# Planted-mutation self-test (i): neutralize the terminal fold. The probe still
# runs and still reports dirty, but nothing folds into the final round — the
# exact regression that would silently restore the false green.
sed 's/^      findings = terminal.concat(findings);$/      findings = findings; \/\/ MUTANT: fold neutralized/' "$LIB" >"$TMP/lib-no-fold.mjs"
if cmp -s "$LIB" "$TMP/lib-no-fold.mjs"; then
    fail "1e self-test: the planted mutation was a no-op — the terminal fold was not found"
fi
if run_node "$TMP/clean-gate.mjs" "$TMP/lib-no-fold.mjs" "$TMP/real-porcelain.txt" >/dev/null 2>&1; then
    fail "1e self-test: the assertions PASSED against a lib whose terminal fold does nothing — 1e is vacuous"
fi
pass "1e self-test: neutralizing the terminal fold turns 1e red"

# Planted-mutation self-test (ii): make parseWorktreeStatus fail OPEN. An
# unobservable worktree would then read as clean, which is the whole failure
# mode this gate exists to remove.
sed "s/^  const unknown = { ran: false, clean: false, paths: \[\], truncated: 0 };\$/  const unknown = { ran: false, clean: true, paths: [], truncated: 0 }; \/\/ MUTANT: fail-open/" "$LIB" >"$TMP/lib-fail-open.mjs"
if cmp -s "$LIB" "$TMP/lib-fail-open.mjs"; then
    fail "1e self-test: the planted mutation was a no-op — the fail-closed sentinel was not found"
fi
if run_node "$TMP/clean-gate.mjs" "$TMP/lib-fail-open.mjs" "$TMP/real-porcelain.txt" >/dev/null 2>&1; then
    fail "1e self-test: the assertions PASSED against a fail-OPEN probe parser — the fail-closed rows are vacuous"
fi
pass "1e self-test: making the probe parser fail OPEN turns 1e red"

# Planted-mutation self-test (iii): drop the commit-sha stamp from
# annotateHandled, so a finding's closing commit is no longer recoverable.
sed "s/^      if (typeof h.commit === 'string') commitById\[h.id\] = h.commit;\$/      \/\/ MUTANT: commit provenance dropped/" "$LIB" >"$TMP/lib-no-commit.mjs"
if cmp -s "$LIB" "$TMP/lib-no-commit.mjs"; then
    fail "1e self-test: the planted mutation was a no-op — the commit stamp was not found"
fi
if run_node "$TMP/clean-gate.mjs" "$TMP/lib-no-commit.mjs" "$TMP/real-porcelain.txt" >/dev/null 2>&1; then
    fail "1e self-test: the assertions PASSED against a lib that drops the commit provenance — AC2 is vacuous"
fi
pass "1e self-test: dropping the closing-commit stamp turns 1e red"

# Positive control: the real lib still passes after the three mutations, so the
# self-tests discriminate rather than reject everything.
run_node "$TMP/clean-gate.mjs" "$LIB" "$TMP/real-porcelain.txt" >/dev/null 2>&1 ||
    fail "1e self-test: the real lib fails after the self-tests — the detector rejects everything"
pass "1e self-test: the unmutated lib still passes (the self-tests discriminate)"

# --- 2. BLOCK DRIFT GATE -----------------------------------------------------
say "2. Block drift: the dispatch-outcome region is byte-identical (lib vs workflow)"

# Extract the region strictly BETWEEN the marker lines (exclusive), matched only
# on the "// >>> " comment token so an incidental in-block mention cannot truncate.
extract_block() {
    awk '
        index($0, ">>> dispatch-outcome:begin") { infence = 1; next }
        index($0, ">>> dispatch-outcome:end") { infence = 0 }
        infence { print }
    ' "$1"
}

blocks_equal() {
    extract_block "$1" >"$TMP/_a" 2>/dev/null
    extract_block "$2" >"$TMP/_b" 2>/dev/null
    [ -s "$TMP/_a" ] && diff -q "$TMP/_a" "$TMP/_b" >/dev/null 2>&1
}

extract_block "$LIB" >"$TMP/lib-block"
[ -s "$TMP/lib-block" ] || fail "no dispatch-outcome block found between markers in $LIB"
extract_block "$WF" >"$TMP/wf-block"
[ -s "$TMP/wf-block" ] || fail "no dispatch-outcome block found between markers in $WF"

if diff -u "$TMP/lib-block" "$TMP/wf-block" >/dev/null 2>&1; then
    pass "dispatch-outcome block matches byte-for-byte between lib and workflow"
else
    printf '\n' >&2
    diff -u "$TMP/lib-block" "$TMP/wf-block" >&2 || true
    fail "dispatch-outcome block DRIFTED — copy the lib block verbatim into $WF"
fi

# Self-test: prove the byte-equality gate is not a no-op. On scratch copies, a
# mutation inside the workflow's block MUST break equality; a restore MUST heal it.
say "2b. Block drift detector fires on planted drift (self-test)"
cp "$LIB" "$TMP/lib.scratch"
cp "$WF" "$TMP/wf.scratch"
blocks_equal "$TMP/lib.scratch" "$TMP/wf.scratch" || fail "scratch copies should match before mutation"
sed 's/phase fetch failed/planted drift/' "$TMP/wf.scratch" >"$TMP/wf.mut" && mv "$TMP/wf.mut" "$TMP/wf.scratch"
if blocks_equal "$TMP/lib.scratch" "$TMP/wf.scratch"; then
    fail "byte-equality gate did NOT detect a planted mutation inside the block"
fi
cp "$WF" "$TMP/wf.scratch"
blocks_equal "$TMP/lib.scratch" "$TMP/wf.scratch" || fail "restore did not heal the byte-equality gate"
pass "drift detector fails on a planted mutation and heals on restore"

# Non-vacuity: byte-equality between two EMPTY extractions would also "pass", so
# assert the region actually carries the budget machinery in BOTH files. A
# partial hand-mirror cannot slip through by extracting nothing.
for sym in runPlanGate runCodeGate parseBudget parseDispatchArgs codeReviewRounds DEFAULT_MAX_PLAN_REVISE DEFAULT_MAX_CODE_REWORK; do
    grep -q "$sym" "$TMP/lib-block" || fail "dispatch-outcome block in the LIB is missing $sym"
    grep -q "$sym" "$TMP/wf-block" || fail "dispatch-outcome block in the WORKFLOW is missing $sym (partial mirror?)"
done
pass "dispatch-outcome block carries the budget gates + validators in both files"

# --- 3. STATIC INVARIANTS ----------------------------------------------------
say "3. Static invariants on the workflow source (AC-1 / AC-2 / AC-3 / AC-5)"

# AC-1: no land-time completion (`Done:`) directive inside a STAMPED region.
#
# Scoped, not whole-file: the canonical review source now carries a skill-only
# `review-gate-spec` section documenting the trailer, and that section sits
# OUTSIDE the stamped block (so it is never copied here). A whole-file grep would
# therefore be both over-broad and, worse, would push authors to omit the policy
# from the shared source entirely. Extract exactly the two stamped regions and
# grep only those.
extract_stamped_regions() {
    awk '
        index($0, ">>> review-refute-fix:begin") { inr = 1; next }
        index($0, ">>> review-refute-fix:end")   { inr = 0; next }
        index($0, ">>> dispatch-outcome:begin")  { inr = 1; next }
        index($0, ">>> dispatch-outcome:end")    { inr = 0; next }
        inr { print }
    ' "$1"
}
extract_stamped_regions "$WF" >"$TMP/stamped-regions"
[ -s "$TMP/stamped-regions" ] ||
    fail "AC-1: extracted an EMPTY stamped region from $WF — the extractor or the markers are broken"
if grep -n 'Done:' "$TMP/stamped-regions" >&2; then
    fail "AC-1: the stamped regions of rdm-wf-dispatch-phase.js must not contain a 'Done:' line (land-time only)"
fi

# Self-test A: a directive planted INSIDE a stamped region must fire the detector.
{
    printf '// >>> review-refute-fix:begin <<<\n'
    printf '// Done: rm/phase-1-x\n'
    printf '// >>> review-refute-fix:end <<<\n'
} >"$TMP/planted-inside.js"
extract_stamped_regions "$TMP/planted-inside.js" >"$TMP/planted-inside-region"
grep -q 'Done:' "$TMP/planted-inside-region" ||
    fail "AC-1 detector broken — a directive planted INSIDE a stamped region was not detected"

# Self-test B: gate prose carrying the literal OUTSIDE every stamped region must
# NOT fire — that is exactly where the canonical source keeps the policy.
{
    printf '// >>> review-refute-fix:begin <<<\n'
    printf 'const clean = true\n'
    printf '// >>> review-refute-fix:end <<<\n'
    printf '// gate prose: amend Done: <roadmap>/<phase> at land time\n'
} >"$TMP/planted-outside.js"
extract_stamped_regions "$TMP/planted-outside.js" >"$TMP/planted-outside-region"
if grep -q 'Done:' "$TMP/planted-outside-region"; then
    fail "AC-1 detector is over-broad — prose outside every stamped region must not be flagged"
fi
pass "AC-1: no 'Done:' directive in either stamped region; detector fires inside and stays silent outside"

# AC-5: shared per-roadmap worktree entered via `rdm worktree add` in a prompt;
# `isolation:` (the isolation:'worktree' agent option) must NOT be used.
if grep -n 'isolation' "$WF" >/dev/null 2>&1; then
    grep -n 'isolation' "$WF" >&2 || true
    fail "AC-5: rdm-wf-dispatch-phase.js must not use isolation:'worktree' — enter the shared worktree via Bash"
fi
grep -q 'worktree add' "$WF" || fail "AC-5: expected a './target/debug/rdm worktree add' instruction in an agent prompt"
printf "  isolation: 'worktree',\n" >"$TMP/planted-iso.js"
grep -q 'isolation' "$TMP/planted-iso.js" || fail "AC-5 detector broken — grep 'isolation' missed a planted flag"
pass "AC-5: shared 'worktree add' prompt present; no isolation flag; detector catches a planted one"

# AC-3: both stamped review markers present; no import/require/nested workflow().
grep -q '>>> review-refute-fix:begin' "$WF" || fail "AC-3: missing review-refute-fix:begin marker"
grep -q '>>> review-refute-fix:end' "$WF" || fail "AC-3: missing review-refute-fix:end marker"
grep -q '>>> dispatch-outcome:begin' "$WF" || fail "AC-3: missing dispatch-outcome:begin marker"
grep -q '>>> dispatch-outcome:end' "$WF" || fail "AC-3: missing dispatch-outcome:end marker"
if grep -nE '(^|[^A-Za-z_])import[ (]' "$WF" >/dev/null 2>&1; then
    grep -nE '(^|[^A-Za-z_])import[ (]' "$WF" >&2 || true
    fail "AC-3: rdm-wf-dispatch-phase.js must not import (the runtime forbids it — sharing is by stamped copy)"
fi
if grep -nE '(^|[^A-Za-z_])require\(' "$WF" >/dev/null 2>&1; then
    fail "AC-3: rdm-wf-dispatch-phase.js must not require() (the runtime forbids it)"
fi
if grep -n 'workflow(' "$WF" >/dev/null 2>&1; then
    grep -n 'workflow(' "$WF" >&2 || true
    fail "AC-3: rdm-wf-dispatch-phase.js must not nest a workflow() call — both review gates are inline copies"
fi
printf "import x from 'y'\n" >"$TMP/planted-import.js"
grep -qE '(^|[^A-Za-z_])import[ (]' "$TMP/planted-import.js" || fail "AC-3 import detector broken"
printf "const x = require('y')\n" >"$TMP/planted-req.js"
grep -qE '(^|[^A-Za-z_])require\(' "$TMP/planted-req.js" || fail "AC-3 require detector broken"
printf "const r = workflow('sub')\n" >"$TMP/planted-wf.js"
grep -q 'workflow(' "$TMP/planted-wf.js" || fail "AC-3 workflow() detector broken"
pass "AC-3: both review markers present; no import/require/nested workflow(); detectors catch planted ones"

# AC-2: planner, plan-reviewer, and implementer are SEPARATE agent() calls —
# distinct labels, with plan and implement labels that differ.
NLABELS=$(grep -oE "label: '[^']+'" "$WF" | sort -u | wc -l | tr -d ' ')
[ "$NLABELS" -ge 3 ] || fail "AC-2: expected >=3 distinct agent labels, found $NLABELS"
grep -q "label: 'fetch:" "$WF" || fail "AC-2: missing a fetch agent label"
grep -q "label: 'plan:" "$WF" || fail "AC-2: missing a planner agent label"
grep -q "label: 'implement:" "$WF" || fail "AC-2: missing an implementer agent label"
# plan vs implement labels must be distinct label namespaces.
if grep -oE "label: '[^']+'" "$WF" | grep -q "label: 'plan:.*implement"; then
    fail "AC-2: planner and implementer must use separate labels"
fi
pass "AC-2: $NLABELS distinct agent labels; separate fetch/plan/implement calls"

# AC-CANONICAL-REVIEW: the code-review stage IS the canonical review, fed by the
# canonical `deriveSignals`. Two halves:
#   (a) exactly ONE code-review construction site — `buildReviewPipeline('code')`
#       — so there is no second, independent code-review prompt builder;
#   (b) the driver actually THREADS diff signals into it, via a mechanical diff
#       agent, so the canonical `when` triggers (tests/architecture/api-docs/
#       changelog/security) and deriveSignals are live rather than dead code.
# Count BINDING sites (`… = buildReviewPipeline('code')`), not prose mentions —
# the driver comments name the constructor when explaining the wiring.
CODE_PIPELINES=$(grep -cE "= *buildReviewPipeline\('code'\)" "$WF" | tr -d ' ')
[ "$CODE_PIPELINES" -eq 1 ] ||
    fail "AC-CANONICAL-REVIEW: expected exactly ONE buildReviewPipeline('code') construction, found $CODE_PIPELINES"
# No second code-review prompt builder may exist alongside the canonical one:
# `findPrompt`/`refutePrompt` are declared once each (inside the stamped block).
for sym in 'function findPrompt(' 'function refutePrompt('; do
    N=$(grep -cF "$sym" "$WF" | tr -d ' ')
    [ "$N" -eq 1 ] ||
        fail "AC-CANONICAL-REVIEW: '$sym' must be declared exactly once (found $N) — a second review prompt builder is an independent code-review path"
done
grep -qF 'deriveSignals(' "$WF" ||
    fail "AC-CANONICAL-REVIEW: the driver must call deriveSignals( to select code-review dimensions from the real diff"
grep -qF 'signals: signals' "$WF" ||
    fail "AC-CANONICAL-REVIEW: the driver must pass 'signals:' into the code-review call"
grep -qF "label: 'diff:signals'" "$WF" ||
    fail "AC-CANONICAL-REVIEW: a mechanical 'diff:signals' agent must supply the diff (AC-MODEL covers its explicit model:)"
grep -qF 'git diff --name-only main...HEAD' "$WF" ||
    fail "AC-CANONICAL-REVIEW: the diff agent must use the three-dot branch diff (main...HEAD)"
# Fail-open: the degraded path must call the review with NO signals key at all.
# Passing `{}` would read every `when` predicate as falsy and silently drop
# tests/api-docs/changelog/security coverage exactly when the driver knew least.
if grep -qF 'signals: {}' "$WF"; then
    fail "AC-CANONICAL-REVIEW: the fail-open path must OMIT signals entirely, never pass an empty {}"
fi
grep -qF 'fail-open' "$WF" ||
    fail "AC-CANONICAL-REVIEW: the driver must document the signals fail-open contract"
# Planted-string self-tests: prove each new detector actually fires.
sed "s/= buildReviewPipeline('code')/= buildReviewPipelineX('code')/" "$WF" >"$TMP/planted-nocode.js"
if [ "$(grep -cE "= *buildReviewPipeline\('code'\)" "$TMP/planted-nocode.js" | tr -d ' ')" -ne 0 ]; then
    fail "AC-CANONICAL-REVIEW detector broken — removing the code pipeline was not detected"
fi
sed 's/deriveSignals(/deriveSignalsX(/g' "$WF" >"$TMP/planted-nosig.js"
if grep -qF 'deriveSignals(' "$TMP/planted-nosig.js"; then
    fail "AC-CANONICAL-REVIEW detector broken — a removed deriveSignals( call was not detected"
fi
cp "$WF" "$TMP/planted-emptysig.js"
printf '\nconst bad = runCodeReview({ signals: {} })\n' >>"$TMP/planted-emptysig.js"
grep -qF 'signals: {}' "$TMP/planted-emptysig.js" ||
    fail "AC-CANONICAL-REVIEW detector broken — a planted empty-signals call was not detected"
pass "AC-CANONICAL-REVIEW: one canonical code pipeline, signals threaded from a real diff, fail-open omits signals; detectors fire on planted mutations"

# AC-CANONICAL-PLAN-REVIEW: the plan gate uses the SAME canonical, generated
# review pipeline as the code gate above — no hand-maintained plan-review
# logic (unify-plan-review roadmap, phase 3). Four invariants:
#   (a) exactly ONE plan-review construction site — `buildReviewPipeline('plan')`
#       — so there is no second, independent plan-review prompt builder;
#   (b) that binding (`runPlanReview`) is the SOLE `review:` callback passed
#       into runPlanGate;
#   (c) the plan gate's documented fail-open/no-signals contract — the
#       "SIGNALS SITE (plan gate)" comment — is still present, so a future edit
#       can't silently start threading signals/targetType into the plan gate in
#       violation of the deferral the comment itself records; and
#   (d) the plan-review call itself carries no `signals` key (inverse of the
#       code gate's AC-CANONICAL-REVIEW check above — the plan gate must NOT be
#       signals-fed).
# Count BINDING sites (`… = buildReviewPipeline('plan')`), not prose mentions.
PLAN_PIPELINES=$(grep -cE "= *buildReviewPipeline\('plan'\)" "$WF" | tr -d ' ')
[ "$PLAN_PIPELINES" -eq 1 ] ||
    fail "AC-CANONICAL-PLAN-REVIEW: expected exactly ONE buildReviewPipeline('plan') construction, found $PLAN_PIPELINES"
grep -qF 'review: async (doc) => runPlanReview(' "$WF" ||
    fail "AC-CANONICAL-PLAN-REVIEW: the plan gate's 'review:' dependency must call the canonical runPlanReview binding directly"
grep -qF 'SIGNALS SITE (plan gate)' "$WF" ||
    fail "AC-CANONICAL-PLAN-REVIEW: missing the plan gate's fail-open/no-signals contract comment"
# Fail-open: unlike the code gate, the plan gate must never pass a 'signals'
# key at all — deriveSignals-driven dimension selection is deferred to a later
# roadmap phase per the comment above. Scan only the line(s) that actually
# invoke runPlanReview(, not the whole file, so unrelated 'signals' mentions
# elsewhere (e.g. the code gate's own signals wiring) don't false-positive.
if grep -E 'runPlanReview\(' "$WF" | grep -q 'signals'; then
    fail "AC-CANONICAL-PLAN-REVIEW: the plan gate must not pass 'signals' into runPlanReview — that threading is deferred (see the SIGNALS SITE comment)"
fi
# Planted-mutation self-tests: prove each detector actually fires.
sed "s/= buildReviewPipeline('plan')/= buildReviewPipelineX('plan')/" "$WF" >"$TMP/planted-noplan.js"
if [ "$(grep -cE "= *buildReviewPipeline\('plan'\)" "$TMP/planted-noplan.js" | tr -d ' ')" -ne 0 ]; then
    fail "AC-CANONICAL-PLAN-REVIEW detector broken — renaming the plan pipeline binding was not detected"
fi
sed '/SIGNALS SITE (plan gate)/,/Do not add it here\./d' "$WF" >"$TMP/planted-nocomment.js"
if grep -qF 'SIGNALS SITE (plan gate)' "$TMP/planted-nocomment.js"; then
    fail "AC-CANONICAL-PLAN-REVIEW detector broken — stripping the fail-open comment was not detected"
fi
cp "$WF" "$TMP/planted-dupplan.js"
printf "\nconst runPlanReview2 = buildReviewPipeline('plan')\n" >>"$TMP/planted-dupplan.js"
if [ "$(grep -cE "= *buildReviewPipeline\('plan'\)" "$TMP/planted-dupplan.js" | tr -d ' ')" -eq 1 ]; then
    fail "AC-CANONICAL-PLAN-REVIEW detector broken — a planted duplicate plan pipeline was not detected"
fi
cp "$WF" "$TMP/planted-plansig.js"
printf "\nconst bad = runPlanReview({ target: 't', signals: {} })\n" >>"$TMP/planted-plansig.js"
if ! grep -E 'runPlanReview\(' "$TMP/planted-plansig.js" | grep -q 'signals'; then
    fail "AC-CANONICAL-PLAN-REVIEW detector broken — a planted signals-bearing runPlanReview call was not detected"
fi
pass "AC-CANONICAL-PLAN-REVIEW: one canonical plan pipeline, bound as runPlanGate's sole review callback, fail-open comment present and signals omitted; detectors fire on planted mutations"

# AC-PLAN-INTENT: the plan gate threads the parent roadmap's recorded `## Intent`
# as a VALUE (`intent:`) and still passes NO `signals` key. Both halves matter:
#   * dropping `intent:` silently disables the intent-alignment dimension for
#     every dispatched phase — the gate would still be green, just blind;
#   * adding `signals` would violate the SIGNALS SITE deferral above AND drop
#     `unit-of-work` for phases (currently selected via the null fail-open), a
#     silent coverage regression on top of a red harness.
# The no-signals half is already asserted above; this adds the positive half and
# the extraction it depends on.
grep -qF 'review: async (doc) => runPlanReview({ target: renderPlanDoc(doc), intent: planIntent.intent,' "$WF" ||
    fail "AC-PLAN-INTENT: the plan gate's review callback must thread intent: planIntent.intent (the recorded ## Intent, as a VALUE)"
grep -qF 'const planIntent = extractIntent(phaseMeta.roadmapBody)' "$WF" ||
    fail "AC-PLAN-INTENT: the plan gate must derive its intent from the Stage-0 fetch's roadmapBody via extractIntent"
# roadmapBody rides on the EXISTING Stage-0 agent — it must be OPTIONAL in the
# schema (never in `required`), so a caller-supplied metadata hoist that predates
# this field keeps working and simply degrades to no intent.
grep -qE "^ *roadmapBody: \{ type: 'string' \}," "$WF" ||
    fail "AC-PLAN-INTENT: PHASE_META_SCHEMA must declare an optional roadmapBody property"
if grep -qE "required: \[[^]]*roadmapBody" "$WF"; then
    fail "AC-PLAN-INTENT: roadmapBody must NOT be in PHASE_META_SCHEMA's required list — an older caller hoist must still work"
fi
pass "AC-PLAN-INTENT: the plan gate threads intent as a value, derived from an OPTIONAL Stage-0 roadmapBody, and passes no signals"

# Planted-mutation self-tests: prove both new detectors fire.
sed 's/intent: planIntent\.intent, //' "$WF" >"$TMP/planted-nointent.js"
if grep -qF 'review: async (doc) => runPlanReview({ target: renderPlanDoc(doc), intent: planIntent.intent,' "$TMP/planted-nointent.js"; then
    fail "AC-PLAN-INTENT detector broken — stripping the intent thread was not detected"
fi
sed "s/^\( *\)roadmapBody: { type: 'string' },/\1roadmapBodyX: { type: 'string' },/" "$WF" >"$TMP/planted-noschema.js"
if grep -qE "^ *roadmapBody: \{ type: 'string' \}," "$TMP/planted-noschema.js"; then
    fail "AC-PLAN-INTENT detector broken — renaming the roadmapBody schema property was not detected"
fi
sed "s/required: \['roadmap', 'phase', 'stem', 'model', 'body', 'models'\]/required: ['roadmap', 'phase', 'stem', 'model', 'body', 'models', 'roadmapBody']/" "$WF" >"$TMP/planted-reqbody.js"
if ! grep -qE "required: \[[^]]*roadmapBody" "$TMP/planted-reqbody.js"; then
    fail "AC-PLAN-INTENT detector broken — a planted required roadmapBody was not detected"
fi
pass "AC-PLAN-INTENT: detectors fire on a stripped intent thread, a renamed schema property, and a required roadmapBody"

# Driver arg hardening: dispatch-phase is invoked DIRECTLY via the Workflow tool
# (rdm-do --auto, hand-run single phases), so an LLM-authored stringified `args`
# payload must be coerced before roadmap/phase/planOnly are derived from it.
grep -q "typeof dispatchArgs === 'string'" "$WF" ||
    fail "driver must coerce a stringified args payload before deriving roadmap/phase/planOnly"
grep -q 'JSON.parse(dispatchArgs)' "$WF" ||
    fail "driver must JSON.parse a stringified args payload"
if grep -n 'args && args\.roadmap' "$WF" >/dev/null 2>&1; then
    grep -n 'args && args\.roadmap' "$WF" >&2 || true
    fail "driver still derives roadmap from the un-coerced args — derive from dispatchArgs instead"
fi
pass "driver coerces a stringified args payload; un-coerced form is gone"

# AC-2 (seeding nuance / plan-review strengthening): the implementer prompt is
# built from the phase body + approved plan doc ONLY — it must never interpolate
# the plan-review findings/transcript (`planFindings`). Code-review findings on
# the rework pass are a different variable and ARE allowed.
awk '/^function buildImplementPrompt\(/{p=1} p{print} p&&/^}/{exit}' "$WF" >"$TMP/impl-fn"
[ -s "$TMP/impl-fn" ] || fail "could not extract buildImplementPrompt from $WF"
grep -q 'phaseBody' "$TMP/impl-fn" || fail "AC-2: implementer prompt must be seeded from phaseBody"
grep -q 'planDocText' "$TMP/impl-fn" || fail "AC-2: implementer prompt must be seeded from the approved plan doc"
if grep -q 'planFindings' "$TMP/impl-fn"; then
    fail "AC-2: implementer prompt must NOT interpolate plan-review findings (planFindings)"
fi
pass "AC-2: implementer seeded from phase body + plan doc only, not the plan-review transcript"

# AC-MODEL: every dispatch-path agent() call carries an explicit `model:`.
#
# The extractor deliberately skips COMMENT lines: `rdm-wf-dispatch-phase.js` contains
# prose mentioning `agent()` (e.g. the meta.phases note "no agent() call uses
# it"), and a naive matcher would emit those as option blocks and fail on a
# correct implementation. A whole-file `grep model:` is also NOT acceptable here
# — it is satisfied by any unrelated occurrence and cannot tell a wired call from
# an unwired one.
#
# WHITELIST: the Stage-0 bootstrap fetch (label fetch:phase-meta / fetch:task-meta)
# is exempt by design — it is the call that PRODUCES the model map, so it cannot
# know its own model before running.
agent_option_blocks() {
    awk '
      # skip single-line comments so prose mentioning agent() never opens a block
      /^[[:space:]]*\/\// { next }
      # Track "saw agent(, still hunting for the opening {" ACROSS lines: a call
      # whose options object opens on a later line must not be invisible to the
      # sweep (a silently-skipped block is a false green, not a pass).
      !inblk && index($0, "agent(") { pending = 1 }
      pending && index($0, "{") { inblk = 1; pending = 0; buf = $0; next }
      inblk { buf = buf "\n" $0 }
      inblk && /^[[:space:]]*\}\)/ { print buf "\n---END---"; inblk = 0; buf = "" }
    ' "$1"
}

# Independent count of real agent()/_agent() call sites, excluding comments.
agent_call_count() {
    grep -vE '^[[:space:]]*//' "$1" | grep -cE '(^|[^A-Za-z_])_?agent\('
}

agent_option_blocks "$WF" >"$TMP/agent-blocks"
[ -s "$TMP/agent-blocks" ] || fail "AC-MODEL: could not extract any agent() option blocks"

# Cross-check: every real call site must have produced a block. Without this, an
# extractor that silently drops a block reports bad=0 and passes — the exact
# "captures 7 of 8 and passes" false green.
EXTRACTED=$(grep -c -- '---END---' "$TMP/agent-blocks")
CALLSITES=$(agent_call_count "$WF")
[ "$EXTRACTED" -eq "$CALLSITES" ] ||
    fail "AC-MODEL: extracted $EXTRACTED option blocks but found $CALLSITES agent() call sites — the sweep is blind to at least one"
pass "AC-MODEL: extracted one option block per agent() call site ($EXTRACTED)"

# Every block must carry `model:` unless it is a whitelisted bootstrap fetch.
assert_model_sweep() {
    awk '
      BEGIN { RS = "---END---"; bad = 0 }
      /label:/ {
        if ($0 ~ /label: .fetch:(phase|task)-meta./) next   # whitelisted bootstrap
        if ($0 !~ /model:/) { bad++ }
      }
      END { exit (bad > 0) }
    ' "$1"
}
assert_model_sweep "$TMP/agent-blocks" ||
    fail "AC-MODEL: a non-bootstrap agent() call is missing an explicit model:"
pass "AC-MODEL: every dispatch-path agent() call carries an explicit model: (bootstrap fetch whitelisted)"

# Self-test: strip one model: key and prove the sweep fails.
sed '/model: models\.plan,/d' "$TMP/agent-blocks" >"$TMP/agent-blocks-mutant"
if assert_model_sweep "$TMP/agent-blocks-mutant"; then
    fail "AC-MODEL: sweep missed a removed model: key"
fi
pass "AC-MODEL: sweep detector fires when a model: key is removed"

# Self-test: the extractor must NOT emit the prose comment on line 28 as a block.
if grep -q "no agent() call uses it" "$TMP/agent-blocks"; then
    fail "AC-MODEL: extractor wrongly captured a prose comment mentioning agent()"
fi
pass "AC-MODEL: extractor skips prose comments mentioning agent()"

# AC-MODEL MINIMALITY (regularize-mechanical-agents): the bootstrap-fetch
# whitelist above (`label: .fetch:(phase|task)-meta.`) is meant to exempt
# EXACTLY the two Stage-0 metadata fetches from the explicit-`model:`
# requirement — no more. `rdm-wf-dispatch-phase.js`'s own Stage-0 mechanism is
# unchanged by the `regularize-mechanical-agents` phase (the CLI-lane fix
# lives one level up, in the `rdm-autopilot` skill, which now hoists a
# complete `phaseMeta` so the Stage-0 agent never runs at all on that path) —
# so the whitelist that remains here must still cover exactly these two
# labels and nothing else. A whitelist that silently widened to cover a third
# label would let AC-MODEL pass vacuously for whatever new unsized call rode
# in on the broadened pattern.
count_whitelisted_labels() {
    pattern="$1"
    grep -oE "label: '[a-zA-Z0-9:_-]+'" "$TMP/agent-blocks" | sort -u | grep -cE "$pattern"
}
WL_COUNT=$(count_whitelisted_labels "label: .fetch:(phase|task)-meta.")
[ "$WL_COUNT" -eq 2 ] ||
    fail "AC-MODEL minimality: the bootstrap-fetch whitelist must cover exactly 2 distinct labels (fetch:phase-meta, fetch:task-meta), matched $WL_COUNT"
pass "AC-MODEL minimality: bootstrap-fetch whitelist covers exactly fetch:phase-meta/fetch:task-meta ($WL_COUNT labels)"

# Self-test: broaden the whitelist pattern as if a bogus third label had been
# added to it, and confirm the minimality check turns red — the broadened
# pattern now over-matches a real, non-bootstrap label already present in the
# file (`stamp:in-progress`), proving the count assertion above is load-bearing.
WL_COUNT_MUTANT=$(count_whitelisted_labels "label: .(fetch:(phase|task)-meta|stamp:in-progress).")
[ "$WL_COUNT_MUTANT" -eq 2 ] &&
    fail "AC-MODEL minimality self-test: broadening the whitelist pattern with a bogus third label did not change the matched count — the self-test is vacuous"
pass "AC-MODEL minimality self-test: a broadened (bogus-third-label) whitelist pattern is correctly detected (would fail the real check)"

# AC-MECHANICAL-TIER: the two mechanical Bash agents dispatch-phase runs after
# Stage 0 (stamp:in-progress, diff:signals) must resolve to `models.mechanical`
# — the small/mechanical tier Stage 0 resolves via `rdm model resolve
# mechanical` — rather than borrowing `models.review_find` (the reviewer
# tier). Reuses the already-extracted "$TMP/agent-blocks" from AC-MODEL above
# rather than re-extracting.
# shellcheck disable=SC1091
. "$REPO_ROOT/scripts/lib/mechanical-tier-check.sh"

assert_label_model "$TMP/agent-blocks" 'stamp:in-progress' 'models.mechanical' ||
    fail "AC-MECHANICAL-TIER: stamp:in-progress must resolve to models.mechanical"
assert_label_model "$TMP/agent-blocks" 'diff:signals' 'models.mechanical' ||
    fail "AC-MECHANICAL-TIER: diff:signals must resolve to models.mechanical"
pass "AC-MECHANICAL-TIER: stamp:in-progress and diff:signals resolve to models.mechanical"

# Self-test: plant a repoint from models.mechanical back to models.review_find
# on both labels and prove the check now fails; the unmodified extraction
# above already proved it passes.
sed 's/model: models\.mechanical,/model: models.review_find,/' "$TMP/agent-blocks" >"$TMP/mech-mutant"
if assert_label_model "$TMP/mech-mutant" 'stamp:in-progress' 'models.mechanical'; then
    fail "AC-MECHANICAL-TIER: detector missed a stamp:in-progress repoint to models.review_find"
fi
if assert_label_model "$TMP/mech-mutant" 'diff:signals' 'models.mechanical'; then
    fail "AC-MECHANICAL-TIER: detector missed a diff:signals repoint to models.review_find"
fi
pass "AC-MECHANICAL-TIER: detector fires when either label is repointed to models.review_find"

# AC-NULLGUARD: both null guards exist and gate a real code path.
#
# The unresolved-model guard lives in the un-exported top-level driver body
# (ambient agent/top-level await), so no Node unit test can reach it — a static
# structural gate is the only thing standing between it and silent deletion. The
# null-plan/null-revise guard now lives inside runPlanGate in the copied block
# (section 1c drives it behaviorally); this static check keeps a second, cheaper
# lock on it. They are the safety net
# for spike consequence 3: agent() RESOLVES to null on an unknown model id, so
# without them a misconfigured [models] binding yields a null plan / an
# all-inherited-model dispatch with nothing objecting.
assert_driver_null_guards() {
    # (a) an incomplete resolved-model map must short-circuit to fetchError
    grep -q 'if (unresolvedStep) {' "$1" || return 1
    awk '/if \(unresolvedStep\) \{/{p=1} p{print} p&&/^\}/{exit}' "$1" |
        grep -q 'fetchError: true' || return 1
    # (b) a null plan agent result must short-circuit to fetchError
    grep -q 'if (planDoc === null || planDoc === undefined) {' "$1" || return 1
    awk '/if \(planDoc === null/{p=1} p{print} p&&/^\}/{exit}' "$1" |
        grep -q 'fetchError: true' || return 1
    return 0
}
assert_driver_null_guards "$WF" ||
    fail "AC-NULLGUARD: the driver must guard an incomplete model map and a null plan agent result"
pass "AC-NULLGUARD: driver guards both an unresolved model map and a null plan result"

# Self-tests: removing either guard must fire the detector.
sed '/if (unresolvedStep) {/,/^}/d' "$WF" >"$TMP/ng-mutant-a"
if assert_driver_null_guards "$TMP/ng-mutant-a"; then
    fail "AC-NULLGUARD: detector missed a removed unresolved-model guard"
fi
sed '/if (planDoc === null || planDoc === undefined) {/,/^}/d' "$WF" >"$TMP/ng-mutant-b"
if assert_driver_null_guards "$TMP/ng-mutant-b"; then
    fail "AC-NULLGUARD: detector missed a removed null-plan guard"
fi
pass "AC-NULLGUARD: detector fires when either driver guard is removed"

# AC-TIER: the tier hint reaches plan/implement only. review-find/review-verify
# must resolve with NO --tier: the caller hint has top precedence in core's
# resolve_tier and ReviewVerify defaults to Large, so any hint can only DOWNGRADE
# the verifier (measured: `resolve review-verify` -> opus, `--tier medium` -> sonnet).
# Scope the check PER PROMPT FUNCTION. A whole-file grep cannot tell which prompt
# a `--tier` belongs to, so `--tier` wrongly added to the TASK prompt would hide
# behind the phase prompt's legitimate occurrences.
extract_fn_body() {
    awk -v fn="$2" 'index($0, "function " fn "(") { p = 1 } p { print } p && /^\}/ { exit }' "$1"
}

assert_tier_scoping() {
    extract_fn_body "$1" buildFetchPrompt >"$TMP/fn-phase"
    extract_fn_body "$1" buildTaskFetchPrompt >"$TMP/fn-task"
    [ -s "$TMP/fn-phase" ] && [ -s "$TMP/fn-task" ] || return 1
    # Phase prompt: --tier on plan/implement only.
    grep -q 'model resolve plan --tier' "$TMP/fn-phase" || return 1
    grep -q 'model resolve implement --tier' "$TMP/fn-phase" || return 1
    grep -q 'review-find --tier' "$TMP/fn-phase" && return 1
    grep -q 'review-verify --tier' "$TMP/fn-phase" && return 1
    # Task prompt: a task carries no tier, so no resolver COMMAND may carry
    # --tier. Match the command shape, not a bare '--tier' — both prompts contain
    # the prose "with NO --tier argument", which a bare grep would false-positive.
    grep -qE 'model resolve [a-z-]+ --tier' "$TMP/fn-task" && return 1
    return 0
}
assert_tier_scoping "$WF" ||
    fail "AC-TIER: --tier must appear only on plan/implement in the phase prompt, and nowhere in the task prompt"
pass "AC-TIER: tier hint scoped per prompt (phase: plan/implement only; task: none)"

sed 's|model resolve review-verify|model resolve review-verify --tier T|' "$WF" >"$TMP/tier-mutant"
if assert_tier_scoping "$TMP/tier-mutant"; then
    fail "AC-TIER: detector missed a --tier added to a review resolve"
fi
pass "AC-TIER: detector fires when --tier is added to a review resolve"

# Self-test: a --tier smuggled into the TASK prompt must also fire. A whole-file
# grep would miss this because the phase prompt legitimately contains the same
# substrings.
awk '
  index($0, "function buildTaskFetchPrompt(") { p = 1 }
  p && index($0, "model resolve plan") { sub(/model resolve plan/, "model resolve plan --tier medium") }
  p && /^\}/ { p = 0 }
  { print }
' "$WF" >"$TMP/tier-task-mutant"
if assert_tier_scoping "$TMP/tier-task-mutant"; then
    fail "AC-TIER: detector missed a --tier smuggled into the task prompt"
fi
pass "AC-TIER: detector fires when --tier is added to the task prompt"

# AC-PLAN-INTENT-PROMPT: the Stage-0 phase prompt is the ONLY in-workflow code
# path that reads the parent roadmap's body, and `roadmapBody` is the ONLY name
# `extractIntent(phaseMeta.roadmapBody)` reads it back under. AC-PLAN-INTENT
# above is purely driver-side (the schema property and the `intent:` thread), so
# a mistyped command or a renamed field IN THE PROMPT would leave every check
# green while silently disabling `intent-alignment` for every dispatched phase.
# Scope this PER PROMPT FUNCTION (the AC-TIER convention): a whole-file grep
# cannot tell the phase prompt from the task prompt, and the task prompt must
# NOT gain a roadmap read — a task has no parent roadmap.
assert_fetch_intent_prompt() {
    extract_fn_body "$1" buildFetchPrompt >"$TMP/fn-intent-phase"
    extract_fn_body "$1" buildTaskFetchPrompt >"$TMP/fn-intent-task"
    [ -s "$TMP/fn-intent-phase" ] && [ -s "$TMP/fn-intent-task" ] || return 1
    # The phase prompt reads the roadmap and names the field the driver reads back.
    grep -qF "' roadmap show ' + roadmap + proj + ' --format json'" "$TMP/fn-intent-phase" || return 1
    # shellcheck disable=SC2016  # literal prompt prose, deliberately unexpanded
    grep -qF 'as `roadmapBody`' "$TMP/fn-intent-phase" || return 1
    # VERBATIM, not summarized — the finder is shown the recorded intent as written.
    grep -qF 'VERBATIM' "$TMP/fn-intent-phase" || return 1
    # Optional, never invented: a failed roadmap read omits the field.
    # shellcheck disable=SC2016  # literal prompt prose, deliberately unexpanded
    grep -qF 'omit `roadmapBody`' "$TMP/fn-intent-phase" || return 1
    # The task prompt has no parent roadmap and must not read one.
    grep -q 'roadmap show' "$TMP/fn-intent-task" && return 1
    grep -q 'roadmapBody' "$TMP/fn-intent-task" && return 1
    return 0
}
assert_fetch_intent_prompt "$WF" ||
    fail "AC-PLAN-INTENT-PROMPT: buildFetchPrompt must read 'roadmap show ... --format json' and return its body VERBATIM as an omittable 'roadmapBody'; buildTaskFetchPrompt must do neither"
pass "AC-PLAN-INTENT-PROMPT: the Stage-0 phase prompt reads the roadmap body verbatim as an optional roadmapBody; the task prompt does not"

# Self-tests: the three ways this prompt could silently rot.
sed "s|' roadmap show ' + roadmap + proj + ' --format json'|' roadmap shwo ' + roadmap + proj + ' --format json'|" "$WF" >"$TMP/intent-prompt-cmd-mutant"
if assert_fetch_intent_prompt "$TMP/intent-prompt-cmd-mutant"; then
    fail "AC-PLAN-INTENT-PROMPT detector missed a mistyped roadmap show command"
fi
pass "AC-PLAN-INTENT-PROMPT detector fires on a mistyped roadmap show command"

# shellcheck disable=SC2016  # literal prompt prose, deliberately unexpanded
sed 's|as `roadmapBody`|as `roadmapText`|' "$WF" >"$TMP/intent-prompt-field-mutant"
if assert_fetch_intent_prompt "$TMP/intent-prompt-field-mutant"; then
    fail "AC-PLAN-INTENT-PROMPT detector missed a renamed roadmapBody field"
fi
pass "AC-PLAN-INTENT-PROMPT detector fires on a renamed roadmapBody field"

awk '
  index($0, "function buildTaskFetchPrompt(") { p = 1 }
  p && index($0, "TASK_META object") { print "    \x27  rdm roadmap show x --format json -> roadmapBody\x27,"; }
  p && /^\}/ { p = 0 }
  { print }
' "$WF" >"$TMP/intent-prompt-task-mutant"
if assert_fetch_intent_prompt "$TMP/intent-prompt-task-mutant"; then
    fail "AC-PLAN-INTENT-PROMPT detector missed a roadmap read smuggled into the task prompt"
fi
pass "AC-PLAN-INTENT-PROMPT detector fires on a roadmap read smuggled into the task prompt"

# AC-PLAN-INTENT-HOIST: `roadmapBody` is OPTIONAL in PHASE_META_SCHEMA and absent
# from hoistedMetaComplete's key list, so a hoisted phaseMeta is accepted without
# it — and a hoisted dispatch skips Stage 0, the only in-workflow reader of the
# roadmap body. The three CLI shims that hoist must therefore fetch and forward
# it themselves; otherwise `intent-alignment` is inert on the primary autonomous
# path while the gate still reports green. This gates the PROSE of all three
# shims and their shipped templates; scripts/verify-workflow-dispatch.sh's
# section 6 gates the resulting BEHAVIOR, and scripts/verify-skill-autopilot.sh
# gates the autopilot loop's own copy.
for shim in \
    "$REPO_ROOT/.claude/skills/rdm-autopilot/SKILL.md" \
    "$REPO_ROOT/.claude/skills/rdm-do/SKILL.md" \
    "$REPO_ROOT/.claude/skills/rdm-dispatch-phase/SKILL.md" \
    "$REPO_ROOT/rdm-core/src/templates/skill-autopilot-cli.md" \
    "$REPO_ROOT/rdm-core/src/templates/skill-do-cli.md" \
    "$REPO_ROOT/rdm-core/src/templates/skill-dispatch-phase-cli.md"; do
    [ -f "$shim" ] || fail "AC-PLAN-INTENT-HOIST: hoisting shim not found: $shim"
    grep -q 'roadmap show' "$shim" ||
        fail "AC-PLAN-INTENT-HOIST: $shim hoists phaseMeta but never reads the roadmap body — intent-alignment would be inert on this path"
    grep -qF 'roadmapBody' "$shim" ||
        fail "AC-PLAN-INTENT-HOIST: $shim never names roadmapBody — the roadmap body it reads would never reach the plan gate"
    # The field must be IN the assembled phaseMeta object, not merely mentioned.
    # The literal also pins `verify` (the phase-time verification command the
    # hoist must forward, see docs/verify-gate.md): a hoist that omits it is
    # rejected by hoistedMetaComplete and silently costs a Stage-0 agent.
    grep -qF 'body, roadmapBody, verify, directives, directivesSkipped, models:' "$shim" ||
        fail "AC-PLAN-INTENT-HOIST: $shim must carry roadmapBody, verify AND the two directive keys inside the assembled phaseMeta object literal"
    # `--raw` is load-bearing, not cosmetic: a bare `config get` prints
    # `<value>  (source: repo config)`, and every one of these shims tells the
    # agent to keep the printed value VERBATIM. Without --raw the annotation
    # rides along into the hoisted `verify` field.
    grep -qF 'config get dispatch.verify --raw' "$shim" ||
        fail "AC-PLAN-INTENT-HOIST: $shim hoists a meta payload but never reads dispatch.verify --raw — hoistedMetaComplete would reject it, or the hoisted value would carry a (source: ...) annotation"
    # PROJECT DIRECTIVES (dispatch-dev-discipline phase 3). Same structural trap
    # as roadmapBody: `directives` is OPTIONAL and out of hoistedMetaComplete's
    # key list, so a hoisting caller that omits it turns directive injection off
    # for that dispatch with nothing reporting it. Unlike `verify`, an empty or
    # failed read must NOT abandon the hoist — absent directives are normal — and
    # that instruction is exactly what a shim can get wrong by copying the verify
    # bullet, so the prose is pinned too.
    grep -qF 'dispatch directives --format json' "$shim" ||
        fail "AC-DIRECTIVES-HOIST: $shim hoists a meta payload but never runs \`rdm dispatch directives --format json\` — a hoisting caller skips Stage 0, so project directives would silently never be injected on this path"
    grep -qF 'directivesSkipped' "$shim" ||
        fail "AC-DIRECTIVES-HOIST: $shim never names directivesSkipped — a withheld project rule would be unobservable on this path"
    grep -qF 'must NOT abandon the hoist' "$shim" ||
        fail "AC-DIRECTIVES-HOIST: $shim must say explicitly that an empty or failed directive read does NOT abandon the hoist (absent directives are normal, unlike an unresolvable verify command)"
done
pass "AC-PLAN-INTENT-HOIST: all three hoisting shims and their shipped CLI templates fetch the roadmap body and the verify command, and forward both"
pass "AC-DIRECTIVES-HOIST: all six hoisting shims resolve project directives, forward both keys, and state that an empty read never abandons the hoist"

# The MCP flavors deliberately do NOT hoist (no model-resolve tool), so they must
# not grow a roadmapBody instruction either — the in-workflow Stage-0 fetch is
# their path and it already reads the roadmap body.
for mcp in \
    "$REPO_ROOT/rdm-core/src/templates/skill-autopilot-mcp.md" \
    "$REPO_ROOT/rdm-core/src/templates/skill-do-mcp.md" \
    "$REPO_ROOT/rdm-core/src/templates/skill-dispatch-phase-mcp.md"; do
    [ -f "$mcp" ] || fail "AC-PLAN-INTENT-HOIST: MCP template not found: $mcp"
    if grep -qF 'roadmapBody' "$mcp"; then
        fail "AC-PLAN-INTENT-HOIST: $mcp must not hoist roadmapBody — MCP has no model-resolve tool, so the in-workflow Stage-0 fetch is its path"
    fi
done
pass "AC-PLAN-INTENT-HOIST: the three MCP flavors carry no roadmapBody hoist"

# Self-test: strip the field from one shim's phaseMeta literal and confirm detection.
sed 's/body, roadmapBody, verify, models:/body, models:/' "$REPO_ROOT/.claude/skills/rdm-do/SKILL.md" >"$TMP/hoist-shim-mutant.md"
if grep -qF 'body, roadmapBody, verify, models:' "$TMP/hoist-shim-mutant.md"; then
    fail "AC-PLAN-INTENT-HOIST detector broken — roadmapBody stripped from the phaseMeta literal was not detected"
fi
pass "AC-PLAN-INTENT-HOIST detector fires when roadmapBody is stripped from a shim's phaseMeta literal"

# Self-test: drop --raw from one shim's declared-key read and confirm detection.
sed 's/config get dispatch.verify --raw/config get dispatch.verify/' "$REPO_ROOT/.claude/skills/rdm-do/SKILL.md" >"$TMP/hoist-shim-raw-mutant.md"
if grep -qF 'config get dispatch.verify --raw' "$TMP/hoist-shim-raw-mutant.md"; then
    fail "AC-PLAN-INTENT-HOIST detector broken — a --raw-less declared-key read was not produced"
fi
pass "AC-PLAN-INTENT-HOIST detector fires when a shim drops --raw from its declared-key read"

# AC-STAMP: dispatch-phase stamps the phase/task in-progress, best-effort, right
# after Stage 0 (metadata + model resolution) and before the plan gate.
#
#   (a) presence — the `stamp:in-progress` agent label and both the phase-mode
#       (`rdm phase update`) and task-mode (`rdm task update`) status commands.
#   (b) ordering — the call site sits textually AFTER Stage 0 resolves
#       `reviewModels` (the last stage-0 local) and BEFORE the first
#       `runPlanGate(` call (the plan gate).
#   (c) guarded — an immediately-preceding `if (!planOnly) {` wraps the call, so
#       a --plan-only pass (which does no implementation) never misreports the
#       item as in-progress.
#   (d) fails quietly — the text strictly between the stamp label and the next
#       `runPlanGate(` call contains no `return itemOutcome`/`return {`, proving
#       a failed stamp cannot short-circuit the dispatch.
grep -q "label: 'stamp:in-progress'" "$WF" || fail "AC-STAMP: missing the stamp:in-progress agent label"
grep -qF "' task update ' + target + ' --status in-progress" "$WF" ||
    fail "AC-STAMP: missing the task-mode 'rdm task update ... --status in-progress' command"
grep -qF "' phase update ' +" "$WF" || fail "AC-STAMP: missing the phase-mode 'rdm phase update' command"
grep -qF -- '--status in-progress' "$WF" || fail "AC-STAMP: missing a '--status in-progress' status string"
pass "AC-STAMP: stamp:in-progress label and both phase/task status commands are present"

# AC-STAMP Phase-mode scoped assertion: extract and normalize the buildStampInProgressPrompt
# function body to verify the phase-mode branch carries all required tokens:
# --status in-progress --no-edit --roadmap, roadmapSlugArg, and the two
# parameterization helpers (the project flag is no longer a `--project rdm`
# literal — see section 9 — so the check is that the builder DERIVES both the
# binary and the flag rather than hardcoding either).
# The phase-mode command is multi-line, so grep on a single normalized line.
assert_stamp_phase_mode() {
    extract_fn_body "$1" buildStampInProgressPrompt >"$TMP/stamp-fn"
    [ -s "$TMP/stamp-fn" ] || return 1
    # Normalize: collapse newlines and multiple spaces into single spaces
    cat "$TMP/stamp-fn" | tr -d '\n' | tr -s ' ' >"$TMP/stamp-normalized"
    [ -s "$TMP/stamp-normalized" ] || return 1
    # Check phase-mode tokens in the normalized body. The phase-mode branch
    # must contain: --status in-progress --no-edit --roadmap, roadmapSlugArg,
    # resolveRdmBin( and projectFlag(
    grep -qF -- '--status in-progress --no-edit --roadmap' "$TMP/stamp-normalized" || return 1
    grep -qF 'roadmapSlugArg' "$TMP/stamp-normalized" || return 1
    grep -qF 'resolveRdmBin(' "$TMP/stamp-normalized" || return 1
    grep -qF 'projectFlag(' "$TMP/stamp-normalized" || return 1
    return 0
}

assert_stamp_phase_mode "$WF" ||
    fail "AC-STAMP: phase-mode buildStampInProgressPrompt must contain --status in-progress --no-edit --roadmap, roadmapSlugArg, resolveRdmBin( and projectFlag("
pass "AC-STAMP: phase-mode command includes required --roadmap argument and all tokens"

# Self-test: prove the phase-mode assertion fires on a --roadmap deletion.
# Create a scratch copy and remove the roadmapSlugArg variable entirely from the
# multi-line concatenation by deleting its line.
cp "$WF" "$TMP/stamp-mutant-phase-roadmap.js"
sed '/function buildStampInProgressPrompt/,/^}/{ /roadmapSlugArg/d; }' "$WF" >"$TMP/stamp-mutant-phase-roadmap.js"
if assert_stamp_phase_mode "$TMP/stamp-mutant-phase-roadmap.js"; then
    fail "AC-STAMP: phase-mode detector missed a deleted --roadmap argument"
fi
pass "AC-STAMP: phase-mode detector fires when --roadmap is removed from the phase-mode command"

assert_stamp_ordering() {
    s0=$(grep -n "const reviewModels = {" "$1" | head -1 | cut -d: -f1)
    st=$(grep -n "label: 'stamp:in-progress'" "$1" | head -1 | cut -d: -f1)
    pg=$(grep -n 'await runPlanGate(' "$1" | head -1 | cut -d: -f1)
    [ -n "$s0" ] && [ -n "$st" ] && [ -n "$pg" ] || return 1
    [ "$s0" -lt "$st" ] && [ "$st" -lt "$pg" ]
}
assert_stamp_ordering "$WF" ||
    fail "AC-STAMP: the stamp call must sit after Stage 0's model resolution and before the first runPlanGate( call"
pass "AC-STAMP: stamp call site sits after Stage 0 and before the plan gate"

assert_stamp_guarded() {
    awk '/if \(!planOnly\) \{/{p=1} p{print} p&&/^\}/{exit}' "$1" | grep -q "label: 'stamp:in-progress'"
}
assert_stamp_guarded "$WF" ||
    fail "AC-STAMP: the stamp call must be wrapped in an immediately-preceding 'if (!planOnly) {' guard"
pass "AC-STAMP: stamp call is guarded by 'if (!planOnly) {'"

# Text strictly between the stamp label and the next runPlanGate( call must
# contain no early-return — a stamp failure can only log, never short-circuit
# the dispatch the way a genuine Stage-0 fetch failure legitimately does.
assert_stamp_fails_quietly() {
    awk '
        index($0, "label: \x27stamp:in-progress\x27") { p = 1; next }
        index($0, "await runPlanGate(") { exit }
        p { print }
    ' "$1" >"$TMP/stamp-to-gate"
    [ -s "$TMP/stamp-to-gate" ] || return 1
    ! grep -qE 'return itemOutcome|return \{' "$TMP/stamp-to-gate"
}
assert_stamp_fails_quietly "$WF" ||
    fail "AC-STAMP: a stamp failure must not early-return between the stamp call and the plan gate"
pass "AC-STAMP: no early-return between the stamp call and the plan gate — a failed stamp degrades quietly"

# Self-tests: prove each detector is load-bearing.
sed '/if (!planOnly) {/,/^}/d' "$WF" >"$TMP/stamp-mutant-guard"
if assert_stamp_guarded "$TMP/stamp-mutant-guard"; then
    fail "AC-STAMP: guard detector missed a removed 'if (!planOnly) {' wrapper"
fi
pass "AC-STAMP: guard detector fires when the planOnly wrapper is stripped"

sed "s/' task update '/' task updateX '/" "$WF" >"$TMP/stamp-mutant-cmd"
if grep -qF "' task update ' + target + ' --status in-progress" "$TMP/stamp-mutant-cmd"; then
    fail "AC-STAMP: presence detector missed a mangled task-mode command string"
fi
pass "AC-STAMP: presence detector fires when the task-mode command string is mangled"

cp "$WF" "$TMP/stamp-mutant-return.js"
awk '
    index($0, "label: \x27stamp:in-progress\x27") && !done { print; print "      return itemOutcome({ fetchError: true })"; done = 1; next }
    { print }
' "$WF" >"$TMP/stamp-mutant-return.js"
if assert_stamp_fails_quietly "$TMP/stamp-mutant-return.js"; then
    fail "AC-STAMP: fails-quietly detector missed a planted 'return itemOutcome' between the stamp and the plan gate"
fi
pass "AC-STAMP: fails-quietly detector fires on a planted early-return between the stamp and the plan gate"

# AC-4 (driver-level reinforcement): the retry loops are BOUNDED. "Bounded" is a
# dataflow property no grep can decide, so the check is split in two:
#
#   * SEMANTIC half — section 1c drives runPlanGate/runCodeGate in Node and
#     proves a never-clean stage terminates at exactly `budget + 1` reviews.
#   * SYNTACTIC half (here) — the DRIVER REGION carries no `while` at all, and
#     only `for` headers on an exact literal allowlist. A loose "looks bounded"
#     regex would ratchet the guard down to nothing, so the allowlist is exact.
#
# Scope: the two stamped lib blocks (`rdm-wf-review-refute-fix`, `dispatch-outcome`) are
# deliberately EXCLUDED — the budget loops legitimately live in the dispatch-
# outcome block, and this phase does not own the review block. Scoping is by
# MARKER TOKEN, never line numbers, so it survives either block growing.
driver_region() {
    awk '
        index($0, ">>> review-refute-fix:begin") { skip = 1 }
        index($0, ">>> dispatch-outcome:begin") { skip = 1 }
        !skip { print }
        index($0, ">>> review-refute-fix:end") { skip = 0 }
        index($0, ">>> dispatch-outcome:end") { skip = 0 }
    ' "$1"
}

# The ONLY `for` headers permitted in the driver region, as exact literals.
ALLOWED_FOR_HEADERS='for (let i = 0; i < maxPlanRevise; i++) {
for (let i = 0; i < maxCodeRework; i++) {'

assert_no_while_in_driver() {
    driver_region "$1" | grep -qE '(^|[^A-Za-z_])while[[:space:]]*\(' && return 1
    return 0
}

driver_for_headers() {
    driver_region "$1" |
        grep -oE '(^|[^A-Za-z_])for[[:space:]]*\(.*' |
        sed -e 's/^[^A-Za-z_]*//' -e 's/[[:space:]]*$//'
}

assert_for_headers_allowlisted() {
    driver_for_headers "$1" >"$TMP/for-headers"
    bad=0
    while IFS= read -r hdr; do
        [ -n "$hdr" ] || continue
        printf '%s\n' "$ALLOWED_FOR_HEADERS" | grep -Fxq -- "$hdr" || {
            printf 'disallowed for-header: %s\n' "$hdr" >&2
            bad=1
        }
    done <"$TMP/for-headers"
    [ "$bad" -eq 0 ]
}

assert_no_while_in_driver "$WF" ||
    fail "AC-4: no 'while' is permitted in rdm-wf-dispatch-phase.js's DRIVER REGION (the stamped review-refute-fix and dispatch-outcome blocks are deliberately out of scope — this phase does not own them; the budget loops belong in the dispatch-outcome block)"
assert_for_headers_allowlisted "$WF" ||
    fail "AC-4: a 'for' header in rdm-wf-dispatch-phase.js's DRIVER REGION is not on the allowlist (only the two budget loops are permitted, and only in the dispatch-outcome block — the stamped blocks themselves are out of scope)"
pass "AC-4: driver region has no 'while' and only allowlisted 'for' headers"

# Self-test A: a planted `while (true)` in the driver region trips the while rule
# and NOT the for rule.
cp "$WF" "$TMP/mutant-while.js"
printf '\nwhile (true) {}\n' >>"$TMP/mutant-while.js"
if assert_no_while_in_driver "$TMP/mutant-while.js"; then
    fail "AC-4: while rule missed a planted 'while (true)' in the driver region"
fi
assert_for_headers_allowlisted "$TMP/mutant-while.js" ||
    fail "AC-4: the for rule wrongly fired on a planted while — the two rules must trip independently"

# Self-test B: a planted unbounded `for (;;)` trips the for rule and NOT the
# while rule.
cp "$WF" "$TMP/mutant-for.js"
printf '\nfor (;;) {}\n' >>"$TMP/mutant-for.js"
if assert_for_headers_allowlisted "$TMP/mutant-for.js" 2>/dev/null; then
    fail "AC-4: for allowlist missed a planted unbounded 'for (;;)' in the driver region"
fi
assert_no_while_in_driver "$TMP/mutant-for.js" ||
    fail "AC-4: the while rule wrongly fired on a planted for — the two rules must trip independently"

# Self-test B2: a bare-literal bound is still NOT on the allowlist.
cp "$WF" "$TMP/mutant-for-literal.js"
printf '\nfor (let i = 0; i < 3; i++) {}\n' >>"$TMP/mutant-for-literal.js"
if assert_for_headers_allowlisted "$TMP/mutant-for-literal.js" 2>/dev/null; then
    fail "AC-4: for allowlist accepted a non-allowlisted bare-literal loop header"
fi

# Self-test C (negative case): a LEGAL budget-bounded for header passes both.
cp "$WF" "$TMP/mutant-for-legal.js"
printf '\nfor (let i = 0; i < maxPlanRevise; i++) {\n}\n' >>"$TMP/mutant-for-legal.js"
assert_no_while_in_driver "$TMP/mutant-for-legal.js" ||
    fail "AC-4: while rule wrongly fired on a legal budget-bounded for"
assert_for_headers_allowlisted "$TMP/mutant-for-legal.js" ||
    fail "AC-4: for allowlist wrongly rejected an allowlisted budget-bounded header"

# Self-test D (scoping is real): a `while (true)` INSIDE the dispatch-outcome
# markers must NOT trip the driver-region rules.
awk '
  { print }
  index($0, ">>> dispatch-outcome:begin") { print "while (true) { break }" }
' "$WF" >"$TMP/mutant-inblock.js"
grep -q 'while (true)' "$TMP/mutant-inblock.js" || fail "AC-4 scoping self-test did not plant its loop"
assert_no_while_in_driver "$TMP/mutant-inblock.js" ||
    fail "AC-4: driver-region scoping is not real — a loop inside the stamped block tripped the driver rule"
pass "AC-4: while/for mutants trip their own rule independently; in-block loops are correctly out of scope"

# meta.phases must list EXACTLY the distinct `phase:` values the driver + the
# inlined review block emit — no emitted-but-undeclared phase (e.g. Find/Refute
# from the stamped block) and no declared-but-never-emitted phase (e.g. a stale
# CodeReview). Compare the two sorted-unique sets.
DECLARED_PHASES=$(declared_phases "$WF")
EMITTED_PHASES=$(emitted_phases "$WF")
if [ "$DECLARED_PHASES" = "$EMITTED_PHASES" ]; then
    pass "meta.phases lists exactly the emitted phase: literals ($(echo "$EMITTED_PHASES" | tr '\n' ' '))"
else
    printf 'declared (meta.phases): %s\n' "$(echo "$DECLARED_PHASES" | tr '\n' ' ')" >&2
    printf 'emitted   (phase: ...): %s\n' "$(echo "$EMITTED_PHASES" | tr '\n' ' ')" >&2
    fail "meta.phases drift: declared phases != emitted phase: literals (add/remove entries to match)"
fi
# Self-test: a planted emitted-but-undeclared phase MUST break the check.
sed "s/phase: 'Plan',/phase: 'Ghost',/" "$WF" >"$TMP/wf.phase.scratch"
if [ "$(declared_phases "$TMP/wf.phase.scratch")" = "$(emitted_phases "$TMP/wf.phase.scratch")" ]; then
    fail "meta.phases consistency check did NOT catch a planted undeclared phase"
fi
pass "meta.phases consistency detector catches a planted undeclared phase"

# --- 3-verify. NO TOOLCHAIN LITERAL IN THE VERIFY PATH ------------------------
# The verify gate's check set is project-supplied DATA. rdm's own resolution and
# execution path must therefore name no language, package manager, or test tool
# — the not-hardcoded-to-Rust constraint is satisfied by CONSTRUCTION, not by a
# fixture. The new code is fenced with `verify-gate:begin/end` markers in every
# copy so this grep has a precisely scoped region rather than a whole-file
# approximation, and an OCCURRENCE FLOOR guarantees a vanished or renamed region
# fails loudly instead of passing vacuously.
say "3-verify. The fenced verify-gate regions name no toolchain literal (with an occurrence floor)"

extract_verify_fence() {
    awk '
        index($0, ">>> verify-gate:begin") { infence = 1; next }
        index($0, ">>> verify-gate:end") { infence = 0; next }
        infence { print }
    ' "$1"
}

# The CURATED literal list. Deliberately EXCLUDES `make` and `just`: both occur
# frequently as ordinary English in these files' existing prose ("make sure",
# "just the"), so including them would make the gate un-passable for reasons
# that have nothing to do with toolchains. That rationale is recorded
# QUALITATIVELY on purpose — an occurrence count baked in here would rot the
# moment any prose in these files moves. Every candidate below was re-derived
# with `grep -ciw` against all four copies at implementation time and is
# genuinely zero-occurrence.
VERIFY_TOOLCHAIN_LITERALS='cargo|npm|pnpm|yarn|pytest|gradle|mvn|nextest|clippy|rustfmt|dotnet|rake|tox|go test|hk run'

# The identifiers a real verify-gate region must contain. The floor is 3
# DISTINCT matches, so a region that was gutted down to a stub cannot pass.
VERIFY_REQUIRED_IDENTS='dispatch\.verify|verifyCommand|verify:run|extractVerifyCommand|buildVerifyPrompt'

assert_verify_fence_clean() {
    f=$1
    minfloor=$2
    extract_verify_fence "$f" >"$TMP/verify-fence.txt"
    [ -s "$TMP/verify-fence.txt" ] || {
        VERIFY_FENCE_ERR="the verify-gate fenced region in $f is EMPTY or missing"
        return 1
    }
    hits=$(grep -cE "$VERIFY_REQUIRED_IDENTS" "$TMP/verify-fence.txt" | tr -d ' ')
    [ "$hits" -ge "$minfloor" ] || {
        VERIFY_FENCE_ERR="the verify-gate region in $f matched only $hits of the required identifiers (floor $minfloor)"
        return 1
    }
    if grep -nEiw "$VERIFY_TOOLCHAIN_LITERALS" "$TMP/verify-fence.txt" >"$TMP/verify-fence-hits.txt"; then
        VERIFY_FENCE_ERR="toolchain literal(s) in $f's verify-gate region: $(cat "$TMP/verify-fence-hits.txt")"
        return 1
    fi
    return 0
}

# Every copy, per-file (mirroring § 9a's per-copy zeroing) so a half-applied
# propagation cannot pass. The lib carries only the pure-helper fence; the three
# workflow copies additionally carry the two driver fences.
VERIFY_FENCE_ERR=''
for f in \
    "$LIB" \
    "$WF" \
    "$REPO_ROOT/rdm-core/src/templates/workflows/rdm-wf-dispatch-phase.js" \
    "$REPO_ROOT/plugins/rdm/workflows/rdm-wf-dispatch-phase.js"; do
    [ -f "$f" ] || fail "3-verify: expected copy not found: $f"
    assert_verify_fence_clean "$f" 3 || fail "3-verify: $VERIFY_FENCE_ERR"
done
pass "3-verify: all four copies carry a non-empty verify-gate region with no toolchain literal"

# Self-test (a): a planted toolchain literal must FIRE the grep.
awk '
    index($0, ">>> verify-gate:begin") { print; print "// cargo nextest run"; next }
    { print }
' "$WF" >"$TMP/wf-planted-toolchain.js"
if cmp -s "$WF" "$TMP/wf-planted-toolchain.js"; then
    fail "3-verify self-test: the planted toolchain literal was a no-op"
fi
if assert_verify_fence_clean "$TMP/wf-planted-toolchain.js" 3; then
    fail "3-verify self-test: a planted 'cargo nextest run' inside the fence was NOT detected"
fi
pass "3-verify self-test: a planted toolchain literal inside the fence is detected"

# Self-test (b): a deleted region must FIRE the occurrence floor.
grep -v 'verify-gate:begin' "$WF" | grep -v 'verify-gate:end' >"$TMP/wf-no-fence.js"
if assert_verify_fence_clean "$TMP/wf-no-fence.js" 3; then
    fail "3-verify self-test: a REMOVED verify-gate region was not detected — the floor is vacuous"
fi
pass "3-verify self-test: a removed verify-gate region fires the occurrence floor"

# Self-test (c): the unmutated files still pass, so (a) and (b) are
# discriminating rather than blanket rejecters.
assert_verify_fence_clean "$WF" 3 || fail "3-verify self-test: the real workflow fails after the self-tests — the detector rejects everything"
pass "3-verify self-test: the unmutated workflow still passes (the detector discriminates)"

# AC4's NO-SECOND-COUNTER proof: the verify gate reuses maxCodeRework, so no
# verify-flavored budget name may exist anywhere under .claude/workflows/.
if grep -rnE 'maxVerify|verifyBudget|DEFAULT_MAX_VERIFY|VERIFY_BUDGET' "$REPO_ROOT/.claude/workflows/" >"$TMP/second-counter.txt" 2>/dev/null; then
    fail "3-verify: a second verify budget was introduced — the gate must reuse maxCodeRework: $(cat "$TMP/second-counter.txt")"
fi
printf 'const maxVerify = 3\n' >"$TMP/planted-budget.js"
grep -qE 'maxVerify|verifyBudget|DEFAULT_MAX_VERIFY|VERIFY_BUDGET' "$TMP/planted-budget.js" ||
    fail "3-verify: the second-counter detector is broken — a planted maxVerify was not matched"
pass "3-verify: no second verify budget exists under .claude/workflows/ (detector verified on a planted name)"

# AC5's static half: no verify-flavored OUTCOME literal was introduced anywhere.
for f in \
    "$LIB" \
    "$WF" \
    "$REPO_ROOT/rdm-core/src/templates/workflows/rdm-wf-dispatch-phase.js" \
    "$REPO_ROOT/plugins/rdm/workflows/rdm-wf-dispatch-phase.js"; do
    if grep -nE "outcome: *'[^']*verif" "$f" >/dev/null 2>&1; then
        fail "3-verify: $f introduces a verify-flavored outcome literal — the OUTCOME vocabulary must stay reviewed|rework|escalated"
    fi
done
pass "3-verify: no verify-flavored outcome literal in any copy"

# AC6's static half: the code-gate `review:` closure must contain no verify
# reference, so a retry inside review cannot multiply the run. The window runs
# from the `review: async () => {` inside the runCodeGate deps to the `act:` key.
awk '/^    review: async \(\) => \{/{p=1} p{print} p&&/^    act: async/{exit}' "$WF" >"$TMP/review-closure.txt"
[ -s "$TMP/review-closure.txt" ] || fail "3-verify: could not extract the code-gate review closure"
if grep -niE 'verify' "$TMP/review-closure.txt" >/dev/null 2>&1; then
    fail "3-verify: the code-gate review closure references verify — the single call site must live in runCodeGate only"
fi
pass "3-verify: the code-gate review closure contains no verify reference"

# --- 3-clean. THE SHIPPED DRIVER ACTUALLY WIRES THE CLEANLINESS PROBE ---------
# § 1e proves the LOGIC: a `d.clean` dep that reports a dirty tree drives the
# unit to rework. It deliberately treats an ABSENT dep as a SKIP, because every
# pre-existing fake-driven scenario omits it. That makes non-vacuity a STATIC
# question — is the probe wired in the shipped driver at all? — and this section
# is the answer. Without it, deleting the `clean:` binding would leave every
# harness section green while shipping the original defect.
say "3-clean. Every copy wires the clean:check probe, and the fenced regions carry the real helpers"

extract_clean_fence() {
    awk '
        index($0, ">>> clean-worktree:begin") { infence = 1; next }
        index($0, ">>> clean-worktree:end") { infence = 0; next }
        infence { print }
    ' "$1"
}

# The identifiers a real clean-worktree region must contain. Floor of 4 DISTINCT
# matches, so a region gutted to a stub cannot pass.
CLEAN_REQUIRED_IDENTS='parseWorktreeStatus|dirtyWorktreeFinding|actChangedCode|WORKTREE_CLEAN_SCHEMA|buildCleanCheckPrompt|clean:check'

assert_clean_fence() {
    f=$1
    minfloor=$2
    extract_clean_fence "$f" >"$TMP/clean-fence.txt"
    [ -s "$TMP/clean-fence.txt" ] || {
        CLEAN_FENCE_ERR="the clean-worktree fenced region in $f is EMPTY or missing"
        return 1
    }
    hits=$(grep -cE "$CLEAN_REQUIRED_IDENTS" "$TMP/clean-fence.txt" | tr -d ' ')
    [ "$hits" -ge "$minfloor" ] || {
        CLEAN_FENCE_ERR="the clean-worktree region in $f matched only $hits of the required identifiers (floor $minfloor)"
        return 1
    }
    return 0
}

CLEAN_FENCE_ERR=''
for f in \
    "$LIB" \
    "$WF" \
    "$REPO_ROOT/rdm-core/src/templates/workflows/rdm-wf-dispatch-phase.js" \
    "$REPO_ROOT/plugins/rdm/workflows/rdm-wf-dispatch-phase.js"; do
    [ -f "$f" ] || fail "3-clean: expected copy not found: $f"
    assert_clean_fence "$f" 4 || fail "3-clean: $CLEAN_FENCE_ERR"
done
pass "3-clean: all four copies carry a non-empty clean-worktree region above the occurrence floor"

# The DRIVER wiring, per shipped copy: the dep must be bound with the right
# label, model class, and schema, and runCodeGate must consult it.
assert_clean_wired() {
    f=$1
    grep -qF "clean: async () =>" "$f" || {
        CLEAN_WIRE_ERR="$f: runCodeGate's deps object has no \`clean\` binding"
        return 1
    }
    grep -qF "label: 'clean:check'" "$f" || {
        CLEAN_WIRE_ERR="$f: the cleanliness probe has no clean:check label"
        return 1
    }
    grep -qF 'schema: WORKTREE_CLEAN_SCHEMA' "$f" || {
        CLEAN_WIRE_ERR="$f: the cleanliness probe declares no WORKTREE_CLEAN_SCHEMA"
        return 1
    }
    grep -qF 'buildCleanCheckPrompt(worktreeRef, cfg)' "$f" || {
        CLEAN_WIRE_ERR="$f: the cleanliness probe is not built from the stamped prompt builder"
        return 1
    }
    grep -qF "typeof d.clean === 'function'" "$f" || {
        CLEAN_WIRE_ERR="$f: runCodeGate never consults the clean dep"
        return 1
    }
    return 0
}

CLEAN_WIRE_ERR=''
for f in \
    "$WF" \
    "$REPO_ROOT/rdm-core/src/templates/workflows/rdm-wf-dispatch-phase.js" \
    "$REPO_ROOT/plugins/rdm/workflows/rdm-wf-dispatch-phase.js"; do
    assert_clean_wired "$f" || fail "3-clean: $CLEAN_WIRE_ERR"
done
pass "3-clean: every shipped workflow copy binds clean:check with the WORKTREE_CLEAN schema and runCodeGate consults it"

# Self-test (a): delete the `clean` dep binding from a scratch workflow.
grep -v "clean: async () =>" "$WF" >"$TMP/wf-no-clean-dep.js"
if cmp -s "$WF" "$TMP/wf-no-clean-dep.js"; then
    fail "3-clean self-test: the planted mutation was a no-op — no clean dep binding was found"
fi
if assert_clean_wired "$TMP/wf-no-clean-dep.js"; then
    fail "3-clean self-test: a workflow with NO clean dep passed — the wiring gate is vacuous"
fi
pass "3-clean self-test: deleting the clean dep binding is detected"

# Self-test (b): keep the binding but drop the label, so the inventory doc and
# any label-scoped tooling would silently lose the probe.
sed "s/label: 'clean:check'/label: 'zz-renamed'/" "$WF" >"$TMP/wf-relabelled-clean.js"
if cmp -s "$WF" "$TMP/wf-relabelled-clean.js"; then
    fail "3-clean self-test: the planted relabel was a no-op"
fi
if assert_clean_wired "$TMP/wf-relabelled-clean.js"; then
    fail "3-clean self-test: a relabelled probe passed — the label assertion is vacuous"
fi
pass "3-clean self-test: relabelling the probe is detected"

# Self-test (c): a removed fence must fire the occurrence floor.
grep -v 'clean-worktree:begin' "$WF" | grep -v 'clean-worktree:end' >"$TMP/wf-no-clean-fence.js"
if assert_clean_fence "$TMP/wf-no-clean-fence.js" 4; then
    fail "3-clean self-test: a REMOVED clean-worktree region was not detected — the floor is vacuous"
fi
pass "3-clean self-test: a removed clean-worktree region fires the occurrence floor"

# Positive control: the real workflow still passes both detectors.
CLEAN_WIRE_ERR=''
assert_clean_wired "$WF" || fail "3-clean self-test: the real workflow fails after the self-tests — the detector rejects everything"
assert_clean_fence "$WF" 4 || fail "3-clean self-test: the real workflow's fence fails after the self-tests"
pass "3-clean self-test: the unmutated workflow still passes (the detectors discriminate)"

# The probe must never become a mutation surface. Both the probe prompt and the
# act prompt name the destructive shortcuts explicitly so an agent cannot reach
# for one to make the tree LOOK clean.
grep -qF 'git clean' "$LIB" || fail "3-clean: the act prompt must explicitly forbid \`git clean\` as a shortcut"
grep -qF 'Never amend an existing commit' "$LIB" ||
    fail "3-clean: the act prompt must forbid amending — the implementation commit predates the finding"
pass "3-clean: the act prompt forbids the destructive shortcuts and amending"

# --- 4. MODULE PARSE ---------------------------------------------------------
say "4. Module parse: rdm-wf-dispatch-phase.js loads under module semantics (no SyntaxError)"

if parse_workflow "$WF" >/dev/null 2>&1; then
    pass "rdm-wf-dispatch-phase.js parses under module semantics (top-level meta declared once)"
else
    parse_workflow "$WF" >&2 || true
    fail "rdm-wf-dispatch-phase.js does NOT parse — fix the SyntaxError (e.g. a duplicate top-level 'meta')"
fi

# Self-test: prove the parse gate is non-vacuous. Injecting a redeclared top-level
# `meta` into a scratch copy MUST make the gate FAIL; the unmodified file PASSES.
say "4b. Parse gate fires on a planted syntax error (self-test)"
cp "$WF" "$TMP/wf.parse.scratch"
printf '\nlet meta = null\n' >>"$TMP/wf.parse.scratch"
if parse_workflow "$TMP/wf.parse.scratch" >/dev/null 2>&1; then
    fail "parse gate did NOT catch a planted duplicate top-level 'meta' declaration"
fi
if parse_workflow "$WF" >/dev/null 2>&1; then
    pass "parse gate fails on a planted syntax error and passes the unmodified file"
else
    fail "parse gate regressed on the unmodified file after the self-test"
fi

# --- 5. DOC / CONSTANT AGREEMENT ---------------------------------------------
say "5. docs/escalation-protocol.md § Budgets states the same numbers the code uses"

DOC="$REPO_ROOT/docs/escalation-protocol.md"
# autopilot is a prose skill now (workflow-orchestration roadmap phase 3): there
# is no lib/autopilot.mjs to read a `const` out of anymore. Its two budgets
# (DEFAULT_MAX_REWORK, DEFAULT_GLOBAL_BUDGET) are read straight out of the
# skill's own prose, which states them literally as `` `NAME = N` `` for exactly
# this purpose — see .claude/skills/rdm-autopilot/SKILL.md.
AUTOPILOT_SKILL="$REPO_ROOT/.claude/skills/rdm-autopilot/SKILL.md"
# DEFAULT_MAX_CODE_REWORK was lifted into the canonical review source alongside
# classifyOutcome; the plan-revise budget stays with the dispatch decision core.
REVIEW_LIB="$REPO_ROOT/.claude/workflows/lib/review.mjs"
[ -f "$DOC" ] || fail "escalation protocol doc not found: $DOC"
[ -f "$AUTOPILOT_SKILL" ] || fail "autopilot skill not found: $AUTOPILOT_SKILL"
[ -f "$REVIEW_LIB" ] || fail "canonical review lib not found: $REVIEW_LIB"

const_value() {
    grep -oE "const $2 = [0-9]+" "$1" | head -1 | grep -oE '[0-9]+$'
}

# The skill states its budgets in prose (`` `NAME = N` ``), not as a JS `const`.
skill_const_value() {
    grep -oE "$2 = [0-9]+" "$1" | head -1 | grep -oE '[0-9]+$'
}

assert_doc_agrees() {
    doc=$1
    pr=$(const_value "$LIB" DEFAULT_MAX_PLAN_REVISE)
    cr=$(const_value "$REVIEW_LIB" DEFAULT_MAX_CODE_REWORK)
    ar=$(skill_const_value "$AUTOPILOT_SKILL" DEFAULT_MAX_REWORK)
    gb=$(skill_const_value "$AUTOPILOT_SKILL" DEFAULT_GLOBAL_BUDGET)
    [ -n "$pr" ] && [ -n "$cr" ] && [ -n "$ar" ] && [ -n "$gb" ] || return 1
    grep -qF "Plan-revise budget = $pr" "$doc" || {
        printf 'doc does not state: Plan-revise budget = %s\n' "$pr" >&2
        return 1
    }
    grep -qF "Code-rework budget = $cr" "$doc" || {
        printf 'doc does not state: Code-rework budget = %s\n' "$cr" >&2
        return 1
    }
    grep -qF "Autopilot rework re-dispatch budget = $ar" "$doc" || {
        printf 'doc does not state: Autopilot rework re-dispatch budget = %s\n' "$ar" >&2
        return 1
    }
    grep -qF "Autopilot global step budget = $gb" "$doc" || {
        printf 'doc does not state: Autopilot global step budget = %s\n' "$gb" >&2
        return 1
    }
    return 0
}

assert_doc_agrees "$DOC" ||
    fail "docs/escalation-protocol.md § Budgets disagrees with the constants in lib/dispatch-phase.mjs / lib/review.mjs / the rdm-autopilot SKILL.md"
pass "all four budgets are named in the doc with exactly the values the code/skill declare"

# The doc must also spell out the attempt sequence, the independence of the two
# in-run budgets, the per-run override names, and the which-lane divergence note.
for needle in 'revise 1' 'revise 2' 'escalate' 'independent' 'maxPlanRevise' 'maxCodeRework' 'rdm-core/src/templates/'; do
    grep -qF -- "$needle" "$DOC" || fail "docs/escalation-protocol.md § Budgets must mention: $needle"
done
# The which-lane note must record that the shipped prose templates remain at 1.
grep -qE 'rdm-core/src/templates/' "$DOC" || fail "missing the which-lane note"
grep -qE 'remain at 1|stay at 1|still 1' "$DOC" ||
    fail "the which-lane note must state that the shipped prose templates remain at 1"
pass "doc carries the attempt sequence, independence, override names, and the which-lane note"

# Self-test: a doc whose numbers were rewritten must FAIL the agreement check.
sed 's/budget = 2/budget = 1/g' "$DOC" >"$TMP/doc.scratch"
if assert_doc_agrees "$TMP/doc.scratch" 2>/dev/null; then
    fail "doc/constant agreement check did NOT fire on a doc with rewritten budget values"
fi
pass "doc/constant agreement detector fires on rewritten budget values"

# --- 6. HOIST / ABSORB --------------------------------------------------------
# Phase 3 of the workflow-token-reduction roadmap eliminates mechanical
# subagents by never spawning them (docs/mechanical-agent-inventory.md):
#
#   * HOIST      — `args.phaseMeta` / `args.taskMeta` replace the Stage-0
#                  `fetch:phase-meta` / `fetch:task-meta` agent, behind an
#                  ALL-OR-NOTHING `hoistedMetaComplete` guard (non-empty body
#                  plus all five resolved model ids).
#   * REDUNDANCY — `args.alreadyInProgress` suppresses `stamp:in-progress` when
#                  the caller already performed that write, independently of the
#                  pre-existing `!planOnly` guard.
#   * ABSORB     — the implementer returns its own branch diff, which the review
#                  closure consumes ONE-SHOT in place of the `diff:signals`
#                  agent; a null/empty/absent return falls back to that agent.
#
# Every one of the three is OPTIONAL: the original agent call is reached through
# an `else` branch and is never deleted, so a direct `Workflow` invocation with
# the pre-phase args shape behaves exactly as before. This section drives the
# REAL driver in Node under a recording fake agent (the wrapper technique from
# scripts/verify-workflow-review-outcome.sh section 3) and asserts both halves,
# then plants the mutations that must break each assertion.
say "6. Hoist/absorb: caller-supplied args eliminate mechanical agents, and every one falls back when absent"

cat >"$TMP/hoist.mjs" <<'NODE_HOIST'
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const wfPath = process.argv[2];
let src = fs.readFileSync(wfPath, 'utf8');
src = src.replace(/^export /m, '');

// Wrap the workflow script's top-level body (which uses `export`, a top-level
// `return`, and top-level `await`) in an async function taking the Workflow
// runtime's ambient globals as parameters, so the REAL driver runs unmodified.
const wrapperPath = path.join(os.tmpdir(), 'verify-workflow-dispatch-hoist-wrapped.mjs');
fs.writeFileSync(wrapperPath, 'export default async function(args, agent, pipeline, parallel, log) {\n' + src + '\n}\n');
const mod = await import('file://' + wrapperPath + '?t=' + process.pid);
const rawRun = mod.default;
// dispatch-phase's environment contract makes `rdmBin` OPTIONAL: an absent key
// defaults to a plain `rdm` on PATH (gated independently by section 9c). This
// section is about the HOIST args, not the environment axes, so every run here
// gets the explicit `'rdm'` PATH sentinel injected rather than relying on the
// default at 19 call sites — keeping this section's assertions independent of
// the defaulting behavior 9c owns. A caller-supplied `rdmBin`/`project` wins.
const run = (args, ...rest) => rawRun({ rdmBin: 'rdm', ...args }, ...rest);

async function refParallel(thunks) {
  return Promise.all(thunks.map((t) => Promise.resolve().then(t).catch(() => null)));
}
async function refPipeline(items, ...stages) {
  return Promise.all(
    items.map(async (item, i) => {
      let acc = item;
      for (const stage of stages) {
        try {
          acc = await stage(acc, item, i);
        } catch {
          return null;
        }
      }
      return acc;
    })
  );
}

const MODELS = {
  plan: 'm-plan',
  implement: 'm-impl',
  review_find: 'm-find',
  review_verify: 'm-verify',
  mechanical: 'm-mech',
};
const PHASE_META = { roadmap: 'rm', phase: '1', stem: 'phase-1-x', model: 'medium', body: 'PHASE BODY TEXT', verify: 'sh scripts/verify-all.sh', models: MODELS };
const TASK_META = { task: 'my-task', body: 'TASK BODY TEXT', verify: 'sh scripts/verify-all.sh', models: MODELS };
const PLAN_DOC = {
  steps_per_ac: [{ ac: 'AC1', steps: ['do it'] }],
  file_map: [{ path: 'a.rs', change: 'edit' }],
  tests_per_ac: [{ ac: 'AC1', test: 't' }],
  edge_cases: [],
  cross_phase_deps: [],
  summary: 'plan',
};

// makeAgent(o) — a recording fake `agent`. Every label the driver can emit is
// answered; `o.planFindings` seeds the PLAN-mode finders (forcing the
// escalation path when blocking), `o.codeFindingsByRound` seeds the CODE-mode
// finders per review round, and `o.implementResult` is what the implementer
// returns (an array = one entry per round).
function makeAgent(o) {
  o = o || {};
  const calls = [];
  let codeRound = -1;
  let implementRound = -1;
  const agent = async (prompt, opts) => {
    const label = (opts && opts.label) || '';
    calls.push({ label, prompt, opts });
    if (label === 'fetch:phase-meta') return o.fetchResult === undefined ? PHASE_META : o.fetchResult;
    if (label === 'fetch:task-meta') return o.fetchResult === undefined ? TASK_META : o.fetchResult;
    if (label === 'stamp:in-progress') return { ok: true };
    if (label === 'verify:run') return { exitCode: 0, output: '' };
    if (label === 'clean:check') return { porcelain: '' };
    if (label === 'plan:author' || label === 'plan:revise') return PLAN_DOC;
    if (label === 'act:code') return { handled: [] };
    if (label === 'diff:signals') return o.diffResult === undefined ? { changedFiles: ['fallback.rs'], diffText: '' } : o.diffResult;
    if (label === 'implement:worktree' || label === 'implement:rework') {
      implementRound++;
      const r = o.implementResult;
      if (r === undefined) return undefined;
      return Array.isArray(r) ? r[implementRound] : r;
    }
    const parts = label.split(':');
    if (parts[0] === 'find') {
      const mode = parts[1];
      const dim = parts[2];
      if (mode === 'plan') return { findings: (o.planFindings || {})[dim] || [] };
      // Code mode: the `ac` dimension returns { ac, findings }.
      if (dim === 'ac') {
        codeRound++;
        const round = Math.floor(codeRound);
        const seeds = (o.codeFindingsByRound || [])[round] || {};
        return { ac: o.acTable || [], findings: seeds.ac || [] };
      }
      const round = Math.max(0, Math.floor(codeRound));
      const seeds = (o.codeFindingsByRound || [])[round] || {};
      return { findings: seeds[dim] || [] };
    }
    if (parts[0] === 'refute') return { refuted: false, confidence: 95 };
    throw new Error('unexpected agent label: ' + label);
  };
  return { agent, calls, labels: () => calls.map((c) => c.label) };
}

const nolog = () => {};
const count = (a, label) => a.calls.filter((c) => c.label === label).length;

// ============================================================================
// (a)/(b)/(c) phaseMeta hoist, fallback, and all-or-nothing rejection.
// ============================================================================
{
  const a = makeAgent({});
  const out = await run({ roadmap: 'rm', phase: '1', phaseMeta: PHASE_META }, a.agent, refPipeline, refParallel, nolog);
  assert.equal(count(a, 'fetch:phase-meta'), 0, '(a) complete phaseMeta -> no fetch:phase-meta agent call');
  assert.ok(a.calls.some((c) => c.label === 'plan:author' && c.prompt.includes('PHASE BODY TEXT')), '(a) the hoisted body reaches the planner');
  assert.ok(a.calls.some((c) => c.label === 'plan:author' && c.opts.model === 'm-plan'), '(a) the hoisted model ids are used');
  assert.equal(out.outcome, 'reviewed');
}
{
  const a = makeAgent({});
  await run({ roadmap: 'rm', phase: '1' }, a.agent, refPipeline, refParallel, nolog);
  assert.equal(count(a, 'fetch:phase-meta'), 1, '(b) phaseMeta absent -> exactly one fetch:phase-meta agent call');
}
for (const [name, bad] of [
  ['null', null],
  ['wrong type', 'phase-1-x'],
  ['empty body', { ...PHASE_META, body: '' }],
  ['no models', { ...PHASE_META, models: undefined }],
  ['missing one model id', { ...PHASE_META, models: { ...MODELS, mechanical: '' } }],
  ['four of five model ids', { ...PHASE_META, models: { plan: 'a', implement: 'b', review_find: 'c', review_verify: 'd' } }],
  // The difficulty TIER is part of the completeness set in phase mode. It has no
  // recoverable fallback other than a hard-coded 'medium', and that default
  // LOOSENS the code gate on a `large` phase (hasBlocking stops treating a
  // surviving `concern` as blocking) — so a tier-less payload must be rejected,
  // not silently downgraded.
  ['missing model tier', (() => { const m = { ...PHASE_META }; delete m.model; return m; })()],
  ['empty model tier', { ...PHASE_META, model: '' }],
  ['blank model tier', { ...PHASE_META, model: '   ' }],
  ['non-string model tier', { ...PHASE_META, model: 3 }],
]) {
  const a = makeAgent({});
  await run({ roadmap: 'rm', phase: '1', phaseMeta: bad }, a.agent, refPipeline, refParallel, nolog);
  assert.equal(count(a, 'fetch:phase-meta'), 1, '(c) incomplete phaseMeta (' + name + ') is REJECTED -> the fetch agent runs');
}
// Positive counterpart, and the reason the tier is in the completeness set at
// all: a hoisted `large` tier must reach the gate AS `large`, not be flattened
// to the 'medium' default. hasBlocking() treats a surviving concern-severity
// finding as blocking at `large` and NOT at `medium`, so with one identical
// concern seed the two tiers must produce DIFFERENT outcomes. If the tier were
// silently lost, both runs would land on the medium (looser) branch — which is
// exactly the silent gate-weakening this guard exists to prevent.
{
  // Seeded on EVERY round so a rework round cannot launder the finding away —
  // the tier, not the rework budget, has to be what separates the two runs.
  const concern = { correctness: [{ id: 'k1', concern: 'correctness', severity: 'concern', confidence: 95, what_fails: 'x' }] };
  const concernSeed = [concern, concern, concern, concern];
  const big = makeAgent({ codeFindingsByRound: concernSeed });
  const outBig = await run({ roadmap: 'rm', phase: '1', phaseMeta: { ...PHASE_META, model: 'large' } }, big.agent, refPipeline, refParallel, nolog);
  assert.equal(count(big, 'fetch:phase-meta'), 0, '(c+) a complete large-tier phaseMeta is accepted');
  assert.notEqual(outBig.outcome, 'reviewed', '(c+) at large tier a surviving concern finding is blocking');
  const mid = makeAgent({ codeFindingsByRound: concernSeed });
  const outMid = await run({ roadmap: 'rm', phase: '1', phaseMeta: { ...PHASE_META, model: 'medium' } }, mid.agent, refPipeline, refParallel, nolog);
  assert.equal(outMid.outcome, 'reviewed', '(c+) at medium tier the same concern finding is NOT blocking');
  assert.notEqual(outBig.outcome, outMid.outcome, '(c+) the hoisted tier demonstrably changes gate strictness, so losing it is not cosmetic');
}
console.log('6a OK: phaseMeta hoist / fallback / all-or-nothing rejection (incl. difficulty tier)');

// ============================================================================
// (d) the same trio for taskMeta / fetch:task-meta.
// ============================================================================
{
  const a = makeAgent({});
  await run({ task: 'my-task', taskMeta: TASK_META }, a.agent, refPipeline, refParallel, nolog);
  assert.equal(count(a, 'fetch:task-meta'), 0, '(d) complete taskMeta -> no fetch:task-meta agent call');
  assert.ok(a.calls.some((c) => c.label === 'plan:author' && c.prompt.includes('TASK BODY TEXT')), '(d) the hoisted task body reaches the planner');
}
{
  const a = makeAgent({});
  await run({ task: 'my-task' }, a.agent, refPipeline, refParallel, nolog);
  assert.equal(count(a, 'fetch:task-meta'), 1, '(d) taskMeta absent -> exactly one fetch:task-meta agent call');
}
{
  const a = makeAgent({});
  await run({ task: 'my-task', taskMeta: { ...TASK_META, models: { ...MODELS, plan: '' } } }, a.agent, refPipeline, refParallel, nolog);
  assert.equal(count(a, 'fetch:task-meta'), 1, '(d) incomplete taskMeta is REJECTED -> the fetch agent runs');
  // A phase-mode hoist must never satisfy task mode and vice versa.
  const b = makeAgent({});
  await run({ task: 'my-task', phaseMeta: PHASE_META }, b.agent, refPipeline, refParallel, nolog);
  assert.equal(count(b, 'fetch:task-meta'), 1, '(d) a phaseMeta payload never satisfies task mode');
}
console.log('6b OK: taskMeta hoist / fallback / rejection, and mode isolation');

// ============================================================================
// (e)/(f) alreadyInProgress suppresses the stamp; --plan-only still suppresses
// it regardless, and the two guards are independent.
// ============================================================================
{
  const a = makeAgent({});
  await run({ roadmap: 'rm', phase: '1', phaseMeta: PHASE_META, alreadyInProgress: true }, a.agent, refPipeline, refParallel, nolog);
  assert.equal(count(a, 'stamp:in-progress'), 0, '(e) alreadyInProgress: true -> zero stamp:in-progress calls');
}
{
  const a = makeAgent({});
  await run({ roadmap: 'rm', phase: '1', phaseMeta: PHASE_META }, a.agent, refPipeline, refParallel, nolog);
  assert.equal(count(a, 'stamp:in-progress'), 1, '(e) alreadyInProgress absent -> exactly one stamp:in-progress call');
}
for (const flag of [true, false, undefined]) {
  const a = makeAgent({});
  const args = { roadmap: 'rm', phase: '1', phaseMeta: PHASE_META, planOnly: true };
  if (flag !== undefined) args.alreadyInProgress = flag;
  await run(args, a.agent, refPipeline, refParallel, nolog);
  assert.equal(count(a, 'stamp:in-progress'), 0, '(f) planOnly -> zero stamps whatever alreadyInProgress is (' + String(flag) + ')');
}
console.log('6c OK: alreadyInProgress suppression, and the --plan-only guard is independent of it');

// ============================================================================
// (g)/(h)/(i) diff ABSORPTION into the implementer, its fallback, and per-round
// freshness (a one-shot pendingDiff — round 2 never inherits round 1's diff).
// ============================================================================
const ABSORBED = { changedFiles: ['src/api/index.ts'], diffText: '+export function foo() {}' };
{
  const a = makeAgent({ implementResult: ABSORBED });
  await run({ roadmap: 'rm', phase: '1', phaseMeta: PHASE_META }, a.agent, refPipeline, refParallel, nolog);
  assert.equal(count(a, 'diff:signals'), 0, '(g) an implementer-returned diff -> zero diff:signals calls');
  const dims = a.calls.filter((c) => c.label.startsWith('find:code:')).map((c) => c.label).sort();
  assert.ok(dims.includes('find:code:api-docs'), '(g) the absorbed diff threads deriveSignals output (the added export triggers api-docs)');
  assert.ok(!dims.includes('find:code:changelog'), '(g) a non-triggered dimension stays off — signals really were computed');
  // The implementer prompt must carry the same commands and truncation the
  // diff:signals prompt uses, or deriveSignals sees different input.
  const impl = a.calls.find((c) => c.label === 'implement:worktree');
  assert.ok(impl.prompt.includes('git diff --name-only main...HEAD'), '(g) implementer prompt asks for the same three-dot name-only diff');
  assert.ok(impl.prompt.includes('git diff main...HEAD'), '(g) implementer prompt asks for the same three-dot full diff');
  assert.ok(impl.prompt.includes('40000'), '(g) implementer prompt uses the same 40000-char truncation');
  assert.equal(impl.opts.model, 'm-impl', '(g) the implementer keeps its own model — absorption never re-tiers it');
}
for (const [name, r] of [
  ['nothing', undefined],
  ['null', null],
  ['empty changedFiles', { changedFiles: [], diffText: '' }],
  ['no changedFiles key', { diffText: 'x' }],
]) {
  const a = makeAgent({ implementResult: r });
  await run({ roadmap: 'rm', phase: '1', phaseMeta: PHASE_META }, a.agent, refPipeline, refParallel, nolog);
  assert.equal(count(a, 'diff:signals'), 1, '(h) implementer returned ' + name + ' -> exactly one diff:signals call for that round');
}
{
  // (i) Two-round rework: each round's signals come from THAT round's own
  // implementer return. Round 1 ADDS AN EXPORT (api-docs on via
  // publicApiChanged, changelog off); round 2 adds user-visible OUTPUT instead
  // (changelog on via userFacing, api-docs off). Both are content-derived — the
  // round-2 diff text is load-bearing, since an empty-but-non-null diffText is
  // "read, nothing matched" and would be a confident false.
  const blocking = { ac: [{ id: 'b1', concern: 'ac', severity: 'blocking', confidence: 95, what_fails: 'x' }] };
  const a = makeAgent({
    implementResult: [ABSORBED, { changedFiles: ['src/cli/main.ts'], diffText: '+  console.log("done");\n' }],
    codeFindingsByRound: [blocking, {}],
  });
  await run({ roadmap: 'rm', phase: '1', phaseMeta: PHASE_META, maxCodeRework: 1 }, a.agent, refPipeline, refParallel, nolog);
  assert.equal(count(a, 'diff:signals'), 0, '(i) both rounds absorbed -> no diff:signals call at all');
  assert.equal(count(a, 'implement:rework'), 1, '(i) exactly one rework round ran');
  const findLabels = a.calls.filter((c) => c.label.startsWith('find:code:')).map((c) => c.label);
  const changelogRuns = findLabels.filter((l) => l === 'find:code:changelog').length;
  assert.equal(changelogRuns, 1, '(i) changelog fires in round 2 ONLY — round 1 did not see the round-2 file (no stale diff)');
  const apiDocRuns = findLabels.filter((l) => l === 'find:code:api-docs').length;
  assert.equal(apiDocRuns, 1, '(i) api-docs fires in round 1 ONLY — round 2 did not inherit round 1 diff (pendingDiff is one-shot)');
}
{
  // (i2) The direct staleness probe: round 1 absorbs a diff, round 2's
  // implementer returns NOTHING. Round 2 must fall back to the diff:signals
  // agent rather than silently reusing round 1's diff.
  const blocking = { ac: [{ id: 'b1', concern: 'ac', severity: 'blocking', confidence: 95, what_fails: 'x' }] };
  const a = makeAgent({ implementResult: [ABSORBED, null], codeFindingsByRound: [blocking, {}] });
  await run({ roadmap: 'rm', phase: '1', phaseMeta: PHASE_META, maxCodeRework: 1 }, a.agent, refPipeline, refParallel, nolog);
  assert.equal(count(a, 'implement:rework'), 1, '(i2) exactly one rework round ran');
  assert.equal(count(a, 'diff:signals'), 1, '(i2) round 2 returned no diff -> exactly one diff:signals call, never round 1 stale reuse');
  const findLabels = a.calls.filter((c) => c.label.startsWith('find:code:')).map((c) => c.label);
  assert.equal(findLabels.filter((l) => l === 'find:code:api-docs').length, 1, '(i2) api-docs fires only in round 1 — round 2 used the fallback diff');
}
console.log('6d OK: diff absorbed from the implementer, falls back per round, and is one-shot across rework rounds');

// ============================================================================
// (AC9) Outcome equivalence + the in-progress observability invariant.
// ============================================================================
const BLOCKING_PLAN = { coherence: [{ id: 'p1', concern: 'coherence', severity: 'blocking', confidence: 95, what_fails: 'vague' }] };
const BLOCKING_CODE = [{ ac: [{ id: 'c1', concern: 'ac', severity: 'blocking', confidence: 95, what_fails: 'x' }] }];
const SEEDS = [
  ['clean review', {}, {}],
  ['blocking code finding', { codeFindingsByRound: BLOCKING_CODE }, {}],
  ['AC-only gap', { acTable: [{ criterion: 'AC1', status: 'FAIL', evidence: 'e' }] }, {}],
  ['plan escalation', { planFindings: BLOCKING_PLAN }, {}],
  ['fetch failure', { fetchResult: null }, { noMetaHoist: true }],
  ['plan-only', {}, { extra: { planOnly: true } }],
];
for (const [name, seed, cfg] of SEEDS) {
  const base = { roadmap: 'rm', phase: '1', ...(cfg.extra || {}) };
  const a = makeAgent(seed);
  const outPlain = await run(base, a.agent, refPipeline, refParallel, nolog);
  const b = makeAgent(seed);
  const hoisted = { ...base, alreadyInProgress: true };
  if (!cfg.noMetaHoist) hoisted.phaseMeta = PHASE_META;
  const outHoisted = await run(hoisted, b.agent, refPipeline, refParallel, nolog);
  assert.deepEqual(outHoisted, outPlain, 'AC9: OUTCOME deep-equal with and without the hoists (' + name + ')');
}
{
  // Observability: on a plan-ESCALATION run with no caller stamp, the item is
  // still stamped in-progress BEFORE the planner runs — the exact gap that
  // forbids folding the stamp into the implementer, which never runs here.
  const a = makeAgent({ planFindings: BLOCKING_PLAN });
  const out = await run({ roadmap: 'rm', phase: '1', phaseMeta: PHASE_META }, a.agent, refPipeline, refParallel, nolog);
  assert.equal(out.outcome, 'escalated', 'AC9: the blocking plan finding really did escalate');
  assert.equal(count(a, 'implement:worktree') + count(a, 'implement:rework'), 0, 'AC9: no implementer runs on the escalation path');
  const labels = a.labels();
  const stampAt = labels.indexOf('stamp:in-progress');
  const planAt = labels.indexOf('plan:author');
  assert.ok(stampAt >= 0, 'AC9: the escalation path still stamps in-progress');
  assert.ok(stampAt < planAt, 'AC9: the stamp precedes the first plan:author call');
}
console.log('6e OK: outcome equivalence across six seeds, and in-progress precedes the plan gate even on an escalation');


// ============================================================================
// (6g) roadmapBody -> extractIntent -> the intent-alignment finder prompt.
//
// The intent-alignment dimension is only as live as the ONE field that feeds
// it. Everything upstream of `phaseMeta.roadmapBody` is prose (the Stage-0
// prompt, and the three CLI shims' hoist procedures) and everything downstream
// is covered by verify-workflow-review.sh; this section drives the REAL driver
// end to end and proves the recorded intent actually reaches a finder — for
// BOTH paths that can produce the field:
//
//   (i)  the caller HOIST, assembled exactly as the rdm-autopilot / rdm-do
//        --auto / rdm-dispatch-phase SKILL.md procedures document it. This is
//        the primary autonomous path and the one that skips Stage 0 entirely,
//        so a shim that forgets `roadmapBody` leaves the dimension inert while
//        the gate still reports green.
//   (ii) the in-workflow Stage-0 fetch agent's return value, which must survive
//        PHASE_META_SCHEMA and reach the same place.
//
// and the documented degradation:
//
//   (iii) a hoist WITHOUT `roadmapBody` (an older caller) — no intent reaches
//         the finder, and the run still completes normally rather than blocking.
// ============================================================================
{
  const GOAL_SENTENCE = 'downstream consumers can install and run the lane without hand-wiring anything';
  const ROADMAP_BODY = [
    'Some roadmap preamble that is not the intent.',
    '',
    '## Intent',
    '',
    '**Goal.** ' + GOAL_SENTENCE,
    '**Non-goals.** Rewriting the emitters.',
    '**Done looks like.** An operator installs the plugin and dispatches a phase with no manual edits.',
    '',
    '## Phases',
    '',
    'not intent',
  ].join('\n');

  // Exactly the object the three CLI shims' prose assembles, field for field.
  const SKILL_SHAPED_META = {
    roadmap: 'rm',
    phase: '1',
    stem: 'phase-1-x',
    model: 'medium',
    body: 'PHASE BODY TEXT',
    roadmapBody: ROADMAP_BODY,
    verify: 'sh scripts/verify-all.sh',
    models: MODELS,
  };

  const intentPrompt = (a) => {
    const c = a.calls.find((x) => x.label === 'find:plan:intent-alignment');
    return c ? c.prompt : null;
  };

  // (i) The caller hoist.
  {
    const a = makeAgent({});
    await run({ roadmap: 'rm', phase: '1', phaseMeta: SKILL_SHAPED_META }, a.agent, refPipeline, refParallel, nolog);
    assert.equal(
      count(a, 'fetch:phase-meta'),
      0,
      '(6g-i) the skill-shaped phaseMeta (now carrying roadmapBody) is still accepted by hoistedMetaComplete'
    );
    const p = intentPrompt(a);
    assert.ok(p, '(6g-i) the intent-alignment finder is dispatched on the plan gate');
    assert.ok(
      p.includes(GOAL_SENTENCE),
      '(6g-i) the hoisted roadmapBody recorded Goal reaches the intent-alignment finder verbatim'
    );
    assert.ok(
      p.includes('**Done looks like.**'),
      '(6g-i) the whole recorded ## Intent section is threaded, not just the Goal line'
    );
  }

  // (ii) The in-workflow Stage-0 fetch.
  {
    const a = makeAgent({ fetchResult: { ...PHASE_META, roadmapBody: ROADMAP_BODY } });
    await run({ roadmap: 'rm', phase: '1' }, a.agent, refPipeline, refParallel, nolog);
    assert.equal(count(a, 'fetch:phase-meta'), 1, '(6g-ii) with no hoist, Stage 0 runs');
    const p = intentPrompt(a);
    assert.ok(p, '(6g-ii) the intent-alignment finder is dispatched');
    assert.ok(
      p.includes(GOAL_SENTENCE),
      '(6g-ii) a roadmapBody returned by the Stage-0 fetch agent survives PHASE_META_SCHEMA and reaches the finder'
    );
  }

  // (iii) The documented degradation: an older caller hoist with no roadmapBody.
  // No intent reaches the finder, and — critically — the run still completes.
  {
    const a = makeAgent({});
    const out = await run({ roadmap: 'rm', phase: '1', phaseMeta: PHASE_META }, a.agent, refPipeline, refParallel, nolog);
    assert.equal(count(a, 'fetch:phase-meta'), 0, '(6g-iii) a roadmapBody-less hoist is still accepted');
    const p = intentPrompt(a);
    assert.ok(p, '(6g-iii) selection still fail-opens and the finder still runs');
    assert.equal(
      p.includes(GOAL_SENTENCE),
      false,
      '(6g-iii) with no roadmapBody, no recorded intent is threaded'
    );
    assert.equal(out.outcome, 'reviewed', '(6g-iii) a missing intent never blocks the dispatch');
  }
}
console.log('6g OK: roadmapBody reaches the intent-alignment finder via BOTH the caller hoist and the Stage-0 fetch, and degrades quietly when absent');

// ============================================================================
// (6h) The DRIVER's own unresolved-verify short-circuit, driven end to end.
//
// Section 1d unit-tests buildOutcome/buildTaskOutcome({ verifyUnresolved: true })
// directly, which proves the OUTCOME shape but bypasses the driver entirely —
// nothing there exercises the driver's own
// `const verifyCommand = extractVerifyCommand(phaseMeta)` /
// `if (!planOnly && verifyCommand === '')` wiring, its placement BEFORE the
// in-progress stamp, or its --plan-only carve-out. Every other driven run in
// this file feeds a fixture whose `verify` field is populated, so an inverted
// condition, a relocated check, or a lookup typo would go unnoticed. This
// section closes that gap; 6f (7)–(10) plant the mutations that prove it.
//
// NOTE the fixture route: `hoistedMetaComplete` REQUIRES a non-empty verify
// (an incomplete hoist falls back to the fetch agent by design), so the only
// way an empty command can reach the driver is through the Stage-0 fetch —
// which is exactly how it reaches it in production too.
// ============================================================================
{
  const DISCOVERY_SOURCES = ['dispatch.verify', '.github/workflows/', 'docs/principles.md', 'CLAUDE.md'];
  const noVerifyPhase = (() => { const m = { ...PHASE_META }; delete m.verify; return m; })();
  const noVerifyTask = (() => { const m = { ...TASK_META }; delete m.verify; return m; })();

  // (i) Phase mode, three ways of saying "nothing resolved": the key absent,
  //     an empty string, and a whitespace-only string. All three escalate.
  for (const [name, meta] of [
    ['absent', noVerifyPhase],
    ['empty string', { ...PHASE_META, verify: '' }],
    ['whitespace only', { ...PHASE_META, verify: '   \t ' }],
    ['non-string', { ...PHASE_META, verify: 42 }],
  ]) {
    const a = makeAgent({ fetchResult: meta });
    const out = await run({ roadmap: 'rm', phase: '1' }, a.agent, refPipeline, refParallel, nolog);
    assert.equal(out.outcome, 'escalated', '(6h-i) an unresolvable verify command escalates (' + name + ')');
    assert.equal(out.status, 'blocked', '(6h-i) ... and parks the phase blocked (' + name + ')');
    assert.equal(out.writesCompletion, false, '(6h-i) ... and never authorizes a completion trailer (' + name + ')');
    // The escalation must beat the stamp, the planner and every implementer —
    // the driver comment claims "before the in-progress stamp, before planning,
    // before any implementer", and this is what holds it to that.
    assert.equal(count(a, 'stamp:in-progress'), 0, '(6h-i) ... short-circuits BEFORE the in-progress stamp (' + name + ')');
    assert.equal(count(a, 'plan:author'), 0, '(6h-i) ... before the planner (' + name + ')');
    assert.equal(count(a, 'implement:worktree') + count(a, 'implement:rework'), 0, '(6h-i) ... and before any implementer (' + name + ')');
    assert.equal(count(a, 'verify:run'), 0, '(6h-i) ... and never runs a verify agent with no command (' + name + ')');
    assert.equal(count(a, 'fetch:phase-meta'), 1, '(6h-i) ... after Stage 0 actually ran (' + name + ')');
    for (const src of DISCOVERY_SOURCES) {
      assert.ok(out.summary.includes(src), '(6h-i) the summary names ' + src + ' (' + name + ')');
      assert.ok(out.reason.includes(src), '(6h-i) the reason names ' + src + ' (' + name + ')');
    }
  }

  // (ii) The CONTROL: the identical run with a real command resolves normally.
  //      Without this, (i) could be passing for any unrelated reason.
  {
    const a = makeAgent({ fetchResult: PHASE_META });
    const out = await run({ roadmap: 'rm', phase: '1' }, a.agent, refPipeline, refParallel, nolog);
    assert.equal(out.outcome, 'reviewed', '(6h-ii) a resolvable verify command does NOT escalate');
    assert.equal(count(a, 'stamp:in-progress'), 1, '(6h-ii) ... and the stamp the escalation skipped does run here');
    assert.equal(count(a, 'verify:run'), 1, '(6h-ii) ... and the verify agent runs exactly once');
  }

  // (iii) Task mode reaches the same short-circuit through buildTaskOutcome.
  {
    const a = makeAgent({ fetchResult: noVerifyTask });
    const out = await run({ task: 'my-task' }, a.agent, refPipeline, refParallel, nolog);
    assert.equal(out.outcome, 'escalated', '(6h-iii) task mode escalates on an unresolvable verify command');
    assert.equal(out.status, 'blocked', '(6h-iii) ... parking the task blocked');
    assert.equal(out.task, 'my-task', '(6h-iii) ... with the task identifier, not a phase one');
    assert.equal(count(a, 'stamp:in-progress') + count(a, 'plan:author'), 0, '(6h-iii) ... before the stamp and the planner');
  }

  // (iv) The --plan-only carve-out: a plan-only pass does no implementation, so
  //      the SAME unresolvable command must NOT escalate. This is the single
  //      `!planOnly` point of control, asserted directly rather than by grep.
  for (const [name, meta, args] of [
    ['phase', noVerifyPhase, { roadmap: 'rm', phase: '1', planOnly: true }],
    ['task', noVerifyTask, { task: 'my-task', planOnly: true }],
  ]) {
    const a = makeAgent({ fetchResult: meta });
    const out = await run(args, a.agent, refPipeline, refParallel, nolog);
    assert.equal(out.outcome, 'reviewed', '(6h-iv) --plan-only with no resolvable verify command does NOT escalate (' + name + ')');
    assert.ok(out.summary.startsWith('plan-only:'), '(6h-iv) ... it returns the ordinary plan-only OUTCOME (' + name + ')');
    assert.ok(count(a, 'plan:author') >= 1, '(6h-iv) ... and the plan gate really ran (' + name + ')');
    assert.equal(count(a, 'implement:worktree') + count(a, 'verify:run'), 0, '(6h-iv) ... while still implementing and verifying nothing (' + name + ')');
  }
}
console.log('6h OK: the driver escalates on an unresolvable verify command before the stamp/planner/implementer, task mode included, and --plan-only is carved out');

// ============================================================================
// (6i) A `config get`-ANNOTATED value survives the whole driver as the bare
//      command. The Stage-0 prompt asks for `--raw`, but the value crosses an
//      LLM: if a fetch agent ever returns the annotated line
//      `<cmd>  (source: repo config)`, the verify agent must be handed `<cmd>`
//      — running the whole line is a bash syntax error, and every declared-key
//      dispatch would then fail and blame the project's own command.
//
//      Asserted END TO END rather than on the helper alone, because the helper
//      unit test (1d(a)) cannot show that the driver routes the fetched value
//      through it before building the verify prompt.
// ============================================================================
{
  const CLEAN = 'sh scripts/verify-all.sh';
  for (const [name, meta, args, label] of [
    ['phase', PHASE_META, { roadmap: 'rm', phase: '1' }, 'fetch:phase-meta'],
    ['task', TASK_META, { task: 'my-task' }, 'fetch:task-meta'],
  ]) {
    const annotated = { ...meta, verify: CLEAN + '  (source: repo config)' };
    const a = makeAgent({ fetchResult: annotated });
    const out = await run(args, a.agent, refPipeline, refParallel, nolog);
    assert.equal(count(a, label), 1, '(6i) Stage 0 fetched the metadata (' + name + ')');
    const verifyCalls = a.calls.filter((c) => c.label === 'verify:run');
    assert.equal(verifyCalls.length, 1, '(6i) the verify agent ran exactly once (' + name + ')');
    assert.ok(verifyCalls[0].prompt.includes(CLEAN), '(6i) the verify prompt carries the bare command (' + name + ')');
    assert.ok(!verifyCalls[0].prompt.includes('(source:'), '(6i) ... and NOT the `(source: ...)` annotation (' + name + ')');
    const impl = a.calls.filter((c) => c.label === 'implement:worktree');
    assert.ok(impl.length >= 1 && !impl[0].prompt.includes('(source:'), '(6i) the implementer tooling line is unannotated too (' + name + ')');
    assert.equal(out.outcome, 'reviewed', '(6i) and the annotated value still resolves to a runnable command (' + name + ')');
  }
}
console.log('6i OK: a `config get`-annotated verify value reaches the verify agent as the bare command');
console.log('ALL HOIST/ABSORB CHECKS PASSED');
NODE_HOIST

if run_node "$TMP/hoist.mjs" "$WF"; then
    pass "hoist/absorb: real driver verified under a recording fake agent"
else
    fail "hoist/absorb checks failed against $WF"
fi

# --- 6f. Planted-mutation self-tests ------------------------------------------
# Each assertion above is only load-bearing if the corresponding production
# mutation makes it fail. Every mutant must break the run; each is discarded
# afterwards (the real file is never modified).
say "6f. Hoist/absorb planted-mutation self-tests"

assert_mutant_fails() {
    mutant=$1
    desc=$2
    if cmp -s "$WF" "$mutant"; then
        fail "6f: planted mutation was a no-op — $desc"
    fi
    if run_node "$TMP/hoist.mjs" "$mutant" >/dev/null 2>&1; then
        fail "6f: hoist/absorb checks PASSED against a mutant that $desc — the assertions are vacuous"
    fi
    pass "6f: assertions fire when the production code $desc"
}

# (1) Delete the meta fetch fallback: make the hoist unconditional.
sed 's/^if (hoistedMetaComplete(hoistedMeta, isTask)) {$/if (true) { phaseMeta = hoistedMeta;/' "$WF" >"$TMP/mutant-no-meta-fallback.js"
assert_mutant_fails "$TMP/mutant-no-meta-fallback.js" "drops the fetch:phase-meta fallback branch"

# (2) Weaken the all-or-nothing guard to "any object".
sed 's/^if (hoistedMetaComplete(hoistedMeta, isTask)) {$/if (hoistedMeta \&\& typeof hoistedMeta === "object") {/' "$WF" >"$TMP/mutant-weak-guard.js"
assert_mutant_fails "$TMP/mutant-weak-guard.js" "accepts an incomplete hoisted meta (guard weakened to any object)"

# (2b) Drop the difficulty-tier requirement from the phase-mode guard, so a
#      tier-less payload is accepted and silently flattened to 'medium'. This is
#      the one weakening the shape-only guard would not have caught: the payload
#      is still schema-valid and still carries every model id.
awk 'index($0, "// Phase mode only: the difficulty tier has no recoverable fallback.") { skip = 1; print; next }
     skip { skip = 0; next }
     { print }' "$WF" >"$TMP/mutant-no-tier-guard.js"
if cmp -s "$WF" "$TMP/mutant-no-tier-guard.js"; then
    fail "6f: planted mutation was a no-op — drops the phase-mode difficulty-tier requirement"
fi
assert_mutant_fails "$TMP/mutant-no-tier-guard.js" "drops the phase-mode difficulty-tier requirement (a tier-less hoist silently loosens the code gate)"

# (3) Break the one-shot property, so round 2 inherits round 1's diff.
#     `pendingDiff` is cleared at TWO sites by design (defence in depth): the
#     implement dep's `: null` branch on an unusable return, and the review
#     closure's read-and-clear. Either alone is sufficient, so a mutation that
#     removes only one is provably a no-op — the self-test must remove BOTH to
#     actually produce a stale diff, and that is exactly what assertion (i2)
#     catches.
sed -e 's/^      pendingDiff = null$//' \
    -e 's/^      pendingDiff = r \&\& Array.isArray(r.changedFiles) \&\& r.changedFiles.length > 0 ? r : null$/      if (r \&\& Array.isArray(r.changedFiles) \&\& r.changedFiles.length > 0) pendingDiff = r/' \
    "$WF" >"$TMP/mutant-stale-diff.js"
assert_mutant_fails "$TMP/mutant-stale-diff.js" "breaks the one-shot pendingDiff contract at both clearing sites (stale round-1 diff leaks into round 2)"

# (4) Hard-code alreadyInProgress true, so the stamp never runs.
sed 's/^const alreadyInProgress = dispatchArgs.alreadyInProgress$/const alreadyInProgress = true/' "$WF" >"$TMP/mutant-always-stamped.js"
assert_mutant_fails "$TMP/mutant-always-stamped.js" "hard-codes alreadyInProgress to true (the stamp never runs)"

# (5) Drop the diff:signals fallback, so an implementer that returns nothing
#     leaves the review with no signals at all.
awk '
    index($0, "const diffFromImplementer = pendingDiff") { print; next }
    index($0, "      if (diffFromImplementer) {") { print "      if (true) {"; next }
    { print }
' "$WF" >"$TMP/mutant-no-diff-fallback.js"
assert_mutant_fails "$TMP/mutant-no-diff-fallback.js" "drops the diff:signals fallback branch"

# (6) Sever the roadmapBody -> intent thread, so a hoisted (or fetched)
#     roadmap body never reaches the plan gate. This is the exact silent
#     failure mode that made the dimension inert on the primary autonomous
#     path: nothing errors, the gate still reports green, and only 6g's
#     prompt-content assertion notices.
sed 's/const planIntent = extractIntent(phaseMeta.roadmapBody)/const planIntent = extractIntent(undefined)/' "$WF" >"$TMP/mutant-no-intent-thread.js"
assert_mutant_fails "$TMP/mutant-no-intent-thread.js" "severs the roadmapBody -> extractIntent -> plan-gate thread (intent-alignment goes silently inert)"

# (7)-(10) The DRIVER's unresolved-verify short-circuit (6h). Four independent
# ways to break it, each of which must red-light a different 6h assertion.

# (7) Delete the guard outright, so an unresolvable verify command is silently
#     skipped and the dispatch proceeds to report a pass it never verified.
sed "s/^if (!planOnly && verifyCommand === '') {\$/if (false) {/" "$WF" >"$TMP/mutant-no-verify-escalation.js"
assert_mutant_fails "$TMP/mutant-no-verify-escalation.js" "deletes the unresolved-verify escalation guard (an unverifiable dispatch reports success)"

# (8) Invert the --plan-only carve-out, so a plan-only pass escalates and a real
#     implementation run does not — the exact single-point-of-control inversion
#     6h-i and 6h-iv jointly pin.
sed "s/^if (!planOnly && verifyCommand === '') {\$/if (planOnly \&\& verifyCommand === '') {/" "$WF" >"$TMP/mutant-inverted-plan-only.js"
assert_mutant_fails "$TMP/mutant-inverted-plan-only.js" "inverts the --plan-only condition on the unresolved-verify guard"

# (9) Turn the metadata lookup into a no-op that always yields a command, the
#     shape a `phaseMeta.verify` typo would take: nothing throws, nothing logs,
#     and the gate silently stops being able to fire.
sed 's/^const verifyCommand = extractVerifyCommand(phaseMeta)$/const verifyCommand = "sh nope.sh"/' "$WF" >"$TMP/mutant-verify-lookup-noop.js"
assert_mutant_fails "$TMP/mutant-verify-lookup-noop.js" "makes the verify-command lookup a constant (the unresolved case can never be detected)"

# (10) MOVE the check below the in-progress stamp rather than deleting it. The
#      outcome is still `escalated`, so only 6h-i's ORDERING assertions
#      (stamp:in-progress === 0) can catch this — which is precisely why they
#      are asserted on call counts rather than left as a source comment.
#      Scoped past `dispatch-outcome:end` and to the FIRST fence after it: the
#      file carries three `verify-gate` fences (one in the copied block, the
#      Stage-0 short-circuit, and the verify dep binding), and relocating the
#      wrong one would delete helpers and fail for an unrelated reason.
awk '
    index($0, ">>> dispatch-outcome:end <<<") { past = 1; print; next }
    past && !moved && index($0, ">>> verify-gate:begin <<<") { buf = $0 "\n"; grab = 1; next }
    grab { buf = buf $0 "\n"; if (index($0, ">>> verify-gate:end <<<")) { grab = 0; moved = 1 } next }
    moved && buf != "" && index($0, "// Stages A + B: author the plan from ONLY the phase body") { printf "%s", buf; buf = ""; print; next }
    { print }
' "$WF" >"$TMP/mutant-verify-after-stamp.js"
# The relocation must preserve every line — a mutant that DELETES the region
# would fail 6h for the wrong reason (missing behavior, not wrong ordering).
if [ "$(grep -c '' "$WF")" != "$(grep -c '' "$TMP/mutant-verify-after-stamp.js")" ]; then
    fail "6f(10): the relocation mutant changed the line count — it deleted code instead of moving it"
fi
assert_mutant_fails "$TMP/mutant-verify-after-stamp.js" "moves the unresolved-verify check BELOW the in-progress stamp (the item is stamped in-progress for a run that never starts)"

# (11) Stop stripping the `(source: ...)` annotation `rdm config get` appends
#      when it is not asked for `--raw`. The value still resolves (it is a
#      non-empty single-line string), the dispatch still runs, and the ONLY
#      observable consequence is that the verify agent is handed an unrunnable
#      line — which is exactly what 6i pins.
sed "s/^  const trimmed = v.trim().replace(CONFIG_SOURCE_SUFFIX, '').trim();\$/  const trimmed = v.trim();/" "$WF" >"$TMP/mutant-keep-source-suffix.js"
assert_mutant_fails "$TMP/mutant-keep-source-suffix.js" "stops stripping the 'config get' source annotation (the verify agent is handed an unrunnable line)"

#     (The sibling mutation — deleting roadmapBody from PHASE_META_SCHEMA — is
#     deliberately NOT self-tested here: this section drives a FAKE agent, which
#     does no schema validation, so the property is unobservable dynamically.
#     AC-PLAN-INTENT above owns that direction statically.

# --- 7. INVENTORY DOC ---------------------------------------------------------
# docs/mechanical-agent-inventory.md is phase 3's primary deliverable and phase
# 4's input. It is gated here rather than left as prose because a stale
# inventory is worse than none: phase 4 scopes its work off the "irreducible"
# section. The checks are derived LIVE from the workflow scripts, so a newly
# added mechanical label with no row fails, and a transcribed (stale) total
# fails too.
say "7. Inventory doc: docs/mechanical-agent-inventory.md is complete and live-consistent"

INV="$REPO_ROOT/docs/mechanical-agent-inventory.md"
[ -f "$INV" ] || fail "7: docs/mechanical-agent-inventory.md is missing — it is this phase's primary deliverable"

# The LIVE call-site total, re-derived exactly as the doc documents.
LIVE_TOTAL=$(grep -h "label: *['\"]" "$REPO_ROOT"/.claude/workflows/*.js |
    grep -vc 'spike-agent-type' 2>/dev/null || true)
LIVE_TOTAL=$(for f in "$REPO_ROOT"/.claude/workflows/*.js; do
    case "$f" in *spike-agent-type.js) continue ;; esac
    grep -c "label: *['\"]" "$f" || true
done | awk '{s += $1} END {print s+0}')
[ "$LIVE_TOTAL" -gt 0 ] || fail "7: live call-site grep returned 0 — the derivation itself is broken"

assert_doc_total() {
    grep -qF "**$LIVE_TOTAL labelled \`agent()\` call sites**" "$1" &&
        grep -qE "^\| \*\*total\*\* \| \*\*$LIVE_TOTAL\*\* \|" "$1"
}
assert_doc_total "$INV" ||
    fail "7: the doc's stated call-site total does not equal the LIVE grep count ($LIVE_TOTAL) — it was transcribed, not derived"
pass "7: doc's stated call-site total matches the live grep ($LIVE_TOTAL)"

# Self-test: a rewritten total must be caught.
sed "s/\*\*$LIVE_TOTAL labelled/**999 labelled/" "$INV" >"$TMP/inv-bad-total.md"
if assert_doc_total "$TMP/inv-bad-total.md"; then
    fail "7: total detector missed a rewritten call-site count"
fi
pass "7: total detector fires on a rewritten call-site count"

# Every MECHANICAL label emitted by the workflow scripts must have a row. Labels
# are read live and normalized to their static prefix (the part before any
# runtime concatenation), which is exactly how the doc names them. The judgment
# set is excluded by an EXPLICIT allowlist, not a loose regex, so a newly added
# mechanical label can never be silently classified as judgment.
# EXACT static-label allowlist for the judgment set (never a prefix match: an
# `estimate:`-prefixed rule would also swallow the mechanical estimate:list /
# estimate:write: / estimate:tier: labels).
JUDGMENT_LABELS="find: refute: plan:author plan:revise implement:rework implement:worktree act: act:code analyze: synthesize:draft estimate:rate: estimate:"

live_mechanical_labels() {
    for f in "$REPO_ROOT"/.claude/workflows/*.js; do
        case "$f" in *spike-agent-type.js) continue ;; esac
        grep -oE "label: '[^']*'" "$f" | sed "s/label: '//;s/'\$//"
    done | sort -u | while read -r lbl; do
        [ -n "$lbl" ] || continue
        skip=0
        for j in $JUDGMENT_LABELS; do
            [ "$lbl" = "$j" ] && skip=1
        done
        [ "$skip" -eq 1 ] || printf '%s\n' "$lbl"
    done
}

# assert_doc_has_every_row <doc> — the actual detector, applied to whatever doc
# path it is handed, so the self-test below drives the SAME code the real check
# does rather than re-testing sed.
assert_doc_has_every_row() {
    doc=$1
    missing=""
    for lbl in $(live_mechanical_labels); do
        grep -qF "\`$lbl" "$doc" || missing="$missing $lbl"
    done
    [ -z "$missing" ] || {
        DOC_MISSING_ROWS="$missing"
        return 1
    }
    return 0
}

DOC_MISSING_ROWS=""
assert_doc_has_every_row "$INV" ||
    fail "7: mechanical label(s) with no row in the inventory doc:$DOC_MISSING_ROWS"
pass "7: every live mechanical label has a row in the inventory doc ($(live_mechanical_labels | wc -l | tr -d ' ') static labels checked)"

# Self-test: strip every mention of one mechanical label from the doc and require
# the SAME detector to fail on it.
sed 's/fetch:report/zz-removed-label/g' "$INV" >"$TMP/inv-missing-row.md"
if assert_doc_has_every_row "$TMP/inv-missing-row.md"; then
    fail "7: row detector missed a doc with no fetch:report row — the check is vacuous"
fi
pass "7: row detector fires when a mechanical label loses its row"

# Vocabulary: each of the four classification words, each of the three
# maintenance-route words, and each of the three caller-surface values appears.
for word in hoistable absorbable redundant irreducible; do
    grep -qi "$word" "$INV" || fail "7: the doc never uses the classification word '$word'"
done
for word in stamped byte-copied unprojected; do
    grep -qi "$word" "$INV" || fail "7: the doc never names the maintenance route '$word'"
done
for word in 'distributed shim' 'local shim only' 'no caller'; do
    grep -qiF "$word" "$INV" || fail "7: the doc never records the caller surface '$word'"
done
pass "7: all four classifications, three maintenance routes, and three caller surfaces are named"

# The irreducible section must name phase 4's scope explicitly.
grep -qE '^## Irreducible \(phase 4 scope\)' "$INV" ||
    fail "7: the doc must carry an '## Irreducible (phase 4 scope)' section"
for needle in 'stamp:in-progress' 'estimate:write' 'advance:' 'park:' 'gate:clear-tag' 'gate:persist' 'act:round-note' 'write:draft' 'gather:' 'fallback path' 'convert-remaining-skill-templates-to-workflow-shims'; do
    grep -qF "$needle" "$INV" ||
        fail "7: the irreducible/phase-4 material must name '$needle'"
done
pass "7: the irreducible set names the stamp, every write-class label, the fallback paths, and the follow-up task"

# The measured delta must be recorded per class against the baseline's figures.
grep -qF 'docs/token-baseline.json' "$INV" || fail "7: the doc must reference docs/token-baseline.json"
for cls in fetch stamp model diff estimate; do
    grep -qE "^\| \`?$cls\`?" "$INV" || fail "7: no before/after row for the '$cls' agent class"
done
pass "7: a before/after agent-count row is recorded for each of the fetch/stamp/model/diff/estimate classes"

# The two recorded plan-review corruption runs must be cited by id.
for run in wf_e3402021-0af wf_f4be8027-dbb; do
    grep -qF "$run" "$INV" || fail "7: the doc must cite the recorded corruption run $run"
done
pass "7: both recorded plan-review corruption runs are cited by id"

# --- 8. MEASURED-DELTA SELF-CONSISTENCY ---------------------------------------
# scripts/measure-hoist-delta.mjs executes the real, post-change dispatch-phase
# driver under a recording fake agent and counts the mechanical subagents each
# run actually spawns, then prices them off docs/token-baseline.json. Its
# --check mode asserts those computed figures appear VERBATIM in the inventory
# doc. That mode is the only thing standing between the doc's "Direct
# measurement" section and a stale hand-transcription, so it is run here rather
# than left as a documented-but-unexecuted invocation: CI runs every
# scripts/verify-*.sh, and measure-hoist-delta.mjs is not one.
say "8. Measured delta: measure-hoist-delta.mjs --check agrees with the inventory doc"

HOIST_DELTA="$REPO_ROOT/scripts/measure-hoist-delta.mjs"
[ -f "$HOIST_DELTA" ] || fail "8: scripts/measure-hoist-delta.mjs is missing"

run_node "$HOIST_DELTA" --check "$INV" >"$TMP/hoist-check.out" 2>&1 ||
    fail "8: measure-hoist-delta.mjs --check failed against the inventory doc:
$(cat "$TMP/hoist-check.out")"
pass "8: the inventory doc carries the figures measure-hoist-delta.mjs computes from the shipped code"

# The checker must also be able to recompute independently of the doc — a
# --check that silently succeeded on any input would be worthless.
run_node "$HOIST_DELTA" --format json >"$TMP/hoist-delta.json" 2>/dev/null ||
    fail "8: measure-hoist-delta.mjs --format json did not run"
grep -q '"agentsEliminated"' "$TMP/hoist-delta.json" ||
    fail "8: measure-hoist-delta.mjs --format json emitted no agentsEliminated field"
pass "8: measure-hoist-delta.mjs recomputes the delta from the shipped code"

# Self-test A — a doc whose token figure has been rewritten must be REJECTED.
# This proves --check reads the doc's numbers rather than merely existing.
sed 's/1,536,932/1,536,933/' "$INV" >"$TMP/inv-bad-tokens.md"
if run_node "$HOIST_DELTA" --check "$TMP/inv-bad-tokens.md" >/dev/null 2>&1; then
    fail "8: --check accepted a doc with a rewritten token figure — the gate is vacuous"
fi
pass "8: --check fires on a rewritten measured-token figure"

# Self-test B — a doc that has dropped one of the measured labels must be
# REJECTED, so a future elimination that changes WHICH agents disappear cannot
# leave the doc silently listing the old set.
sed 's/diff:signals/zz-removed-label/g' "$INV" >"$TMP/inv-bad-label.md"
if run_node "$HOIST_DELTA" --check "$TMP/inv-bad-label.md" >/dev/null 2>&1; then
    fail "8: --check accepted a doc missing a measured label row — the gate is vacuous"
fi
pass "8: --check fires when the doc drops a measured label"

# ...and the unmutated doc still passes, so the two self-tests above are not
# just reporting a checker that rejects everything.
run_node "$HOIST_DELTA" --check "$INV" >/dev/null 2>&1 ||
    fail "8: --check rejects the real doc after the self-tests — the detector rejects everything"
pass "8: --check still accepts the real doc (the self-tests are discriminating, not blanket)"

# --- 9. PARAMETERIZATION ------------------------------------------------------
# dispatch-phase names NO particular rdm executable and NO particular rdm
# project: both arrive as RUNTIME args (`rdmBin`, `project`) and are threaded
# into every prompt that shells out. Five sub-gates:
#
#   9a — per-file literal zeroing across all THREE copies (lib, workflow,
#        shipped template), asserted PER FILE so a half-applied edit cannot pass.
#   9b — a DRIVEN prompt capture: run the real workflow four ways
#        (phase/task x project/no-project) under a capturing fake agent, tokenize
#        every emitted `rdm <subcommand>` occurrence, and check it against the
#        project-agnostic allow-list expressed AS DATA.
#   9c — the `rdmBin` defaulting rule (absent -> a plain `rdm`, wrong-type still
#        throws), plus a grep proving the default is a plain fallback and NOT an
#        existence preflight; 9c-inventory checks every copy in the tree carries
#        it and none reads the environment; 9c-dogfood gates the compensating
#        `RDM_BIN` control in .mise.toml and the contract doc.
#   9d — planted-mutation self-tests for 9b.
#   9e — the repo-wide nested-`workflow()` negative, comment-filtered.
say "9. Parameterization: no hardcoded rdm binary or project; the environment axes are runtime args"

TEMPLATE_WF="$REPO_ROOT/rdm-core/src/templates/workflows/rdm-wf-dispatch-phase.js"
[ -f "$TEMPLATE_WF" ] || fail "9: shipped workflow template not found: $TEMPLATE_WF"

# --- 9a. Per-file literal zeroing ---------------------------------------------
say "9a. Per-file literal zeroing (lib, workflow, shipped template)"

# assert_no_env_literals <file> — zero occurrences of THIS repo's dev binary path
# and zero of THIS repo's project flag. Deliberately per-file: a concatenated
# stream would let a zero in one copy mask a hit in another, which is exactly the
# half-applied-edit failure mode (the block is byte-copied, the driver is not).
# Comments and prose count too — a leftover explanatory comment naming either
# literal is the same staleness hazard as code.
assert_no_env_literals() {
    _f=$1
    _bin=$(grep -c 'target/debug/rdm' "$_f" || true)
    _proj=$(grep -c -- '--project rdm' "$_f" || true)
    [ "$_bin" -eq 0 ] && [ "$_proj" -eq 0 ]
}

for f in "$LIB" "$WF" "$TEMPLATE_WF"; do
    if assert_no_env_literals "$f"; then
        pass "9a: ${f#"$REPO_ROOT"/} carries neither 'target/debug/rdm' nor '--project rdm'"
    else
        grep -n 'target/debug/rdm' "$f" >&2 || true
        grep -n -- '--project rdm' "$f" >&2 || true
        fail "9a: $f still hardcodes this repo's rdm binary and/or project — both must be runtime args"
    fi
done

# Self-test: plant the literals into EACH file in turn and prove the per-file
# check fires on each one individually (so restoring only two of three cannot go
# green).
_i=0
for f in "$LIB" "$WF" "$TEMPLATE_WF"; do
    _i=$((_i + 1))
    cp "$f" "$TMP/env-mutant-$_i"
    printf '\n// planted: ./target/debug/rdm phase show --project rdm\n' >>"$TMP/env-mutant-$_i"
    if assert_no_env_literals "$TMP/env-mutant-$_i"; then
        fail "9a: the per-file literal check did not fire on a planted literal in $f — the gate is vacuous"
    fi
done
pass "9a: the per-file check fires independently on all three planted mutants"

# --- 9b. Driven prompt capture ------------------------------------------------
say "9b. Driven prompt capture: every emitted rdm invocation honors the allow-list"

cat >"$TMP/paramz.mjs" <<'NODE_PARAMZ'
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const wfPath = process.argv[2];
const src = fs.readFileSync(wfPath, 'utf8').replace(/^export /m, '');
const wrapperPath = path.join(os.tmpdir(), 'verify-workflow-dispatch-paramz-wrapped.mjs');
fs.writeFileSync(wrapperPath, 'export default async function(args, agent, pipeline, parallel, log) {\n' + src + '\n}\n');
const mod = await import('file://' + wrapperPath + '?t=' + process.pid);
const run = mod.default;

async function refParallel(thunks) {
  return Promise.all(thunks.map((t) => Promise.resolve().then(t).catch(() => null)));
}
async function refPipeline(items, ...stages) {
  return Promise.all(
    items.map(async (item, i) => {
      let acc = item;
      for (const stage of stages) {
        try {
          acc = await stage(acc, item, i);
        } catch {
          return null;
        }
      }
      return acc;
    })
  );
}

// The injected binary is deliberately NOT a plausible real path, so a
// re-hardcoded './target/debug/rdm' anywhere shows up as a mismatch rather than
// blending in.
const FAKE_BIN = '/fake/bin/rdm';

// The PROJECT-AGNOSTIC ALLOW-LIST, expressed as DATA (the step-1 contract in
// lib/dispatch-phase.mjs). These subcommands reject `--project` outright, so
// they must carry NO project flag; everything else is project-scoped and MUST
// carry it whenever a project was configured.
// `config get` is project-agnostic too: `rdm config get` takes no --project.
// `dispatch directives` likewise: it scans a SOURCE tree named by --dir and reads
// the plan repo's repo-level `dispatch.directives` key, neither of which is
// project-scoped, so the subcommand rejects --project exactly as `config get`
// does.
const PROJECT_AGNOSTIC = ['model resolve', 'config get', 'dispatch directives', 'commit', 'status', 'discard'];

const MODELS = { plan: 'm-plan', implement: 'm-impl', review_find: 'm-find', review_verify: 'm-verify', mechanical: 'm-mech' };
const PHASE_META = { roadmap: 'rm', phase: 'phase-1-x', stem: 'phase-1-x', model: 'medium', body: 'PHASE BODY TEXT', verify: 'sh scripts/verify-all.sh', models: MODELS };
const TASK_META = { task: 'my-task', body: 'TASK BODY TEXT', verify: 'sh scripts/verify-all.sh', models: MODELS };
const PLAN_DOC = {
  steps_per_ac: [{ ac: 'AC1', steps: ['do it'] }],
  file_map: [{ path: 'a.rs', change: 'edit' }],
  tests_per_ac: [{ ac: 'AC1', test: 't' }],
  edge_cases: [],
  cross_phase_deps: [],
  summary: 'plan',
};

// A capturing fake agent: records every prompt, answers every label the driver
// can emit. `act:code` is answered too, so the Act step's prompt (built inside
// the stamped block) is captured as well.
function makeCapture() {
  const prompts = [];
  const agent = async (prompt, opts) => {
    prompts.push(String(prompt));
    const label = (opts && opts.label) || '';
    if (label === 'fetch:phase-meta') return PHASE_META;
    if (label === 'fetch:task-meta') return TASK_META;
    if (label === 'stamp:in-progress') return { ok: true };
    if (label === 'verify:run') return { exitCode: 0, output: '' };
    if (label === 'clean:check') return { porcelain: '' };
    if (label === 'plan:author' || label === 'plan:revise') return PLAN_DOC;
    if (label === 'act:code') return { handled: [] };
    if (label === 'diff:signals') return { changedFiles: ['rdm-core/src/lib.rs'], diffText: '' };
    if (label === 'implement:worktree' || label === 'implement:rework') return undefined;
    const parts = label.split(':');
    if (parts[0] === 'find') {
      // Seed ONE non-gating survivor so the code gate's Act step runs and its
      // prompt (which contains a project-scoped `task create`) is captured.
      if (parts[1] === 'code' && parts[2] === 'ac') {
        return { ac: [], findings: [{ id: 'f1', severity: 'suggestion', confidence: 90, what_fails: 'x' }] };
      }
      if (parts[2] === 'ac') return { ac: [], findings: [] };
      return { findings: [] };
    }
    if (parts[0] === 'refute') return { refuted: false, confidence: 95 };
    throw new Error('unexpected agent label: ' + label);
  };
  return { agent, prompts };
}

// Tokenize `<bin> <subcommand>` occurrences out of a prompt. The binary token is
// whatever non-space run precedes the subcommand, so a re-hardcoded path is
// caught by comparison rather than by being silently skipped.
const INVOCATION = /(^|[\s`])((?:[^\s`]*\/)?rdm)\s+([a-z][a-z-]*(?:\s+[a-z][a-z-]*)?)/g;

function scan(prompts) {
  const out = [];
  for (const p of prompts) {
    for (const line of p.split('\n')) {
      INVOCATION.lastIndex = 0;
      let m;
      while ((m = INVOCATION.exec(line)) !== null) {
        out.push({ bin: m[2], two: m[3], one: m[3].split(/\s+/)[0], line });
      }
    }
  }
  return out;
}

// The subcommand is the longest form the tokenizer matched (every rdm
// subcommand this lane emits is two words except bare `rdm commit`), so
// `phase show` and `model resolve` are distinguished rather than collapsed to
// their shared first word.
function subcommandOf(occ) {
  return occ.two;
}

async function capture(args) {
  const c = makeCapture();
  await run(args, c.agent, refPipeline, refParallel, () => {});
  return scan(c.prompts);
}

const BASE_PHASE = { roadmap: 'rm', phase: 'phase-1-x' };
const BASE_TASK = { task: 'my-task' };

for (const [mode, base] of [['phase', BASE_PHASE], ['task', BASE_TASK]]) {
  // --- Run A: a project IS configured.
  const withProject = await capture({ ...base, rdmBin: FAKE_BIN, project: 'demo' });
  assert.ok(withProject.length > 0, mode + ': the scan found no rdm invocations at all — it cannot pass vacuously');

  const seen = new Set();
  for (const occ of withProject) {
    assert.equal(occ.bin, FAKE_BIN, mode + ': an rdm invocation used ' + occ.bin + ' instead of the injected rdmBin: ' + occ.line);
    const sub = subcommandOf(occ);
    seen.add(sub);
    const agnostic = PROJECT_AGNOSTIC.includes(sub);
    const hasFlag = occ.line.includes(' --project demo');
    if (agnostic) {
      assert.ok(!occ.line.includes('--project'), mode + ': project-agnostic `rdm ' + sub + '` must carry NO project flag: ' + occ.line);
    } else {
      assert.ok(hasFlag, mode + ': project-scoped `rdm ' + sub + '` must carry " --project demo": ' + occ.line);
    }
  }

  // Non-vacuity floors: the scan must actually have reached the project-scoped
  // command shapes, not merely found nothing to object to.
  const need = mode === 'task'
    ? ['task show', 'task update', 'worktree add', 'task create']
    : ['phase show', 'phase update', 'worktree add', 'task create'];
  for (const n of need) {
    assert.ok(seen.has(n), mode + ': expected at least one `rdm ' + n + '` occurrence, saw: ' + [...seen].join(', '));
  }
  const resolves = withProject.filter((o) => o.two === 'model resolve').length;
  assert.ok(resolves >= 5, mode + ': expected >= 5 `rdm model resolve` occurrences, found ' + resolves);

  // --- Run B: NO project configured -> not a single --project anywhere.
  const noProject = await capture({ ...base, rdmBin: FAKE_BIN });
  assert.ok(noProject.length > 0, mode + ': the no-project scan found no rdm invocations at all');
  for (const occ of noProject) {
    assert.equal(occ.bin, FAKE_BIN, mode + ' (no project): an rdm invocation used ' + occ.bin + ': ' + occ.line);
  }
  const stray = noProject.filter((o) => o.line.includes('--project'));
  assert.equal(stray.length, 0, mode + ' (no project): expected zero --project occurrences, found: ' + stray.map((o) => o.line).join(' | '));
}

// Determinism: the same args produce byte-identical prompt captures.
const d1 = await capture({ ...BASE_PHASE, rdmBin: FAKE_BIN, project: 'demo' });
const d2 = await capture({ ...BASE_PHASE, rdmBin: FAKE_BIN, project: 'demo' });
assert.deepEqual(d2, d1, 'the prompt capture must be deterministic across identical runs');

console.log('all parameterization prompt-capture assertions passed');
NODE_PARAMZ

if run_node "$TMP/paramz.mjs" "$WF"; then
    pass "9b: every emitted rdm invocation uses the injected binary and honors the project-agnostic allow-list (phase + task, with and without a project)"
else
    fail "9b: parameterization prompt-capture assertions failed"
fi

# --- 9c. Defaulted rdmBin -----------------------------------------------------
say "9c. Defaulted rdmBin: an absent arg resolves to a plain 'rdm', a wrong-TYPE arg still throws, and neither is an existence preflight"

cat >"$TMP/rdmbin.mjs" <<'NODE_RDMBIN'
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const libPath = process.argv[2];
const wfPath = process.argv[3];
const { parseDispatchArgs, projectFlag, resolveRdmBin, parseProjectArg } = await import('file://' + libPath);

// (1a) An ABSENT-ish value DEFAULTS to a plain `rdm` on PATH. A plugin-installed
// consumer has no repo-local build path to pass, so PATH is the shipped default;
// this repo's own stale-global hazard is handled by RDM_BIN in .mise.toml
// (asserted in § 9c-dogfood below), not by refusing to resolve.
for (const absent of [undefined, null, '', '   ', '\t']) {
  assert.equal(resolveRdmBin(absent), 'rdm', 'an absent rdmBin (' + JSON.stringify(absent) + ') must default to "rdm"');
  assert.equal(
    parseDispatchArgs({ roadmap: 'r', phase: 'p', rdmBin: absent }).rdmBin,
    'rdm',
    'parseDispatchArgs must default an absent rdmBin (' + JSON.stringify(absent) + ') to "rdm"'
  );
}
// An args payload with no rdmBin KEY AT ALL defaults too, in both modes.
assert.equal(parseDispatchArgs({ roadmap: 'r', phase: 'p' }).rdmBin, 'rdm', 'a missing rdmBin key defaults to "rdm"');
assert.equal(parseDispatchArgs({ task: 't' }).rdmBin, 'rdm', 'task mode defaults rdmBin too');
// A stringified payload is coerced first, then defaulted the same way.
assert.equal(
  parseDispatchArgs(JSON.stringify({ roadmap: 'r', phase: 'p' })).rdmBin,
  'rdm',
  'a stringified payload defaults rdmBin to "rdm"'
);

// (1b) A present-but-wrong-TYPE value STILL throws. Degrading a `rdmBin: 42`
// typo to PATH would reintroduce exactly the silent-wrong-binary hazard the
// absent-value default does not need.
for (const bad of [42, {}, [], true]) {
  assert.throws(
    () => parseDispatchArgs({ roadmap: 'r', phase: 'p', rdmBin: bad }),
    /rdmBin/,
    'a non-string rdmBin (' + JSON.stringify(bad) + ') must still throw'
  );
}
// The type error must stay actionable: it names the sentinel and the default.
try {
  parseDispatchArgs({ roadmap: 'r', phase: 'p', rdmBin: 42 });
  assert.fail('expected a throw');
} catch (e) {
  assert.match(e.message, /rdmBin must be a string/, 'the message names the type requirement');
  assert.match(e.message, /"rdm"/, 'the message names the explicit PATH sentinel');
  assert.match(e.message, /PATH/, 'the message names what omitting the arg entirely does');
}

// (2) The explicit sentinel is accepted VERBATIM — a downstream repo that wants
// PATH resolution opts in on purpose.
assert.equal(parseDispatchArgs({ roadmap: 'r', phase: 'p', rdmBin: 'rdm' }).rdmBin, 'rdm', "the 'rdm' sentinel is accepted verbatim");
assert.equal(parseDispatchArgs({ roadmap: 'r', phase: 'p', rdmBin: '/opt/x/rdm' }).rdmBin, '/opt/x/rdm', 'an absolute path is accepted verbatim');
assert.equal(resolveRdmBin('rdm'), 'rdm', 'resolveRdmBin passes the sentinel through');
// DISCRIMINATING: the sentinel path is VERBATIM pass-through, not the default
// branch. A trailing space survives, which the `return 'rdm'` default could not
// produce — so this assertion fails if the two paths are ever conflated.
assert.equal(resolveRdmBin('rdm '), 'rdm ', 'a non-empty value is returned verbatim, not normalized to the default');

// (3) `project` is OPTIONAL, falsy means NO flag, and a hostile value is
// rejected rather than escaped (it is interpolated into a Bash-agent prompt).
assert.equal(parseDispatchArgs({ roadmap: 'r', phase: 'p', rdmBin: 'rdm' }).project, '', 'an absent project means no flag');
for (const falsy of [undefined, null, '', 0, false]) {
  assert.equal(parseProjectArg(falsy), '', 'falsy project ' + JSON.stringify(falsy) + ' means no flag');
  assert.equal(projectFlag({ project: parseProjectArg(falsy) }), '', 'a falsy project emits no flag at all');
}
assert.equal(projectFlag({ project: 'demo' }), ' --project demo', 'a configured project emits the flag');
assert.equal(projectFlag(null), '', 'a null cfg emits no flag');
for (const hostile of ['a b', 'a;rm -rf /', '$(x)', '`x`', 'a\nb', 'a|b', 7, {}]) {
  assert.throws(() => parseProjectArg(hostile), /project must be a plain project name/, 'hostile project ' + JSON.stringify(hostile) + ' must be rejected');
}
assert.equal(parseDispatchArgs({ roadmap: 'r', phase: 'p', rdmBin: 'rdm', project: 'rdm-atlas.v2_x' }).project, 'rdm-atlas.v2_x', 'a plain project name survives');

// (4) Driving the wrapped workflow with NO rdmBin at all must now RESOLVE (it no
// longer throws), and every rdm command it emits must name a bare `rdm` — never
// a repo-local build path leaking back in as a hardcoded literal.
const src = fs.readFileSync(wfPath, 'utf8').replace(/^export /m, '');
const wrapperPath = path.join(os.tmpdir(), 'verify-workflow-dispatch-rdmbin-wrapped.mjs');
fs.writeFileSync(wrapperPath, 'export default async function(args, agent, pipeline, parallel, log) {\n' + src + '\n}\n');
const mod = await import('file://' + wrapperPath + '?t=' + process.pid);

// The same capturing-spy idiom § 9b uses, kept minimal: record every prompt and
// answer every label the driver can emit.
const MODELS = { plan: 'm-plan', implement: 'm-impl', review_find: 'm-find', review_verify: 'm-verify', mechanical: 'm-mech' };
const PHASE_META = { roadmap: 'rm', phase: 'phase-1-x', stem: 'phase-1-x', model: 'medium', body: 'BODY', verify: 'sh scripts/verify-all.sh', models: MODELS };
const PLAN_DOC = {
  steps_per_ac: [{ ac: 'AC1', steps: ['do it'] }],
  file_map: [{ path: 'a.rs', change: 'edit' }],
  tests_per_ac: [{ ac: 'AC1', test: 't' }],
  edge_cases: [],
  cross_phase_deps: [],
  summary: 'plan',
};
const prompts = [];
let agentCalls = 0;
const spy = async (prompt, opts) => {
  agentCalls++;
  prompts.push(String(prompt));
  const label = (opts && opts.label) || '';
  if (label === 'fetch:phase-meta') return PHASE_META;
  if (label === 'stamp:in-progress') return { ok: true };
  if (label === 'verify:run') return { exitCode: 0, output: '' };
  if (label === 'clean:check') return { porcelain: '' };
  if (label === 'plan:author' || label === 'plan:revise') return PLAN_DOC;
  if (label === 'act:code') return { handled: [] };
  if (label === 'diff:signals') return { changedFiles: ['rdm-core/src/lib.rs'], diffText: '' };
  if (label === 'implement:worktree' || label === 'implement:rework') return undefined;
  const parts = label.split(':');
  if (parts[0] === 'find') return parts[2] === 'ac' ? { ac: [], findings: [] } : { findings: [] };
  if (parts[0] === 'refute') return { refuted: false, confidence: 95 };
  return null;
};
async function refParallel(thunks) {
  return Promise.all(thunks.map((t) => Promise.resolve().then(t).catch(() => null)));
}
async function refPipeline(items, ...stages) {
  return Promise.all(
    items.map(async (item, i) => {
      let acc = item;
      for (const stage of stages) {
        try {
          acc = await stage(acc, item, i);
        } catch {
          return null;
        }
      }
      return acc;
    })
  );
}
const out = await mod.default({ roadmap: 'rm', phase: 'phase-1-x' }, spy, refPipeline, refParallel, () => {});
assert.ok(out && typeof out === 'object', 'a Workflow invocation with no rdmBin must resolve to an OUTCOME, not throw');
assert.ok(agentCalls > 0, 'the run must actually have dispatched agents — otherwise this assertion is vacuous');

// Every rdm invocation in every captured prompt names the bare default.
const INVOCATION = /(^|[\s`])((?:[^\s`]*\/)?rdm)\s+[a-z][a-z-]*/g;
let invocations = 0;
for (const p of prompts) {
  INVOCATION.lastIndex = 0;
  let m;
  while ((m = INVOCATION.exec(p)) !== null) {
    invocations++;
    assert.equal(m[2], 'rdm', 'an emitted command used ' + m[2] + ' instead of the bare `rdm` default');
  }
}
assert.ok(invocations > 0, 'the scan found no rdm invocations at all — it cannot pass vacuously');
for (const p of prompts) {
  assert.ok(!p.includes('./target/debug/rdm'), 'a repo-local build path leaked into a prompt under the bare default');
}

console.log('all defaulted rdmBin assertions passed');
NODE_RDMBIN

if run_node "$TMP/rdmbin.mjs" "$LIB" "$WF"; then
    pass "9c: rdmBin defaults to a bare 'rdm' when absent (every emitted command uses it), a wrong-TYPE value still throws, the 'rdm' sentinel passes through verbatim, and project is validated"
else
    fail "9c: defaulted rdmBin assertions failed"
fi

# The default must NOT be an existence preflight. `which -a rdm` resolves to the
# stale global build in this repo, so an existence check passes while running
# exactly the binary the development-build rule forbids — a probe would not close
# the dogfood hazard, it would only hide it. The default stays a plain fallback.
# COMMENT LINES ARE STRIPPED FIRST: the contract's own rationale prose names
# `which rdm` / `command -v` precisely to record why they were REJECTED, and an
# unfiltered grep would flag that explanation as the thing it forbids.
assert_no_existence_preflight() {
    grep -vE '^[[:space:]]*(//|\*|/\*)' "$1" |
        grep -nE 'which +(-a +)?rdm|command -v|existsSync|accessSync|statSync' >"$TMP/preflight-hits" 2>/dev/null || true
    [ ! -s "$TMP/preflight-hits" ]
}
# Widened beyond the three dispatch-phase copies: the estimate and
# review-refute-fix engines carry their own resolveRdmBin and are subject to the
# same no-probe rule, so a probe planted in any of them must be caught here too.
cat >"$TMP/preflight-files" <<EOF
$LIB
$WF
$TEMPLATE_WF
$REPO_ROOT/.claude/workflows/lib/estimate.mjs
$REPO_ROOT/.claude/workflows/rdm-wf-estimate.js
$REPO_ROOT/.claude/workflows/rdm-wf-review-refute-fix.js
$REPO_ROOT/rdm-core/src/templates/workflows/rdm-wf-review-refute-fix.js
EOF
PREFLIGHT_COUNT=0
while IFS= read -r f; do
    [ -n "$f" ] || continue
    [ -f "$f" ] || fail "9c: expected preflight-checked file is missing: $f"
    if ! assert_no_existence_preflight "$f"; then
        cat "$TMP/preflight-hits" >&2
        fail "9c: $f must not resolve rdmBin via an existence preflight — the default is a plain fallback"
    fi
    PREFLIGHT_COUNT=$((PREFLIGHT_COUNT + 1))
done <"$TMP/preflight-files"
pass "9c: no existence preflight (which rdm / command -v / existsSync) in any of the $PREFLIGHT_COUNT resolveRdmBin-bearing copies"

# Self-test on a NEWLY-COVERED file, proving the widened list is non-vacuous.
cp "$REPO_ROOT/.claude/workflows/rdm-wf-review-refute-fix.js" "$TMP/preflight-mutant-widened.js"
printf "\nconst ok = accessSync(rdmBin)\n" >>"$TMP/preflight-mutant-widened.js"
if assert_no_existence_preflight "$TMP/preflight-mutant-widened.js"; then
    fail "9c: the widened existence-preflight coverage missed a planted accessSync in review-refute-fix — the widening is vacuous"
fi
pass "9c: the existence-preflight detector fires on a planted probe in a newly-covered copy"

# Self-test: a planted preflight in executable (non-comment) code must fire.
cp "$WF" "$TMP/preflight-mutant.js"
printf "\nconst ok = existsSync(rdmBin)\n" >>"$TMP/preflight-mutant.js"
if assert_no_existence_preflight "$TMP/preflight-mutant.js"; then
    fail "9c: the existence-preflight detector missed a planted existsSync call — the gate is vacuous"
fi
pass "9c: the existence-preflight detector fires on planted code while ignoring the rationale prose"

# --- 9c-inventory. EVERY resolveRdmBin copy carries the new default -----------
say "9c-inventory: every resolveRdmBin copy in the tree defaults to 'rdm', and none reads the environment"

# The contract reversal spans three canonical sources and several byte-identical
# copies across three different propagation mechanisms (generator-stamped,
# hand-copied, and plugin-regenerated). A missed copy would keep throwing on an
# absent value while the rest of the lane defaults — so the inventory is derived
# MECHANICALLY here rather than hand-listed, and every hit must carry the default.
(
    cd "$REPO_ROOT" &&
        grep -rl "function resolveRdmBin" \
            --include=*.js --include=*.mjs \
            .claude/workflows rdm-core/src/templates/workflows plugins 2>/dev/null | sort
) >"$TMP/rdmbin-files"
RDMBIN_COUNT=$(grep -c . <"$TMP/rdmbin-files" || true)
# Floor: 5 under .claude/workflows + 2 template twins + 2 in the emitted plugin
# tree. A deleted copy must surface as a failure, not as a smaller green run.
if [ "$RDMBIN_COUNT" -lt 9 ]; then
    cat "$TMP/rdmbin-files" >&2
    fail "9c-inventory: expected at least 9 resolveRdmBin-bearing files, found $RDMBIN_COUNT — a copy was deleted or the inventory grep drifted"
fi
# The first listed copy is the fixture both planted-mutation self-tests mutate.
RDMBIN_FIRST=$(head -n 1 "$TMP/rdmbin-files")

# Extract each resolveRdmBin body and require the literal default in it.
assert_defaults_to_rdm() {
    awk '
        /^function resolveRdmBin\(/ { inside = 1 }
        inside { print }
        inside && /^}/ { exit }
    ' "$1" | grep -q "return 'rdm';"
}
while IFS= read -r f; do
    [ -n "$f" ] || continue
    if ! assert_defaults_to_rdm "$REPO_ROOT/$f"; then
        fail "9c-inventory: $f's resolveRdmBin does not default an absent value to 'rdm' — the contract reversal missed this copy"
    fi
done <"$TMP/rdmbin-files"
pass "9c-inventory: all $RDMBIN_COUNT resolveRdmBin copies default an absent value to 'rdm'"

# Self-test: revert ONE copy to the old throw-only body (drop the default branch
# outright, exactly as a missed copy would look) and the check must fire.
sed "/^  if (value === undefined .*return 'rdm';/d" \
    "$REPO_ROOT/$RDMBIN_FIRST" >"$TMP/rdmbin-inventory-mutant.js"
if cmp -s "$REPO_ROOT/$RDMBIN_FIRST" "$TMP/rdmbin-inventory-mutant.js"; then
    fail "9c-inventory: the revert-to-throw mutation did not apply — the self-test is not exercising anything"
fi
if assert_defaults_to_rdm "$TMP/rdmbin-inventory-mutant.js"; then
    fail "9c-inventory: the detector missed a copy reverted to the old fail-closed body — the gate is vacuous"
fi
pass "9c-inventory: the detector fires on a copy reverted to the old fail-closed body"

# The Workflow runtime has no env access, so the JS must never try to read
# RDM_BIN itself — resolution happens in the CALLING SKILL and arrives as an
# argument. Comment lines are stripped first (the rationale prose names RDM_BIN
# precisely to say where it IS read).
assert_no_env_read() {
    grep -vE '^[[:space:]]*(//|\*|/\*|#)' "$1" |
        grep -nE 'process\.env|RDM_BIN' >"$TMP/envread-hits" 2>/dev/null || true
    [ ! -s "$TMP/envread-hits" ]
}
while IFS= read -r f; do
    [ -n "$f" ] || continue
    if ! assert_no_env_read "$REPO_ROOT/$f"; then
        cat "$TMP/envread-hits" >&2
        fail "9c-inventory: $f reads the environment — the Workflow runtime has none; the calling skill resolves \$RDM_BIN and passes it down"
    fi
done <"$TMP/rdmbin-files"
pass "9c-inventory: no resolveRdmBin-bearing workflow file reads process.env / RDM_BIN in executable code"

# Self-test: a planted env read in executable code must fire.
cp "$REPO_ROOT/$RDMBIN_FIRST" "$TMP/envread-mutant.js"
printf "\nconst b = process.env.RDM_BIN\n" >>"$TMP/envread-mutant.js"
if assert_no_env_read "$TMP/envread-mutant.js"; then
    fail "9c-inventory: the env-read detector missed a planted process.env.RDM_BIN — the gate is vacuous"
fi
pass "9c-inventory: the env-read detector fires on planted code while ignoring the rationale prose"

# --- 9c-dogfood. The compensating control for the removed fail-closed guard ----
# Numbered 9c-dogfood rather than 9d so the existing 9d/9e sections — which carry
# their own planted-mutation self-tests — are not renumbered.
say "9c-dogfood. .mise.toml pins RDM_BIN to this repo's debug build, and docs record the new contract"

# rdmBin now defaults to a plain `rdm` on PATH. Inside THIS repo that default is
# a stale global build the development-build rule forbids, so `RDM_BIN` in
# .mise.toml is the compensating control that keeps dogfood sessions on the
# development binary. A compensating control that is not itself gated is not a
# control, hence this assertion.
#
# It asserts the VALUE SHAPE, statically, and never shells out to $RDM_BIN: CI
# runs these harnesses without a mise environment, so a runtime dependency on the
# exported variable would be red on every runner.
assert_mise_sets_rdm_bin() {
    _mise_file=$1
    env_section=$(awk '/^\[env\]/{inenv=1;next} /^\[/{inenv=0} inenv' "$_mise_file")
    printf '%s\n' "$env_section" | grep -qE '^[[:space:]]*RDM_BIN[[:space:]]*=' || return 1
    # The value must be a REPO-LOCAL BUILD PATH — not a bare `rdm` (the shipped
    # default, i.e. the exact regression this control guards) and not some
    # arbitrary absolute path outside the repo.
    printf '%s\n' "$env_section" |
        grep -E '^[[:space:]]*RDM_BIN[[:space:]]*=' |
        grep -qE 'target/debug/rdm"?[[:space:]]*$'
}

MISE_TOML="$REPO_ROOT/.mise.toml"
[ -f "$MISE_TOML" ] || fail "9c-dogfood: $MISE_TOML is missing"
if ! assert_mise_sets_rdm_bin "$MISE_TOML"; then
    fail "9c-dogfood: .mise.toml [env] must set RDM_BIN to a repo-local target/debug/rdm path — it is the compensating control for rdmBin defaulting to a PATH-resolved rdm"
fi
pass "9c-dogfood: .mise.toml [env] pins RDM_BIN to a repo-local target/debug/rdm build"

# Self-test (i): the RDM_BIN line deleted entirely -> the detector must FAIL.
sed '/^[[:space:]]*RDM_BIN[[:space:]]*=/d' "$MISE_TOML" >"$TMP/mise-mut-deleted.toml"
if cmp -s "$MISE_TOML" "$TMP/mise-mut-deleted.toml"; then
    fail "9c-dogfood(i): the RDM_BIN deletion did not apply — the self-test is not exercising anything"
fi
if assert_mise_sets_rdm_bin "$TMP/mise-mut-deleted.toml"; then
    fail "9c-dogfood(i): the detector accepted a .mise.toml with no RDM_BIN at all — the gate is vacuous"
fi
pass "9c-dogfood(i): the detector fires when RDM_BIN is removed"

# Self-test (ii): RDM_BIN set to the bare shipped default -> the detector must
# FAIL. This is the exact regression the control exists to prevent: a dogfood
# session silently running whichever global rdm is first on PATH.
sed 's|^[[:space:]]*RDM_BIN[[:space:]]*=.*|RDM_BIN = "rdm"|' "$MISE_TOML" >"$TMP/mise-mut-bare.toml"
if cmp -s "$MISE_TOML" "$TMP/mise-mut-bare.toml"; then
    fail "9c-dogfood(ii): the bare-rdm mutation did not apply — the self-test is not exercising anything"
fi
if assert_mise_sets_rdm_bin "$TMP/mise-mut-bare.toml"; then
    fail "9c-dogfood(ii): the detector accepted RDM_BIN=\"rdm\" — it does not distinguish the repo-local build from the shipped default"
fi
pass "9c-dogfood(ii): the detector fires when RDM_BIN is downgraded to the bare shipped default"

# Positive control: the detector still passes on the real file, so the two
# self-tests above cannot be satisfied by a detector that always fails.
if ! assert_mise_sets_rdm_bin "$MISE_TOML"; then
    fail "9c-dogfood: the detector rejects the real .mise.toml — it always fails and proves nothing"
fi
pass "9c-dogfood: positive control — the detector still accepts the real .mise.toml"

# The contract doc must describe the new default and the override. (An assertion
# on a CONTRACT DOC is permitted; the categorical ban is on CHANGELOG.md alone.)
SCHEMAS_DOC="$REPO_ROOT/docs/workflow-schemas.md"
assert_schemas_doc_records_default() {
    _doc_file=$1
    grep -q 'RDM_BIN' "$_doc_file" || return 1
    # No surviving claim that rdmBin is a required arg with no fallback.
    # shellcheck disable=SC2016  # a literal markdown/backtick pattern, not a shell expansion
    grep -qE '`rdmBin`[^|]*\| *\*\*yes\*\*|no fallback path at all' "$_doc_file" && return 1
    # The no-existence-preflight rule must survive the rewrite.
    grep -q 'existence preflight' "$_doc_file" || return 1
    return 0
}
[ -f "$SCHEMAS_DOC" ] || fail "9c-dogfood: $SCHEMAS_DOC is missing"
if ! assert_schemas_doc_records_default "$SCHEMAS_DOC"; then
    fail "9c-dogfood: docs/workflow-schemas.md must name RDM_BIN, drop the required/no-fallback claims, and keep the no-existence-preflight rule"
fi
pass "9c-dogfood: docs/workflow-schemas.md records the rdm default, the RDM_BIN override, and the surviving no-preflight rule"

# Self-test: reinstating the old required-arg table row must fire the detector.
cp "$SCHEMAS_DOC" "$TMP/schemas-mut.md"
# shellcheck disable=SC2016  # literal markdown, deliberately unexpanded
printf '\n| `rdmBin`  | **yes**  | the executable |\n' >>"$TMP/schemas-mut.md"
if cmp -s "$SCHEMAS_DOC" "$TMP/schemas-mut.md"; then
    fail "9c-dogfood(iii): the schemas-doc mutation did not apply"
fi
if assert_schemas_doc_records_default "$TMP/schemas-mut.md"; then
    fail "9c-dogfood(iii): the doc detector missed a reinstated required-rdmBin table row — the gate is vacuous"
fi
pass "9c-dogfood(iii): the doc detector fires on a reinstated required-rdmBin row"

# --- 9c-single-source. The resolution ORDER lives in exactly two places -------
# `--rdm-bin` -> `RDM_BIN` -> plain `rdm` is stated by PLUGIN_RDM_BIN_NOTE
# (appended to emitted PLUGIN skills as a `## Resolving \`rdmBin\`` section) and
# by docs/workflow-schemas.md. Skill BODIES state only the default and point at
# one of those two. Restating the ordered steps in every body would give a future
# change to the order a dozen hand-maintained call sites with no gate — which is
# exactly the drift this check exists to prevent.
say "9c-single-source: no skill body restates the rdmBin resolution order"

# A restatement is an RDM_BIN mention carrying precedence language. The appended
# plugin note is one of the two canonical statements, so it (and everything after
# it) is stripped before the check.
assert_no_restated_rdm_bin_order() {
    _ss_file=$1
    awk '/^## Resolving `rdmBin`/ { exit } { print }' "$_ss_file" >"$TMP/ss-body.md"
    if grep -E 'RDM_BIN' "$TMP/ss-body.md" | grep -qiE 'then|first|else'; then
        return 1
    fi
    return 0
}

SS_FILES=$(
    grep -rl 'rdmBin' \
        "$REPO_ROOT/.claude/skills" \
        "$REPO_ROOT/rdm-core/src/templates" \
        "$REPO_ROOT/plugins/rdm/skills" \
        --include='SKILL.md' --include='skill-*.md' 2>/dev/null | sort
)
SS_COUNT=$(printf '%s\n' "$SS_FILES" | grep -c . || true)
if [ "$SS_COUNT" -lt 12 ]; then
    fail "9c-single-source: expected at least 12 rdmBin-bearing skill bodies, found $SS_COUNT — the inventory grep drifted"
fi
for f in $SS_FILES; do
    assert_no_restated_rdm_bin_order "$f" ||
        fail "9c-single-source: $f restates the rdmBin resolution order — state the default and point at PLUGIN_RDM_BIN_NOTE or docs/workflow-schemas.md instead"
done
pass "9c-single-source: none of the $SS_COUNT rdmBin-bearing skill bodies restates the order"

# Self-test: plant a restatement and the detector must fire.
SS_VICTIM="$REPO_ROOT/rdm-core/src/templates/skill-dispatch-phase-cli.md"
cp "$SS_VICTIM" "$TMP/ss-mutant.md"
# shellcheck disable=SC2016  # literal prose, deliberately unexpanded
printf '\nResolve an explicit `--rdm-bin <path>` first, then `$RDM_BIN` if set, then a plain `rdm`.\n' >>"$TMP/ss-mutant.md"
if cmp -s "$SS_VICTIM" "$TMP/ss-mutant.md"; then
    fail "9c-single-source: the restatement mutation did not apply — the self-test is not exercising anything"
fi
if assert_no_restated_rdm_bin_order "$TMP/ss-mutant.md"; then
    fail "9c-single-source: the detector missed a planted restatement of the resolution order — the gate is vacuous"
fi
pass "9c-single-source: the detector fires on a planted restatement of the order"

# Positive control on an emitted PLUGIN skill: its appended note DOES state the
# order, and stripping it must not make the check vacuous — the body above the
# note is still scanned.
SS_PLUGIN="$REPO_ROOT/plugins/rdm/skills/dispatch-phase/SKILL.md"
if [ -f "$SS_PLUGIN" ]; then
    # shellcheck disable=SC2016  # a literal markdown heading, not a shell expansion
    grep -q '^## Resolving `rdmBin`' "$SS_PLUGIN" ||
        fail "9c-single-source: the emitted plugin skill lost its appended \`## Resolving \`rdmBin\`\` note — that note is one of the two canonical statements of the order"
    cp "$SS_PLUGIN" "$TMP/ss-plugin-mutant.md"
    # shellcheck disable=SC2016  # literal prose, deliberately unexpanded
    awk 'NR==1 { print; print "Resolve `--rdm-bin` first, then `$RDM_BIN`."; next } { print }' \
        "$SS_PLUGIN" >"$TMP/ss-plugin-mutant.md"
    if assert_no_restated_rdm_bin_order "$TMP/ss-plugin-mutant.md"; then
        fail "9c-single-source: stripping the appended note also hid a restatement ABOVE it — the strip is too greedy"
    fi
    pass "9c-single-source: the appended plugin note survives, and stripping it does not hide a restatement above it"
fi

# --- 9d. Planted-mutation self-tests for 9b -----------------------------------
say "9d. Planted-mutation self-tests: the allow-list assertion is not vacuous"

# (i) projectFlag returns the flag UNCONDITIONALLY -> a `model resolve` line
#     wrongly gains --project, which check (2) must reject.
sed "s|return cfg \&\& cfg.project ? ' --project ' + cfg.project : '';|return ' --project ' + ((cfg \&\& cfg.project) \|\| 'demo');|" "$WF" >"$TMP/pz-mut-uncond.js"
if cmp -s "$WF" "$TMP/pz-mut-uncond.js"; then
    fail "9d(i): the projectFlag mutation did not apply — the self-test is not exercising anything"
fi
if run_node "$TMP/paramz.mjs" "$TMP/pz-mut-uncond.js" >/dev/null 2>&1; then
    fail "9d(i): an unconditional projectFlag was NOT detected — the allow-list assertion is vacuous"
fi
pass "9d(i): detector fires when projectFlag stops honoring the allow-list"

# (ii) a project-scoped builder DROPS its flag (the diff-signals worktree add).
sed "s|'  ' + bin + ' worktree add ' + worktreeRef + proj,|'  ' + bin + ' worktree add ' + worktreeRef,|g" "$WF" >"$TMP/pz-mut-drop.js"
if cmp -s "$WF" "$TMP/pz-mut-drop.js"; then
    fail "9d(ii): the dropped-flag mutation did not apply"
fi
if run_node "$TMP/paramz.mjs" "$TMP/pz-mut-drop.js" >/dev/null 2>&1; then
    fail "9d(ii): a project-scoped command that dropped its project flag was NOT detected"
fi
pass "9d(ii): detector fires when a project-scoped builder drops '+ proj'"

# (iii) a builder RE-HARDCODES this repo's dev binary path.
sed "s|'  ' + bin + ' phase show |'  ./target/debug/rdm phase show |" "$WF" >"$TMP/pz-mut-bin.js"
if cmp -s "$WF" "$TMP/pz-mut-bin.js"; then
    fail "9d(iii): the re-hardcoded-binary mutation did not apply"
fi
if run_node "$TMP/paramz.mjs" "$TMP/pz-mut-bin.js" >/dev/null 2>&1; then
    fail "9d(iii): a re-hardcoded rdm binary was NOT detected"
fi
pass "9d(iii): detector fires when a builder re-hardcodes the rdm binary"

# --- 9e. Repo-wide nested-workflow() negative ---------------------------------
# The parameterization must not have been implemented by nesting a sub-workflow
# (the runtime allows exactly one level, and dispatch-phase already spends it
# on the stamped review block). Comment lines are stripped first: today the only
# `workflow(` hits under .claude/workflows/ are prose comments in
# rdm-wf-review-refute-fix.js, so an unfiltered grep would be meaningless here.
say "9e. Negative: no nested workflow() call site anywhere under .claude/workflows/"

filtered_workflow_calls() {
    grep -rn 'workflow(' "$1" 2>/dev/null |
        grep -vE '^[^:]+:[0-9]+:[[:space:]]*(//|\*|/\*)' || true
}

NESTED=$(filtered_workflow_calls "$REPO_ROOT/.claude/workflows")
if [ -n "$NESTED" ]; then
    printf '%s\n' "$NESTED" >&2
    fail "9e: a nested workflow() call site appeared under .claude/workflows/ — the one-level nesting budget is already spent"
fi
pass "9e: zero nested workflow() call sites under .claude/workflows/ (comment lines excluded)"

mkdir -p "$TMP/nested-plant"
printf "const r = await workflow('sub', {})\n" >"$TMP/nested-plant/x.js"
printf "// a prose comment mentioning workflow( must NOT fire\n" >>"$TMP/nested-plant/x.js"
PLANTED=$(filtered_workflow_calls "$TMP/nested-plant")
[ -n "$PLANTED" ] || fail "9e: the filtered grep missed a planted nested workflow() call — the detector is broken"
if printf '%s\n' "$PLANTED" | grep -q 'prose comment'; then
    fail "9e: the filtered grep is over-broad — it flagged a comment line"
fi
pass "9e: detector fires on a planted nested workflow() call and stays silent on a comment"

# --- 10. VERIFY-GATE DOCS ------------------------------------------------------
# The two-layer split is the whole point of the gate, and it is the half a reader
# is most likely to get wrong (by pushing the slow suite into pre-commit). Gate
# the doc's content, not merely its existence.
#
# NOTE the interaction with § 3-verify: docs/verify-gate.md deliberately names
# `hk`, `make` and `just` as examples of where task-running belongs, so the
# toolchain-literal grep stays scoped to the fenced workflow regions and must
# NEVER be widened to docs.
say "10. docs/verify-gate.md recommends a pre-commit hook and justifies the phase-time split"

VERIFY_DOC="$REPO_ROOT/docs/verify-gate.md"
[ -f "$VERIFY_DOC" ] || fail "10: docs/verify-gate.md is missing — it is this phase's docs deliverable"

assert_verify_doc() {
    doc=$1
    for needle in 'hk' 'hk.pkl' 'pre-commit' 'dispatch.verify' 'scripts/verify-'; do
        grep -qF "$needle" "$doc" || {
            VERIFY_DOC_ERR="docs/verify-gate.md never names '$needle'"
            return 1
        }
    done
    # The why-not-there rationale: one paragraph must carry BOTH `slow` and
    # `pre-commit`, so the split is justified rather than merely asserted.
    awk 'BEGIN { RS = "" } /slow/ && /pre-commit/ { found = 1 } END { exit (found ? 0 : 1) }' "$doc" || {
        VERIFY_DOC_ERR="no single paragraph in docs/verify-gate.md explains why SLOW suites do not belong in PRE-COMMIT"
        return 1
    }
    # The explicit anti-recommendation.
    grep -qiE 'do \*?\*?not\*?\*? *(close this|push)' "$doc" || {
        VERIFY_DOC_ERR="docs/verify-gate.md carries no explicit anti-recommendation against pushing slow suites into pre-commit"
        return 1
    }
    # The resolution ladder and the not-a-task-runner non-goal.
    for needle in '.github/workflows' 'principles.md' 'AGENTS.md' 'not a task runner'; do
        grep -qF "$needle" "$doc" || {
            VERIFY_DOC_ERR="docs/verify-gate.md never names '$needle'"
            return 1
        }
    done
    return 0
}

VERIFY_DOC_ERR=''
assert_verify_doc "$VERIFY_DOC" || fail "10: $VERIFY_DOC_ERR"
pass "10: docs/verify-gate.md names hk/hk.pkl/pre-commit, the resolution ladder, the non-goal, and justifies the split"

# Self-test: strip `hk` from a scratch copy and require the check to fire.
sed 's/hk/zz-removed-runner/g' "$VERIFY_DOC" >"$TMP/verify-doc-no-hk.md"
if assert_verify_doc "$TMP/verify-doc-no-hk.md"; then
    fail "10: the doc check PASSED against a copy with no 'hk' — it is vacuous"
fi
pass "10: the doc check fires when the recommended pre-commit runner is stripped"

# Self-test: strip the anti-recommendation and require the check to fire.
sed 's/do \*\*not\*\*/consider whether to/g' "$VERIFY_DOC" >"$TMP/verify-doc-no-anti.md"
if assert_verify_doc "$TMP/verify-doc-no-anti.md" && [ -z "$VERIFY_DOC_ERR" ]; then
    fail "10: the doc check PASSED against a copy with no anti-recommendation"
fi
pass "10: the doc check fires when the anti-recommendation is removed"

# ...and the real doc still passes, so both self-tests are discriminating.
VERIFY_DOC_ERR=''
assert_verify_doc "$VERIFY_DOC" || fail "10: the real doc fails after the self-tests — the detector rejects everything"
pass "10: the real doc still passes (the self-tests discriminate)"

# The schema reference must LINK to the canonical write-up rather than restate it.
grep -qF 'verify-gate.md' "$REPO_ROOT/docs/workflow-schemas.md" ||
    fail "10: docs/workflow-schemas.md must link to docs/verify-gate.md from its Verify gate section"
grep -qF 'verify:run' "$REPO_ROOT/docs/workflow-schemas.md" ||
    fail "10: docs/workflow-schemas.md must name the verify:run label"
grep -qF 'verify-gate.md' "$REPO_ROOT/CLAUDE.md" ||
    fail "10: CLAUDE.md must point at docs/verify-gate.md"
pass "10: docs/workflow-schemas.md and CLAUDE.md both point at the canonical write-up"

say "verify-workflow-dispatch.sh: ALL GREEN"
