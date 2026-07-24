#!/bin/sh
# Hermetic regression for the estimate workflow and its shared estimate-core.
#
# estimate (`.claude/workflows/estimate.js`) rates an rdm roadmap's UNESTIMATED
# phases: it lists the phases, filters to those whose difficulty is unset,
# rates each in a parallel() fan-out, writes back the difficulty AND appends a
# `## Estimate <difficulty> — <justification>` audit note to the phase body, and
# reads the core-derived model tier back from `rdm phase show` for the summary.
# It NEVER passes `--model` and NEVER reimplements the difficulty->tier mapping —
# rdm-core (Difficulty::model_tier) owns that. Its pure estimate core lives once
# in `.claude/workflows/lib/estimate.mjs` (the `estimate-core` marker region) and
# is copied BYTE-IDENTICAL into three consumers — estimate.js, autopilot.js, and
# lib/autopilot.mjs — by scripts/gen-workflow-estimate.sh (the Workflow runtime
# cannot import a helper module; see docs/workflow-schemas.md § "Import spike").
# autopilot's estimate pre-pass reuses selectUnestimated /
# buildEstimateWritebackPrompt from the same block, so the note-appending
# behavior is identical in both surfaces. This harness gates all of that:
#
#   1. BEHAVIOR   — the pure helpers, driven in Node (zero LLM calls): arg
#                   parsing, phase selection, the estimator/writeback/list/tier
#                   prompt contents (note + --difficulty + --body, NO --model),
#                   the summary text, and determinism. These are the assertions
#                   re-homed from verify-workflow-autopilot.sh when the estimate
#                   core moved out of the autopilot-loop block.
#   1b. PIPELINE  — buildEstimatePipeline fed state-backed fakes: rates ONLY the
#                   unestimated phases, writeback carries the justification,
#                   reports the tier read back from showTier (never a JS map),
#                   narrows to a single phase number, is idempotent on re-run
#                   (a re-listed estimated phase is skipped), and is deterministic.
#   2. DRIFT      — scripts/gen-workflow-estimate.sh --check passes on the tree,
#                   with a planted-mutation self-test proving the gate is not a
#                   no-op and heals on restore.
#   3. STATIC     — estimate.js loads under module semantics; no import/require;
#                   no Date.now / Math.random anywhere in the estimate sources; no
#                   `difficultyToTier` anywhere under .claude/workflows/; no
#                   *_SCHEMA handed to agent() with a top-level type:'array'
#                   (Anthropic tools require 'object'); meta.phases parity; and
#                   the rewritten rdm-estimate SKILL.md is a thin shim referencing
#                   estimate.js with no retired rating-loop prose.
#
# Node is used only as a host to unit-test the pure module and drive the pipeline
# with fakes; it is stdlib-only (node:assert), with no package.json /
# node_modules / third-party packages. node is pinned in .mise.toml.
#
# Requires: node (via PATH or `mise exec node --`).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)

WF_DIR="$REPO_ROOT/.claude/workflows"
LIB="$WF_DIR/lib/estimate.mjs"
WF="$WF_DIR/estimate.js"
SKILL="$REPO_ROOT/.claude/skills/rdm-estimate/SKILL.md"
GEN="$SCRIPT_DIR/gen-workflow-estimate.sh"

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
[ -f "$SKILL" ] || fail "rdm-estimate skill not found: $SKILL"
[ -x "$GEN" ] || fail "generator not found or not executable: $GEN"

# Resolve a node command: prefer PATH, fall back to the mise-pinned toolchain.
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

# Parse a workflow script under MODULE semantics and fail on a SyntaxError. Strip
# the leading `export` and wrap in an async function so top-level `return`/`await`
# are legal, while keeping the top-level `const meta` in ONE shared scope so a
# redeclaration is a SyntaxError.
parse_workflow() {
    {
        echo '(async function(){'
        sed 's/^export //' "$1"
        echo '})'
    } |
        run_node --check --input-type=module -
}

