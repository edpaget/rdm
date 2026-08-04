#!/bin/sh
# Hermetic regression for the estimate workflow and its shared estimate-core.
#
# estimate (`.claude/workflows/rdm-wf-estimate.js`) rates an rdm roadmap's UNESTIMATED
# phases: it lists the phases, filters to those whose difficulty is unset,
# rates each in a parallel() fan-out, writes back the difficulty AND appends a
# `## Estimate <difficulty> — <justification>` audit note to the phase body, and
# reads the core-derived model tier back from `rdm phase show` for the summary.
# It NEVER passes `--model` and NEVER reimplements the difficulty->tier mapping —
# rdm-core (Difficulty::model_tier) owns that. Its pure estimate core lives once
# in `.claude/workflows/lib/estimate.mjs` (the `estimate-core` marker region) and
# is copied BYTE-IDENTICAL into a single consumer — rdm-wf-estimate.js — by
# scripts/gen-workflow-estimate.sh (the Workflow runtime cannot import a helper
# module; see docs/workflow-schemas.md § "Import spike"). The prose
# `rdm-autopilot` skill's estimate pre-pass invokes this same `rdm-wf-estimate`
# Workflow directly via the Workflow tool rather than reusing a stamped copy of
# this block (workflow-orchestration roadmap, phase 3 retired the earlier
# `autopilot.js`/`lib/autopilot.mjs` stamped copy). This harness gates all of
# that:
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
#                   Also drives the failure branches: a writeback reporting
#                   ok:false, and a parallelRate/writeback/showTier that THROWS,
#                   proving the pipeline degrades (logs + skips/continues) instead
#                   of misreporting a failed stem as estimated or aborting the run.
#   2. DRIFT      — scripts/gen-workflow-estimate.sh --check passes on the tree,
#                   with a planted-mutation self-test proving the gate is not a
#                   no-op and heals on restore.
#   3. STATIC     — rdm-wf-estimate.js loads under module semantics; no import/require;
#                   no Date.now / Math.random anywhere in the estimate sources; no
#                   `difficultyToTier` anywhere under .claude/workflows/; no
#                   *_SCHEMA handed to agent() with a top-level type:'array'
#                   (Anthropic tools require 'object'); meta.phases parity; and
#                   the rewritten rdm-estimate SKILL.md is a thin shim referencing
#                   rdm-wf-estimate.js with no retired rating-loop prose.
#   5. HERMETIC   — a temp git-backed plan repo seeded via the REAL target/debug/rdm
#      SEED         binary (mixed estimated/unestimated phases), whose actual
#                   `rdm phase list --format json` output is fed through
#                   selectUnestimated and buildEstimatePipeline with real-binary
#                   deps (real list / phase update --difficulty --body / phase show).
#                   This backs AC1 against the CLI's real JSON shape, so any drift
#                   between the field names the pure JS assumes (stem/difficulty/
#                   model) and what rdm-core emits is caught — not just the
#                   hand-fabricated fakes of sections 1/1b.
#   9. PARAM      — estimate names NO particular rdm executable and NO particular
#                   rdm project: both are RUNTIME args (`rdmBin`, `project`),
#                   the same contract dispatch-phase landed. Per-file literal
#                   zeroing with planted mutants (9a), a driven prompt capture
#                   checking every emitted `rdm <subcommand>` against the
#                   project-agnostic allow-list expressed AS DATA (9b), the
#                   fail-closed `rdmBin` rule (9c), and self-tests proving 9b is
#                   not vacuous (9d).
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
WF="$WF_DIR/rdm-wf-estimate.js"
SKILL="$REPO_ROOT/.claude/skills/rdm-estimate/SKILL.md"
GEN="$SCRIPT_DIR/gen-workflow-estimate.sh"
RDM_BIN="$REPO_ROOT/target/debug/rdm"

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
[ -x "$RDM_BIN" ] || fail "$RDM_BIN not found or not executable — run 'cargo build' first."

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
// The required-ROADMAP throw runs FIRST, before the environment axes, so its
// actionable message survives for the most common mis-invocation (§ 9c pins the
// rdmBin half of the same parse).
assert.throws(() => parseEstimateArgs({}), /roadmap slug is required/, 'roadmap slug required');
assert.throws(() => parseEstimateArgs({ roadmap: '' }), /roadmap slug is required/, 'empty roadmap rejected');
assert.deepEqual(
  parseEstimateArgs({ roadmap: 'rm', rdmBin: 'rdm' }),
  { roadmap: 'rm', phase: null, rdmBin: 'rdm', project: '' },
  'defaults: no phase narrowing, no project flag'
);
assert.equal(parseEstimateArgs({ roadmap: 'rm', phase: 3, rdmBin: 'rdm' }).phase, 3, 'phase number kept');
assert.equal(parseEstimateArgs({ roadmap: 'rm', phase: '2', rdmBin: 'rdm' }).phase, 2, 'phase number coerced from string');
assert.throws(() => parseEstimateArgs({ roadmap: 'rm', phase: 0, rdmBin: 'rdm' }), /positive integer/, 'phase 0 rejected');
assert.throws(() => parseEstimateArgs({ roadmap: 'rm', phase: -1, rdmBin: 'rdm' }), /positive integer/, 'phase negative rejected');
// A caller may stringify the Workflow tool payload; coerce it instead of failing.
assert.equal(parseEstimateArgs('{"roadmap":"rm","rdmBin":"rdm"}').roadmap, 'rm', 'stringified JSON args coerced');
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
// Parameterized: the binary and the project flag come from the trailing cfg, and
// the project flag is concatenated BEFORE ' --format json' so the command shape
// is unchanged from the pre-parameterization literal.
const CFG = { rdmBin: '/fake/bin/rdm', project: 'demo' };
const listPrompt = buildEstimateListPrompt('rm', CFG);
assert.ok(
  listPrompt.includes('/fake/bin/rdm phase list --roadmap rm --project demo --format json'),
  'list prompt runs phase list for the roadmap with the injected binary and project'
);
assert.ok(listPrompt.includes('--format json'), 'list prompt asks for JSON');
assert.ok(
  buildEstimateListPrompt('rm', { rdmBin: 'rdm' }).includes('rdm phase list --roadmap rm --format json'),
  'no project configured -> the list prompt emits no project flag at all'
);

// --- buildEstimatorPrompt (now requires a justification) ---------------------
const ratePrompt = buildEstimatorPrompt('some phase body');
assert.ok(ratePrompt.includes('some phase body'), 'estimator embeds the phase body');
assert.ok(ratePrompt.includes('trivial, easy, moderate, hard'), 'estimator lists the difficulty vocabulary');
assert.ok(/justification/i.test(ratePrompt), 'estimator asks for a justification');
assert.ok(ratePrompt.includes('"justification"'), 'estimator return schema includes the justification field');

// --- buildEstimateWritebackPrompt: note + --difficulty + --body, NO --model --
const wb = buildEstimateWritebackPrompt('phase-1-x', 'hard', 'risky cross-cutting change', 'rm', CFG);
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
const tierPrompt = buildEstimateTierPrompt('phase-1-x', 'rm', CFG);
assert.ok(
  tierPrompt.includes('/fake/bin/rdm phase show phase-1-x --roadmap rm --project demo --format json'),
  'tier prompt reads the phase back with the injected binary and project'
);
assert.ok(tierPrompt.includes('phase show phase-1-x'), 'tier prompt reads the phase back');
assert.ok(tierPrompt.includes('"model"'), 'tier prompt returns the core-derived model field');

