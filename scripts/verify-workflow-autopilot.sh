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
# docs/workflow-schemas.md § "Import spike"). This harness gates four things:
#
#   1. BEHAVIOR   — the pure helpers, driven in Node (zero LLM calls): arg
#                   parsing, phase selection, outcome interpretation, tier
#                   resolution, prompt contents, the batched summary, and
#                   determinism.
#   1b. DRIVEN LOOP — buildAutopilot fed state-backed fakes (a mutable status
#                   Map): drive-to-reviewed, rework->park, escalated, budget
#                   stops, the estimate pre-pass, --plan-only, and mid-tier
#                   defaulting — asserting the loop advances off PERSISTED status.
#   2. BLOCK DRIFT — the `autopilot-loop` region is byte-identical between the lib
#                   source of truth and the stamped workflow script (with a
#                   planted-mutation self-test proving the gate is not a no-op).
#   3. STATIC INVARIANTS — grep-based assertions: exactly one nested
#                   workflow('dispatch-phase') call; dispatch-phase.js nests none;
#                   no import/require; both markers; advance/park status writes; no
#                   land/merge/main-mutation prompt string; no *_SCHEMA handed to
#                   agent() uses a top-level type:'array' (Anthropic tools require
#                   'object') with a planted-mutation self-test; meta.phases parity.
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
  selectUnestimated,
  difficultyToTier,
  resolveTier,
  interpretNext,
  buildParkReason,
  interpretOutcome,
  stepBudgetExhausted,
  maxPhasesReached,
  buildFetchNextPrompt,
  buildEstimateListPrompt,
  buildEstimatorPrompt,
  buildEstimateWritebackPrompt,
  buildAdvancePrompt,
  buildParkPrompt,
  buildSummary,
} = m;

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
  },
  'defaults (both dispatch-phase budget overrides unset)'
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

// --- selectUnestimated -------------------------------------------------------
assert.deepEqual(
  selectUnestimated([
    { stem: 'a' },
    { stem: 'b', difficulty: 'hard' },
    { stem: 'c', model: 'small' },
    { stem: 'd', difficulty: 'easy', model: 'small' },
  ]),
  ['a'],
  'only phases with NO difficulty and NO model are unestimated'
);
assert.deepEqual(selectUnestimated([]), [], 'empty list');
assert.deepEqual(selectUnestimated(null), [], 'non-array tolerated');

// --- difficultyToTier --------------------------------------------------------
assert.equal(difficultyToTier('trivial'), 'small');
assert.equal(difficultyToTier('easy'), 'small');
assert.equal(difficultyToTier('moderate'), 'medium');
assert.equal(difficultyToTier('hard'), 'large');
assert.equal(difficultyToTier('bogus'), 'medium', 'unknown difficulty defaults medium');

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
const allPrompts = [
  buildFetchNextPrompt('rm'),
  buildEstimateListPrompt('rm'),
  buildEstimatorPrompt('a phase body'),
  buildEstimateWritebackPrompt('phase-1-x', 'hard', 'rm'),
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
const { buildAutopilot, DEFAULT_MAX_REWORK } = m;

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

  const callLog = [];
  const advanceCalls = [];
  const parkCalls = [];
  const dispatchCalls = [];
  const parallelEstimateCalls = [];
  const writebackCalls = [];

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
    estimateList: async () => {
      callLog.push('estimateList');
      if (o.estimateList) return o.estimateList;
      // Default: fully estimated so the pre-pass is skipped.
      return o.phases.map((p) => ({ stem: p.stem, status: p.status, difficulty: 'moderate', model: 'medium' }));
    },
    parallelEstimate: async (unestimated) => {
      parallelEstimateCalls.push(unestimated.slice());
      callLog.push('parallelEstimate');
      return o.estimates || [];
    },
    estimateWriteback: async (stem, difficulty) => {
      writebackCalls.push({ stem, difficulty });
      callLog.push('writeback:' + stem);
    },
    fetchNext: async () => {
      callLog.push('fetchNext');
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
    advance: async (stem) => {
      advanceCalls.push(stem);
      callLog.push('advance:' + stem);
      statusMap.set(stem, 'reviewed');
      return { ok: true };
    },
    park: async (stem, reason) => {
      parkCalls.push({ stem, reason });
      callLog.push('park:' + stem);
      statusMap.set(stem, 'blocked');
      return { ok: true };
    },
  };
  return { fakes, statusMap, callLog, advanceCalls, parkCalls, dispatchCalls, parallelEstimateCalls, writebackCalls };
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
      { stem: 'phase-1-a', difficulty: 'easy' },
      { stem: 'phase-2-b', difficulty: 'moderate' },
    ],
  });
  await buildAutopilot(h.fakes)({ roadmap: 'rm', globalBudget: 20 });
  assert.equal(h.parallelEstimateCalls.length, 1, 'parallelEstimate called exactly once');
  assert.deepEqual(h.parallelEstimateCalls[0].sort(), ['phase-1-a', 'phase-2-b'], 'rated exactly the 2 unestimated');
  assert.deepEqual(h.writebackCalls.map((w) => w.stem).sort(), ['phase-1-a', 'phase-2-b'], 'writeback per rated stem');
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

console.log('all autopilot driven-loop assertions passed');
NODE_TEST

if run_node "$TMP/driven.mjs" "$LIB" "$DISPATCH_LIB"; then
    pass "loop drives to reviewed / parks / budgets / pre-pass / plan-only / mid-tier / budget passthrough"
else
    fail "autopilot driven-loop assertions failed"
fi

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