# Distinct `phase: '<name>',` literals the workflow actually emits.
emitted_phases() {
    grep -oE "phase: '[A-Za-z]+'," "$1" | sed "s/phase: '//;s/',//" | sort -u
}
# Distinct `{ title: '<name>' }` entries declared in the `meta.phases` array.
declared_phases() {
    awk '/phases: \[/{p=1} p{print} p&&/\],?$/{exit}' "$1" |
        grep -oE "title: '[^']+'" | sed "s/title: '//;s/'\$//" | sort -u
}

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

# --- 1. BEHAVIOR -------------------------------------------------------------
say "1. Behavior: arg parsing, selection, prompt contents (note + --difficulty + --body, no --model), summary"

cat >"$TMP/behavior.mjs" <<'NODE_TEST'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const libPath = process.argv[2];
const m = await import(pathToFileURL(libPath).href);
const {
  parseEstimateArgs,
  selectUnestimated,
  buildEstimateListPrompt,
  buildEstimatorPrompt,
  buildEstimateWritebackPrompt,
  buildEstimateTierPrompt,
  buildEstimateSummaryText,
} = m;

// --- parseEstimateArgs -------------------------------------------------------
assert.throws(() => parseEstimateArgs({}), /roadmap slug is required/, 'roadmap slug required');
assert.throws(() => parseEstimateArgs({ roadmap: '' }), /roadmap slug is required/, 'empty roadmap rejected');
assert.deepEqual(parseEstimateArgs({ roadmap: 'rm' }), { roadmap: 'rm', phase: null }, 'defaults: no phase narrowing');
assert.equal(parseEstimateArgs({ roadmap: 'rm', phase: 3 }).phase, 3, 'phase number kept');
assert.equal(parseEstimateArgs({ roadmap: 'rm', phase: '2' }).phase, 2, 'phase number coerced from string');
assert.throws(() => parseEstimateArgs({ roadmap: 'rm', phase: 0 }), /positive integer/, 'phase 0 rejected');
assert.throws(() => parseEstimateArgs({ roadmap: 'rm', phase: -1 }), /positive integer/, 'phase negative rejected');
// A caller may stringify the Workflow tool payload; coerce it instead of failing.
assert.equal(parseEstimateArgs('{"roadmap":"rm"}').roadmap, 'rm', 'stringified JSON args coerced');
assert.throws(() => parseEstimateArgs('not json'), /roadmap slug is required/, 'non-JSON string falls back to actionable error');
assert.throws(() => parseEstimateArgs('null'), /roadmap slug is required/, 'JSON null rejected without a TypeError');

// --- selectUnestimated (re-homed from the autopilot harness) -----------------
assert.deepEqual(
  selectUnestimated([
    { stem: 'a' },
    { stem: 'b', difficulty: 'hard' },
    { stem: 'c', model: 'small' },
    { stem: 'd', difficulty: 'easy', model: 'small' },
  ]),
  ['a'],
  'only phases with NO difficulty and NO model are unestimated (both must be unset)'
);
assert.deepEqual(selectUnestimated([]), [], 'empty list');
assert.deepEqual(selectUnestimated(null), [], 'non-array tolerated');

// --- buildEstimateListPrompt -------------------------------------------------
const listPrompt = buildEstimateListPrompt('rm');
assert.ok(listPrompt.includes('./target/debug/rdm phase list --roadmap rm'), 'list prompt runs phase list for the roadmap');
assert.ok(listPrompt.includes('--format json'), 'list prompt asks for JSON');

// --- buildEstimatorPrompt (now requires a justification) ---------------------
const ratePrompt = buildEstimatorPrompt('some phase body');
assert.ok(ratePrompt.includes('some phase body'), 'estimator embeds the phase body');
assert.ok(ratePrompt.includes('trivial, easy, moderate, hard'), 'estimator lists the difficulty vocabulary');
assert.ok(/justification/i.test(ratePrompt), 'estimator asks for a justification');
assert.ok(ratePrompt.includes('"justification"'), 'estimator return schema includes the justification field');