// --- no prompt builder leaks a land/merge/main-mutation/completion directive -
const FORBIDDEN = ['Done:', '--land', '--commit', 'git merge', 'git push', 'checkout main'];
function hasForbidden(s) {
  return FORBIDDEN.some((f) => s.includes(f));
}
const allPrompts = [
  buildEstimateListPrompt('rm', CFG),
  buildEstimatorPrompt('a phase body'),
  buildEstimateWritebackPrompt('phase-1-x', 'hard', 'why', 'rm', CFG),
  buildEstimateTierPrompt('phase-1-x', 'rm', CFG),
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
const { buildEstimatePipeline, buildEstimateSummaryText } = m;

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
// The two un-targeted phases are STILL UNESTIMATED here — they must be reported
// as `deferred` (not targeted this run), NOT mislabeled `skipped, already
// estimated`. This is the exact regression that a summary-membership-only
// assertion missed, so assert the rendered summary text's truthfulness too.
{
  const h = makeFakes([
    { number: 1, stem: 'phase-1-a' },
    { number: 2, stem: 'phase-2-b' },
    { number: 3, stem: 'phase-3-c' },
  ]);
  const summary = await buildEstimatePipeline(h.fakes)({ roadmap: 'rm', phase: 2 });
  assert.deepEqual(h.rateCalls, [['phase-2-b']], 'narrowed run rates ONLY the named phase number');
  assert.deepEqual(summary.estimated.map((e) => e.stem), ['phase-2-b'], 'only the narrowed phase is estimated');
  assert.deepEqual(summary.skipped, [], 'NO phase is already estimated, so nothing is reported as skipped');
  assert.deepEqual(
    summary.deferred,
    ['phase-1-a', 'phase-3-c'],
    'the other still-unestimated phases are DEFERRED (not targeted this run), not mislabeled skipped'
  );
  const text = buildEstimateSummaryText(summary);
  assert.ok(
    !/phase-1-a|phase-3-c/.test(text.split('\n').find((l) => l.startsWith('skipped, already estimated')) || ''),
    'the summary text never calls a still-unestimated deferred phase "already estimated"'
  );
  assert.ok(
    /deferred, still unestimated[^\n]*phase-1-a, phase-3-c/.test(text),
    'the summary text reports the deferred phases under an accurate "still unestimated" heading'
  );
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
  assert.deepEqual(summary, { roadmap: 'rm', estimated: [], skipped: [], deferred: [] }, 'empty roadmap yields a deterministic zero summary');
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

// === a writeback reporting { ok: false } is skipped, never misreported ======
// rdm-core can legitimately report failure (an unresolvable difficulty, a stale
// stem, a transient CLI error). Such a stem must NOT land in `estimated`, must
// not have its tier read back, and must not abort the run.
{
  const h = makeFakes([
    { number: 1, stem: 'phase-1-a' },
    { number: 2, stem: 'phase-2-b' },
  ]);
  h.fakes.writeback = async (stem, difficulty, justification, roadmap) => {
    h.writebackCalls.push({ stem, difficulty, justification, roadmap });
    if (stem === 'phase-1-a') return { ok: false }; // core reports the write failed
    const cur = h.map.get(stem) || {};
    h.map.set(stem, { number: cur.number, difficulty, model: TIER[difficulty] || 'medium' });
    return { ok: true };
  };
  const summary = await buildEstimatePipeline(h.fakes)({ roadmap: 'rm' });
  assert.deepEqual(h.writebackCalls.map((w) => w.stem), ['phase-1-a', 'phase-2-b'], 'both stems are attempted');
  assert.deepEqual(summary.estimated.map((e) => e.stem), ['phase-2-b'], 'an ok:false writeback is NOT reported as estimated');
  assert.deepEqual(h.showTierCalls.map((s) => s.stem), ['phase-2-b'], 'the tier is read back only for the successfully-written stem');
  assert.ok(h.logs.some((l) => l.includes('phase-1-a')), 'the failed writeback is logged');
}

// === a wholesale parallelRate() throw degrades to a no-op, never propagates ==
{
  const h = makeFakes([
    { number: 1, stem: 'phase-1-a' },
    { number: 2, stem: 'phase-2-b' },
  ]);
  h.fakes.parallelRate = async () => {
    throw new Error('rater fan-out crashed');
  };
  const summary = await buildEstimatePipeline(h.fakes)({ roadmap: 'rm' });
  assert.deepEqual(summary.estimated, [], 'a rater throw yields nothing estimated (the exception is caught)');
  assert.deepEqual(h.writebackCalls, [], 'nothing is written back when the rater throws wholesale');
  // Both were SELECTED for rating, so neither is "skipped" (skipped = the
  // already-estimated phases only) — they just silently drop, logged.
  assert.deepEqual(summary.skipped, [], 'targeted-but-failed stems are not misreported as already-estimated');
  assert.ok(h.logs.some((l) => /rating pass failed/i.test(l)), 'the wholesale rater failure is logged');
}

// === a per-stem writeback() throw is caught; the run continues ==============
{
  const h = makeFakes([
    { number: 1, stem: 'phase-1-a' },
    { number: 2, stem: 'phase-2-b' },
  ]);
  h.fakes.writeback = async (stem, difficulty, justification, roadmap) => {
    h.writebackCalls.push({ stem, difficulty, justification, roadmap });
    if (stem === 'phase-1-a') throw new Error('transient CLI error');
    const cur = h.map.get(stem) || {};
    h.map.set(stem, { number: cur.number, difficulty, model: TIER[difficulty] || 'medium' });
    return { ok: true };
  };
  const summary = await buildEstimatePipeline(h.fakes)({ roadmap: 'rm' });
  assert.deepEqual(summary.estimated.map((e) => e.stem), ['phase-2-b'], 'a thrown writeback is caught and its stem skipped; the other still lands');
  assert.ok(h.logs.some((l) => l.includes('phase-1-a')), 'the thrown writeback is logged');
}

// === a showTier() throw leaves the stem estimated with an empty tier ========
{
  const h = makeFakes([{ number: 1, stem: 'phase-1-a' }]);
  h.fakes.showTier = async () => {
    throw new Error('read-back failed');
  };
  const summary = await buildEstimatePipeline(h.fakes)({ roadmap: 'rm' });
  assert.deepEqual(summary.estimated.map((e) => e.stem), ['phase-1-a'], 'the writeback succeeded, so the stem is still estimated');
  assert.equal(summary.estimated[0].tier, '', 'a thrown tier read-back degrades to an empty tier, not an aborted run');
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
    pass "estimate-core is in sync in rdm-wf-estimate.js"
else
    "$GEN" --check >&2 || true
    fail "estimate-core DRIFTED — run scripts/gen-workflow-estimate.sh"
fi

# Self-test: mutate a consumer's estimate-core region in a scratch clone and prove
# --check FAILS, then restore and prove it heals. We drive the generator against a
# scratch repo so the real tree is never touched.
say "2b. Drift detector fires on planted drift inside a consumer's estimate-core region (self-test)"
# Plant the mutation directly in rdm-wf-estimate.js, run --check, then restore.
cp "$WF" "$TMP/rdm-wf-estimate.js.orig"
# Mutate one line inside the estimate-core region (the summary header string).
sed 's/estimate summary for roadmap/PLANTED DRIFT for roadmap/' "$WF" >"$TMP/rdm-wf-estimate.js.mut"
cp "$TMP/rdm-wf-estimate.js.mut" "$WF"
if "$GEN" --check >/dev/null 2>&1; then
    cp "$TMP/rdm-wf-estimate.js.orig" "$WF"
    fail "drift gate did NOT fire on a planted mutation inside rdm-wf-estimate.js's estimate-core region"
fi
cp "$TMP/rdm-wf-estimate.js.orig" "$WF"
if "$GEN" --check >/dev/null 2>&1; then
    pass "drift detector fires on a planted mutation and heals on restore"
else
    "$GEN" --check >&2 || true
    fail "restore did not heal the drift gate"
fi

# --- 3. STATIC INVARIANTS ----------------------------------------------------
say "3. Static invariants on rdm-wf-estimate.js and the estimate sources"

# 3a. Module parse.
if parse_workflow "$WF" >/dev/null 2>&1; then
    pass "rdm-wf-estimate.js parses under module semantics (top-level meta declared once)"
else
    parse_workflow "$WF" >&2 || true
    fail "rdm-wf-estimate.js does NOT parse — fix the SyntaxError"
fi

# 3b. No import/require (the runtime forbids it — sharing is by stamped copy).
if grep -nE '(^|[^A-Za-z_])import[ (]' "$WF" >/dev/null 2>&1; then
    grep -nE '(^|[^A-Za-z_])import[ (]' "$WF" >&2 || true
    fail "rdm-wf-estimate.js must not import (the runtime forbids it — sharing is by stamped copy)"
fi
if grep -nE '(^|[^A-Za-z_])require\(' "$WF" >/dev/null 2>&1; then
    fail "rdm-wf-estimate.js must not require() (the runtime forbids it)"
fi
grep -q '>>> estimate-core:begin' "$WF" || fail "missing estimate-core:begin marker in rdm-wf-estimate.js"
grep -q '>>> estimate-core:end' "$WF" || fail "missing estimate-core:end marker in rdm-wf-estimate.js"
pass "no import/require; both estimate-core markers present in rdm-wf-estimate.js"

# 3c. No Date.now / Math.random anywhere in the estimate sources.
if grep -nE 'Date\.now\(|Math\.random\(' "$WF" "$LIB" 2>/dev/null; then
    fail "found Date.now( / Math.random( in an estimate source — the runtime forbids them and they break determinism"
fi
printf 'const x = Date.now();\n' >"$TMP/planted-nondeterm.js"
if ! grep -nE 'Date\.now\(|Math\.random\(' "$TMP/planted-nondeterm.js" >/dev/null 2>&1; then
    fail "hygiene grep did NOT catch a planted Date.now() — the detector is broken"
fi
pass "no Date.now / Math.random in rdm-wf-estimate.js or lib/estimate.mjs; detector catches a planted one"

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
grep -q 'r.phases' "$WF" || fail "the list realDep must unwrap the PHASE_LIST_SCHEMA wrapper (expected 'r.phases' in rdm-wf-estimate.js)"
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

# 3g. AC-MECHANICAL-TIER: the mechanical fetch/write/tier-read agents resolve
# to the mechanical model; the judgment rater does not.
# shellcheck disable=SC1091
. "$REPO_ROOT/scripts/lib/mechanical-tier-check.sh"

agent_option_blocks "$WF" >"$TMP/mech-blocks"
[ -s "$TMP/mech-blocks" ] || fail "AC-MECHANICAL-TIER: could not extract any agent() option blocks from rdm-wf-estimate.js"

assert_label_model "$TMP/mech-blocks" 'estimate:list' 'mechanicalModel' ||
    fail "AC-MECHANICAL-TIER: estimate:list must resolve to model: mechanicalModel"
assert_label_model "$TMP/mech-blocks" 'estimate:write:' 'mechanicalModel' ||
    fail "AC-MECHANICAL-TIER: every estimate:write:<stem> call must resolve to model: mechanicalModel"
assert_label_model "$TMP/mech-blocks" 'estimate:tier:' 'mechanicalModel' ||
    fail "AC-MECHANICAL-TIER: every estimate:tier:<stem> call must resolve to model: mechanicalModel"
pass "AC-MECHANICAL-TIER: estimate:list, estimate:write:<stem>, and estimate:tier:<stem> resolve to model: mechanicalModel"

# Negative: estimate:rate:<stem> is the judgment rater and must NOT be pinned
# to the mechanical tier.
assert_label_not_model "$TMP/mech-blocks" 'estimate:rate:' 'mechanicalModel' ||
    fail "AC-MECHANICAL-TIER: estimate:rate:<stem> must NOT be pinned to model: mechanicalModel (judgment stage)"
pass "AC-MECHANICAL-TIER: estimate:rate:<stem> is left unpinned (judgment stage)"

# Self-test: plant a repoint away from mechanicalModel on estimate:list and
# prove the check now fails; restore and prove it passes again.
sed "/label: 'estimate:list'/,/^    })/ s/model: mechanicalModel,/model: 'claude-opus-4-8',/" "$WF" >"$TMP/wf.mech-mutant"
agent_option_blocks "$TMP/wf.mech-mutant" >"$TMP/mech-blocks-mutant"
if assert_label_model "$TMP/mech-blocks-mutant" 'estimate:list' 'mechanicalModel'; then
    fail "AC-MECHANICAL-TIER: detector missed an estimate:list repoint away from mechanicalModel"
fi
pass "AC-MECHANICAL-TIER: detector fires when estimate:list is repointed away from mechanicalModel"

# --- 4. SKILL SHIM -----------------------------------------------------------
say "4. rdm-estimate SKILL.md is a thin shim referencing rdm-wf-estimate.js with no retired rating-loop prose"

grep -qF '.claude/workflows/rdm-wf-estimate.js' "$SKILL" || fail "SKILL.md must reference '.claude/workflows/rdm-wf-estimate.js'"
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
pass "SKILL.md is a thin shim: references rdm-wf-estimate.js, no retired rating-loop prose"

# --- 5. HERMETIC SEED (real target/debug/rdm) --------------------------------
say "5. Hermetic seed: real rdm JSON drives selectUnestimated / buildEstimatePipeline against a temp plan repo"

PLAN="$TMP/plan"
PROJ="est-verify"
ROADMAP="rm-est"
rdmbin() { "$RDM_BIN" --root "$PLAN" "$@"; }

mkdir -p "$PLAN"
rdmbin init --default-project "$PROJ" >/dev/null
rdmbin roadmap create "$ROADMAP" --title "Estimate RM" --body "seed" \
    --no-edit --project "$PROJ" >/dev/null
rdmbin phase create a --title "A" --number 1 --body "phase a body" \
    --no-edit --roadmap "$ROADMAP" --project "$PROJ" >/dev/null
rdmbin phase create b --title "B" --number 2 --body "phase b body" \
    --no-edit --roadmap "$ROADMAP" --project "$PROJ" >/dev/null
rdmbin phase create c --title "C" --number 3 --body "phase c body" \
    --no-edit --roadmap "$ROADMAP" --project "$PROJ" >/dev/null
# Pre-estimate phase 2 so it must be SKIPPED (real rdm derives model=large from
# difficulty=hard — no --model passed).
rdmbin phase update phase-2-b --difficulty hard \
    --no-edit --roadmap "$ROADMAP" --project "$PROJ" >/dev/null
rdmbin commit -m "seed: estimate harness fixtures" >/dev/null
pass "seeded a real roadmap: phase-1-a / phase-3-c unestimated, phase-2-b pre-estimated (hard)"

cat >"$TMP/real.mjs" <<'NODE_TEST'
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';

const [libPath, RDM, PLAN, PROJ, ROADMAP] = process.argv.slice(2);
const m = await import(pathToFileURL(libPath).href);
const { selectUnestimated, buildEstimatePipeline } = m;

// rdm prints clean JSON on stdout (informational notices go to stderr); still
// parse only the leading JSON value defensively (handles a mixed array/object
// with a trailing notice).
function rdm(args) {
  // Capture stdout; silence rdm's informational stderr notices (staged/uncommitted).
  return execFileSync(RDM, ['--root', PLAN, ...args], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] });
}
function parseLeadingJson(text) {
  const s = text.replace(/^\s+/, '');
  let depth = 0;
  let started = false;
  let end = -1;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (c === '{' || c === '[') {
      depth++;
      started = true;
    } else if (c === '}' || c === ']') {
      depth--;
      if (started && depth === 0) {
        end = i + 1;
        break;
      }
    }
  }
  return JSON.parse(end === -1 ? s : s.slice(0, end));
}
function rdmJson(args) {
  return parseLeadingJson(rdm(args));
}

