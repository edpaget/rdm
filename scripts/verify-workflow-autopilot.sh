#!/bin/sh
# Hermetic regression for the autopilot roadmap-driving workflow.
#
# autopilot (`.claude/workflows/autopilot.js`) is the active driver of rdm's
# autonomous lane: given ONE roadmap slug it runs an estimate pre-pass, then
# loops `rdm next` -> dispatch-phase (via the one allowed `workflow()` nesting)
# -> interpret the OUTCOME -> PERSIST status (advance to reviewed / park blocked),
# bounded by a global step budget and `--max-phases`, always ending with a
# batched summary and never touching `main`. Its pure control core lives once in
# `.claude/workflows/lib/autopilot.mjs` and is copied BYTE-IDENTICAL into the
# workflow script (the Workflow runtime cannot import a helper module — see
# docs/workflow-schemas.md § "Import spike"). The five MECHANICAL agents
# (fetch-next, estimate-list, estimate-writeback, advance, park) run on a model
# resolved ONCE per run via `rdm model resolve mechanical` (a deliberately
# unsized bootstrap call, mirroring dispatch-phase's Stage-0 exemption); an
# unresolvable mechanical model stops the run immediately with a distinct,
# greppable stop reason rather than silently falling back to the session model.
# The three state-writing mechanical agents (estimate-writeback, advance, park)
# treat a null/empty ack as a FAILURE rather than silent success. This harness
# gates all of that:
#
#   1. BEHAVIOR   — the pure helpers, driven in Node (zero LLM calls): arg
#                   parsing, phase selection, outcome interpretation, tier
#                   resolution, prompt contents (including
#                   buildMechanicalModelPrompt), the batched summary, and
#                   determinism.
#   1b. DRIVEN LOOP — buildAutopilot fed state-backed fakes (a mutable status
#                   Map): drive-to-reviewed, rework->park, escalated, budget
#                   stops, the estimate pre-pass, --plan-only, mid-tier
#                   defaulting, mechanical-model threading into all five
#                   mechanical deps (and NOT into the judgment estimator),
#                   mechanical-model-unresolved fail-fast, advance-null treated
#                   as failure (parked, absent from completed[]), and
#                   park-null-on-every-attempt still summarizing — asserting the
#                   loop advances off PERSISTED status.
#   2. BLOCK DRIFT — the `autopilot-loop` region is byte-identical between the lib
#                   source of truth and the stamped workflow script (with a
#                   planted-mutation self-test proving the gate is not a no-op).
#   3. STATIC INVARIANTS — grep-based assertions: exactly one nested
#                   workflow('dispatch-phase') call; dispatch-phase.js nests none;
#                   no import/require; both markers; advance/park status writes; no
#                   land/merge/main-mutation prompt string; no *_SCHEMA handed to
#                   agent() uses a top-level type:'array' (Anthropic tools require
#                   'object') with a planted-mutation self-test; meta.phases parity.
#   3b. AC-MODEL   — every agent() call inside the five mechanical dep functions
#                   (fetchNext, estimateList, estimateWriteback, advance, park)
#                   carries an explicit `model:`; the resolveMechanicalModel
#                   bootstrap call is whitelisted as exempt by design (it is the
#                   call that PRODUCES the model id). A planted-mutation
#                   self-test proves the sweep is not a no-op.
#   4. MODULE PARSE — autopilot.js loads under module semantics (no SyntaxError),
#                   with a planted duplicate-meta self-test.
#   6. LAND-TIME TRAILER — hermetic, against the real binary: a trailer-less
#                   commit on a roadmap branch (exactly what an autopilot run
#                   leaves) gains its completion trailer from `rdm hook done-line`
#                   + `git commit --amend`, with NO rebase, and `rdm hook
#                   post-commit` then flips the phase to done.
#
# *** INVARIANT INVERSION (unify-code-review phase 6) ***
# This harness used to assert the land-time completion trailer was absent from
# `autopilot.js` ANYWHERE — an absolute whole-file rule written when nothing wrote
# the trailer at all. Phase 4 moved that write to LAND time (`rdm-land` synthesizes
# it via `rdm hook done-line`), so the rule is now SCOPED — forbidden in every
# built prompt and in autopilot's own code, allowed in explanatory comments that
# name the land-time writer — and PAIRED with the positive Section 6 regression
# proving the trailer really does arrive without a manual rebase. See the
# annotated comment at the AC-6 block in Section 3.
#
# Node is used only as a host to unit-test the pure module and drive the loop with
# fakes; it is stdlib-only (node:assert), with no package.json / node_modules /
# third-party packages. node is pinned in .mise.toml.
#
# Requires: node (via PATH or `mise exec node --`).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)

LIB="$REPO_ROOT/.claude/workflows/lib/autopilot.mjs"
WF="$REPO_ROOT/.claude/workflows/autopilot.js"
DISPATCH_WF="$REPO_ROOT/.claude/workflows/dispatch-phase.js"
DISPATCH_LIB="$REPO_ROOT/.claude/workflows/lib/dispatch-phase.mjs"

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
[ -f "$DISPATCH_WF" ] || fail "dispatch-phase workflow not found: $DISPATCH_WF"

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

# Parse a workflow script under MODULE semantics and fail on a SyntaxError. Same
# transform as verify-workflow-dispatch.sh: strip the leading `export` and wrap in
# an async function so top-level `return`/`await` are legal, while keeping the
# top-level `const meta` in ONE shared scope so a redeclaration is a SyntaxError.
parse_workflow() {
    {
        echo '(async function(){'
        sed 's/^export //' "$1"
        echo '})'
    } |
        run_node --check --input-type=module -
}

# Distinct `phase: '<name>',` literals the workflow actually emits (the real deps'
# agent() calls), one per line, sorted-unique.
emitted_phases() {
    grep -oE "phase: '[A-Za-z]+'," "$1" | sed "s/phase: '//;s/',//" | sort -u
}

# Distinct `{ title: '<name>' }` entries declared in the `meta.phases` array,
# scoped by awk to the array window so nothing outside it is picked up.
declared_phases() {
    awk '/phases: \[/{p=1} p{print} p&&/^  \],?$/{exit}' "$1" |
        grep -oE "title: '[^']+'" | sed "s/title: '//;s/'\$//" | sort -u
}

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

# --- 1. BEHAVIOR -------------------------------------------------------------
say "1. Behavior: arg parsing, selection, outcome interpretation, tiers, prompts, summary"

cat >"$TMP/behavior.mjs" <<'NODE_TEST'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const libPath = process.argv[2];
const m = await import(pathToFileURL(libPath).href);
const {
  DEFAULT_GLOBAL_BUDGET,
  DEFAULT_MAX_REWORK,
  parseAutopilotArgs,
  resolveTier,
  interpretNext,
  buildParkReason,
  interpretOutcome,
  stepBudgetExhausted,
  maxPhasesReached,
  buildMechanicalModelPrompt,
  buildFetchNextPrompt,
  buildAdvancePrompt,
  buildParkPrompt,
  buildSummary,
} = m;
// NOTE: selectUnestimated / buildEstimateListPrompt / buildEstimatorPrompt /
// buildEstimateWritebackPrompt moved to lib/estimate.mjs (estimate-core) and are
// covered by scripts/verify-workflow-estimate.sh; the vestigial difficulty->tier
// JS map was dropped (rdm-core owns that policy). None are imported here anymore.

// --- parseAutopilotArgs ------------------------------------------------------
assert.throws(() => parseAutopilotArgs({}), /roadmap slug is required/, 'roadmap slug is required');
assert.throws(() => parseAutopilotArgs({ roadmap: '' }), /roadmap slug is required/, 'empty roadmap rejected');
const base = parseAutopilotArgs({ roadmap: 'rm' });
assert.deepEqual(
  base,
  {
    roadmap: 'rm',
    maxPhases: null,
    planOnly: false,
    globalBudget: DEFAULT_GLOBAL_BUDGET,
    maxPlanRevise: null,
    maxCodeRework: null,
    // The three optional caller-supplied hoists default to null (absent) — the
    // in-workflow dep call is then the path taken, which is exactly what a
    // direct `Workflow` invocation does.
    mechanicalModel: null,
    phaseList: null,
    next: null,
  },
  'defaults (both dispatch-phase budget overrides unset; all three hoists absent)'
);
assert.ok(!('land' in base), 'parsed config never carries a land flag');
assert.equal(parseAutopilotArgs({ roadmap: 'rm', maxPhases: 3 }).maxPhases, 3, 'maxPhases int');
assert.equal(parseAutopilotArgs({ roadmap: 'rm', maxPhases: '2' }).maxPhases, 2, 'maxPhases coerced from string');
assert.throws(() => parseAutopilotArgs({ roadmap: 'rm', maxPhases: 0 }), /positive integer/, 'maxPhases 0 rejected');
assert.throws(() => parseAutopilotArgs({ roadmap: 'rm', maxPhases: -1 }), /positive integer/, 'maxPhases negative rejected');
assert.equal(parseAutopilotArgs({ roadmap: 'rm', planOnly: true }).planOnly, true, 'planOnly boolean');
assert.equal(parseAutopilotArgs({ roadmap: 'rm', globalBudget: 5 }).globalBudget, 5, 'globalBudget override');
// A caller may stringify the Workflow tool payload; coerce it instead of failing.
assert.equal(parseAutopilotArgs('{"roadmap":"rm"}').roadmap, 'rm', 'stringified JSON args coerced');
assert.equal(parseAutopilotArgs('{"roadmap":"rm","maxPhases":"2"}').maxPhases, 2, 'stringified args keep field coercion');
assert.throws(() => parseAutopilotArgs('not json'), /roadmap slug is required/, 'non-JSON string falls back to actionable error');
assert.throws(() => parseAutopilotArgs('{}'), /roadmap slug is required/, 'stringified empty object still rejected');
assert.throws(() => parseAutopilotArgs('"rm"'), /roadmap slug is required/, 'JSON string primitive rejected, not dereferenced');
assert.throws(() => parseAutopilotArgs('null'), /roadmap slug is required/, 'JSON null rejected without a TypeError');

// selectUnestimated moved to lib/estimate.mjs — its truth table is now asserted
// in scripts/verify-workflow-estimate.sh. The difficulty->tier JS map was
// dropped outright (rdm-core's Difficulty::model_tier is the sole home).

