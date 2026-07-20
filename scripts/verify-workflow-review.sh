#!/bin/sh
# Hermetic regression for the review-refute-fix shared workflow pipeline.
#
# review-refute-fix is the single-source-of-truth review pipeline in
# `.claude/workflows/lib/review-refute-fix.mjs`, stamped into workflow-script
# consumers by `scripts/gen-workflow-review.sh` (the Workflow runtime cannot
# import a helper module — see docs/workflow-schemas.md § "Import spike"). This
# harness gates three things so a refactor can't silently break the autonomous
# review lane:
#
#   1. DRIFT   — every consumer is in sync with the source block (gen --check).
#   2. HYGIENE — no forbidden nondeterministic global (Date.now / Math.random)
#                creeps into a workflow script (the runtime forbids them).
#   3. BEHAVIOR — the pure pipeline logic, driven in Node with an injected fake
#                 agent + reference pipeline/parallel (zero LLM calls):
#                   * a planted refutable finding is dropped, a planted real one
#                     survives, and a not-refuted-but-low-confidence finding is
#                     dropped by the confidence floor;
#                   * a FRESH refuter grades each finding (separate agent call —
#                     the finder never grades its own work);
#                   * the review target (context) is threaded into every prompt;
#                   * output is deterministic across runs (total-order ranking);
#                   * BOTH `code` and `plan` dimension sets work behind `mode`.
#
# Node is used only as a host to unit-test the shared module's pure logic; it is
# stdlib-only (node:assert), with no package.json / node_modules / third-party
# packages. node is pinned in .mise.toml.
#
# Requires: node (via PATH or `mise exec node --`).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)

LIB="$REPO_ROOT/.claude/workflows/lib/review-refute-fix.mjs"
GEN="$REPO_ROOT/scripts/gen-workflow-review.sh"
WF_DIR="$REPO_ROOT/.claude/workflows"

# Clear rdm-related env vars inherited from the caller's shell for hermeticity.
unset RDM_ROOT RDM_PROJECT RDM_STAGE RDM_FORMAT RDM_PLAN_REPO RDM_PLAN_REPO_TOKEN RDM_PLAN_REPO_PATH 2>/dev/null || true

say() { printf '\n\033[1;34m==>\033[0m %s\n' "$*"; }
fail() {
    printf '\n\033[1;31m[FAIL]\033[0m %s\n' "$*" >&2
    exit 1
}
pass() { printf '\033[1;32m[ok]\033[0m %s\n' "$*"; }

[ -f "$LIB" ] || fail "source module not found: $LIB"
[ -f "$GEN" ] || fail "generator not found: $GEN"

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

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

# --- 1. DRIFT ----------------------------------------------------------------
say "1. Drift: every consumer is in sync with the source block"
if sh "$GEN" --check; then
    pass "gen-workflow-review.sh --check clean"
else
    fail "a workflow consumer drifted from $LIB — run scripts/gen-workflow-review.sh"
fi

# --- 1b. DRIFT DETECTOR SELF-TEST --------------------------------------------
# Prove the drift gate is not a no-op: on a hermetic scratch copy, a mutation
# inside the generated block MUST make --check fail, and regeneration MUST heal
# it. Without this, a future regression to gen-workflow-review.sh (inverted exit
# code, a consumer list that stops matching the real file) would silently pass
# whenever the real tree happens to be clean.
say "1b. Drift detector fires on planted drift (self-test)"
SCRATCH="$TMP/scratch"
mkdir -p "$SCRATCH/scripts" "$SCRATCH/.claude/workflows/lib"
cp "$GEN" "$SCRATCH/scripts/gen-workflow-review.sh"
cp "$LIB" "$SCRATCH/.claude/workflows/lib/review-refute-fix.mjs"
cp "$WF_DIR/review-refute-fix.js" "$SCRATCH/.claude/workflows/review-refute-fix.js"
# gen-workflow-review.sh lists every consumer; the scratch tree must carry them
# all or the scratch --check fails on a missing consumer rather than on drift.
cp "$WF_DIR/dispatch-phase.js" "$SCRATCH/.claude/workflows/dispatch-phase.js"
sh "$SCRATCH/scripts/gen-workflow-review.sh" --check >/dev/null 2>&1 ||
    fail "scratch --check should pass on a clean copy"