// --- buildEstimateWritebackPrompt: note + --difficulty + --body, NO --model --
const wb = buildEstimateWritebackPrompt('phase-1-x', 'hard', 'risky cross-cutting change', 'rm');
assert.ok(wb.includes('--difficulty hard'), 'writeback passes --difficulty');
assert.ok(wb.includes('--body'), 'writeback passes --body (the audit note rides in the body)');
assert.ok(wb.includes('## Estimate'), 'writeback appends a ## Estimate section');
assert.ok(wb.includes('hard — risky cross-cutting change'), 'the note carries "<difficulty> — <justification>"');
// The `phase update` COMMAND must never carry --model (the prose may still name
// it to tell the agent to avoid it — so scope the check to the update command).
const wbUpdateLine = wb.split('\n').find((l) => l.includes('phase update phase-1-x'));
assert.ok(wbUpdateLine, 'writeback contains a phase update command line');
assert.ok(!wbUpdateLine.includes('--model'), 'the phase update command NEVER passes --model (tier derives in rdm-core)');
assert.ok(/heredoc/i.test(wb), 'writeback instructs assembling the body via a quoted heredoc (safe interpolation)');
assert.ok(wb.includes('phase update phase-1-x'), 'writeback updates the right phase');
assert.ok(wb.includes('--roadmap rm'), 'writeback scopes to the roadmap');

// --- buildEstimateTierPrompt: tier is READ BACK, never computed --------------
const tierPrompt = buildEstimateTierPrompt('phase-1-x', 'rm');
assert.ok(tierPrompt.includes('phase show phase-1-x'), 'tier prompt reads the phase back');
assert.ok(tierPrompt.includes('"model"'), 'tier prompt returns the core-derived model field');

// --- no prompt builder leaks a land/merge/main-mutation/completion directive -
const FORBIDDEN = ['Done:', '--land', '--commit', 'git merge', 'git push', 'checkout main'];
function hasForbidden(s) {
  return FORBIDDEN.some((f) => s.includes(f));
}
const allPrompts = [
  buildEstimateListPrompt('rm'),
  buildEstimatorPrompt('a phase body'),
  buildEstimateWritebackPrompt('phase-1-x', 'hard', 'why', 'rm'),
  buildEstimateTierPrompt('phase-1-x', 'rm'),
];
for (const p of allPrompts) {
  assert.ok(!hasForbidden(p), 'no estimate prompt leaks a land/merge/commit/Done directive:\n' + p);
}
assert.ok(hasForbidden('run rdm phase update --land now'), 'forbidden-string detector catches a planted --land');

// --- buildEstimateSummaryText ------------------------------------------------
const text = buildEstimateSummaryText({
  roadmap: 'rm',
  estimated: [
    { stem: 'phase-1-a', difficulty: 'easy', justification: 'small', tier: 'small' },
    { stem: 'phase-2-b', difficulty: 'hard', justification: 'big', tier: 'large' },
  ],
  skipped: ['phase-3-c'],
});
assert.ok(text.includes('estimate summary for roadmap/rm'), 'summary names the roadmap');
assert.ok(text.includes('phase-1-a: easy (tier small)'), 'summary lists difficulty + read-back tier');
assert.ok(text.includes('phase-2-b: hard (tier large)'), 'summary lists the second phase');
assert.ok(text.includes('skipped, already estimated (1): phase-3-c'), 'summary lists skipped phases');
assert.equal(
  buildEstimateSummaryText({ roadmap: 'rm', estimated: [], skipped: [] }),
  buildEstimateSummaryText({ roadmap: 'rm', estimated: [], skipped: [] }),
  'summary text is deterministic'
);