// --- Field-shape fidelity: REAL phase list feeds selectUnestimated -----------
// The real CLI OMITS difficulty/model on an unestimated phase (skip_serializing_if
// none). If selectUnestimated read the wrong field names, this would mis-select.
const listed = rdmJson(['phase', 'list', '--roadmap', ROADMAP, '--project', PROJ, '--format', 'json']);
assert.ok(Array.isArray(listed) && listed.length === 3, 'real phase list returns the three seeded phases');
assert.deepEqual(
  selectUnestimated(listed).slice().sort(),
  ['phase-1-a', 'phase-3-c'],
  'real phase list: only the two truly-unestimated phases select (phase-2-b has difficulty+model set)'
);

// --- Drive buildEstimatePipeline with REAL-binary deps -----------------------
// Only the LLM rating is faked (deterministic 'moderate'); list / writeback /
// showTier all hit the real rdm binary, so the derived tier and the ## Estimate
// note are exercised against rdm-core's actual behavior.
const rateCalls = [];
const deps = {
  log: () => {},
  list: async (roadmap) => rdmJson(['phase', 'list', '--roadmap', roadmap, '--project', PROJ, '--format', 'json']),
  parallelRate: async (stems) => {
    rateCalls.push(stems.slice());
    return stems.map((stem) => ({ stem, difficulty: 'moderate', justification: 'seeded justification for ' + stem }));
  },
  writeback: async (stem, difficulty, justification, roadmap) => {
    const cur = rdmJson(['phase', 'show', stem, '--roadmap', roadmap, '--project', PROJ, '--format', 'json']);
    const body = (cur.body || '') + '\n\n## Estimate\n\n' + difficulty + ' — ' + justification + '\n';
    // Never --model: rdm-core derives the tier from --difficulty.
    rdm(['phase', 'update', stem, '--difficulty', difficulty, '--body', body, '--no-edit', '--roadmap', roadmap, '--project', PROJ]);
    const after = rdmJson(['phase', 'show', stem, '--roadmap', roadmap, '--project', PROJ, '--format', 'json']);
    return { ok: after.difficulty === difficulty && (after.body || '').includes('## Estimate') };
  },
  showTier: async (stem, roadmap) => {
    const j = rdmJson(['phase', 'show', stem, '--roadmap', roadmap, '--project', PROJ, '--format', 'json']);
    return j.model || '';
  },
};