// --- resolveTier -------------------------------------------------------------
assert.equal(resolveTier('small'), 'small');
assert.equal(resolveTier('medium'), 'medium');
assert.equal(resolveTier('large'), 'large');
assert.equal(resolveTier(''), 'medium', 'empty model -> medium');
assert.equal(resolveTier(undefined), 'medium', 'unset model -> medium');
assert.equal(resolveTier('opus'), 'medium', 'non-tier value -> medium');

// --- interpretNext -----------------------------------------------------------
assert.deepEqual(
  interpretNext({ result: 'phase', stem: 's', number: 2, model: 'large' }),
  { kind: 'phase', stem: 's', number: 2, model: 'large' },
  'phase result'
);
assert.deepEqual(interpretNext({ result: 'nothing' }), { kind: 'stop', reason: 'nothing' }, 'nothing result');
assert.deepEqual(
  interpretNext({ result: 'blocked-on-dependencies', unmet: ['x'] }),
  { kind: 'stop', reason: 'blocked-on-dependencies', unmet: ['x'] },
  'blocked-on-dependencies result'
);
assert.equal(interpretNext(null).reason, 'nothing', 'null defaults to stop-nothing');

// --- buildParkReason ---------------------------------------------------------
assert.equal(buildParkReason('plan', 'why'), '[plan] why');
assert.equal(buildParkReason('code', 'boom'), '[code] boom');

// --- interpretOutcome truth table --------------------------------------------
// The legacy bare-STRING form carries no policy, so `status` falls back to the
// reviewed literal; the OUTCOME-OBJECT form below reads it off the OUTCOME.
assert.deepEqual(
  interpretOutcome('reviewed', { planOnly: false, reworkCount: 0, maxRework: 1 }),
  { action: 'advance', status: 'reviewed' },
  'reviewed + normal -> advance (string form falls back to the reviewed status)'
);
assert.deepEqual(
  interpretOutcome('reviewed', { planOnly: true, reworkCount: 0, maxRework: 1 }),
  { action: 'noop-vetted' },
  'reviewed + plan-only -> noop-vetted'
);
assert.deepEqual(
  interpretOutcome('rework', { planOnly: false, reworkCount: 0, maxRework: 1 }),
  { action: 'retry' },
  'rework under budget -> retry'
);
const parkExhausted = interpretOutcome('rework', { planOnly: false, reworkCount: 1, maxRework: 1 });
assert.equal(parkExhausted.action, 'park', 'rework at budget -> park');
assert.ok(parkExhausted.reason.startsWith('[code]'), 'rework-exhausted park is tagged [code]');
const parkEsc = interpretOutcome('escalated', { planOnly: false, reworkCount: 0, maxRework: 1 });
assert.equal(parkEsc.action, 'park', 'escalated -> park');
assert.ok(parkEsc.reason.startsWith('[plan]'), 'escalated park is tagged [plan]');
const parkUnknown = interpretOutcome('weird', { planOnly: false, reworkCount: 0, maxRework: 1 });
assert.equal(parkUnknown.action, 'park', 'unknown outcome -> park');
assert.ok(parkUnknown.reason.startsWith('[code]'), 'unknown-outcome park is tagged [code]');

// --- interpretOutcome reads the OUTCOME's canonical policy -------------------
// The whole OUTCOME OBJECT is accepted; the status and the gate-tagged reason
// come OFF it (dispatch-phase projects them from lib/review.mjs's status
// mapping) instead of being restated here. This is the duplicate being retired.
const advFromOutcome = interpretOutcome(
  { outcome: 'reviewed', status: 'reviewed', writesCompletion: true, reason: '' },
  { planOnly: false, reworkCount: 0, maxRework: 1 }
);
assert.equal(advFromOutcome.action, 'advance', 'a reviewed OUTCOME object advances');
assert.equal(advFromOutcome.status, 'reviewed', 'the advance status comes FROM the OUTCOME');
const advCustom = interpretOutcome(
  { outcome: 'reviewed', status: 'some-other-status' },
  { planOnly: false, reworkCount: 0, maxRework: 1 }
);
assert.equal(
  advCustom.status,
  'some-other-status',
  'the advance status is READ from the OUTCOME, not hardcoded (a changed mapping propagates)'
);
const escFromOutcome = interpretOutcome(
  { outcome: 'escalated', status: 'blocked', writesCompletion: false, reason: '[plan] plan gate escalated: boom' },
  { planOnly: false, reworkCount: 0, maxRework: 1 }
);
assert.equal(escFromOutcome.action, 'park', 'an escalated OUTCOME object parks');
assert.equal(
  escFromOutcome.reason,
  '[plan] plan gate escalated: boom',
  "the park reason is the OUTCOME's own gate-tagged reason"
);
// The rework-budget park stays THIS loop's decision: dispatch's `in-progress`
// describes one dispatch, but a phase whose roadmap-level retry budget is spent
// belongs in the blocked escalation queue.
const reworkExhaustedObj = interpretOutcome(
  { outcome: 'rework', status: 'in-progress', writesCompletion: false, reason: '[code] code rework unresolved: x' },
  { planOnly: false, reworkCount: 1, maxRework: 1 }
);
assert.equal(reworkExhaustedObj.action, 'park', 'a rework OUTCOME at budget still parks');
assert.ok(reworkExhaustedObj.reason.startsWith('[code]'), 'the budget park keeps its [code] tag');
// buildAdvancePrompt interpolates the OUTCOME-supplied status, never a literal.
assert.ok(
  buildAdvancePrompt('phase-1-x', 'rm', 'some-other-status').includes('--status some-other-status'),
  'buildAdvancePrompt interpolates the supplied status rather than a hardcoded reviewed'
);

// --- budget boundaries -------------------------------------------------------
assert.equal(stepBudgetExhausted(2, 3), false, 'under global budget');
assert.equal(stepBudgetExhausted(3, 3), true, 'at global budget');
assert.equal(stepBudgetExhausted(4, 3), true, 'over global budget');
assert.equal(maxPhasesReached(1, null), false, 'null max never trips');
assert.equal(maxPhasesReached(1, 2), false, 'under max');
assert.equal(maxPhasesReached(2, 2), true, 'at max');

// --- buildMechanicalModelPrompt -----------------------------------------------
const mechPrompt = buildMechanicalModelPrompt();
assert.ok(
  mechPrompt.includes('./target/debug/rdm model resolve mechanical'),
  'buildMechanicalModelPrompt embeds the exact mechanical-resolve command'
);

// --- prompt contents ---------------------------------------------------------
const adv = buildAdvancePrompt('phase-1-x', 'rm');
for (const needle of ['phase update', '--status reviewed', 'phase-1-x', 'rm', '--no-edit', '--project rdm']) {
  assert.ok(adv.includes(needle), 'advance prompt contains ' + needle);
}
for (const forbidden of ['Done:', '--land', '--commit']) {
  assert.ok(!adv.includes(forbidden), 'advance prompt must NOT contain ' + forbidden);
}
const park = buildParkPrompt('phase-1-x', '[code] boom', 'rm');
assert.ok(park.includes('--status blocked'), 'park prompt sets --status blocked');
assert.ok(park.includes('--reason'), 'park prompt passes --reason');

// No prompt builder ever leaks a land/merge/main-mutation or completion directive.
const FORBIDDEN = ['Done:', '--land', '--commit', 'git merge', 'git push', 'checkout main'];
function hasForbidden(s) {
  return FORBIDDEN.some((f) => s.includes(f));
}
// The estimate prompt builders moved to lib/estimate.mjs; their forbidden-string
// sweep lives in scripts/verify-workflow-estimate.sh now.
const allPrompts = [
  buildMechanicalModelPrompt(),
  buildFetchNextPrompt('rm'),
  buildAdvancePrompt('phase-1-x', 'rm'),
  buildParkPrompt('phase-1-x', '[code] boom', 'rm'),
];
for (const p of allPrompts) {
  assert.ok(!hasForbidden(p), 'no prompt leaks a land/merge/commit/Done directive:\n' + p);
}
// Planted-string self-test: the detector must FIRE on a forbidden string.
assert.ok(hasForbidden('run rdm phase update --land now'), 'forbidden-string detector catches a planted --land');

// --- buildSummary ------------------------------------------------------------
const summary = buildSummary({
  roadmap: 'rm',
  completed: ['phase-1-a', 'phase-2-b'],
  escalations: [
    { stem: 'phase-3-c', reason: '[plan] dispatch escalated at the plan gate' },
    { stem: 'phase-4-d', reason: '[code] rework budget exhausted' },
  ],
  stopReason: 'nothing',
});
assert.ok(summary.includes('phases completed (2): phase-1-a, phase-2-b'), 'completed listed in order');
assert.ok(summary.indexOf('phase-1-a') < summary.indexOf('phase-2-b'), 'completed order preserved');
assert.ok(summary.includes('phase-3-c [plan]'), 'plan escalation tagged plan');
assert.ok(summary.includes('phase-4-d [code]'), 'code escalation tagged code');
assert.ok(summary.includes('rdm review blocked --project rdm'), 'points at the blocked queue');
assert.ok(summary.includes('roadmap/rm'), 'names the roadmap branch');
assert.ok(summary.includes('main is never touched'), 'states main is never touched');
assert.ok(summary.includes('stop reason: nothing'), 'records the stop reason');
assert.ok(!summary.includes('land') && !summary.includes('merge'), 'summary uses no land/merge verb');
// Empty-escalation summary still emits and stays clean.
const cleanSummary = buildSummary({ roadmap: 'rm', completed: ['phase-1-a'], escalations: [], stopReason: 'budget' });
assert.ok(cleanSummary.includes('escalations awaiting review (0): none'), 'clean run reports zero escalations');
assert.ok(cleanSummary.includes('main is never touched'), 'clean summary still states main untouched');

// Determinism — two identical buildSummary calls are byte-equal.
const st = { roadmap: 'rm', completed: ['a', 'b'], escalations: [{ stem: 'c', reason: '[code] x' }], stopReason: 'nothing' };
assert.equal(buildSummary(st), buildSummary(st), 'buildSummary is deterministic');

console.log('all autopilot behavior assertions passed');
NODE_TEST

if run_node "$TMP/behavior.mjs" "$LIB"; then
    pass "pure helpers verified (args, selection, outcome table, tiers, prompts, summary, determinism)"
else
    fail "autopilot behavior assertions failed"
fi