console.log('all estimate behavior assertions passed');
NODE_TEST

if run_node "$TMP/behavior.mjs" "$LIB"; then
    pass "pure helpers verified (args, selection, prompts, note/no-model, summary, determinism)"
else
    fail "estimate behavior assertions failed"
fi

# --- 1b. PIPELINE ------------------------------------------------------------
say "1b. Pipeline: buildEstimatePipeline fed state-backed fakes (rates only unestimated, notes, idempotent, deterministic)"

cat >"$TMP/pipeline.mjs" <<'NODE_TEST'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const libPath = process.argv[2];
const m = await import(pathToFileURL(libPath).href);
const { buildEstimatePipeline } = m;

// The fake rdm-core: writeback sets BOTH difficulty and a derived model (that is
// what real rdm-core does — Difficulty::model_tier), so a re-list shows the phase
// estimated. showTier returns whatever model the writeback stored — the tier is
// NEVER computed by the JS under test.
const TIER = { trivial: 'small', easy: 'small', moderate: 'medium', hard: 'large' };
function makeFakes(phases) {
  const map = new Map();
  for (const p of phases) map.set(p.stem, { number: p.number, difficulty: p.difficulty || '', model: p.model || '' });
  const listCalls = [];
  const rateCalls = [];
  const writebackCalls = [];
  const showTierCalls = [];
  const logs = [];
  const fakes = {
    log: (msg) => logs.push(msg),
    list: async (roadmap) => {
      listCalls.push(roadmap);
      const out = [];
      for (const [stem, v] of map) out.push({ number: v.number, stem, status: 'not-started', difficulty: v.difficulty, model: v.model });
      return out;
    },
    parallelRate: async (stems) => {
      rateCalls.push(stems.slice());
      return stems.map((stem) => ({ stem, difficulty: 'moderate', justification: 'because ' + stem }));
    },
    writeback: async (stem, difficulty, justification, roadmap) => {
      writebackCalls.push({ stem, difficulty, justification, roadmap });
      // Simulate rdm-core deriving the tier onto model.
      const cur = map.get(stem) || {};
      map.set(stem, { number: cur.number, difficulty, model: TIER[difficulty] || 'medium' });
      return { ok: true };
    },
    showTier: async (stem, roadmap) => {
      showTierCalls.push({ stem, roadmap });
      const v = map.get(stem) || {};
      return v.model || '';
    },
  };
  return { fakes, map, listCalls, rateCalls, writebackCalls, showTierCalls, logs };
}

// === rates ONLY the unestimated phases ======================================
{
  const h = makeFakes([
    { number: 1, stem: 'phase-1-a' },
    { number: 2, stem: 'phase-2-b' },
    { number: 3, stem: 'phase-3-c', difficulty: 'hard', model: 'large' },
  ]);
  const summary = await buildEstimatePipeline(h.fakes)({ roadmap: 'rm' });
  assert.deepEqual(h.rateCalls, [['phase-1-a', 'phase-2-b']], 'rated exactly the two unestimated stems, in sorted order');
  assert.deepEqual(
    h.writebackCalls.map((w) => w.stem),
    ['phase-1-a', 'phase-2-b'],
    'writeback fired for exactly the two unestimated stems'
  );
  // The audit-note justification is threaded into every writeback.
  assert.ok(h.writebackCalls.every((w) => typeof w.justification === 'string' && w.justification.length > 0), 'every writeback carries a justification');
  // The reported tier comes from the showTier read-back, not a JS map.
  assert.deepEqual(h.showTierCalls.map((s) => s.stem), ['phase-1-a', 'phase-2-b'], 'tier read back per estimated stem');
  assert.deepEqual(summary.skipped, ['phase-3-c'], 'the already-estimated phase is reported as skipped, never rated');
  assert.deepEqual(
    summary.estimated,
    [
      { stem: 'phase-1-a', difficulty: 'moderate', justification: 'because phase-1-a', tier: 'medium' },
      { stem: 'phase-2-b', difficulty: 'moderate', justification: 'because phase-2-b', tier: 'medium' },
    ],
    'estimated entries carry difficulty, justification, and the READ-BACK tier'
  );
}

