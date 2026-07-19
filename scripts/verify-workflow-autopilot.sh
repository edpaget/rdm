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
assert.deepEqual(base, { roadmap: 'rm', maxPhases: null, planOnly: false, globalBudget: DEFAULT_GLOBAL_BUDGET }, 'defaults');
assert.ok(!('land' in base), 'parsed config never carries a land flag');
assert.equal(parseAutopilotArgs({ roadmap: 'rm', maxPhases: 3 }).maxPhases, 3, 'maxPhases int');
assert.equal(parseAutopilotArgs({ roadmap: 'rm', maxPhases: '2' }).maxPhases, 2, 'maxPhases coerced from string');
assert.throws(() => parseAutopilotArgs({ roadmap: 'rm', maxPhases: 0 }), /positive integer/, 'maxPhases 0 rejected');
assert.throws(() => parseAutopilotArgs({ roadmap: 'rm', maxPhases: -1 }), /positive integer/, 'maxPhases negative rejected');
assert.equal(parseAutopilotArgs({ roadmap: 'rm', planOnly: true }).planOnly, true, 'planOnly boolean');
assert.equal(parseAutopilotArgs({ roadmap: 'rm', globalBudget: 5 }).globalBudget, 5, 'globalBudget override');

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
assert.deepEqual(
  interpretOutcome('reviewed', { planOnly: false, reworkCount: 0, maxRework: 1 }),
  { action: 'advance' },
  'reviewed + normal -> advance'
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

console.log('all autopilot driven-loop assertions passed');
NODE_TEST

if run_node "$TMP/driven.mjs" "$LIB"; then
    pass "loop drives to reviewed / parks / budgets / pre-pass / plan-only / mid-tier off persisted status"
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

# AC-6 / status persistence: an advance prompt writes --status reviewed and a park
# prompt writes --status blocked (this is what drives the loop off PERSISTED
# status); and NO source string lands, merges, or mutates main. (Prompt-scoped
# forbiddens like --land / --commit are asserted over the built prompt strings in
# Section 1's node sweep; here we forbid true main-mutation verbs anywhere in the
# source, since autopilot NEVER touches main.)
grep -q -- '--status reviewed' "$WF" || fail "expected a '--status reviewed' advance write in autopilot.js"
grep -q -- '--status blocked' "$WF" || fail "expected a '--status blocked' park write in autopilot.js"
for forbidden in 'git merge' 'git push' 'checkout main' 'Done:'; do
    if grep -qF -- "$forbidden" "$WF"; then
        fail "AC-6: autopilot.js must not contain a main-mutation/land string: $forbidden"
    fi
done
printf 'git merge main\n' >"$TMP/planted-merge.js"
grep -qF -- 'git merge' "$TMP/planted-merge.js" || fail "main-mutation detector broken — grep missed a planted 'git merge'"
pass "AC-6: advance/park status writes present; no git-merge/push/checkout-main/Done string; detector catches a planted one"

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

say "verify-workflow-autopilot.sh: ALL GREEN"