# --- 1b. DRIVEN LOOP ---------------------------------------------------------
say "1b. Driven loop: buildAutopilot fed state-backed fakes (advances off PERSISTED status)"

cat >"$TMP/driven.mjs" <<'NODE_TEST'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const libPath = process.argv[2];
const m = await import(pathToFileURL(libPath).href);
const { buildAutopilot, DEFAULT_MAX_REWORK, DEFAULT_MAX_ADVANCE_ATTEMPTS, DEFAULT_MAX_PARK_ATTEMPTS } = m;

const orderOf = (stem) => parseInt(String(stem).split('-')[1], 10);

// Build state-backed fakes around a mutable Map<stem,status>. fetchNext computes
// the lowest not-started/in-progress stem from the MUTATED map; advance sets
// reviewed; park sets blocked; dispatch returns scripted outcomes per stem.
function makeFakes(opts) {
  const o = opts || {};
  const statusMap = new Map();
  for (const p of o.phases) statusMap.set(p.stem, p.status);
  const models = o.models || {};
  const dispatchScript = o.dispatchScript || {};
  const dispatchIdx = new Map();
  // The resolved mechanical model every pre-existing test needs so buildAutopilot
  // doesn't immediately short-circuit on 'mechanical-model-unresolved'. Tests that
  // want to exercise the unresolved path opt in via `{ mechanicalModel: '' }`.
  const mechanicalModel = o.mechanicalModel !== undefined ? o.mechanicalModel : 'haiku';

  const callLog = [];
  const resolveCalls = [];
  const advanceCalls = [];
  const parkCalls = [];
  const dispatchCalls = [];
  const parallelEstimateCalls = [];
  const writebackCalls = [];
  // Per-call `model` arguments captured for each of the five mechanical dep
  // calls, so a test can assert the resolved mechanical model actually reached
  // every one of them (and reached NO OTHER call, e.g. parallelEstimate).
  const modelCalls = { estimateList: [], estimateWriteback: [], fetchNext: [], advance: [], park: [] };

  function lowestActionable() {
    let best = null;
    for (const [stem, status] of statusMap) {
      if (status === 'not-started' || status === 'in-progress') {
        if (best === null || orderOf(stem) < orderOf(best)) best = stem;
      }
    }
    return best;
  }

  const fakes = {
    log: (msg) => callLog.push('log:' + msg),
    resolveMechanicalModel: async () => {
      resolveCalls.push(1);
      callLog.push('resolveMechanicalModel');
      return mechanicalModel;
    },
    estimateList: async (slug, model) => {
      callLog.push('estimateList');
      modelCalls.estimateList.push(model);
      if (o.estimateList) return o.estimateList;
      // Default: fully estimated so the pre-pass is skipped.
      return o.phases.map((p) => ({ stem: p.stem, status: p.status, difficulty: 'moderate', model: 'medium' }));
    },
    parallelEstimate: async (unestimated) => {
      parallelEstimateCalls.push(unestimated.slice());
      callLog.push('parallelEstimate');
      return o.estimates || [];
    },
    estimateWriteback: async (stem, difficulty, justification, roadmap, model) => {
      writebackCalls.push({ stem, difficulty, justification });
      modelCalls.estimateWriteback.push(model);
      callLog.push('writeback:' + stem);
      return { ok: true };
    },
    fetchNext: async (roadmap, model) => {
      callLog.push('fetchNext');
      modelCalls.fetchNext.push(model);
      const stem = lowestActionable();
      if (!stem) return { result: 'nothing' };
      return { result: 'phase', stem, number: orderOf(stem), model: models[stem] };
    },
    dispatch: async (slug, stem, planOnly) => {
      dispatchCalls.push({ slug, stem, planOnly });
      callLog.push('dispatch:' + stem);
      const script = dispatchScript[stem] || ['reviewed'];
      const i = dispatchIdx.get(stem) || 0;
      const outcome = script[Math.min(i, script.length - 1)];
      dispatchIdx.set(stem, i + 1);
      return { roadmap: slug, phase: stem, outcome, summary: 's', findings: [] };
    },
    advance: async (stem, roadmap, status, model) => {
      advanceCalls.push(stem);
      modelCalls.advance.push(model);
      callLog.push('advance:' + stem);
      statusMap.set(stem, status || 'reviewed');
      return { ok: true };
    },
    park: async (stem, reason, roadmap, model) => {
      parkCalls.push({ stem, reason });
      modelCalls.park.push(model);
      callLog.push('park:' + stem);
      statusMap.set(stem, 'blocked');
      return { ok: true };
    },
  };
  return {
    fakes,
    statusMap,
    callLog,
    resolveCalls,
    advanceCalls,
    parkCalls,
    dispatchCalls,
    parallelEstimateCalls,
    writebackCalls,
    modelCalls,
    mechanicalModel,
  };
}

// === drive-to-reviewed: 3 phases all reviewed -> advance once each, in order,
// terminating BECAUSE the mutated map has nothing actionable left. ===========
{
  const h = makeFakes({
    phases: [
      { stem: 'phase-1-a', status: 'not-started' },
      { stem: 'phase-2-b', status: 'not-started' },
      { stem: 'phase-3-c', status: 'not-started' },
    ],
  });
  const summary = await buildAutopilot(h.fakes)({ roadmap: 'rm', globalBudget: 20 });
  assert.deepEqual(h.advanceCalls, ['phase-1-a', 'phase-2-b', 'phase-3-c'], 'advance fires once per phase, in order');
  assert.deepEqual(h.parkCalls, [], 'park never fires on a clean drive');
  assert.equal(h.statusMap.get('phase-1-a'), 'reviewed');
  assert.equal(h.statusMap.get('phase-3-c'), 'reviewed');
  assert.deepEqual(h.parallelEstimateCalls, [], 'pre-pass skipped when fully estimated');
  assert.ok(summary.includes('phases completed (3): phase-1-a, phase-2-b, phase-3-c'), 'completed in order');
  assert.ok(summary.includes('stop reason: nothing'), 'terminated on nothing (mutated state)');
}

// === rework then park: p1 reworks past budget -> re-dispatch up to budget then
// park(p1) mutates blocked, fetchNext steps to p2, run completes. =============
{
  const reworks = Array(DEFAULT_MAX_REWORK + 1).fill('rework');
  const h = makeFakes({
    phases: [
      { stem: 'phase-1-a', status: 'not-started' },
      { stem: 'phase-2-b', status: 'not-started' },
    ],
    dispatchScript: { 'phase-1-a': reworks },
  });
  const summary = await buildAutopilot(h.fakes)({ roadmap: 'rm', globalBudget: 20 });
  const p1Dispatches = h.dispatchCalls.filter((c) => c.stem === 'phase-1-a').length;
  assert.equal(p1Dispatches, DEFAULT_MAX_REWORK + 1, 're-dispatches up to the rework budget');
  assert.equal(h.parkCalls.length, 1, 'p1 parked exactly once');
  assert.equal(h.parkCalls[0].stem, 'phase-1-a');
  assert.ok(h.parkCalls[0].reason.startsWith('[code]'), 'rework-exhausted park tagged [code]');
  assert.equal(h.statusMap.get('phase-1-a'), 'blocked', 'p1 mutated to blocked');
  assert.ok(h.advanceCalls.includes('phase-2-b'), 'loop stepped to p2 and advanced it');
  assert.ok(summary.includes('stop reason: nothing'), 'run completes');
}

// === escalated: p1 dispatch=escalated -> park called, escalation recorded, step. =
{
  const h = makeFakes({
    phases: [
      { stem: 'phase-1-a', status: 'not-started' },
      { stem: 'phase-2-b', status: 'not-started' },
    ],
    dispatchScript: { 'phase-1-a': ['escalated'] },
  });
  const summary = await buildAutopilot(h.fakes)({ roadmap: 'rm', globalBudget: 20 });
  assert.equal(h.parkCalls.length, 1, 'escalated phase parked once');
  assert.equal(h.parkCalls[0].stem, 'phase-1-a');
  assert.ok(h.parkCalls[0].reason.startsWith('[plan]'), 'escalation parked tagged [plan]');
  assert.equal(h.statusMap.get('phase-1-a'), 'blocked', 'escalated phase mutated blocked');
  assert.ok(h.advanceCalls.includes('phase-2-b'), 'stepped past the escalation');
  assert.ok(summary.includes('phase-1-a [plan]'), 'summary records the plan escalation');
}

// === budget stop (--max-phases): 5 phases, maxPhases=2 -> stop after 2. =======
{
  const phases = [1, 2, 3, 4, 5].map((n) => ({ stem: 'phase-' + n + '-p', status: 'not-started' }));
  const h = makeFakes({ phases });
  const summary = await buildAutopilot(h.fakes)({ roadmap: 'rm', maxPhases: 2, globalBudget: 20 });
  assert.equal(h.dispatchCalls.length, 2, 'exactly maxPhases dispatches');
  assert.ok(summary.includes('phases completed (2):'), 'two phases completed');
  assert.ok(summary.includes('stop reason: budget'), 'stopped on budget');
  assert.equal(h.statusMap.get('phase-3-p'), 'not-started', 'phases remain unworked');
}

// === global-step-budget variant: globalBudget=3 -> stop after 3 dispatches. ===
{
  const phases = [1, 2, 3, 4, 5].map((n) => ({ stem: 'phase-' + n + '-p', status: 'not-started' }));
  const h = makeFakes({ phases });
  const summary = await buildAutopilot(h.fakes)({ roadmap: 'rm', globalBudget: 3 });
  assert.equal(h.dispatchCalls.length, 3, 'stops at the global step budget');
  assert.ok(summary.includes('stop reason: budget'), 'stopped on budget');
}