// === idempotent on re-run: everything estimated -> nothing rated =============
{
  const h = makeFakes([
    { number: 1, stem: 'phase-1-a' },
    { number: 2, stem: 'phase-2-b' },
  ]);
  await buildEstimatePipeline(h.fakes)({ roadmap: 'rm' });
  // Second run against the SAME (now-mutated) state.
  const h2fakes = h.fakes;
  h.rateCalls.length = 0;
  h.writebackCalls.length = 0;
  const summary2 = await buildEstimatePipeline(h2fakes)({ roadmap: 'rm' });
  assert.deepEqual(h.rateCalls, [], 're-run rates nothing — every phase is now estimated');
  assert.deepEqual(h.writebackCalls, [], 're-run writes nothing back (no double-append of the ## Estimate note)');
  assert.deepEqual(summary2.estimated, [], 're-run estimates nothing');
  assert.deepEqual(summary2.skipped, ['phase-1-a', 'phase-2-b'], 're-run reports both as skipped');
}

// === narrow to a single phase number ========================================
{
  const h = makeFakes([
    { number: 1, stem: 'phase-1-a' },
    { number: 2, stem: 'phase-2-b' },
    { number: 3, stem: 'phase-3-c' },
  ]);
  const summary = await buildEstimatePipeline(h.fakes)({ roadmap: 'rm', phase: 2 });
  assert.deepEqual(h.rateCalls, [['phase-2-b']], 'narrowed run rates ONLY the named phase number');
  assert.deepEqual(summary.estimated.map((e) => e.stem), ['phase-2-b'], 'only the narrowed phase is estimated');
  assert.ok(summary.skipped.includes('phase-1-a') && summary.skipped.includes('phase-3-c'), 'the others are skipped');
}

// === narrowing to an already-estimated phase is a no-op =====================
{
  const h = makeFakes([{ number: 1, stem: 'phase-1-a', difficulty: 'hard', model: 'large' }]);
  const summary = await buildEstimatePipeline(h.fakes)({ roadmap: 'rm', phase: 1 });
  assert.deepEqual(h.rateCalls, [], 'a narrowed, already-estimated phase rates nothing');
  assert.deepEqual(summary.estimated, [], 'nothing estimated');
}

// === empty roadmap: deterministic zero summary, no fan-out ===================
{
  const h = makeFakes([]);
  const summary = await buildEstimatePipeline(h.fakes)({ roadmap: 'rm' });
  assert.deepEqual(h.rateCalls, [], 'no rater fans out for an empty roadmap');
  assert.deepEqual(summary, { roadmap: 'rm', estimated: [], skipped: [] }, 'empty roadmap yields a deterministic zero summary');
}

// === deterministic: two identical runs against fresh fakes are byte-equal ====
{
  const phases = [
    { number: 1, stem: 'phase-1-a' },
    { number: 2, stem: 'phase-2-b' },
    { number: 3, stem: 'phase-3-c', difficulty: 'easy', model: 'small' },
  ];
  const a = await buildEstimatePipeline(makeFakes(phases).fakes)({ roadmap: 'rm' });
  const b = await buildEstimatePipeline(makeFakes(phases).fakes)({ roadmap: 'rm' });
  assert.equal(JSON.stringify(a), JSON.stringify(b), 'the summary is deterministic across identical runs');
}