const summary = await buildEstimatePipeline(deps)({ roadmap: ROADMAP });
assert.deepEqual(rateCalls, [['phase-1-a', 'phase-3-c']], 'the pipeline rated exactly the two real-unestimated stems');
assert.deepEqual(
  summary.estimated.map((e) => e.stem).slice().sort(),
  ['phase-1-a', 'phase-3-c'],
  'exactly the two unestimated phases were estimated end-to-end against the real binary'
);
assert.deepEqual(summary.skipped, ['phase-2-b'], 'the pre-estimated phase-2-b is skipped, never rated');

for (const e of summary.estimated) {
  // The reported tier is read back from rdm-core (moderate -> medium), not a JS map.
  assert.equal(e.tier, 'medium', 'the tier is read back from rdm-core (moderate derives medium)');
  const shown = rdmJson(['phase', 'show', e.stem, '--roadmap', ROADMAP, '--project', PROJ, '--format', 'json']);
  assert.equal(shown.difficulty, 'moderate', 'the real phase now carries difficulty=moderate');
  assert.equal(shown.model, 'medium', 'rdm-core derived model=medium onto the real phase (no --model passed)');
  assert.ok((shown.body || '').includes('## Estimate'), 'the ## Estimate audit note landed in the real phase body');
  assert.ok((shown.body || '').includes('moderate — seeded justification for ' + e.stem), 'the note carries "<difficulty> — <justification>"');
}

// The pre-estimated phase is untouched: still hard/large, no note appended.
const b = rdmJson(['phase', 'show', 'phase-2-b', '--roadmap', ROADMAP, '--project', PROJ, '--format', 'json']);
assert.equal(b.difficulty, 'hard', 'the skipped phase keeps its original difficulty');
assert.ok(!(b.body || '').includes('## Estimate'), 'the skipped phase never gets a ## Estimate note');

// --- Idempotent re-run against the now-mutated real repo ---------------------
const summary2 = await buildEstimatePipeline(deps)({ roadmap: ROADMAP });
assert.deepEqual(summary2.estimated, [], 're-run against the real repo rates nothing — every phase is now estimated');
assert.deepEqual(summary2.skipped.slice().sort(), ['phase-1-a', 'phase-2-b', 'phase-3-c'], 're-run reports all three as skipped');

console.log('ALL HERMETIC-SEED ASSERTIONS PASSED');
NODE_TEST

if run_node "$TMP/real.mjs" "$LIB" "$RDM_BIN" "$PLAN" "$PROJ" "$ROADMAP"; then
    pass "real rdm JSON round-trips through selectUnestimated / buildEstimatePipeline; tier derives in core; note lands"
else
    fail "hermetic real-binary estimate assertions failed"
fi

# --- HOIST: caller-supplied mechanicalModel / phaseList -----------------------
# Phase 3 of the workflow-token-reduction roadmap eliminates mechanical
# subagents by never spawning them (docs/mechanical-agent-inventory.md). In
# rdm-wf-estimate.js the two hoists live in the DRIVER REGION's realDeps only — the
# stamped `estimate-core` block and scripts/gen-workflow-estimate.sh are
# untouched. Both are OPTIONAL: the original agent call is reached through a
# fall-through and is never deleted, so a direct `Workflow` invocation behaves
# exactly as before.
say "HOIST. rdm-wf-estimate.js driver region: mechanicalModel / phaseList hoists and their fallbacks"

cat >"$TMP/hoist.mjs" <<'NODE_HOIST'
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const wfPath = process.argv[2];
let src = fs.readFileSync(wfPath, 'utf8');
src = src.replace(/^export /m, '');
const wrapperPath = path.join(os.tmpdir(), 'verify-workflow-estimate-hoist-wrapped.mjs');
fs.writeFileSync(wrapperPath, 'export default async function(args, agent, parallel, log) {\n' + src + '\n}\n');
const mod = await import('file://' + wrapperPath + '?t=' + process.pid);
const run = mod.default;

// Every run of the REAL driver must thread the now-required environment arg.
const RDM_BIN_ARG = '/fake/bin/rdm';

const PHASES = [
  { stem: 'phase-1-a', status: 'not-started' },
  { stem: 'phase-2-b', status: 'not-started', difficulty: 'moderate', model: 'medium' },
];

function makeAgent(o) {
  o = o || {};
  const calls = [];
  const agent = async (prompt, opts) => {
    const label = (opts && opts.label) || '';
    calls.push({ label, prompt, opts });
    if (label === 'model:mechanical') return { model: o.model === undefined ? 'agent-haiku' : o.model };
    if (label === 'estimate:list') return { phases: o.phases === undefined ? PHASES : o.phases };
    if (label.startsWith('estimate:rate:')) return { stem: label.slice('estimate:rate:'.length), difficulty: 'easy', justification: 'j' };
    if (label.startsWith('estimate:write:')) return { ok: true };
    if (label.startsWith('estimate:tier:')) return { model: 'small' };
    throw new Error('unexpected agent label: ' + label);
  };
  return { agent, calls, count: (l) => calls.filter((c) => c.label === l).length };
}
const refParallel = async (thunks) => Promise.all(thunks.map((t) => Promise.resolve().then(t).catch(() => null)));
const nolog = () => {};