// === estimate pre-pass: 2 unestimated + 1 estimated -> parallelEstimate once
// with exactly the 2, writeback per stem, BOTH before the first dispatch. =====
{
  const h = makeFakes({
    phases: [
      { stem: 'phase-1-a', status: 'not-started' },
      { stem: 'phase-2-b', status: 'not-started' },
      { stem: 'phase-3-c', status: 'not-started' },
    ],
    estimateList: [
      { stem: 'phase-1-a', status: 'not-started' },
      { stem: 'phase-2-b', status: 'not-started' },
      { stem: 'phase-3-c', status: 'not-started', difficulty: 'hard', model: 'large' },
    ],
    estimates: [
      { stem: 'phase-1-a', difficulty: 'easy', justification: 'one-line change' },
      { stem: 'phase-2-b', difficulty: 'moderate', justification: 'self-contained feature' },
    ],
  });
  await buildAutopilot(h.fakes)({ roadmap: 'rm', globalBudget: 20 });
  assert.equal(h.parallelEstimateCalls.length, 1, 'parallelEstimate called exactly once');
  assert.deepEqual(h.parallelEstimateCalls[0].sort(), ['phase-1-a', 'phase-2-b'], 'rated exactly the 2 unestimated');
  assert.deepEqual(h.writebackCalls.map((w) => w.stem).sort(), ['phase-1-a', 'phase-2-b'], 'writeback per rated stem');
  // The pre-pass now threads the rater's justification into the writeback (which
  // appends the shared ## Estimate audit note) — the behavior change this phase
  // brings to autopilot's pre-pass.
  const wbByStem = Object.fromEntries(h.writebackCalls.map((w) => [w.stem, w]));
  assert.equal(wbByStem['phase-1-a'].justification, 'one-line change', 'justification threaded into writeback for phase-1-a');
  assert.equal(wbByStem['phase-2-b'].justification, 'self-contained feature', 'justification threaded into writeback for phase-2-b');
  const firstDispatch = h.callLog.indexOf('dispatch:phase-1-a');
  assert.ok(h.callLog.indexOf('writeback:phase-1-a') < firstDispatch, 'writeback 1 precedes first dispatch');
  assert.ok(h.callLog.indexOf('writeback:phase-2-b') < firstDispatch, 'writeback 2 precedes first dispatch');
}

// === fully-estimated pre-pass: parallelEstimate NOT called. ===================
{
  const h = makeFakes({
    phases: [{ stem: 'phase-1-a', status: 'not-started' }],
    estimateList: [{ stem: 'phase-1-a', status: 'not-started', difficulty: 'easy', model: 'small' }],
  });
  await buildAutopilot(h.fakes)({ roadmap: 'rm', globalBudget: 20 });
  assert.deepEqual(h.parallelEstimateCalls, [], 'no estimation when every phase is estimated');
}

// === estimate-writeback ack check: writeback failure -> falls back to mid tier ===
// AC1: Seed two phases unestimated; mock estimateWriteback to fail for phase-1-a
// (returning { ok: false }) and succeed for phase-2-b (returning { ok: true }).
// The fake mutates a shared `postWritebackModels` map ONLY on success: phase-2-b
// gets model: 'large', phase-1-a stays absent from the map.
// AC2: Verify the log message 'estimate writeback failed for phase-1-a — it falls back to mid tier' appears.
// AC3: When fetchNext is called during dispatch, it reads from postWritebackModels.
// phase-1-a has no entry (writeback failed), so it returns undefined model, so
// resolveTier defaults to 'medium' — phase-1-a dispatches at mid tier. phase-2-b
// has model: 'large' in the map (writeback succeeded), so it dispatches at large
// tier. This pattern mirrors real behavior: a failed writeback never persists the
// difficulty/model, so a later real fetchNext naturally sees no model.
// AC4: Gutting the ack-check branch's log statement (lines 425-427 of
// lib/autopilot.mjs) causes AC2 to fail (no log entry). AC3 would still pass
// because the tier outcome is determined by the fake's mutation of postWritebackModels,
// not by the log statement itself — but the combined AC2+AC3 assertions prove the
// writeback outcome (success vs failure) is being properly exercised.
{
  const h = makeFakes({
    phases: [
      { stem: 'phase-1-a', status: 'not-started' },
      { stem: 'phase-2-b', status: 'not-started' },
    ],
    models: {}, // Start with no pre-seeded models; they will be populated by successful writebacks only.
    estimateList: [
      { stem: 'phase-1-a', status: 'not-started' },
      { stem: 'phase-2-b', status: 'not-started' },
    ],
    estimates: [
      { stem: 'phase-1-a', difficulty: 'easy', justification: 'quick writeback test' },
      { stem: 'phase-2-b', difficulty: 'moderate', justification: 'moderate work' },
    ],
  });

  // AC1: Mock estimateWriteback to mutate a postWritebackModels map only on success.
  // Use a shared postWritebackModels object so both the fake and the test can observe it.
  const postWritebackModels = {};
  h.fakes.estimateWriteback = async (stem, difficulty, justification, roadmap, model) => {
    h.writebackCalls.push({ stem, difficulty, justification });
    h.modelCalls.estimateWriteback.push(model);
    h.callLog.push('writeback:' + stem);
    if (stem === 'phase-1-a') {
      // Writeback fails: do NOT update postWritebackModels, so the entry remains absent.
      return { ok: false };
    } else if (stem === 'phase-2-b') {
      // Writeback succeeds: update postWritebackModels so fetchNext will see it.
      postWritebackModels[stem] = 'large';
      return { ok: true };
    }
    return { ok: true };
  };

  // Override fetchNext to read from postWritebackModels (which was populated only
  // by successful writebacks). This makes the tier outcome depend on whether each
  // phase's writeback succeeded or failed, mirroring real behavior where a failed
  // writeback never persists the model field.
  const originalFetchNext = h.fakes.fetchNext;
  h.fakes.fetchNext = async (roadmap, model) => {
    const origResult = await originalFetchNext(roadmap, model);
    if (origResult.result === 'nothing') return origResult;
    const stem = origResult.stem;
    // Return the model only if it was set by a successful writeback in postWritebackModels.
    // If the entry is absent (writeback failed), return undefined so resolveTier
    // defaults to 'medium'.
    const phaseModel = postWritebackModels[stem]; // undefined if writeback failed
    return { ...origResult, model: phaseModel };
  };

  await buildAutopilot(h.fakes)({ roadmap: 'rm', globalBudget: 20 });

  // AC2: Assert the log message appears (this WILL fail if the ack-check branch is gutted).
  assert.ok(
    h.callLog.some((l) => l.includes('estimate writeback failed for phase-1-a — it falls back to mid tier')),
    'writeback failure logged with exact phase stem and fallback message'
  );

  // AC3: Assert tier outcomes match the writeback success/failure.
  assert.ok(
    h.callLog.some((l) => l.includes('dispatching phase-1-a (tier medium')),
    'failed-writeback phase-1-a resolves to mid tier because writeback failure left model absent in postWritebackModels'
  );
  assert.ok(
    h.callLog.some((l) => l.includes('dispatching phase-2-b (tier large')),
    'successful-writeback phase-2-b resolves to large tier because writeback success populated postWritebackModels'
  );

  // Note: this tier fallback (absent model -> medium) is distinct from the
  // pre-existing 'unset model' case at line 727–738 because here the model is
  // deliberately absent as a result of writeback failure, not simply never initialized.
}

// === plan-only: reviewed but NOT advanced; planOnlySeen guard terminates. =====
{
  const h = makeFakes({
    phases: [{ stem: 'phase-1-a', status: 'not-started' }],
  });
  const summary = await buildAutopilot(h.fakes)({ roadmap: 'rm', planOnly: true, globalBudget: 20 });
  assert.deepEqual(h.advanceCalls, [], 'advance NEVER called under --plan-only');
  assert.equal(h.dispatchCalls[0].planOnly, true, 'dispatch received planOnly:true');
  assert.equal(h.statusMap.get('phase-1-a'), 'not-started', 'plan-only leaves status unadvanced');
  assert.ok(summary.includes('phases completed (1): phase-1-a'), 'vetted phase recorded as completed');
  assert.ok(summary.includes('stop reason: plan-only-exhausted'), 'guard terminates on re-return');
}

// === mid-tier: a phase with unset model -> resolveTier yields medium at dispatch. =
{
  const h = makeFakes({
    phases: [{ stem: 'phase-1-a', status: 'not-started' }],
    // models map intentionally omitted -> fetchNext returns model undefined.
  });
  await buildAutopilot(h.fakes)({ roadmap: 'rm', globalBudget: 20 });
  assert.ok(
    h.callLog.some((l) => l.includes('dispatching phase-1-a (tier medium')),
    'unset model resolves to the mid tier at dispatch'
  );
}

// === budget passthrough: dispatch-phase's two in-run budgets are forwarded ====
// Behavioral on BOTH halves: the override is observed by a recording dispatch
// fake, and the payload realDeps.dispatch would build from it is fed through
// dispatch-phase's OWN parseDispatchArgs — never a grep for a constant.
{
  const seen = [];
  const h = makeFakes({ phases: [{ stem: 'phase-1-a', status: 'not-started' }] });
  h.fakes.dispatch = async (slug, stem, planOnly, budgets) => {
    seen.push(budgets);
    return { roadmap: slug, phase: stem, outcome: 'reviewed', summary: 's', findings: [] };
  };
  await buildAutopilot(h.fakes)({ roadmap: 'rm', globalBudget: 20, maxPlanRevise: 0, maxCodeRework: 3 });
  assert.equal(seen.length, 1, 'the phase was dispatched once');
  assert.deepEqual(seen[0], { maxPlanRevise: 0, maxCodeRework: 3 }, 'both overrides reach dispatch verbatim (0 is not dropped)');

  const dispatchLib = await import(pathToFileURL(process.argv[3]).href);
  const payload = Object.assign({ roadmap: 'rm', phase: 'phase-1-a', planOnly: false }, seen[0]);
  const parsed = dispatchLib.parseDispatchArgs(payload);
  assert.equal(parsed.maxPlanRevise, 0, 'dispatch-phase observes the maxPlanRevise override end to end');
  assert.equal(parsed.maxCodeRework, 3, 'dispatch-phase observes the maxCodeRework override end to end');

  // No override → no keys forwarded → dispatch-phase applies its OWN defaults.
  const seen2 = [];
  const h2 = makeFakes({ phases: [{ stem: 'phase-1-a', status: 'not-started' }] });
  h2.fakes.dispatch = async (slug, stem, planOnly, budgets) => {
    seen2.push(budgets);
    return { roadmap: slug, phase: stem, outcome: 'reviewed', summary: 's', findings: [] };
  };
  await buildAutopilot(h2.fakes)({ roadmap: 'rm', globalBudget: 20 });
  assert.deepEqual(seen2[0], {}, 'an un-overridden run forwards no budget keys');
  const defaulted = dispatchLib.parseDispatchArgs(Object.assign({ roadmap: 'rm', phase: 'p' }, seen2[0]));
  assert.equal(defaulted.maxPlanRevise, dispatchLib.DEFAULT_MAX_PLAN_REVISE, 'dispatch-phase falls back to its own plan default');
  assert.equal(defaulted.maxCodeRework, dispatchLib.DEFAULT_MAX_CODE_REWORK, 'dispatch-phase falls back to its own code default');
}