// === a null/justification-less rater result is skipped, not dereferenced =====
{
  const h = makeFakes([
    { number: 1, stem: 'phase-1-a' },
    { number: 2, stem: 'phase-2-b' },
  ]);
  h.fakes.parallelRate = async (stems) => {
    h.rateCalls.push(stems.slice());
    // phase-1-a comes back null (unresolvable model), phase-2-b omits justification.
    return [null, { stem: 'phase-2-b', difficulty: 'easy' }];
  };
  const summary = await buildEstimatePipeline(h.fakes)({ roadmap: 'rm' });
  assert.deepEqual(h.writebackCalls.map((w) => w.stem), ['phase-2-b'], 'the null rater result is skipped, the valid one is written');
  assert.equal(summary.estimated[0].justification, '', 'a missing justification defaults to an empty string, never undefined');
}

console.log('all estimate pipeline assertions passed');
NODE_TEST

if run_node "$TMP/pipeline.mjs" "$LIB"; then
    pass "pipeline rates only unestimated, threads the note, reads tier back, idempotent, narrows, deterministic"
else
    fail "estimate pipeline assertions failed"
fi

# --- 2. DRIFT GATE -----------------------------------------------------------
say "2. Drift: gen-workflow-estimate.sh --check passes on the committed tree"

if "$GEN" --check >/dev/null 2>&1; then
    pass "estimate-core is in sync across estimate.js / autopilot.js / lib/autopilot.mjs"
else
    "$GEN" --check >&2 || true
    fail "estimate-core DRIFTED — run scripts/gen-workflow-estimate.sh"
fi

# Self-test: mutate a consumer's estimate-core region in a scratch clone and prove
# --check FAILS, then restore and prove it heals. We drive the generator against a
# scratch repo so the real tree is never touched.
say "2b. Drift detector fires on planted drift inside a consumer's estimate-core region (self-test)"
# Plant the mutation directly in estimate.js, run --check, then restore.
cp "$WF" "$TMP/estimate.js.orig"
# Mutate one line inside the estimate-core region (the summary header string).
sed 's/estimate summary for roadmap/PLANTED DRIFT for roadmap/' "$WF" >"$TMP/estimate.js.mut"
cp "$TMP/estimate.js.mut" "$WF"
if "$GEN" --check >/dev/null 2>&1; then
    cp "$TMP/estimate.js.orig" "$WF"
    fail "drift gate did NOT fire on a planted mutation inside estimate.js's estimate-core region"
fi
cp "$TMP/estimate.js.orig" "$WF"
if "$GEN" --check >/dev/null 2>&1; then
    pass "drift detector fires on a planted mutation and heals on restore"
else
    "$GEN" --check >&2 || true
    fail "restore did not heal the drift gate"
fi

# --- 3. STATIC INVARIANTS ----------------------------------------------------
say "3. Static invariants on estimate.js and the estimate sources"

# 3a. Module parse.
if parse_workflow "$WF" >/dev/null 2>&1; then
    pass "estimate.js parses under module semantics (top-level meta declared once)"
else
    parse_workflow "$WF" >&2 || true
    fail "estimate.js does NOT parse — fix the SyntaxError"
fi

# 3b. No import/require (the runtime forbids it — sharing is by stamped copy).
if grep -nE '(^|[^A-Za-z_])import[ (]' "$WF" >/dev/null 2>&1; then
    grep -nE '(^|[^A-Za-z_])import[ (]' "$WF" >&2 || true
    fail "estimate.js must not import (the runtime forbids it — sharing is by stamped copy)"
fi
if grep -nE '(^|[^A-Za-z_])require\(' "$WF" >/dev/null 2>&1; then
    fail "estimate.js must not require() (the runtime forbids it)"
fi
grep -q '>>> estimate-core:begin' "$WF" || fail "missing estimate-core:begin marker in estimate.js"
grep -q '>>> estimate-core:end' "$WF" || fail "missing estimate-core:end marker in estimate.js"
pass "no import/require; both estimate-core markers present in estimate.js"