{
  // Both hoists supplied -> neither agent runs, and the hoisted values are used.
  const a = makeAgent({});
  const out = await run(
    { roadmap: 'rm', rdmBin: RDM_BIN_ARG, mechanicalModel: 'hoisted-haiku', phaseList: PHASES },
    a.agent,
    refParallel,
    nolog
  );
  assert.equal(a.count('model:mechanical'), 0, 'hoisted mechanicalModel -> no model:mechanical agent call');
  assert.equal(a.count('estimate:list'), 0, 'hoisted phaseList -> no estimate:list agent call');
  assert.deepEqual(out.estimated.map((e) => e.stem), ['phase-1-a'], 'the hoisted list drives the unestimated filter');
  const write = a.calls.find((c) => c.label.startsWith('estimate:write:'));
  assert.equal(write.opts.model, 'hoisted-haiku', 'the hoisted model id pins the mechanical writeback agent');
}
{
  // Neither supplied -> exactly one of each agent, as today, and the SAME result.
  const a = makeAgent({});
  const out = await run({ roadmap: 'rm', rdmBin: RDM_BIN_ARG }, a.agent, refParallel, nolog);
  assert.equal(a.count('model:mechanical'), 1, 'no hoist -> exactly one model:mechanical agent call');
  assert.equal(a.count('estimate:list'), 1, 'no hoist -> exactly one estimate:list agent call');
  assert.deepEqual(out.estimated.map((e) => e.stem), ['phase-1-a'], 'the fallback path produces the same estimate set');
}
for (const [name, bad] of [
  ['null', null],
  ['empty string', ''],
  ['whitespace only', '   '],
  ['wrong type', 42],
]) {
  const a = makeAgent({});
  await run({ roadmap: 'rm', rdmBin: RDM_BIN_ARG, mechanicalModel: bad, phaseList: PHASES }, a.agent, refParallel, nolog);
  assert.equal(a.count('model:mechanical'), 1, 'malformed mechanicalModel (' + name + ') falls back to the agent');
}
for (const [name, bad] of [
  ['null', null],
  ['object', { phases: PHASES }],
  ['string', 'phase-1-a'],
]) {
  const a = makeAgent({});
  await run({ roadmap: 'rm', rdmBin: RDM_BIN_ARG, mechanicalModel: 'hoisted-haiku', phaseList: bad }, a.agent, refParallel, nolog);
  assert.equal(a.count('estimate:list'), 1, 'malformed phaseList (' + name + ') falls back to the agent');
}
{
  // A JSON-STRINGIFIED args payload (which real LLM callers have delivered
  // despite the contract) must still surface both hoists.
  const a = makeAgent({});
  await run(JSON.stringify({ roadmap: 'rm', rdmBin: RDM_BIN_ARG, mechanicalModel: 'hoisted-haiku', phaseList: PHASES }), a.agent, refParallel, nolog);
  assert.equal(a.count('model:mechanical'), 0, 'a stringified args payload still surfaces mechanicalModel');
  assert.equal(a.count('estimate:list'), 0, 'a stringified args payload still surfaces phaseList');
}
console.log('estimate hoist assertions passed');
NODE_HOIST

if run_node "$TMP/hoist.mjs" "$WF"; then
    pass "estimate hoist/fallback verified against the real driver under a recording fake agent"
else
    fail "estimate hoist/fallback assertions failed against $WF"
fi

# Planted-mutation self-tests: each fallback branch must be load-bearing.
assert_wf_mutant_fails() {
    mutant=$1
    desc=$2
    if cmp -s "$WF" "$mutant"; then
        fail "HOIST: planted mutation was a no-op — $desc"
    fi
    if run_node "$TMP/hoist.mjs" "$mutant" >/dev/null 2>&1; then
        fail "HOIST: assertions PASSED against a driver that $desc — they are vacuous"
    fi
    pass "HOIST: assertions fire when the driver $desc"
}

# (1) Drop the model fallback: return the (possibly absent) hoist unconditionally.
awk '
    index($0, "  resolveMechanicalModel: async function () {") { print; print "    return String(rawEstimateArgs.mechanicalModel || \"\").trim()"; skipping = 1; next }
    skipping && index($0, "  },") == 1 { skipping = 0; print; next }
    skipping { next }
    { print }
' "$WF" >"$TMP/mutant-no-model-fallback.js"
assert_wf_mutant_fails "$TMP/mutant-no-model-fallback.js" "drops the model:mechanical fallback"

# (2) Weaken the phaseList guard to "anything truthy".
sed 's/if (Array.isArray(rawEstimateArgs.phaseList)) {/if (rawEstimateArgs.phaseList) {/' "$WF" >"$TMP/mutant-weak-list-guard.js"
assert_wf_mutant_fails "$TMP/mutant-weak-list-guard.js" "weakens the phaseList shape guard to any truthy value"

# --- SHIM: the LOCAL rdm-estimate shim gathers and passes both hoists ---------
# `.claude/skills/rdm-estimate/SKILL.md` is a LOCAL dogfood shim; its distributed
# template (rdm-core/src/templates/skill-estimate-{cli,mcp}.md) is NOT a Workflow
# shim yet (tracked by task convert-remaining-skill-templates-to-workflow-shims),
# so this check belongs here and NOT in verify-agent-config-distribution.sh.
say "HOIST-SHIM. .claude/skills/rdm-estimate/SKILL.md gathers and passes mechanicalModel + phaseList"

assert_shim_gathers() {
    grep -qF 'rdm model resolve mechanical' "$1" || return 1
    grep -qF 'phase list --roadmap <slug>' "$1" || return 1
    grep -qF 'mechanicalModel' "$1" || return 1
    grep -qF 'phaseList' "$1" || return 1
    # Occurrence floor: each key is named in the gathering bullet AND in the
    # workflow-invocation arg object, so a single stray mention cannot satisfy it.
    [ "$(grep -cF 'mechanicalModel' "$1")" -ge 2 ] || return 1
    [ "$(grep -cF 'phaseList' "$1")" -ge 2 ] || return 1
    # ENVIRONMENT ARGS (§ 9): `rdmBin` now DEFAULTS to a plain `rdm` on PATH, so
    # a shim that omits it degrades silently to whatever global rdm is first on
    # PATH — inside this repo, the stale build the development-build rule
    # forbids. That is why this check survives the contract reversal unchanged:
    # it guards a silent wrong-binary failure rather than a loud one. Both keys
    # are named in the config bullet AND in the invocation payload.
    grep -qF 'rdmBin' "$1" || return 1
    grep -qF 'project' "$1" || return 1
    [ "$(grep -cF 'rdmBin' "$1")" -ge 2 ] || return 1
    [ "$(grep -cF 'rdmBin: "./target/debug/rdm"' "$1")" -ge 1 ] || return 1
    return 0
}
assert_shim_gathers "$SKILL" ||
    fail "HOIST-SHIM: $SKILL must gather 'rdm model resolve mechanical' and 'rdm phase list --format json' and pass mechanicalModel + phaseList + rdmBin + project (each named at least twice)"
pass "HOIST-SHIM: the local shim gathers and passes both hoisted args plus rdmBin/project"

sed 's/mechanicalModel/mechModel/g' "$SKILL" >"$TMP/shim-typo.md"
if assert_shim_gathers "$TMP/shim-typo.md"; then
    fail "HOIST-SHIM: detector missed a typo'd arg key in the shim"