// === arg validation for the two new budget flags =============================
{
  const { parseAutopilotArgs } = m;
  const c = parseAutopilotArgs({ roadmap: 'rm', maxPlanRevise: 0, maxCodeRework: '3' });
  assert.equal(c.maxPlanRevise, 0, '0 is a meaningful budget, not "unset"');
  assert.equal(c.maxCodeRework, 3, 'a numeric string parses');
  const unset = parseAutopilotArgs({ roadmap: 'rm' });
  assert.equal(unset.maxPlanRevise, null, 'an unset plan budget is null so the key can be omitted downstream');
  assert.equal(unset.maxCodeRework, null, 'an unset code budget is null so the key can be omitted downstream');
  for (const bad of [-1, 1.5, 'abc', '2abc', NaN, Infinity, {}]) {
    assert.throws(
      () => parseAutopilotArgs({ roadmap: 'rm', maxPlanRevise: bad }),
      /--max-plan-revise must be a non-negative integer/,
      'rejected maxPlanRevise: ' + String(bad)
    );
    assert.throws(
      () => parseAutopilotArgs({ roadmap: 'rm', maxCodeRework: bad }),
      /--max-code-rework must be a non-negative integer/,
      'rejected maxCodeRework: ' + String(bad)
    );
  }
  // autopilot's OWN budgets are untouched by this passthrough.
  assert.equal(m.DEFAULT_MAX_REWORK, 1, 'the roadmap-level rework re-dispatch budget is unchanged');
  assert.equal(m.DEFAULT_GLOBAL_BUDGET, 50, 'the global step budget is unchanged');
}

// === AC2: the resolved mechanical model reaches ALL FIVE mechanical dep calls
// (estimateList, estimateWriteback, fetchNext, advance, park) verbatim, and
// parallelEstimate (the difficulty-rating JUDGMENT agent) receives NO mechanical
// model argument at all. ========================================================
{
  const h = makeFakes({
    mechanicalModel: 'haiku-test',
    phases: [
      { stem: 'phase-1-a', status: 'not-started' },
      { stem: 'phase-2-b', status: 'not-started' },
    ],
    estimateList: [
      { stem: 'phase-1-a', status: 'not-started' },
      { stem: 'phase-2-b', status: 'not-started' },
    ],
    estimates: [
      { stem: 'phase-1-a', difficulty: 'easy', justification: 'one-line change' },
      { stem: 'phase-2-b', difficulty: 'moderate', justification: 'self-contained feature' },
    ],
    dispatchScript: { 'phase-2-b': ['escalated'] },
  });
  let parallelEstimateArgc = null;
  const origParallelEstimate = h.fakes.parallelEstimate;
  h.fakes.parallelEstimate = async function (...cargs) {
    parallelEstimateArgc = cargs.length;
    return origParallelEstimate(...cargs);
  };
  await buildAutopilot(h.fakes)({ roadmap: 'rm', globalBudget: 20 });
  for (const key of ['estimateList', 'estimateWriteback', 'fetchNext', 'advance', 'park']) {
    assert.ok(h.modelCalls[key].length > 0, key + ' was called at least once');
    assert.ok(
      h.modelCalls[key].every((mv) => mv === 'haiku-test'),
      key + ' received the resolved mechanical model on every call'
    );
  }
  assert.equal(parallelEstimateArgc, 1, 'parallelEstimate (the judgment agent) receives NO mechanical-model argument');
}

// === AC1 / AC3: an unresolvable mechanical model stops the run BEFORE any
// mechanical agent runs, with a distinct stop reason, and still returns the
// always-on summary as a plain string rather than throwing. ====================
{
  const h = makeFakes({ mechanicalModel: '', phases: [{ stem: 'phase-1-a', status: 'not-started' }] });
  const summary = await buildAutopilot(h.fakes)({ roadmap: 'rm', globalBudget: 20 });
  assert.equal(typeof summary, 'string', 'runAutopilot resolves to a string, never throws');
  assert.ok(summary.length > 0, 'the summary is non-empty even on the earliest possible stop');
  assert.ok(summary.includes('stop reason: mechanical-model-unresolved'), 'stop reason names the unresolved mechanical model');
  assert.equal(h.callLog.filter((l) => l === 'estimateList').length, 0, 'zero estimateList calls logged');
  assert.equal(h.callLog.filter((l) => l === 'fetchNext').length, 0, 'zero fetchNext calls logged');
  assert.equal(h.dispatchCalls.length, 0, 'zero dispatch calls logged');
}
// Also whitespace-only, not just empty string (an agent() result can trim to
// nothing without being the empty string itself).
{
  const h = makeFakes({ mechanicalModel: '   ', phases: [{ stem: 'phase-1-a', status: 'not-started' }] });
  const summary = await buildAutopilot(h.fakes)({ roadmap: 'rm', globalBudget: 20 });
  assert.ok(summary.includes('stop reason: mechanical-model-unresolved'), 'whitespace-only mechanical model is treated as unresolved');
}

// === AC4a: advance returning null (the exact shape an unresolvable model id
// produces) is treated as a FAILURE — retried up to DEFAULT_MAX_ADVANCE_ATTEMPTS,
// then parked with a [code]-tagged reason, and the stem never lands in
// completed[]. =================================================================
{
  const h = makeFakes({ phases: [{ stem: 'phase-1-a', status: 'not-started' }] });
  h.fakes.advance = async (stem) => {
    h.advanceCalls.push(stem);
    return null;
  };
  const summary = await buildAutopilot(h.fakes)({ roadmap: 'rm', globalBudget: 20 });
  assert.equal(h.advanceCalls.length, DEFAULT_MAX_ADVANCE_ATTEMPTS, 'advance retried exactly DEFAULT_MAX_ADVANCE_ATTEMPTS times');
  assert.equal(h.parkCalls.length, 1, 'phase parked exactly once after advance never confirms');
  assert.ok(h.parkCalls[0].reason.startsWith('[code]'), 'advance-failure park reason tagged [code]');
  assert.ok(
    h.parkCalls[0].reason.includes('advance to reviewed failed repeatedly'),
    'park reason names the advance failure'
  );
  assert.ok(!summary.includes('phases completed (1)'), 'the phase never lands in completed[]');
  assert.ok(summary.includes('phases completed (0)'), 'nothing recorded as completed');
}

// === AC4b: park returning null on EVERY attempt still lets the run resolve to
// a summary (never throws), still records the escalation, and logs the new
// 'no confirmation' warning — an unconfirmed park write must never block the
// final summary. ================================================================
{
  const h = makeFakes({
    phases: [{ stem: 'phase-1-a', status: 'not-started' }],
    dispatchScript: { 'phase-1-a': ['escalated'] },
  });
  h.fakes.park = async (stem, reason) => {
    h.parkCalls.push({ stem, reason });
    // The write itself may well have landed even though the agent could not
    // CONFIRM it (a read-back parse failure, say) — model that realistically
    // by still mutating status, so the loop advances past this phase after
    // park's retry budget is spent, exactly as it would off a real plan repo.
    h.statusMap.set(stem, 'blocked');
    return null;
  };
  const summary = await buildAutopilot(h.fakes)({ roadmap: 'rm', globalBudget: 20 });
  assert.equal(typeof summary, 'string', 'runAutopilot still resolves to a summary string, never throws');
  assert.equal(h.parkCalls.length, DEFAULT_MAX_PARK_ATTEMPTS, 'park retried exactly DEFAULT_MAX_PARK_ATTEMPTS times');
  assert.ok(summary.includes('escalations awaiting review (1)'), 'the escalation is still recorded even though park never confirmed');
  assert.ok(
    h.callLog.some((l) => l.includes('no confirmation')),
    'a captured log entry contains the new "no confirmation" warning text'
  );
}