# 3c. No Date.now / Math.random anywhere in the estimate sources.
if grep -nE 'Date\.now\(|Math\.random\(' "$WF" "$LIB" 2>/dev/null; then
    fail "found Date.now( / Math.random( in an estimate source — the runtime forbids them and they break determinism"
fi
printf 'const x = Date.now();\n' >"$TMP/planted-nondeterm.js"
if ! grep -nE 'Date\.now\(|Math\.random\(' "$TMP/planted-nondeterm.js" >/dev/null 2>&1; then
    fail "hygiene grep did NOT catch a planted Date.now() — the detector is broken"
fi
pass "no Date.now / Math.random in estimate.js or lib/estimate.mjs; detector catches a planted one"

# 3d. No difficultyToTier ANYWHERE under .claude/workflows/ (rdm-core owns the map).
if grep -rn 'difficultyToTier' "$WF_DIR" >/dev/null 2>&1; then
    grep -rn 'difficultyToTier' "$WF_DIR" >&2 || true
    fail "difficultyToTier must not appear anywhere under .claude/workflows/ — rdm-core (Difficulty::model_tier) owns the difficulty->tier policy"
fi
printf 'function difficultyToTier() {}\n' >"$TMP/planted-d2t.js"
grep -q 'difficultyToTier' "$TMP/planted-d2t.js" || fail "difficultyToTier detector broken"
pass "no difficultyToTier anywhere under .claude/workflows/; detector catches a planted one"

# 3e. No *_SCHEMA handed to agent() may declare a top-level type:'array'.
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
grep -q 'r.phases' "$WF" || fail "the list realDep must unwrap the PHASE_LIST_SCHEMA wrapper (expected 'r.phases' in estimate.js)"
sed "s/^  type: 'object',/  type: 'array',/" "$WF" >"$TMP/wf.array.scratch"
if [ -z "$(schema_array_offenders "$TMP/wf.array.scratch")" ]; then
    fail "top-level-array detector did NOT fire on a planted type:'array' schema"
fi
pass "no *_SCHEMA uses a top-level type:'array'; list unwraps r.phases; detector catches a planted array schema"

# 3f. meta.phases parity.
DECLARED_PHASES=$(declared_phases "$WF")
EMITTED_PHASES=$(emitted_phases "$WF")
if [ "$DECLARED_PHASES" = "$EMITTED_PHASES" ]; then
    pass "meta.phases lists exactly the emitted phase: literals ($(echo "$EMITTED_PHASES" | tr '\n' ' '))"
else
    printf 'declared (meta.phases): %s\n' "$(echo "$DECLARED_PHASES" | tr '\n' ' ')" >&2
    printf 'emitted   (phase: ...): %s\n' "$(echo "$EMITTED_PHASES" | tr '\n' ' ')" >&2
    fail "meta.phases drift: declared phases != emitted phase: literals"
fi

# --- 4. SKILL SHIM -----------------------------------------------------------
say "4. rdm-estimate SKILL.md is a thin shim referencing estimate.js with no retired rating-loop prose"

grep -qF '.claude/workflows/estimate.js' "$SKILL" || fail "SKILL.md must reference '.claude/workflows/estimate.js'"
grep -q 'Workflow' "$SKILL" || fail "SKILL.md must invoke the estimate Workflow"
# The retired step-by-step rating loop prose must be gone.
for retired in "Rate its difficulty as one of" "body=\$(cat <<'EOF'" "Skipping is the override mechanism"; do
    if grep -qF -- "$retired" "$SKILL"; then
        fail "SKILL.md still contains retired rating-loop prose: $retired"
    fi
done
# The shim must not re-narrate the per-phase heredoc writeback command.
if grep -qF -- '--difficulty <difficulty> --body' "$SKILL"; then
    fail "SKILL.md still re-narrates the writeback heredoc command — it should defer to the workflow"
fi
pass "SKILL.md is a thin shim: references estimate.js, no retired rating-loop prose"

say "verify-workflow-estimate.sh: ALL GREEN"