fi
sed 's/rdmBin/rdmBn/g' "$SKILL" >"$TMP/shim-typo-bin.md"
if assert_shim_gathers "$TMP/shim-typo-bin.md"; then
    fail "HOIST-SHIM: detector missed a typo'd rdmBin arg key in the shim"
fi
pass "HOIST-SHIM: detector fires on a typo'd arg key in the shim (mechanicalModel and rdmBin)"

# --- 9. PARAMETERIZATION ------------------------------------------------------
# estimate names NO particular rdm executable and NO particular rdm project:
# both arrive as RUNTIME args (`rdmBin`, `project`) and are threaded into every
# prompt that shells out. This mirrors scripts/verify-workflow-dispatch.sh § 9,
# which gates the SAME contract for dispatch-phase — the helpers here are copies
# in shape, not a second contract. Four sub-gates:
#
#   9a — per-file literal zeroing across BOTH copies (lib + workflow), asserted
#        PER FILE so a half-applied edit cannot pass.
#   9b — a DRIVEN prompt capture: run the real workflow under a capturing fake
#        agent, tokenize every emitted `rdm <subcommand>` occurrence, and check
#        it against the project-agnostic allow-list expressed AS DATA.
#   9c — the fail-closed `rdmBin` rule (and the optional-project validation),
#        plus a grep proving it was NOT implemented as an existence preflight.
#   9d — planted-mutation self-tests for 9b.
say "9. Parameterization: no hardcoded rdm binary or project; the environment axes are runtime args"

# --- 9a. Per-file literal zeroing ---------------------------------------------
say "9a. Per-file literal zeroing (lib + workflow)"

# assert_no_env_literals <file> — zero occurrences of THIS repo's dev binary path
# and zero of THIS repo's project flag. Deliberately per-file: a concatenated
# stream would let a zero in one copy mask a hit in the other, which is exactly
# the half-applied-edit failure mode (the estimate-core block is byte-stamped,
# the driver below it is hand-swept). Comments and prose count too — a leftover
# explanatory comment naming either literal is the same staleness hazard.
assert_no_env_literals() {
    _f=$1
    _bin=$(grep -c 'target/debug/rdm' "$_f" || true)
    _proj=$(grep -c -- '--project rdm' "$_f" || true)
    [ "$_bin" -eq 0 ] && [ "$_proj" -eq 0 ]
}

for f in "$LIB" "$WF"; do
    if assert_no_env_literals "$f"; then
        pass "9a: ${f#"$REPO_ROOT"/} carries neither 'target/debug/rdm' nor '--project rdm'"
    else
        grep -n 'target/debug/rdm' "$f" >&2 || true
        grep -n -- '--project rdm' "$f" >&2 || true
        fail "9a: $f still hardcodes this repo's rdm binary and/or project — both must be runtime args"
    fi
done

# Self-test: plant the literals into EACH file in turn and prove the per-file
# check fires on each one individually (so restoring only one cannot go green).
_i=0
for f in "$LIB" "$WF"; do
    _i=$((_i + 1))
    cp "$f" "$TMP/env-mutant-$_i"
    printf '\n// planted: ./target/debug/rdm phase show --project rdm\n' >>"$TMP/env-mutant-$_i"
    if assert_no_env_literals "$TMP/env-mutant-$_i"; then
        fail "9a: the per-file literal check did not fire on a planted literal in $f — the gate is vacuous"
    fi
done
pass "9a: the per-file check fires independently on both planted mutants"

# --- 9b. Driven prompt capture ------------------------------------------------
say "9b. Driven prompt capture: every emitted rdm invocation honors the allow-list"

cat >"$TMP/paramz.mjs" <<'NODE_PARAMZ'
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const wfPath = process.argv[2];
const src = fs.readFileSync(wfPath, 'utf8').replace(/^export /m, '');
const wrapperPath = path.join(os.tmpdir(), 'verify-workflow-estimate-paramz-wrapped.mjs');
fs.writeFileSync(wrapperPath, 'export default async function(args, agent, parallel, log) {\n' + src + '\n}\n');
const mod = await import('file://' + wrapperPath + '?t=' + process.pid);
const run = mod.default;

async function refParallel(thunks) {
  return Promise.all(thunks.map((t) => Promise.resolve().then(t).catch(() => null)));
}

// The injected binary is deliberately NOT a plausible real path, so a
// re-hardcoded './target/debug/rdm' anywhere shows up as a mismatch rather than
// blending in.
const FAKE_BIN = '/fake/bin/rdm';

// The PROJECT-AGNOSTIC ALLOW-LIST, expressed as DATA — the SAME array
// verify-workflow-dispatch.sh § 9b uses (dispatch-phase's landed contract, not
// re-derived here). These subcommands reject `--project` outright, so they must
// carry NO project flag; everything else is project-scoped and MUST carry it
// whenever a project was configured.
const PROJECT_AGNOSTIC = ['model resolve', 'commit', 'status', 'discard'];

const PHASES = [{ number: 1, stem: 'phase-1-x', title: 'X', status: 'not-started' }];

// A capturing fake agent. NOTE: the capture runs deliberately supply NEITHER
// hoist (`mechanicalModel` / `phaseList`) — a hoist short-circuits the agent
// that builds the corresponding prompt, silently narrowing the scan.
function makeCapture() {
  const prompts = [];
  const agent = async (prompt, opts) => {
    prompts.push(String(prompt));
    const label = (opts && opts.label) || '';
    if (label === 'model:mechanical') return { model: 'm-mech' };
    if (label === 'estimate:list') return { phases: PHASES };
    if (label.startsWith('estimate:rate:')) {
      return { stem: label.slice('estimate:rate:'.length), difficulty: 'moderate', justification: 'j' };
    }
    if (label.startsWith('estimate:write:')) return { ok: true };
    if (label.startsWith('estimate:tier:')) return { model: 'medium' };
    throw new Error('unexpected agent label: ' + label);
  };
  return { agent, prompts };
}

// Tokenize `<bin> <subcommand>` occurrences out of a prompt. The binary token is
// whatever non-space run precedes the subcommand, so a re-hardcoded path is
// caught by comparison rather than by being silently skipped. Same regex as
// verify-workflow-dispatch.sh § 9b.
const INVOCATION = /(^|[\s`])((?:[^\s`]*\/)?rdm)\s+([a-z][a-z-]*(?:\s+[a-z][a-z-]*)?)/g;

// Only COMMAND-BEARING lines are tokenized. Every command these prompts emit is
// either an indented command line ('  <bin> phase list …') or a backtick-quoted
// inline directive ('Run `<bin> phase show …`'). Flush-left PROSE that merely
// names the tool — the estimator prompt opens "You are a difficulty-estimation
// agent for a single rdm phase." — is not an invocation, and tokenizing it would
// report a false hit whose "binary" is the bare word `rdm`. The non-vacuity
// floors below prove the filter is not silently dropping real commands.
function isCommandLine(line) {
  return /^\s{2,}\S/.test(line) || line.includes('`');
}

function scan(prompts) {
  const out = [];
  for (const p of prompts) {
    for (const line of p.split('\n')) {
      if (!isCommandLine(line)) continue;
      INVOCATION.lastIndex = 0;
      let m;
      while ((m = INVOCATION.exec(line)) !== null) {
        out.push({ bin: m[2], two: m[3], line });
      }
    }
  }
  return out;
}

async function capture(args) {
  const c = makeCapture();
  await run(args, c.agent, refParallel, () => {});
  return scan(c.prompts);
}

// --- Run A: a project IS configured.
const withProject = await capture({ roadmap: 'rm', rdmBin: FAKE_BIN, project: 'demo' });
assert.ok(withProject.length > 0, 'the scan found no rdm invocations at all — it cannot pass vacuously');