// === HOIST: caller-supplied mechanicalModel / phaseList / next replace their
// dep calls, and every one falls back when absent. `next` is ONE-SHOT — only
// the first loop iteration may consume it, because `rdm next` is what steps the
// cursor forward once advance/park has persisted a status. ====================
const THREE_PHASES = [
  { stem: 'phase-1-a', status: 'not-started' },
  { stem: 'phase-2-b', status: 'not-started' },
  { stem: 'phase-3-c', status: 'not-started' },
];
{
  // mechanicalModel supplied -> the dep is never called; the hoisted id still
  // reaches every mechanical dep call.
  const h = makeFakes({ phases: THREE_PHASES.map((p) => ({ ...p })) });
  await buildAutopilot(h.fakes)({ roadmap: 'rm', globalBudget: 20, mechanicalModel: 'hoisted-haiku' });
  assert.equal(h.resolveCalls.length, 0, 'hoisted mechanicalModel -> resolveMechanicalModel dep never called');
  assert.ok(h.modelCalls.fetchNext.every((m) => m === 'hoisted-haiku'), 'the hoisted model id reaches fetchNext');
  assert.ok(h.modelCalls.advance.every((m) => m === 'hoisted-haiku'), 'the hoisted model id reaches advance');
}
{
  // Absent -> the dep is called exactly once, as today.
  const h = makeFakes({ phases: THREE_PHASES.map((p) => ({ ...p })) });
  await buildAutopilot(h.fakes)({ roadmap: 'rm', globalBudget: 20 });
  assert.equal(h.resolveCalls.length, 1, 'no hoist -> resolveMechanicalModel dep called exactly once');
}
{
  // A whitespace-only / empty hoist must NOT be accepted as a resolved model —
  // it falls through to the dep, whose own empty-string fail-closed stop is
  // preserved unchanged.
  const h = makeFakes({ phases: THREE_PHASES.map((p) => ({ ...p })), mechanicalModel: '' });
  const summary = await buildAutopilot(h.fakes)({ roadmap: 'rm', globalBudget: 20, mechanicalModel: '   ' });
  assert.equal(h.resolveCalls.length, 1, 'a blank hoisted mechanicalModel falls back to the dep');
  assert.ok(summary.includes('mechanical-model-unresolved'), 'the fail-closed empty-model stop is unchanged on the fallback path');
}
{
  // phaseList supplied -> estimateList dep never called, and the hoisted list
  // really drives the unestimated filter (this one has an unrated phase).
  const h = makeFakes({ phases: THREE_PHASES.map((p) => ({ ...p })) });
  await buildAutopilot(h.fakes)({
    roadmap: 'rm',
    globalBudget: 20,
    phaseList: [
      { stem: 'phase-1-a', status: 'not-started', difficulty: 'moderate', model: 'medium' },
      { stem: 'phase-2-b', status: 'not-started' },
      { stem: 'phase-3-c', status: 'not-started', difficulty: 'easy', model: 'small' },
    ],
  });
  assert.ok(!h.callLog.includes('estimateList'), 'hoisted phaseList -> estimateList dep never called');
  assert.deepEqual(h.parallelEstimateCalls, [['phase-2-b']], 'the hoisted list drives selectUnestimated (only the unrated phase is rated)');
}
{
  const h = makeFakes({ phases: THREE_PHASES.map((p) => ({ ...p })) });
  await buildAutopilot(h.fakes)({ roadmap: 'rm', globalBudget: 20 });
  assert.equal(h.callLog.filter((l) => l === 'estimateList').length, 1, 'no hoist -> estimateList dep called exactly once');
}
{
  // A non-array phaseList hoist is rejected and falls back.
  const h = makeFakes({ phases: THREE_PHASES.map((p) => ({ ...p })) });
  await buildAutopilot(h.fakes)({ roadmap: 'rm', globalBudget: 20, phaseList: { phases: [] } });
  assert.equal(h.callLog.filter((l) => l === 'estimateList').length, 1, 'a non-array phaseList hoist falls back to the dep');
}
{
  // ONE-SHOT next: over a 3-phase drive the fetchNext dep is called
  // dispatchCount - 1 + 1 (the terminating 'nothing' read) times — i.e. exactly
  // one fewer than the unhoisted run, and iteration 1 uses the hoisted value.
  const h = makeFakes({ phases: THREE_PHASES.map((p) => ({ ...p })) });
  await buildAutopilot(h.fakes)({
    roadmap: 'rm',
    globalBudget: 20,
    next: { result: 'phase', stem: 'phase-1-a', number: 1, model: 'medium' },
  });
  const hoistedFetches = h.callLog.filter((l) => l === 'fetchNext').length;
  assert.deepEqual(h.dispatchCalls.map((d) => d.stem), ['phase-1-a', 'phase-2-b', 'phase-3-c'], 'the hoisted next does not derail the drive order');

  const b = makeFakes({ phases: THREE_PHASES.map((p) => ({ ...p })) });
  await buildAutopilot(b.fakes)({ roadmap: 'rm', globalBudget: 20 });
  const plainFetches = b.callLog.filter((l) => l === 'fetchNext').length;
  assert.equal(hoistedFetches, plainFetches - 1, 'hoisted next saves EXACTLY one fetchNext call — iterations 2..N always re-read');
  assert.equal(hoistedFetches, b.dispatchCalls.length, 'the saved call is iteration 1 only (one fetchNext per later iteration + the terminating read)');
}
{
  // A stale/lying hoisted next must not be able to re-dispatch forever: it is
  // consumed once, and iteration 2 reads the real (mutated) state.
  const h = makeFakes({ phases: THREE_PHASES.map((p) => ({ ...p })) });
  await buildAutopilot(h.fakes)({
    roadmap: 'rm',
    globalBudget: 20,
    next: { result: 'phase', stem: 'phase-1-a', number: 1, model: 'medium' },
  });
  const firstStems = h.dispatchCalls.map((d) => d.stem);
  assert.equal(new Set(firstStems).size, firstStems.length, 'no phase is dispatched twice — the hoisted next was consumed exactly once');
}
{
  // A non-object next hoist is rejected and falls back.
  const h = makeFakes({ phases: THREE_PHASES.map((p) => ({ ...p })) });
  await buildAutopilot(h.fakes)({ roadmap: 'rm', globalBudget: 20, next: 'phase-1-a' });
  const b = makeFakes({ phases: THREE_PHASES.map((p) => ({ ...p })) });
  await buildAutopilot(b.fakes)({ roadmap: 'rm', globalBudget: 20 });
  assert.equal(
    h.callLog.filter((l) => l === 'fetchNext').length,
    b.callLog.filter((l) => l === 'fetchNext').length,
    'a non-object next hoist falls back to the dep on every iteration'
  );
}
console.log('autopilot hoist assertions passed');

console.log('all autopilot driven-loop assertions passed');
NODE_TEST

if run_node "$TMP/driven.mjs" "$LIB" "$DISPATCH_LIB"; then
    pass "loop drives to reviewed / parks / budgets / pre-pass / plan-only / mid-tier / writeback-ack-check-causality / budget passthrough / hoists + fallbacks"
else
    fail "autopilot driven-loop assertions failed"
fi

# --- 1c. HOIST planted-mutation self-tests ------------------------------------
# The hoist assertions above are only load-bearing if the corresponding
# production mutation makes them fail. Each mutant is driven through the SAME
# driven.mjs and must break it.
say "1c. Hoist planted-mutation self-tests (one-shot next, and each fallback branch)"

assert_lib_mutant_fails() {
    mutant=$1
    desc=$2
    if cmp -s "$LIB" "$mutant"; then
        fail "1c: planted mutation was a no-op — $desc"
    fi
    if run_node "$TMP/driven.mjs" "$mutant" "$DISPATCH_LIB" >/dev/null 2>&1; then
        fail "1c: driven assertions PASSED against a lib that $desc — the hoist assertions are vacuous"
    fi
    pass "1c: assertions fire when the lib $desc"
}

# (1) Make `next` NON-one-shot: never clear pendingNext, so iterations 2..N keep
#     reusing the caller's stale value and the same phase re-dispatches forever.
sed 's/^      pendingNext = null;$//' "$LIB" >"$TMP/mutant-next-not-one-shot.mjs"
assert_lib_mutant_fails "$TMP/mutant-next-not-one-shot.mjs" "makes the hoisted next non-one-shot (pendingNext never cleared)"

# (2) Drop the mechanicalModel fallback: always take the (possibly absent) hoist.
sed "s|^        : await d.resolveMechanicalModel();\$|        : '';|" "$LIB" >"$TMP/mutant-no-model-fallback.mjs"
assert_lib_mutant_fails "$TMP/mutant-no-model-fallback.mjs" "drops the resolveMechanicalModel fallback"

# (3) Drop the phaseList fallback.
sed 's|^    const phaseList = Array.isArray(cfg.phaseList) ? cfg.phaseList : await d.estimateList(roadmap, mechanicalModel);$|    const phaseList = Array.isArray(cfg.phaseList) ? cfg.phaseList : [];|' "$LIB" >"$TMP/mutant-no-list-fallback.mjs"
assert_lib_mutant_fails "$TMP/mutant-no-list-fallback.mjs" "drops the estimateList fallback"

# (4) Weaken the phaseList shape guard to "anything truthy", so a non-array hoist
#     is accepted and the unestimated filter silently sees garbage.
sed 's|^    const phaseList = Array.isArray(cfg.phaseList) ? cfg.phaseList : await d.estimateList(roadmap, mechanicalModel);$|    const phaseList = cfg.phaseList ? cfg.phaseList : await d.estimateList(roadmap, mechanicalModel);|' "$LIB" >"$TMP/mutant-weak-list-guard.mjs"
assert_lib_mutant_fails "$TMP/mutant-weak-list-guard.mjs" "weakens the phaseList shape guard to any truthy value"

# --- 2. BLOCK DRIFT GATE -----------------------------------------------------
say "2. Block drift: the autopilot-loop region is byte-identical (lib vs workflow)"

extract_block() {
    awk '
        index($0, ">>> autopilot-loop:begin") { infence = 1; next }
        index($0, ">>> autopilot-loop:end") { infence = 0 }
        infence { print }
    ' "$1"
}

blocks_equal() {
    extract_block "$1" >"$TMP/_a" 2>/dev/null
    extract_block "$2" >"$TMP/_b" 2>/dev/null
    [ -s "$TMP/_a" ] && diff -q "$TMP/_a" "$TMP/_b" >/dev/null 2>&1
}

extract_block "$LIB" >"$TMP/lib-block"
[ -s "$TMP/lib-block" ] || fail "no autopilot-loop block found between markers in $LIB"
extract_block "$WF" >"$TMP/wf-block"
[ -s "$TMP/wf-block" ] || fail "no autopilot-loop block found between markers in $WF"

if diff -u "$TMP/lib-block" "$TMP/wf-block" >/dev/null 2>&1; then
    pass "autopilot-loop block matches byte-for-byte between lib and workflow"
else
    printf '\n' >&2
    diff -u "$TMP/lib-block" "$TMP/wf-block" >&2 || true
    fail "autopilot-loop block DRIFTED — copy the lib block verbatim into $WF"
fi

# Self-test: prove the byte-equality gate is not a no-op.
say "2b. Block drift detector fires on planted drift (self-test)"
cp "$LIB" "$TMP/lib.scratch"
cp "$WF" "$TMP/wf.scratch"
blocks_equal "$TMP/lib.scratch" "$TMP/wf.scratch" || fail "scratch copies should match before mutation"
sed 's/rework budget exhausted/planted drift/' "$TMP/wf.scratch" >"$TMP/wf.mut" && mv "$TMP/wf.mut" "$TMP/wf.scratch"
if blocks_equal "$TMP/lib.scratch" "$TMP/wf.scratch"; then
    fail "byte-equality gate did NOT detect a planted mutation inside the block"
fi
cp "$WF" "$TMP/wf.scratch"
blocks_equal "$TMP/lib.scratch" "$TMP/wf.scratch" || fail "restore did not heal the byte-equality gate"
pass "drift detector fails on a planted mutation and heals on restore"

# --- 3. STATIC INVARIANTS ----------------------------------------------------
say "3. Static invariants on the workflow source (AC-1 / AC-4 / AC-5 / AC-6)"

