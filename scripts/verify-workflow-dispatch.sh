#!/bin/sh
# Hermetic regression for the dispatch-phase keystone workflow.
#
# dispatch-phase (`.claude/workflows/dispatch-phase.js`) is the unit of
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
WF="$REPO_ROOT/.claude/workflows/dispatch-phase.js"

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

const SHAPE = ['findings', 'outcome', 'phase', 'reason', 'roadmap', 'status', 'summary', 'writesCompletion'];

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

const TASK_SHAPE = ['findings', 'outcome', 'reason', 'status', 'summary', 'task', 'writesCompletion'];

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
      review: async (doc) => {
        assert.ok(doc !== null && doc !== undefined, 'a null plan doc must NEVER reach the reviewer');
        callLog.push('review');
        const r = script[Math.min(reviewIdx, script.length - 1)];
        reviewIdx++;
        return r;
      },
    },
  };
}

function makeCodeFakes(o) {
  const opts = o || {};
  const callLog = [];
  let reviewIdx = 0;
  const script = opts.reviewScript || [[B('never-clean')]];
  return {
    callLog,
    deps: {
      implement: async (notes) => {
        callLog.push(notes == null ? 'implement' : 'rework');
      },
      review: async () => {
        callLog.push('review');
        const r = script[Math.min(reviewIdx, script.length - 1)];
        reviewIdx++;
        return r;
      },
    },
  };
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
  const d = parseDispatchArgs({ roadmap: 'r', phase: 'p' });
  assert.equal(d.maxPlanRevise, DEFAULT_MAX_PLAN_REVISE, 'unset args take the default plan budget');
  assert.equal(d.maxCodeRework, DEFAULT_MAX_CODE_REWORK, 'unset args take the default code budget');
  assert.equal(d.planOnly, false, 'planOnly defaults false');
  const z = parseDispatchArgs({ roadmap: 'r', phase: 'p', maxCodeRework: 0, maxPlanRevise: 0 });
  assert.equal(z.maxCodeRework, 0, 'an explicit 0 survives the args boundary');
  assert.equal(z.maxPlanRevise, 0, 'an explicit 0 survives the args boundary (plan)');
  assert.throws(
    () => parseDispatchArgs({ roadmap: 'r', phase: 'p', maxCodeRework: -1 }),
    /maxCodeRework must be a non-negative integer/,
    'an invalid budget is rejected at parse time, before any agent() call'
  );
  // A stringified payload is coerced ONCE and its budgets are still validated.
  const s = parseDispatchArgs(JSON.stringify({ roadmap: 'r', phase: 'p', maxPlanRevise: 3 }));
  assert.equal(s.roadmap, 'r', 'a stringified payload is coerced');
  assert.equal(s.maxPlanRevise, 3, 'a budget inside a stringified payload is read');
  assert.throws(
    () => parseDispatchArgs(JSON.stringify({ roadmap: 'r', maxPlanRevise: 'nope' })),
    /maxPlanRevise must be a non-negative integer/,
    'a budget inside a stringified payload is still validated'
  );
  const t = parseDispatchArgs({ task: 'my-task' });
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
//   1. a Rust-public-API + no-tests diff turns `api-docs` and `tests` ON;
//   2. round 2 re-derives from the POST-rework tree (a fix that newly touches a
//      public rdm-core item must turn `api-docs` on for round 2);
//   3. an empty/failed diff omits the `signals` KEY ENTIRELY (fail-open), never
//      passing `{}` — selectDimensions treats those two cases differently.
// ============================================================================
{
  const { deriveSignals } = mod;

  const RUST_PUBLIC = ['rdm-core/src/ops/task.rs'];
  const s1 = deriveSignals({ targetType: 'phase', changedFiles: RUST_PUBLIC, diffText: '+pub fn foo() {}\n' });
  assert.equal(s1.publicApiChanged, true, 'a new rdm-core pub item turns publicApiChanged on');
  assert.equal(s1.missingTests, true, 'a code-only diff with no test file turns missingTests on');
  assert.equal(s1.changesLogic, true, 'a .rs change turns changesLogic on');
  assert.equal(s1.targetType, 'phase', 'the target type rides along for the plan-mode trigger');

  // Round 1 touches no Rust; round 2's fix does. Re-derivation must notice.
  const roundDiffs = [
    { changedFiles: ['docs/workflow-schemas.md'], diffText: '+prose\n' },
    { changedFiles: RUST_PUBLIC, diffText: '+pub fn bar() {}\n' },
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
        return [B('still-broken')];
      }
      const signals = deriveSignals({ targetType: 'phase', changedFiles: files, diffText: d.diffText });
      seen.push({ hasSignalsKey: true, signals });
      return roundIdx >= 2 ? [] : [B('round-1-blocker')];
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
      return [];
    },
  };
  await runCodeGate({ maxRework: 0, tier: 'medium' }, failOpenDeps);
  assert.ok(
    !Object.prototype.hasOwnProperty.call(failOpenCall, 'signals'),
    'an empty diff omits the signals KEY entirely (fail-open) — never passes {}'
  );
}

console.log('all dispatch-phase gate assertions passed');
NODE_TEST

if run_node "$TMP/gates.mjs" "$LIB"; then
    pass "budget-bounded gates verified (per-budget call counts, budget-0 regression, independence, boundedness, validation)"
else
    fail "dispatch-phase gate assertions failed"
fi

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
    fail "AC-1: the stamped regions of dispatch-phase.js must not contain a 'Done:' line (land-time only)"
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
    fail "AC-5: dispatch-phase.js must not use isolation:'worktree' — enter the shared worktree via Bash"
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
    fail "AC-3: dispatch-phase.js must not import (the runtime forbids it — sharing is by stamped copy)"