const seen = new Set();
for (const occ of withProject) {
  assert.equal(occ.bin, FAKE_BIN, 'an rdm invocation used ' + occ.bin + ' instead of the injected rdmBin: ' + occ.line);
  seen.add(occ.two);
  const agnostic = PROJECT_AGNOSTIC.includes(occ.two);
  if (agnostic) {
    assert.ok(!occ.line.includes('--project'), 'project-agnostic `rdm ' + occ.two + '` must carry NO project flag: ' + occ.line);
  } else {
    assert.ok(occ.line.includes(' --project demo'), 'project-scoped `rdm ' + occ.two + '` must carry " --project demo": ' + occ.line);
  }
}

// Non-vacuity floors: the scan must actually have reached every command shape,
// not merely found nothing to object to.
for (const n of ['phase list', 'phase show', 'phase update', 'model resolve']) {
  assert.ok(seen.has(n), 'expected at least one `rdm ' + n + '` occurrence, saw: ' + [...seen].join(', '));
}
const resolves = withProject.filter((o) => o.two === 'model resolve');
assert.ok(resolves.length >= 1, 'expected at least one `rdm model resolve` occurrence');
for (const r of resolves) {
  assert.ok(!r.line.includes('--project'), '`rdm model resolve` is on the allow-list and must never gain a project flag: ' + r.line);
}

// --- Run B: NO project configured -> not a single --project anywhere.
const noProject = await capture({ roadmap: 'rm', rdmBin: FAKE_BIN });
assert.ok(noProject.length > 0, 'the no-project scan found no rdm invocations at all');
for (const occ of noProject) {
  assert.equal(occ.bin, FAKE_BIN, '(no project): an rdm invocation used ' + occ.bin + ': ' + occ.line);
}
const stray = noProject.filter((o) => o.line.includes('--project'));
assert.equal(stray.length, 0, '(no project): expected zero --project occurrences, found: ' + stray.map((o) => o.line).join(' | '));

// Determinism: the same args produce byte-identical prompt captures.
const d1 = await capture({ roadmap: 'rm', rdmBin: FAKE_BIN, project: 'demo' });
const d2 = await capture({ roadmap: 'rm', rdmBin: FAKE_BIN, project: 'demo' });
assert.deepEqual(d2, d1, 'the prompt capture must be deterministic across identical runs');

console.log('all estimate parameterization prompt-capture assertions passed');
NODE_PARAMZ

if run_node "$TMP/paramz.mjs" "$WF"; then
    pass "9b: every emitted rdm invocation uses the injected binary and honors the project-agnostic allow-list (with and without a project)"
else
    fail "9b: parameterization prompt-capture assertions failed"
fi

# --- 9c. Fail-closed rdmBin ---------------------------------------------------
say "9c. Defaulted rdmBin: an absent arg resolves to a plain 'rdm', a wrong-TYPE arg still throws, and neither is an existence preflight"

cat >"$TMP/rdmbin.mjs" <<'NODE_RDMBIN'
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const libPath = process.argv[2];
const wfPath = process.argv[3];
const { parseEstimateArgs, projectFlag, resolveRdmBin, parseProjectArg } = await import('file://' + libPath);

// (1a) An ABSENT-ish value DEFAULTS to a plain `rdm` on PATH. A plugin-installed
// consumer has no repo-local build path to pass; this repo's own stale-global
// hazard is handled by RDM_BIN in .mise.toml (gated by
// verify-workflow-dispatch.sh § 9c-dogfood), not by refusing to resolve.
for (const absent of [undefined, null, '', '   ', '\t']) {
  assert.equal(resolveRdmBin(absent), 'rdm', 'an absent rdmBin (' + JSON.stringify(absent) + ') must default to "rdm"');
  assert.equal(
    parseEstimateArgs({ roadmap: 'rm', rdmBin: absent }).rdmBin,
    'rdm',
    'parseEstimateArgs must default an absent rdmBin (' + JSON.stringify(absent) + ') to "rdm"'
  );
}
assert.equal(parseEstimateArgs({ roadmap: 'rm' }).rdmBin, 'rdm', 'a missing rdmBin key defaults to "rdm"');
assert.equal(parseEstimateArgs(JSON.stringify({ roadmap: 'rm' })).rdmBin, 'rdm', 'a stringified payload defaults rdmBin to "rdm"');

// (1b) A present-but-wrong-TYPE value STILL throws — degrading a `rdmBin: 42`
// typo to PATH would reintroduce the silent-wrong-binary hazard.
for (const bad of [42, {}, [], true]) {
  assert.throws(
    () => parseEstimateArgs({ roadmap: 'rm', rdmBin: bad }),
    /rdmBin/,
    'a non-string rdmBin (' + JSON.stringify(bad) + ') must still throw'
  );
}

// ORDERING: the pre-existing required-roadmap throw still runs FIRST. This
// assertion is UNCHANGED and deliberately kept: it now holds for a weaker
// reason (an absent rdmBin no longer competes to throw at all), but it is the
// only guard that the far more common missing-roadmap mis-invocation keeps its
// actionable message, and it must not be collateral damage of the rdmBin edit.
assert.throws(() => parseEstimateArgs({}), /roadmap slug is required/, 'a payload missing BOTH reports the roadmap first');
assert.throws(() => parseEstimateArgs({ rdmBin: 'rdm' }), /roadmap slug is required/, 'roadmap is still checked first');

// The wrong-TYPE error must stay actionable: it names the sentinel and the
// default that omitting the arg entirely would take.
try {
  parseEstimateArgs({ roadmap: 'rm', rdmBin: 42 });
  assert.fail('expected a throw');
} catch (e) {
  assert.match(e.message, /rdmBin must be a string/, 'the message names the type requirement');
  assert.match(e.message, /"rdm"/, 'the message names the explicit PATH sentinel');
  assert.match(e.message, /PATH/, 'the message names what omitting the arg entirely does');
}

// (2) The explicit sentinel is accepted VERBATIM — a downstream repo that wants
// PATH resolution opts in on purpose.
assert.equal(parseEstimateArgs({ roadmap: 'rm', rdmBin: 'rdm' }).rdmBin, 'rdm', "the 'rdm' sentinel is accepted verbatim");
assert.equal(parseEstimateArgs({ roadmap: 'rm', rdmBin: '/opt/x/rdm' }).rdmBin, '/opt/x/rdm', 'an absolute path is accepted verbatim');
assert.equal(resolveRdmBin('rdm'), 'rdm', 'resolveRdmBin passes the sentinel through');
// DISCRIMINATING: the sentinel path is VERBATIM pass-through, not the default
// branch — a trailing space survives, which `return 'rdm'` could not produce.
assert.equal(resolveRdmBin('rdm '), 'rdm ', 'a non-empty value is returned verbatim, not normalized to the default');

// (3) `project` is OPTIONAL, falsy means NO flag, and a hostile value is
// rejected rather than escaped (it is interpolated into a Bash-agent prompt).
assert.equal(parseEstimateArgs({ roadmap: 'rm', rdmBin: 'rdm' }).project, '', 'an absent project means no flag');
for (const falsy of [undefined, null, '', 0, false]) {
  assert.equal(parseProjectArg(falsy), '', 'falsy project ' + JSON.stringify(falsy) + ' means no flag');
  assert.equal(projectFlag({ project: parseProjectArg(falsy) }), '', 'a falsy project emits no flag at all');
}
assert.equal(projectFlag({}), '', 'an empty cfg emits no flag');
assert.equal(projectFlag(null), '', 'a null cfg emits no flag');
assert.equal(projectFlag({ project: 'demo' }), ' --project demo', 'a configured project emits the flag');
for (const hostile of ['a b', 'a;rm -rf /', '$(x)', '`x`', 'a\nb', 'a|b', 7, {}]) {
  assert.throws(() => parseProjectArg(hostile), /project must be a plain project name/, 'hostile project ' + JSON.stringify(hostile) + ' must be rejected');
}
assert.equal(parseEstimateArgs({ roadmap: 'rm', rdmBin: 'rdm', project: 'rdm-atlas.v2_x' }).project, 'rdm-atlas.v2_x', 'a plain project name survives');