# AC-1: exactly ONE nested workflow('dispatch-phase') call (the one allowed level
# of nesting), and dispatch-phase.js nests NO further workflow() call.
NWF=$(grep -c "workflow('dispatch-phase'" "$WF" || true)
[ "$NWF" -eq 1 ] || fail "AC-1: expected exactly one workflow('dispatch-phase') call in autopilot.js, found $NWF"
printf "workflow('dispatch-phase', a)\nworkflow('dispatch-phase', b)\n" >"$TMP/planted-two.js"
[ "$(grep -c "workflow('dispatch-phase'" "$TMP/planted-two.js")" -eq 2 ] || fail "AC-1 detector broken — count missed a planted second call"
if grep -n 'workflow(' "$DISPATCH_WF" >/dev/null 2>&1; then
    grep -n 'workflow(' "$DISPATCH_WF" >&2 || true
    fail "AC-1: dispatch-phase.js must nest no workflow() call (no deeper nesting under autopilot)"
fi
pass "AC-1: exactly one nested workflow('dispatch-phase'); dispatch-phase nests none"

# AC-3-equivalent: the runtime forbids imports; sharing is by stamped copy.
if grep -nE '(^|[^A-Za-z_])import[ (]' "$WF" >/dev/null 2>&1; then
    grep -nE '(^|[^A-Za-z_])import[ (]' "$WF" >&2 || true
    fail "autopilot.js must not import (the runtime forbids it — sharing is by stamped copy)"
fi
if grep -nE '(^|[^A-Za-z_])require\(' "$WF" >/dev/null 2>&1; then
    fail "autopilot.js must not require() (the runtime forbids it)"
fi
grep -q '>>> autopilot-loop:begin' "$WF" || fail "missing autopilot-loop:begin marker"
grep -q '>>> autopilot-loop:end' "$WF" || fail "missing autopilot-loop:end marker"
printf "import x from 'y'\n" >"$TMP/planted-import.js"
grep -qE '(^|[^A-Za-z_])import[ (]' "$TMP/planted-import.js" || fail "import detector broken"
pass "no import/require; both autopilot-loop markers present; detectors catch planted ones"

# AC-6 / status persistence: the advance prompt interpolates the OUTCOME-supplied
# status (the canonical review's status mapping now has exactly one home, in
# lib/review.mjs — autopilot must NOT restate it as a literal) and a park prompt
# writes --status blocked. This is what drives the loop off PERSISTED status.
grep -qF -- "--status ' + advanceStatus" "$WF" ||
    fail "expected the advance prompt to interpolate the OUTCOME-supplied status (--status ' + advanceStatus) in autopilot.js"
grep -q -- '--status blocked' "$WF" || fail "expected a '--status blocked' park write in autopilot.js"
# Self-test: a hardcoded advance status must break the detector.
sed "s/--status ' + advanceStatus/--status reviewed'/" "$WF" >"$TMP/planted-hardcoded.js"
if grep -qF -- "--status ' + advanceStatus" "$TMP/planted-hardcoded.js"; then
    fail "advance-status detector broken — a planted hardcoded status was not detected"
fi

# NO source string lands, merges, or mutates main. (Prompt-scoped forbiddens like
# --land / --commit are asserted over the built prompt strings in Section 1's node
# sweep; here we forbid true main-mutation verbs anywhere in the source, since
# autopilot NEVER touches main.)
#
# *** DELIBERATE INVARIANT INVERSION (unify-code-review phase 6) ***
# The land-time completion trailer used to be forbidden ANYWHERE in this file — an
# absolute whole-file rule. Phase 4 moved the write of that trailer to LAND time
# (`rdm-land` synthesizes it from the OUTCOME identifiers via `rdm hook
# done-line`), so the interesting invariant changed shape:
#   * still absolutely forbidden in every BUILT PROMPT — autopilot must never ask
#     an agent to write the trailer itself (Section 1's FORBIDDEN sweep, unchanged);
#   * still forbidden in autopilot's own CODE — no literal may be constructed here;
#   * now ALLOWED in explanatory COMMENTS, so this file may name the land-time
#     writer and explain who owns the trailer.
# The absence assertion is therefore SCOPED to non-comment lines, and it is paired
# with the new positive land-time regression in Section 6 below, which proves a
# trailer-less autopilot branch actually gains the trailer with no manual rebase.
for forbidden in 'git merge' 'git push' 'checkout main'; do
    if grep -qF -- "$forbidden" "$WF"; then
        fail "AC-6: autopilot.js must not contain a main-mutation/land string: $forbidden"
    fi
done
noncomment_lines() { grep -vE '^[[:space:]]*(//|\*|/\*)' "$1"; }
if noncomment_lines "$WF" | grep -qF -- 'Done:'; then
    noncomment_lines "$WF" | grep -nF -- 'Done:' >&2 || true
    fail "AC-6: autopilot.js CODE must not construct a land-time completion directive (comments may name it; rdm-land writes it)"
fi
printf 'git merge main\n' >"$TMP/planted-merge.js"
grep -qF -- 'git merge' "$TMP/planted-merge.js" || fail "main-mutation detector broken — grep missed a planted 'git merge'"
# Self-tests for the scoped trailer rule: it must FIRE on code and STAY SILENT on a comment.
cp "$WF" "$TMP/planted-trailer-code.js"
printf '\nconst bad = "Done: rm/phase-1-x"\n' >>"$TMP/planted-trailer-code.js"
noncomment_lines "$TMP/planted-trailer-code.js" | grep -qF -- 'Done:' ||
    fail "scoped trailer detector broken — a trailer planted in CODE was not detected"
cp "$WF" "$TMP/planted-trailer-comment.js"
printf '\n// rdm-land amends the Done: trailer at land time.\n' >>"$TMP/planted-trailer-comment.js"
if noncomment_lines "$TMP/planted-trailer-comment.js" | grep -qF -- 'Done:'; then
    fail "scoped trailer detector is over-broad — an explanatory COMMENT naming the trailer must not be flagged"
fi
pass "AC-6: advance interpolates the OUTCOME status; park writes blocked; no main-mutation string; trailer forbidden in code, allowed in prose; detectors fire correctly"

# StructuredOutput schema shape: NO top-level *_SCHEMA handed to agent() may
# declare `type: 'array'`. Anthropic custom tools require input_schema.type ===
# 'object'; a top-level array 400s the StructuredOutput tool (this is the bug
# that silently no-op'd autopilot's estimate pre-pass — every [estimate:list]
# call 400'd, so no difficulty/model tiers persisted). The detector anchors to
# the TOP-LEVEL type ONLY: it reads the line immediately following each
# `const NAME_SCHEMA = {` header, so legitimately-nested `type: 'array'`
# properties (unmet, tags, phases) are never flagged. Formatting assumption:
# each schema is written as `const NAME_SCHEMA = {` with its top-level `type` on
# the immediately following line (NEXT/PHASE_LIST/ESTIMATE/ACK all follow this
# shape); a reflowed single-line definition would evade the grep.
schema_array_offenders() {
    awk '
        /^const [A-Za-z_]+_SCHEMA = \{/ { name = $2; expect = 1; next }
        expect == 1 { if ($0 ~ /type: .array./) print name; expect = 0 }
    ' "$1"
}
OFFENDERS=$(schema_array_offenders "$WF" || true)
if [ -n "$OFFENDERS" ]; then
    printf 'top-level type:array schema(s): %s\n' "$(echo "$OFFENDERS" | tr '\n' ' ')" >&2
    fail "no *_SCHEMA handed to agent() may use a top-level type:'array' (Anthropic tools require 'object'); offending: $OFFENDERS"
fi
# The unwrap half: estimateList must peel the PHASE_LIST_SCHEMA wrapper so the
# in-block selectUnestimated still receives a plain array.
grep -q 'r.phases' "$WF" || fail "estimateList must unwrap the PHASE_LIST_SCHEMA wrapper (expected 'r.phases' in autopilot.js)"
# Planted-mutation self-test: flip every top-level `type: 'object'` back to
# 'array' in a scratch copy and confirm the detector fires (proving it is not a
# no-op).
sed "s/^  type: 'object',/  type: 'array',/" "$WF" >"$TMP/wf.array.scratch"
if [ -z "$(schema_array_offenders "$TMP/wf.array.scratch")" ]; then
    fail "top-level-array detector did NOT fire on a planted type:'array' schema"
fi
pass "no *_SCHEMA uses a top-level type:'array'; estimateList unwraps r.phases; detector catches a planted array schema"

# meta.phases must list EXACTLY the distinct emitted `phase:` literals.
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
sed "s/phase: 'Fetch',/phase: 'Ghost',/" "$WF" >"$TMP/wf.phase.scratch"
if [ "$(declared_phases "$TMP/wf.phase.scratch")" = "$(emitted_phases "$TMP/wf.phase.scratch")" ]; then
    fail "meta.phases consistency check did NOT catch a planted undeclared phase"
fi
pass "meta.phases consistency detector catches a planted undeclared phase"

# --- 3b. AC-MODEL --------------------------------------------------------------
say "3b. AC-MODEL: every agent() call in the five mechanical dep functions carries an explicit model: (bootstrap exempt)"

# Scoped to ONLY the six named realDeps functions that matter here: the five
# mechanical deps (fetchNext, estimateList, estimateWriteback, advance, park)
# plus the resolveMechanicalModel bootstrap (needed so its whitelisted call is
# visible to the extractor at all). This deliberately EXCLUDES parallelEstimate
# (the difficulty-rating JUDGMENT agent, which legitimately carries no
# `model:`) and dispatch (which calls workflow(), not agent()) — a whole-file
# sweep would wrongly flag parallelEstimate's agent() call as a violation.
extract_mechanical_dep_fns() {
    awk '
        /^  (resolveMechanicalModel|estimateList|estimateWriteback|fetchNext|advance|park): async function/ { collect = 1 }
        collect { print }
        collect && /^  \},$/ { collect = 0 }
    ' "$1"
}

extract_mechanical_dep_fns "$WF" >"$TMP/mech-dep-fns"
[ -s "$TMP/mech-dep-fns" ] || fail "AC-MODEL: could not extract any of the five mechanical dep functions from $WF"
# Sanity: all six named functions must actually appear (a rename would silently
# shrink this to nothing useful).
for fn in resolveMechanicalModel estimateList estimateWriteback fetchNext advance park; do
    grep -q "^  $fn: async function" "$TMP/mech-dep-fns" || fail "AC-MODEL: expected to find '$fn: async function' in the extracted region"
done
pass "AC-MODEL: extracted all six named functions (resolveMechanicalModel + the five mechanical deps)"

# Same generic agent()-option-block extractor as verify-workflow-dispatch.sh,
# applied to the scoped region above.
mechanical_agent_option_blocks() {
    awk '
      /^[[:space:]]*\/\// { next }
      !inblk && index($0, "agent(") { pending = 1 }
      pending && index($0, "{") { inblk = 1; pending = 0; buf = $0; next }
      inblk { buf = buf "\n" $0 }
      inblk && /^[[:space:]]*\}\)/ { print buf "\n---END---"; inblk = 0; buf = "" }
    ' "$1"
}
mechanical_agent_call_count() {
    grep -vE '^[[:space:]]*//' "$1" | grep -cE '(^|[^A-Za-z_])_?agent\('
}