fi
if grep -nE '(^|[^A-Za-z_])require\(' "$WF" >/dev/null 2>&1; then
    fail "AC-3: dispatch-phase.js must not require() (the runtime forbids it)"
fi
if grep -n 'workflow(' "$WF" >/dev/null 2>&1; then
    grep -n 'workflow(' "$WF" >&2 || true
    fail "AC-3: dispatch-phase.js must not nest a workflow() call — both review gates are inline copies"
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
# The extractor deliberately skips COMMENT lines: `dispatch-phase.js` contains
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
grep -qF "rdm task update ' + target + ' --status in-progress" "$WF" ||
    fail "AC-STAMP: missing the task-mode 'rdm task update ... --status in-progress' command"
grep -qF "rdm phase update ' +" "$WF" || fail "AC-STAMP: missing the phase-mode 'rdm phase update' command"
grep -qF -- '--status in-progress' "$WF" || fail "AC-STAMP: missing a '--status in-progress' status string"
pass "AC-STAMP: stamp:in-progress label and both phase/task status commands are present"

# AC-STAMP Phase-mode scoped assertion: extract and normalize the buildStampInProgressPrompt
# function body to verify the phase-mode branch carries all required tokens:
# --status in-progress --no-edit --roadmap, roadmapSlugArg, and --project rdm.
# The phase-mode command is multi-line, so grep on a single normalized line.
assert_stamp_phase_mode() {
    extract_fn_body "$1" buildStampInProgressPrompt >"$TMP/stamp-fn"
    [ -s "$TMP/stamp-fn" ] || return 1
    # Normalize: collapse newlines and multiple spaces into single spaces
    cat "$TMP/stamp-fn" | tr -d '\n' | tr -s ' ' >"$TMP/stamp-normalized"
    [ -s "$TMP/stamp-normalized" ] || return 1
    # Check phase-mode tokens in the normalized body. The phase-mode branch
    # must contain: --status in-progress --no-edit --roadmap, roadmapSlugArg, --project rdm
    grep -qF -- '--status in-progress --no-edit --roadmap' "$TMP/stamp-normalized" || return 1
    grep -qF 'roadmapSlugArg' "$TMP/stamp-normalized" || return 1
    grep -qF -- '--project rdm' "$TMP/stamp-normalized" || return 1
    return 0
}

assert_stamp_phase_mode "$WF" ||
    fail "AC-STAMP: phase-mode buildStampInProgressPrompt must contain --status in-progress --no-edit --roadmap, roadmapSlugArg, and --project rdm"
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

sed "s/rdm task update '/rdm task updateX '/" "$WF" >"$TMP/stamp-mutant-cmd"
if grep -qF "rdm task update ' + target + ' --status in-progress" "$TMP/stamp-mutant-cmd"; then
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
# Scope: the two stamped lib blocks (`review-refute-fix`, `dispatch-outcome`) are
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
    fail "AC-4: no 'while' is permitted in dispatch-phase.js's DRIVER REGION (the stamped review-refute-fix and dispatch-outcome blocks are deliberately out of scope — this phase does not own them; the budget loops belong in the dispatch-outcome block)"
assert_for_headers_allowlisted "$WF" ||
    fail "AC-4: a 'for' header in dispatch-phase.js's DRIVER REGION is not on the allowlist (only the two budget loops are permitted, and only in the dispatch-outcome block — the stamped blocks themselves are out of scope)"
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

# --- 4. MODULE PARSE ---------------------------------------------------------
say "4. Module parse: dispatch-phase.js loads under module semantics (no SyntaxError)"

if parse_workflow "$WF" >/dev/null 2>&1; then
    pass "dispatch-phase.js parses under module semantics (top-level meta declared once)"
else
    parse_workflow "$WF" >&2 || true
    fail "dispatch-phase.js does NOT parse — fix the SyntaxError (e.g. a duplicate top-level 'meta')"
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
AUTOPILOT_LIB="$REPO_ROOT/.claude/workflows/lib/autopilot.mjs"
# DEFAULT_MAX_CODE_REWORK was lifted into the canonical review source alongside
# classifyOutcome; the plan-revise budget stays with the dispatch decision core.
REVIEW_LIB="$REPO_ROOT/.claude/workflows/lib/review.mjs"
[ -f "$DOC" ] || fail "escalation protocol doc not found: $DOC"
[ -f "$AUTOPILOT_LIB" ] || fail "autopilot lib not found: $AUTOPILOT_LIB"
[ -f "$REVIEW_LIB" ] || fail "canonical review lib not found: $REVIEW_LIB"

const_value() {
    grep -oE "const $2 = [0-9]+" "$1" | head -1 | grep -oE '[0-9]+$'
}

assert_doc_agrees() {
    doc=$1
    pr=$(const_value "$LIB" DEFAULT_MAX_PLAN_REVISE)
    cr=$(const_value "$REVIEW_LIB" DEFAULT_MAX_CODE_REWORK)
    ar=$(const_value "$AUTOPILOT_LIB" DEFAULT_MAX_REWORK)
    gb=$(const_value "$AUTOPILOT_LIB" DEFAULT_GLOBAL_BUDGET)
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
    fail "docs/escalation-protocol.md § Budgets disagrees with the constants in lib/dispatch-phase.mjs / lib/review.mjs / lib/autopilot.mjs"
pass "all four budgets are named in the doc with exactly the values the code declares"

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

say "verify-workflow-dispatch.sh: ALL GREEN"