// (4) Driving the wrapped workflow with NO rdmBin must now RESOLVE, and every
// rdm command it emits must name the bare `rdm` default — never a repo-local
// build path leaking back in as a hardcoded literal.
const src = fs.readFileSync(wfPath, 'utf8').replace(/^export /m, '');
const wrapperPath = path.join(os.tmpdir(), 'verify-workflow-estimate-rdmbin-wrapped.mjs');
fs.writeFileSync(wrapperPath, 'export default async function(args, agent, parallel, log) {\n' + src + '\n}\n');
const mod = await import('file://' + wrapperPath + '?t=' + process.pid);
const PHASES = [{ number: 1, stem: 'phase-1-x', title: 'X', status: 'not-started' }];
const prompts = [];
let agentCalls = 0;
const spy = async (prompt, opts) => {
  agentCalls++;
  prompts.push(String(prompt));
  const label = (opts && opts.label) || '';
  if (label === 'model:mechanical') return { model: 'm-mech' };
  if (label === 'estimate:list') return { phases: PHASES };
  if (label.startsWith('estimate:rate:')) {
    return { stem: label.slice('estimate:rate:'.length), difficulty: 'moderate', justification: 'j' };
  }
  if (label.startsWith('estimate:write:')) return { ok: true };
  if (label.startsWith('estimate:tier:')) return { model: 'medium' };
  return null;
};
const refParallel = async (thunks) => Promise.all(thunks.map((t) => Promise.resolve().then(t).catch(() => null)));
const out = await mod.default({ roadmap: 'rm' }, spy, refParallel, () => {});
assert.ok(out !== undefined, 'a Workflow invocation with no rdmBin must resolve, not throw');
assert.ok(agentCalls > 0, 'the run must actually have dispatched agents — otherwise this assertion is vacuous');

// Same command-line filter as § 9b: only indented or backtick-quoted lines are
// invocations; flush-left prose merely naming the tool is not.
const INVOCATION = /(^|[\s`])((?:[^\s`]*\/)?rdm)\s+[a-z][a-z-]*/g;
let invocations = 0;
for (const p of prompts) {
  for (const line of p.split('\n')) {
    if (!(/^\s{2,}\S/.test(line) || line.includes('`'))) continue;
    INVOCATION.lastIndex = 0;
    let m;
    while ((m = INVOCATION.exec(line)) !== null) {
      invocations++;
      assert.equal(m[2], 'rdm', 'an emitted command used ' + m[2] + ' instead of the bare `rdm` default: ' + line);
    }
  }
}
assert.ok(invocations > 0, 'the scan found no rdm invocations at all — it cannot pass vacuously');
for (const p of prompts) {
  assert.ok(!p.includes('./target/debug/rdm'), 'a repo-local build path leaked into a prompt under the bare default');
}

console.log('all defaulted rdmBin assertions passed');
NODE_RDMBIN

if run_node "$TMP/rdmbin.mjs" "$LIB" "$WF"; then
    pass "9c: rdmBin defaults to a bare 'rdm' when absent (every emitted command uses it), a wrong-TYPE value still throws, the 'rdm' sentinel passes through verbatim, the roadmap throw still runs first, and project is validated"
else
    fail "9c: defaulted rdmBin assertions failed"
fi

# The guard must NOT be an existence preflight. `which -a rdm` resolves to the
# stale global build in this repo, so an existence check passes while running
# exactly the binary the development-build rule forbids. COMMENT LINES ARE
# STRIPPED FIRST so a rationale comment naming the rejected mechanism is not
# itself flagged.
assert_no_existence_preflight() {
    grep -vE '^[[:space:]]*(//|\*|/\*)' "$1" |
        grep -nE 'which +(-a +)?rdm|command -v|existsSync|accessSync|statSync' >"$TMP/preflight-hits" 2>/dev/null || true
    [ ! -s "$TMP/preflight-hits" ]
}
for f in "$LIB" "$WF"; do
    if ! assert_no_existence_preflight "$f"; then
        cat "$TMP/preflight-hits" >&2
        fail "9c: $f must not implement the rdmBin guard as an existence preflight — the guard is on the ABSENCE of the argument"
    fi
done
pass "9c: no existence preflight (which rdm / command -v / existsSync) in either estimate copy"

cp "$WF" "$TMP/preflight-mutant.js"
printf "\nconst ok = existsSync(rdmBin)\n" >>"$TMP/preflight-mutant.js"
if assert_no_existence_preflight "$TMP/preflight-mutant.js"; then
    fail "9c: the existence-preflight detector missed a planted existsSync call — the gate is vacuous"
fi
pass "9c: the existence-preflight detector fires on planted code while ignoring the rationale prose"

# --- 9d. Planted-mutation self-tests for 9b -----------------------------------
say "9d. Planted-mutation self-tests: the allow-list assertion is not vacuous"

# (i) a builder RE-HARDCODES this repo's dev binary path.
sed "s|+ bin + ' phase list |+ './target/debug/rdm' + ' phase list |" "$WF" >"$TMP/pz-mut-bin.js"
if cmp -s "$WF" "$TMP/pz-mut-bin.js"; then
    fail "9d(i): the re-hardcoded-binary mutation did not apply — the self-test is not exercising anything"
fi
if run_node "$TMP/paramz.mjs" "$TMP/pz-mut-bin.js" >/dev/null 2>&1; then
    fail "9d(i): a re-hardcoded rdm binary was NOT detected — the binary assertion is vacuous"
fi
pass "9d(i): detector fires when a builder re-hardcodes the rdm binary"

# (ii) the ALLOW-LIST member `rdm model resolve` wrongly gains a project flag —
#      a command rdm rejects at runtime that a naive whole-file grep still
#      accepts. This is the exact failure mode 9b exists to catch.
sed "s|' model resolve mechanical',|' model resolve mechanical' + projectFlag(cfg),|" "$WF" >"$TMP/pz-mut-agnostic.js"
if cmp -s "$WF" "$TMP/pz-mut-agnostic.js"; then
    fail "9d(ii): the allow-list mutation did not apply"
fi
if run_node "$TMP/paramz.mjs" "$TMP/pz-mut-agnostic.js" >/dev/null 2>&1; then
    fail "9d(ii): a project flag on 'rdm model resolve' was NOT detected — the allow-list assertion is vacuous"
fi
pass "9d(ii): detector fires when an allow-list subcommand gains a project flag"

# (iii) a project-scoped builder DROPS its flag.
sed "s|+ ' --roadmap ' + slug + proj + ' --format json'|+ ' --roadmap ' + slug + ' --format json'|g" "$WF" >"$TMP/pz-mut-drop.js"
if cmp -s "$WF" "$TMP/pz-mut-drop.js"; then
    fail "9d(iii): the dropped-flag mutation did not apply"
fi
if run_node "$TMP/paramz.mjs" "$TMP/pz-mut-drop.js" >/dev/null 2>&1; then
    fail "9d(iii): a project-scoped command that dropped its project flag was NOT detected"
fi
pass "9d(iii): detector fires when a project-scoped builder drops '+ proj'"

say "verify-workflow-estimate.sh: ALL GREEN"