mechanical_agent_option_blocks "$TMP/mech-dep-fns" >"$TMP/mech-agent-blocks"
[ -s "$TMP/mech-agent-blocks" ] || fail "AC-MODEL: could not extract any agent() option blocks from the mechanical dep functions"

EXTRACTED_MECH=$(grep -c -- '---END---' "$TMP/mech-agent-blocks")
CALLSITES_MECH=$(mechanical_agent_call_count "$TMP/mech-dep-fns")
[ "$EXTRACTED_MECH" -eq "$CALLSITES_MECH" ] ||
    fail "AC-MODEL: extracted $EXTRACTED_MECH option blocks but found $CALLSITES_MECH agent() call sites among the mechanical deps — the sweep is blind to at least one"
pass "AC-MODEL: extracted one option block per agent() call site among the six mechanical/bootstrap functions ($EXTRACTED_MECH)"

# Every block must carry `model:` unless it is the whitelisted bootstrap fetch
# (label 'model:mechanical' — the call that PRODUCES the mechanical model id).
assert_mechanical_model_sweep() {
    awk '
      BEGIN { RS = "---END---"; bad = 0 }
      /label:/ {
        if ($0 ~ /label: .model:mechanical./) next   # whitelisted bootstrap
        if ($0 !~ /model:/) { bad++ }
      }
      END { exit (bad > 0) }
    ' "$1"
}
assert_mechanical_model_sweep "$TMP/mech-agent-blocks" ||
    fail "AC-MODEL: a non-bootstrap agent() call among the five mechanical deps is missing an explicit model:"
pass "AC-MODEL: every mechanical dep's agent() call carries an explicit model: (resolveMechanicalModel bootstrap whitelisted)"

# Self-test: strip one model: key and prove the sweep fails.
sed '/model: model,/d' "$TMP/mech-agent-blocks" >"$TMP/mech-agent-blocks-mutant"
if assert_mechanical_model_sweep "$TMP/mech-agent-blocks-mutant"; then
    fail "AC-MODEL: sweep missed a removed model: key among the mechanical deps"
fi
pass "AC-MODEL: sweep detector fires when a model: key is removed from a mechanical dep"

# --- 4. MODULE PARSE ---------------------------------------------------------
say "4. Module parse: autopilot.js loads under module semantics (no SyntaxError)"

if parse_workflow "$WF" >/dev/null 2>&1; then
    pass "autopilot.js parses under module semantics (top-level meta declared once)"
else
    parse_workflow "$WF" >&2 || true
    fail "autopilot.js does NOT parse — fix the SyntaxError (e.g. a duplicate top-level 'meta')"
fi

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

# --- 5. SIBLING GATE ---------------------------------------------------------
say "5. Sibling gate: dispatch-phase harness stays green after the planOnly edit"
if bash "$SCRIPT_DIR/verify-workflow-dispatch.sh" >/dev/null 2>&1; then
    pass "verify-workflow-dispatch.sh still green (planOnly early-return did not regress it)"
else
    bash "$SCRIPT_DIR/verify-workflow-dispatch.sh" >&2 || true
    fail "verify-workflow-dispatch.sh regressed — the dispatch-phase planOnly edit broke a dispatch invariant"
fi

# --- 6. LAND-TIME COMPLETION TRAILER -----------------------------------------
# The POSITIVE half of the inverted invariant above. Autopilot leaves a reviewed
# phase's branch commit WITHOUT a completion trailer; `rdm-land` is the land-time
# writer. This drives the exact documented rdm-land sequence against the REAL
# binary in a hermetic temp plan + source repo, and asserts the trailer arrives
# with NO rebase and no interactive step, and that the merge hook then completes
# the item.
say "6. Land-time completion trailer: a trailer-less autopilot branch gains it with no rebase"

RDM_BIN="$REPO_ROOT/target/debug/rdm"
[ -x "$RDM_BIN" ] || fail "$RDM_BIN not found or not executable — run 'cargo build' first."

# Hermetic HOME + XDG + git identity so neither rdm nor git touches the real
# developer/CI environment.
export HOME="$TMP/home"
export XDG_CONFIG_HOME="$TMP/xdg-config"
export XDG_DATA_HOME="$TMP/xdg-data"
export XDG_STATE_HOME="$TMP/xdg-state"
mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_STATE_HOME"
export GIT_AUTHOR_NAME="verify-bot"
export GIT_AUTHOR_EMAIL="verify@example.invalid"
export GIT_COMMITTER_NAME="verify-bot"
export GIT_COMMITTER_EMAIL="verify@example.invalid"

PLAN="$TMP/plan"
SRC="$TMP/src"

rdm_plan() (
    RDM_ROOT="$PLAN" "$RDM_BIN" "$@"
)

rdm_plan init --default-project verify >/dev/null
rdm_plan roadmap create rm --title "RM" --body "Land-time trailer regression roadmap." \
    --no-edit --project verify >/dev/null
rdm_plan phase create x --title "Phase X" --number 1 --body "Phase X." \
    --no-edit --roadmap rm --project verify >/dev/null
# Exactly the state autopilot leaves behind: the phase advanced to `reviewed`,
# the work committed on the roadmap branch, nothing landed.
rdm_plan phase update phase-1-x --status reviewed --no-edit --roadmap rm --project verify >/dev/null
rdm_plan commit -m "chore(plan): seed rm/phase-1-x as reviewed" >/dev/null
pass "seeded hermetic plan repo: rm/phase-1-x is reviewed"

# Source repo: a roadmap branch whose tip is the un-pushed reviewed commit with a
# message carrying NO trailer (exactly what a dispatch/autopilot implementer leaves).
mkdir -p "$SRC"
git -C "$SRC" init -q -b main
printf 'seed\n' >"$SRC/README.md"
git -C "$SRC" add README.md
git -C "$SRC" commit -qm "chore: seed"
git -C "$SRC" checkout -q -b roadmap/rm
printf 'work\n' >"$SRC/feature.txt"
git -C "$SRC" add feature.txt
git -C "$SRC" commit -qm "feat: implement phase X"
git -C "$SRC" log -1 --pretty=%B | grep -qF 'Done:' &&
    fail "setup is wrong: the autopilot-shaped commit must start WITHOUT a completion trailer"
pass "roadmap/rm tip is a reviewed, trailer-less, un-pushed commit"

# The documented rdm-land precondition-2 synthesis: ask rdm for the trailer (the
# format string has exactly one home) and amend it onto the branch tip. No
# rebase, no interactive editor.
DONE_LINE=$(rdm_plan hook done-line --roadmap rm --phase phase-1-x) ||
    fail "rdm hook done-line failed — the land path must abort rather than amend an empty trailer"
[ -n "$DONE_LINE" ] || fail "rdm hook done-line printed nothing"
ORIG_MSG=$(git -C "$SRC" log -1 --pretty=%B)
PRE_AMEND_BASE=$(git -C "$SRC" rev-parse HEAD~1)
printf '%s\n\n%s\n' "$ORIG_MSG" "$DONE_LINE" >"$TMP/amend-msg"
GIT_EDITOR=true git -C "$SRC" commit -q --amend -F "$TMP/amend-msg"

git -C "$SRC" log -1 --pretty=%B | grep -qF 'Done: rm/phase-1-x' ||
    fail "the amended commit must carry 'Done: rm/phase-1-x'; got: $(git -C "$SRC" log -1 --pretty=%B)"
pass "the branch tip now carries the completion trailer, synthesized by rdm hook done-line"

# No rebase was needed: the amend preserved the parent commit, and the branch
# still has exactly the same two-commit shape.
[ "$(git -C "$SRC" rev-parse HEAD~1)" = "$PRE_AMEND_BASE" ] ||
    fail "the amend must not have rewritten history below the tip — no rebase is permitted"
[ "$(git -C "$SRC" rev-list --count main..HEAD)" -eq 1 ] ||
    fail "roadmap/rm must still be exactly one commit ahead of main (no rebase, no extra commits)"
[ -z "$(git -C "$SRC" rev-parse -q --verify REBASE_HEAD 2>/dev/null || true)" ] ||
    fail "a rebase was started — the land-time trailer must need none"
[ ! -d "$SRC/.git/rebase-merge" ] && [ ! -d "$SRC/.git/rebase-apply" ] ||
    fail "a rebase directory exists — the land-time trailer must need no rebase"
pass "no rebase and no interactive step were required"

# Land it: fast-forward main, then run the merge-to-main hook. The trailer the
# lander synthesized is what flips the phase reviewed -> done.
git -C "$SRC" checkout -q main
git -C "$SRC" merge -q --ff-only roadmap/rm
LANDED_SHA=$(git -C "$SRC" rev-parse HEAD)
(cd "$SRC" && "$RDM_BIN" --root "$PLAN" hook post-commit) ||
    fail "rdm hook post-commit failed on the landed commit"
PHASE_JSON=$(rdm_plan phase show phase-1-x --roadmap rm --project verify --format json --no-body)
printf '%s' "$PHASE_JSON" | grep -qF '"status": "done"' ||
    fail "the landed trailer must flip rm/phase-1-x to done; got: $PHASE_JSON"
printf '%s' "$PHASE_JSON" | grep -qF "$LANDED_SHA" ||
    fail "the completed phase must record the landed commit SHA $LANDED_SHA; got: $PHASE_JSON"
pass "rdm hook post-commit flipped rm/phase-1-x to done and recorded the landed SHA"

# Negative: `rdm hook done-line` rejects a malformed request, so the land path
# aborts instead of amending an empty trailer.
if rdm_plan hook done-line --roadmap rm >/dev/null 2>&1; then
    fail "rdm hook done-line must reject a request with neither --phase nor --task"
fi
if rdm_plan hook done-line --roadmap rm --phase phase-1-x --task t >/dev/null 2>&1; then
    fail "rdm hook done-line must reject both --phase and --task together"
fi
pass "rdm hook done-line rejects malformed requests, so the lander aborts rather than amending an empty trailer"

say "verify-workflow-autopilot.sh: ALL GREEN"