# Mutate a line INSIDE the generated block, portably (no in-place sed).
sed 's/const CONFIDENCE_FLOOR = 70;/const CONFIDENCE_FLOOR = 999;/' \
    "$SCRATCH/.claude/workflows/review-refute-fix.js" >"$SCRATCH/mutated" &&
    mv "$SCRATCH/mutated" "$SCRATCH/.claude/workflows/review-refute-fix.js"
if sh "$SCRATCH/scripts/gen-workflow-review.sh" --check >/dev/null 2>&1; then
    fail "drift gate did NOT detect planted drift in the scratch consumer"
fi
sh "$SCRATCH/scripts/gen-workflow-review.sh" >/dev/null 2>&1
sh "$SCRATCH/scripts/gen-workflow-review.sh" --check >/dev/null 2>&1 ||
    fail "regeneration did not restore sync in the scratch consumer"
pass "drift detector fails on drift and heals on regenerate"

# --- 2. HYGIENE --------------------------------------------------------------
say "2. Hygiene: no forbidden nondeterministic global in workflow scripts"
if grep -nE 'Date\.now\(|Math\.random\(' "$WF_DIR"/*.js "$WF_DIR"/lib/*.mjs 2>/dev/null; then
    fail "found Date.now( / Math.random( in a workflow script — the runtime forbids them"
fi
# Self-test: the grep MUST catch a planted violation (guards against a glob that
# silently matches zero files, turning the check into a no-op).
printf 'const x = Date.now();\n' >"$SCRATCH/planted.js"
if ! grep -nE 'Date\.now\(|Math\.random\(' "$SCRATCH/planted.js" >/dev/null 2>&1; then
    fail "hygiene grep did NOT catch a planted Date.now() — the detector is broken"
fi
pass "no forbidden globals present; detector catches a planted one"

# --- 3. BEHAVIOR -------------------------------------------------------------
say "3. Behavior: find -> refute -> filter, both modes, deterministic"

cat >"$TMP/test.mjs" <<'NODE_TEST'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const libPath = process.argv[2];
const mod = await import(pathToFileURL(libPath).href);
const { buildReviewPipeline, DIMENSIONS, findPrompt, survives, rankFindings, CONFIDENCE_FLOOR } = mod;

// --- reference pipeline/parallel: faithful to the real Workflow runtime -------
// Both are order-preserving (Promise.all). Their ERROR semantics mirror the
// documented runtime contract, so the module's degraded-path behavior is
// actually exercised here:
//   parallel — "a thunk that throws resolves to null in the result array".
//   pipeline — "a stage that throws drops that item to null, skipping the rest".
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
          return null; // a throwing stage drops this item to null
        }
      }
      return acc;
    })
  );
}

// --- recording fake agent: returns planted data keyed by label, logs calls ----
// label is `find:<mode>:<dimKey>` or `refute:<mode>:<findingId>`.
function makeSpyAgent(plantFindings, plantVerdicts) {
  const calls = [];
  async function agent(prompt, opts) {
    const label = (opts && opts.label) || '';
    calls.push({ label, prompt });
    const parts = label.split(':');
    if (parts[0] === 'find') {
      return { findings: plantFindings[parts[2]] || [] };
    }
    if (parts[0] === 'refute') {
      const findingId = parts.slice(2).join(':');
      return plantVerdicts[findingId] || { refuted: false, confidence: 90 };
    }
    throw new Error('unexpected agent label: ' + label);
  }
  return { agent, calls };
}

function deps(spy) {
  return { agent: spy.agent, pipeline: refPipeline, parallel: refParallel, log: () => {} };
}

const CTX = { target: 'phase widget/phase-1-foo' };

// ============================================================================
// Pure unit checks — the survival rule and the total ordering.
// ============================================================================
assert.equal(CONFIDENCE_FLOOR, 70, 'confidence floor is 70');
assert.equal(survives({ confidence: 70 }, { refuted: false }), true, 'exactly at floor survives');
assert.equal(survives({ confidence: 69 }, { refuted: false }), false, 'below floor dropped');
assert.equal(survives({ confidence: 100 }, { refuted: true }), false, 'refuted dropped regardless of confidence');

const ranked = rankFindings([
  { id: 'b', severity: 'concern', confidence: 80 },
  { id: 'a', severity: 'concern', confidence: 80 },
  { id: 'c', severity: 'blocking', confidence: 10 },
  { id: 'd', severity: 'suggestion', confidence: 99 },
  { id: 'e', severity: 'concern', confidence: 90 },
]);
assert.deepEqual(
  ranked.map((f) => f.id),
  ['c', 'e', 'a', 'b', 'd'],
  'total order: blocking first; concerns by confidence desc then id; suggestion last'
);

// ============================================================================
// CODE mode — refutable dropped, low-confidence dropped, real survives.
// ============================================================================
const codeFindings = {
  ac: [],
  correctness: [
    { id: 'real-bug', concern: 'correctness', severity: 'blocking', confidence: 90, what_fails: 'off-by-one' },
    { id: 'false-alarm', concern: 'correctness', severity: 'concern', confidence: 85, what_fails: 'looks wrong but is fine' },
    { id: 'low-conf', concern: 'correctness', severity: 'suggestion', confidence: 50, what_fails: 'possible nit' },
  ],
  tests: [],
  architecture: [],
};
const codeVerdicts = {
  'real-bug': { refuted: false, confidence: 95 },
  'false-alarm': { refuted: true, confidence: 88 }, // refuter kills it
  'low-conf': { refuted: false, confidence: 40 }, // NOT refuted, but finding confidence < floor
};

const spy = makeSpyAgent(codeFindings, codeVerdicts);
const out = await buildReviewPipeline('code', deps(spy))(CTX);

assert.deepEqual(out.map((f) => f.id), ['real-bug'], 'code: only the real, un-refuted, high-confidence finding survives');

const findCalls = spy.calls.filter((c) => c.label.startsWith('find:'));
const refuteCalls = spy.calls.filter((c) => c.label.startsWith('refute:'));
assert.equal(findCalls.length, 4, 'one finder per code dimension');
assert.equal(refuteCalls.length, 3, 'a fresh refuter per finding');
assert.equal(new Set(findCalls.map((c) => c.label)).size, 4, 'finder labels are distinct per dimension');
// A fresh refuter grades each finding: every refuter call is distinctly labelled
// (no collisions even on duplicate ids), and no refuter reuses a finder's prompt.
assert.equal(new Set(refuteCalls.map((c) => c.label)).size, refuteCalls.length, 'refuter labels are unique per finding');
const findPromptSet = new Set(findCalls.map((c) => c.prompt));
assert.ok(refuteCalls.every((c) => !findPromptSet.has(c.prompt)), 'refuter prompts differ from finder prompts');
assert.ok(findCalls.every((c) => c.prompt.includes(CTX.target)), 'context.target threaded into finder prompts');
assert.ok(refuteCalls.every((c) => c.prompt.includes(CTX.target)), 'context.target threaded into refuter prompts');

// OUTCOME shape sanity (matches the FINDING contract in docs/workflow-schemas.md).
assert.ok(Array.isArray(out), 'OUTCOME is an array');
for (const f of out) {
  assert.equal(typeof f.id, 'string');
  assert.ok(['blocking', 'concern', 'suggestion'].includes(f.severity));
  assert.equal(typeof f.confidence, 'number');
}

// Determinism: two independent runs produce byte-identical output.
const outA = await buildReviewPipeline('code', deps(makeSpyAgent(codeFindings, codeVerdicts)))(CTX);
const outB = await buildReviewPipeline('code', deps(makeSpyAgent(codeFindings, codeVerdicts)))(CTX);
assert.equal(JSON.stringify(outA), JSON.stringify(outB), 'code review output is deterministic across runs');

// ============================================================================
// Resilience — a single agent failure degrades gracefully, never crashes.
// ============================================================================
// A finder that throws drops ONLY its dimension (the runtime pipeline sends a
// thrown stage to null); the other dimensions still complete. Here the only
// planted findings live in `correctness`, so a thrown `correctness` finder
// yields zero survivors WITHOUT rejecting the whole review.
{
  const spyF = makeSpyAgent(codeFindings, codeVerdicts);
  const base = spyF.agent;
  spyF.agent = async (prompt, opts) => {
    if (opts && opts.label === 'find:code:correctness') throw new Error('boom finder');
    return base(prompt, opts);
  };
  const rOut = await buildReviewPipeline('code', deps(spyF))(CTX);
  assert.deepEqual(rOut, [], 'a thrown finder drops its dimension; others survive; no crash');
}

// A refuter that throws must NOT silently drop the finding — a crash is not proof
// of refutation. The finding is kept as un-refuted and survives if confidence ≥
// floor (locks in the module's refuter-error .catch).
{
  const realOnly = {
    ac: [],
    correctness: [{ id: 'infra', concern: 'correctness', severity: 'blocking', confidence: 90, what_fails: 'real bug' }],
    tests: [],
    architecture: [],
  };
  const spyR = makeSpyAgent(realOnly, {});
  const base = spyR.agent;
  spyR.agent = async (prompt, opts) => {
    if (opts && opts.label.startsWith('refute:')) throw new Error('boom refuter');
    return base(prompt, opts);
  };
  const rOut = await buildReviewPipeline('code', deps(spyR))(CTX);
  assert.deepEqual(rOut.map((f) => f.id), ['infra'], 'a refuter crash keeps the finding un-refuted, not silently dropped');
}

// ============================================================================
// PLAN mode — same battery on the plan dimension set (thin variation).
// ============================================================================
assert.deepEqual(
  DIMENSIONS.plan.map((d) => d.key),
  ['coherence', 'architectural-fit', 'unit-of-work'],
  'plan dimension set'
);

const planFindings = {
  coherence: [
    { id: 'vague-step', concern: 'coherence', severity: 'blocking', confidence: 88, what_fails: 'step 3 is ambiguous' },
    { id: 'nonissue', concern: 'coherence', severity: 'concern', confidence: 80, what_fails: 'reads odd but is fine' },
    { id: 'weak-plan', concern: 'coherence', severity: 'suggestion', confidence: 50, what_fails: 'minor plan nit' },
  ],
  'architectural-fit': [],
  'unit-of-work': [],
};
const planVerdicts = {
  'vague-step': { refuted: false, confidence: 90 },
  nonissue: { refuted: true, confidence: 85 }, // refuter kills it
  'weak-plan': { refuted: false, confidence: 40 }, // NOT refuted, but below floor
};

const pspy = makeSpyAgent(planFindings, planVerdicts);
const pout = await buildReviewPipeline('plan', deps(pspy))(CTX);

// refutable dropped, below-floor dropped, real survives — full parity with code mode.
assert.deepEqual(pout.map((f) => f.id), ['vague-step'], 'plan: refutable dropped, below-floor dropped, real survives');
const pFind = pspy.calls.filter((c) => c.label.startsWith('find:'));
const pRefute = pspy.calls.filter((c) => c.label.startsWith('refute:'));
assert.equal(pFind.length, 3, 'one finder per plan dimension');
assert.equal(pRefute.length, 3, 'a fresh refuter per plan finding');
assert.ok(pFind.every((c) => c.label.startsWith('find:plan:')), 'plan finders labelled by dimension');
assert.ok(pFind.every((c) => c.prompt.includes(CTX.target)), 'context.target threaded into plan finder prompts');

// Plan-mode determinism parity with code mode.
const poutA = await buildReviewPipeline('plan', deps(makeSpyAgent(planFindings, planVerdicts)))(CTX);
const poutB = await buildReviewPipeline('plan', deps(makeSpyAgent(planFindings, planVerdicts)))(CTX);
assert.equal(JSON.stringify(poutA), JSON.stringify(poutB), 'plan review output is deterministic across runs');

// Unknown mode is rejected, not silently empty.
assert.throws(() => buildReviewPipeline('bogus', deps(spy)), /unknown review mode/, 'unknown mode throws');

// ============================================================================
// AC2 — code-mode findPrompt output is byte-exact against a baseline captured
// BEFORE the plan-severity-calibration change. Any difference (including a
// single stray byte) means the calibration work leaked into code-mode prompts.
// ============================================================================
const CODE_PROMPT_BASELINE = {
  ac: 'You are a READ-ONLY reviewer. Do not edit any files.\nReview target: phase widget/phase-1-foo.\nInspect the implementation diff (use git log / git diff in the worktree).\nYour single dimension is AC compliance (ac). For each acceptance criterion in the target, rate PASS / FAIL / PARTIAL with evidence (file:line, test name). Flag any criterion that is unmet, ambiguous, or untestable.\nReport only findings you can back with concrete evidence. One strong finding beats five weak ones.\nReturn JSON matching the FINDINGS schema: a `findings` array, each with id, concern, location, severity (blocking|concern|suggestion), confidence (0-100), what_fails, why, recommendation.\nReturn an empty `findings` array if the dimension is clean.',
  correctness: 'You are a READ-ONLY reviewer. Do not edit any files.\nReview target: phase widget/phase-1-foo.\nInspect the implementation diff (use git log / git diff in the worktree).\nYour single dimension is Correctness & error handling (correctness). Logic bugs, edge cases, race conditions, and error paths. In rdm-core, errors must be hand-written matchable enums (no anyhow / type erasure); in rdm-cli / rdm-server, anyhow with .context(). User-facing CLI errors must be actionable.\nReport only findings you can back with concrete evidence. One strong finding beats five weak ones.\nReturn JSON matching the FINDINGS schema: a `findings` array, each with id, concern, location, severity (blocking|concern|suggestion), confidence (0-100), what_fails, why, recommendation.\nReturn an empty `findings` array if the dimension is clean.',
  tests: 'You are a READ-ONLY reviewer. Do not edit any files.\nReview target: phase widget/phase-1-foo.\nInspect the implementation diff (use git log / git diff in the worktree).\nYour single dimension is Tests (tests). Do tests exist and cover the key behaviors and edge cases? Was TDD followed? Are there untested branches or newly added logic with no test?\nReport only findings you can back with concrete evidence. One strong finding beats five weak ones.\nReturn JSON matching the FINDINGS schema: a `findings` array, each with id, concern, location, severity (blocking|concern|suggestion), confidence (0-100), what_fails, why, recommendation.\nReturn an empty `findings` array if the dimension is clean.',
  architecture: 'You are a READ-ONLY reviewer. Do not edit any files.\nReview target: phase widget/phase-1-foo.\nInspect the implementation diff (use git log / git diff in the worktree).\nYour single dimension is Architecture (architecture). Does logic live in rdm-core with cli/server as thin layers? No duplicated logic across interfaces? Correct core/cli/server separation and conventional-commit scope discipline.\nReport only findings you can back with concrete evidence. One strong finding beats five weak ones.\nReturn JSON matching the FINDINGS schema: a `findings` array, each with id, concern, location, severity (blocking|concern|suggestion), confidence (0-100), what_fails, why, recommendation.\nReturn an empty `findings` array if the dimension is clean.',
};
for (const dim of DIMENSIONS.code) {
  assert.equal(
    findPrompt('code', dim, CTX),
    CODE_PROMPT_BASELINE[dim.key],
    'code-mode findPrompt("' + dim.key + '") must stay byte-identical to the pre-calibration baseline'
  );
}
console.log('AC2: code-mode findPrompt output is byte-exact against the pre-calibration baseline');

// ============================================================================
// AC1 — every plan-mode findPrompt output carries the plan-stage severity
// calibration contract (blocking = goal/approach/scope/architectural-constraint
// violation; a proposed-code/shell defect is a concern, not a gate). Exported
// as a function so the scratch mutation self-test (shell section 4) can run
// the identical check against a mutated copy of the module.
// ============================================================================
const PLAN_CALIBRATION_KEYPHRASES = [
  'blocking` means the goal, approach, or scope is wrong, or the plan violates a stated architectural constraint',
  'concern` that rides along as an implementation note for the implementing agent',
];

function assertPlanCalibrationPresent(m) {
  for (const dim of m.DIMENSIONS.plan) {
    const prompt = m.findPrompt('plan', dim, CTX);
    for (const phrase of PLAN_CALIBRATION_KEYPHRASES) {
      assert.ok(
        prompt.includes(phrase),
        'plan-mode findPrompt("' + dim.key + '") is missing calibration keyphrase: ' + phrase
      );
    }
  }
}
assertPlanCalibrationPresent(mod);
console.log('AC1: every plan-mode findPrompt output carries the severity-calibration keyphrases');

// The pre-existing "empty or ambiguous plan is itself blocking" coherence rule
// must survive the calibration edit untouched — not deleted, not overwritten.
assert.ok(
  DIMENSIONS.plan
    .find((d) => d.key === 'coherence')
    .focus.includes('An empty or ambiguous plan is itself a blocking finding'),
  "coherence's pre-existing empty/ambiguous-plan rule must still be present"
);

// ============================================================================
// AC4 — an architectural-violation finding (the review-verify tier-downgrade
// class) still comes back `blocking` on the first pass, ranked ahead of an
// implementation-detail nit that must NOT be `blocking`. Proves the
// calibration prompt text does not, and structurally cannot, cause the
// pipeline itself to downgrade or drop a legitimate architectural blocker —
// paired in one buildReviewPipeline('plan', ...) call per the plan.
// ============================================================================
const calibrationFindings = {
  coherence: [],
  'architectural-fit': [
    {
      id: 'tier-downgrade',
      concern: 'architectural-fit',
      severity: 'blocking',
      confidence: 92,
      what_fails: 'The plan silently downgrades the review tier on failure, violating the stated model-tier binding contract.',
    },
  ],
  'unit-of-work': [
    {
      id: 'impl-nit',
      concern: 'unit-of-work',
      severity: 'concern',
      confidence: 85,
      what_fails: 'Off-by-one in the loop bound of the proposed pseudo-code snippet.',
    },
  ],
};
const calibrationVerdicts = {
  'tier-downgrade': { refuted: false, confidence: 95 },
  'impl-nit': { refuted: false, confidence: 88 },
};
const cspy = makeSpyAgent(calibrationFindings, calibrationVerdicts);
const cout = await buildReviewPipeline('plan', deps(cspy))(CTX);
assert.deepEqual(
  cout.map((f) => f.id),
  ['tier-downgrade', 'impl-nit'],
  'both the architectural-violation finding and the implementation-detail nit survive refutation/floor'
);
assert.equal(
  cout.find((f) => f.id === 'tier-downgrade').severity,
  'blocking',
  'the architectural-violation (tier-downgrade class) finding still yields blocking'
);
assert.notEqual(
  cout.find((f) => f.id === 'impl-nit').severity,
  'blocking',
  'the implementation-detail nit is not blocking'
);
assert.equal(
  cout[0].id,
  'tier-downgrade',
  'the blocking architectural finding ranks ahead of the concern-severity nit'
);
console.log('AC4: an architectural-violation finding still yields blocking, ranked ahead of an implementation nit');

// ============================================================================
// Model threading + the null-agent loud-failure guard.
//
// An unknown model id makes agent() RESOLVE to null (docs/workflow-schemas.md
// § "agent() options spike"). Without a guard, a null finder is laundered into
// [] by the refute stage's `(found && …) || []` and the review reports CLEAN.
// ============================================================================
function nullAgent() {
  const calls = [];
  return {
    calls,
    agent: async (prompt, opts) => {
      calls.push({ label: (opts && opts.label) || '', model: opts && opts.model });
      return null;
    },
  };
}

// (a) With an explicit findModel, an all-null finder sweep must REJECT, never
//     resolve to a clean review.
const nspy = nullAgent();
await assert.rejects(
  buildReviewPipeline('code', deps(nspy))({ ...CTX, findModel: 'bogus-model', verifyModel: 'bogus-model' }),
  /every code dimension finder failed|returned null with model/,
  'all-null finders with an explicit model must fail loudly, not report clean'
);

// (b) The models are actually threaded onto the agent() options.
assert.ok(nspy.calls.length > 0, 'finders were dispatched');
assert.ok(
  nspy.calls.every((c) => c.model === 'bogus-model'),
  'findModel is threaded onto every finder agent() call'
);

// (c) WITHOUT a model (the standalone consumer), today's behavior is preserved:
//     null finders degrade to an empty review rather than throwing.
const nspy2 = nullAgent();
const degraded = await buildReviewPipeline('code', deps(nspy2))(CTX);
assert.deepEqual(degraded, [], 'no-model callers keep the pre-existing lenient behavior');
assert.ok(
  nspy2.calls.every((c) => c.model === undefined),
  'no model key is invented when the caller supplies none'
);

// (d) A genuinely clean review (findings: []) must NOT trip the guard.
const cleanSpy = makeSpyAgent({}, {});
const cleanOut = await buildReviewPipeline('code', deps(cleanSpy))({ ...CTX, findModel: 'haiku', verifyModel: 'opus' });
assert.deepEqual(cleanOut, [], 'a real clean review still returns [] with models set');

console.log('all review-refute-fix behavior assertions passed');
NODE_TEST

if run_node "$TMP/test.mjs" "$LIB"; then
    pass "find -> refute -> filter behavior verified (code + plan, deterministic)"
else
    fail "review-refute-fix behavior assertions failed"
fi

# --- 4. PLAN CALIBRATION MUTATION SELF-TEST -----------------------------------
# Prove the AC1 presence check (embedded in section 3's test.mjs) is not
# vacuous: on a hermetic scratch copy of the lib, strip the
# PLAN_SEVERITY_CALIBRATION constant declaration and its single injection line
# inside findPrompt, then re-run the identical presence assertion against the
# mutated copy and require it to THROW. Mirrors 1b's SCRATCH-only isolation —
# never touches $LIB. A failed/partial strip must still leave valid, importable
# JS (whole-statement removals only), so a parse error can't be mistaken for a
# passing self-test.
say "4. Plan calibration mutation self-test (proves the AC1 check would catch a regression)"
mkdir -p "$SCRATCH/.claude/workflows/lib"
cp "$LIB" "$SCRATCH/.claude/workflows/lib/review-refute-fix.mjs"

# Remove the `const PLAN_SEVERITY_CALIBRATION = ... ;` declaration (spans the
# `const NAME =` line through the line ending in `;`) and the one line that
# pushes it into the prompt. Both are whole-statement removals, so the mutated
# file stays syntactically valid — leaves the block-comment prose above it in
# place, which is harmless.
awk '
    /^const PLAN_SEVERITY_CALIBRATION =$/ { skip = 1 }
    skip && /;$/ { skip = 0; next }
    skip { next }
    /lines\.push\(PLAN_SEVERITY_CALIBRATION\);/ { next }
    { print }
' "$SCRATCH/.claude/workflows/lib/review-refute-fix.mjs" >"$SCRATCH/mutated-lib.mjs"
mv "$SCRATCH/mutated-lib.mjs" "$SCRATCH/.claude/workflows/lib/review-refute-fix.mjs"

if grep -q 'PLAN_SEVERITY_CALIBRATION' "$SCRATCH/.claude/workflows/lib/review-refute-fix.mjs"; then
    fail "mutation setup did not fully strip PLAN_SEVERITY_CALIBRATION from the scratch copy"
fi

cat >"$TMP/mutation-test.mjs" <<'NODE_MUTATION_TEST'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const mutatedLibPath = process.argv[2];
const mod = await import(pathToFileURL(mutatedLibPath).href); // must still parse/import cleanly

const CTX = { target: 'phase widget/phase-1-foo' };
const PLAN_CALIBRATION_KEYPHRASES = [
  'blocking` means the goal, approach, or scope is wrong, or the plan violates a stated architectural constraint',
  'concern` that rides along as an implementation note for the implementing agent',
];

function assertPlanCalibrationPresent(m) {
  for (const dim of m.DIMENSIONS.plan) {
    const prompt = m.findPrompt('plan', dim, CTX);
    for (const phrase of PLAN_CALIBRATION_KEYPHRASES) {
      assert.ok(prompt.includes(phrase), 'missing calibration keyphrase in "' + dim.key + '": ' + phrase);
    }
  }
}

assert.throws(
  () => assertPlanCalibrationPresent(mod),
  'the presence check must FAIL against a mutated copy with the calibration text stripped — the check is vacuous otherwise'
);

console.log('mutation self-test passed: presence check correctly fails on stripped calibration text');
NODE_MUTATION_TEST

if run_node "$TMP/mutation-test.mjs" "$SCRATCH/.claude/workflows/lib/review-refute-fix.mjs"; then
    pass "calibration presence check fires on planted removal (self-test proves it is not vacuous)"
else
    fail "mutation self-test did not behave as expected — either the mutated file failed to import, or the presence check did not fail on stripped calibration text"
fi

say "verify-workflow-review.sh: ALL GREEN"
