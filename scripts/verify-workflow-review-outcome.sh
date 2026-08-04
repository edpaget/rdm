#!/bin/sh
# Hermetic regression for the standalone review-refute-fix workflow's full
# dispatch-shaped OUTCOME path (headless-skill-workflows roadmap).
#
# `.claude/workflows/rdm-wf-review-refute-fix.js` grew from a survivors-only wrapper
# (`{ mode, survivors }`) into a full standalone code-review workflow: when
# invoked as `{ mode: 'code', roadmap, phase }` or `{ mode: 'code', task }`, it
# now derives real diff signals from the item's worktree (mirroring
# dispatch-phase's code gate, same fail-open contract), runs the ONE canonical
# `buildReviewPipeline('code')`, and composes the survivors through ONE
# `classifyOutcome` call plus the existing `statusFor`/`writesCompletion`/
# `summarizeFindings`/`gateFor` helpers into the dispatch-shaped OUTCOME
# contract, with an optional mechanical `args.gate` status-persist step for
# headless/ad hoc callers. Legacy invocations (`mode: 'plan'`, or `mode: 'code'`
# with no item identifier) keep returning the original `{ mode, survivors }`
# shape. None of this touches `.claude/workflows/lib/review.mjs` — this harness
# is a SIBLING of `scripts/verify-workflow-review.sh` (which still gates the
# shared review source and its two `--check` projections), not a replacement.
#
# This harness gates:
#
#   0. BYTE-IDENTICAL COPY — `.claude/workflows/rdm-wf-review-refute-fix.js` and
#      `rdm-core/src/templates/workflows/rdm-wf-review-refute-fix.js` are identical
#      (enforced independently by `cargo test
#      generate_workflows_are_byte_identical_to_source` and
#      `scripts/verify-agent-config-distribution.sh`; this is a fast local
#      cross-check), with a planted-mutation self-test.
#   1. DRIVER-REGION STRUCTURE — extracting ONLY the driver region below the
#      `review-refute-fix:end` marker (the stamped/generated block above it is
#      untouched and already contains an unrelated `function classifyOutcome
#      (input) {` definition, so a WHOLE-FILE grep for `classifyOutcome(` can
#      never equal 1 even in a correct implementation): within that region,
#      exactly one `= *buildReviewPipeline('code')` binding (the same
#      binding-not-substring convention `scripts/verify-workflow-dispatch.sh`
#      already uses) and exactly one `classifyOutcome(` call. A planted
#      duplicate call site proves the check is not vacuous; a
#      passing-on-the-real-file assertion proves it is not accidentally
#      satisfied by scoping to the wrong region, and a whole-file count of 2 for
#      `classifyOutcome(` is asserted directly, to document why the whole-file
#      check would be mathematically impossible to pass.
#   2. HYGIENE — the whole file (unlike section 1, this scan is unambiguous
#      because none of these three literals legitimately occurs anywhere in the
#      file today) contains no `Done:` trailer literal and no
#      `Date.now(`/`Math.random(`, with a planted-violation self-test.
#   3. BEHAVIOR — the real driver, executed in Node with injected fake
#      agent/pipeline/parallel (zero LLM calls, zero rdm state touched): the
#      full OUTCOME shape for a `reviewed` and a `rework` seed; diff-signals
#      fail-open (an empty/thrown diff agent result omits `signals` entirely,
#      so every code dimension fires) versus a populated diff (deriveSignals
#      output threaded through as `signals`); the mutual-exclusion guard on
#      `{ task, roadmap, phase }` all at once; both legacy backward-compatible
#      shapes are unaffected; and the optional `args.gate` mechanical
#      status-persist step (off by default, on only when requested, and never
#      running `rdm commit`).
#   4. SKILL SHIM — `.claude/skills/rdm-review/SKILL.md` references the
#      `rdm-wf-review-refute-fix` workflow AND still carries its interactive
#      Report/Act/Gate sections and the `Done:`-trailer gate mechanism (it must
#      NOT have become a headless pass-through), and
#      `scripts/gen-skill-review.sh --check` still passes in both `--mode code`
#      and `--mode plan` (proving the untouched, separately-generated
#      `skill-review-{cli,mcp}.md` / `skill-plan-review-{cli,mcp}.md` templates
#      are unaffected by this hand-authored prose trim).
#   5. STATIC INVARIANT — `meta.phases` in rdm-wf-review-refute-fix.js lists exactly
#      the distinct `phase:` tags the driver + the inlined review block
#      actually emit (`Find`, `Refute`, `Review`, `Gate`).
#   6. PARAMETERIZATION — review-refute-fix names NO particular rdm executable
#      and NO particular rdm project: both are RUNTIME args (`rdmBin`,
#      `project`), the same contract dispatch-phase landed (gated there by
#      `scripts/verify-workflow-dispatch.sh` § 9). Per-file literal zeroing over
#      BOTH copies with planted mutants, plus a NEGATIVE pin on
#      `lib/review.mjs`'s own counts (6a); a driven prompt capture with
#      `gate: true` and no `diff` hoist, checking every emitted
#      `rdm <subcommand>` against the project-agnostic allow-list expressed AS
#      DATA (6b); the fail-closed `rdmBin` rule AND its documented carve-out —
#      the standalone path throws without it while BOTH legacy survivors-only
#      shapes still succeed without it, because they emit zero rdm invocations
#      (6c); and self-tests proving 6b is not vacuous (6d).
#
# Node is used only as a host to drive the pure/injectable parts of the real
# workflow script; it is stdlib-only (node:assert), no package.json /
# node_modules / third-party packages. node is pinned in .mise.toml.
#
# Requires: node (via PATH or `mise exec node --`).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)

WF="$REPO_ROOT/.claude/workflows/rdm-wf-review-refute-fix.js"
WF_TEMPLATE="$REPO_ROOT/rdm-core/src/templates/workflows/rdm-wf-review-refute-fix.js"
SKILL="$REPO_ROOT/.claude/skills/rdm-review/SKILL.md"
SKILL_GEN="$REPO_ROOT/scripts/gen-skill-review.sh"
MARKER_END='review-refute-fix:end'

# Clear rdm-related env vars inherited from the caller's shell for hermeticity.
unset RDM_ROOT RDM_PROJECT RDM_STAGE RDM_FORMAT RDM_PLAN_REPO RDM_PLAN_REPO_TOKEN RDM_PLAN_REPO_PATH 2>/dev/null || true

say() { printf '\n\033[1;34m==>\033[0m %s\n' "$*"; }
fail() {
    printf '\n\033[1;31m[FAIL]\033[0m %s\n' "$*" >&2
    exit 1
}
pass() { printf '\033[1;32m[ok]\033[0m %s\n' "$*"; }

[ -f "$WF" ] || fail "workflow script not found: $WF"
[ -f "$WF_TEMPLATE" ] || fail "shipped template not found: $WF_TEMPLATE"
[ -f "$SKILL" ] || fail "skill not found: $SKILL"
[ -f "$SKILL_GEN" ] || fail "skill generator not found: $SKILL_GEN"

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

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

# --- 0. BYTE-IDENTICAL COPY ---------------------------------------------------
say "0. Byte-identical copy: dogfood workflow vs shipped template"
if diff -u "$WF" "$WF_TEMPLATE" >/dev/null 2>&1; then
    pass "rdm-wf-review-refute-fix.js and its shipped template copy are byte-identical"
else
    diff -u "$WF" "$WF_TEMPLATE" >&2 || true
    fail "$WF and $WF_TEMPLATE have drifted — mirror the edit byte-for-byte"
fi

# Self-test: a planted difference must be detected.
cp "$WF_TEMPLATE" "$TMP/planted-template.js"
printf '\n// planted drift\n' >>"$TMP/planted-template.js"
if diff -q "$WF" "$TMP/planted-template.js" >/dev/null 2>&1; then
    fail "byte-identical detector broken — planted drift was not detected"
fi
pass "byte-identical detector fires on planted drift"

# --- 1. DRIVER-REGION STRUCTURE ------------------------------------------------
say "1. Driver region: exactly one buildReviewPipeline('code') binding, one classifyOutcome( call"

extract_driver_region() {
    awk -v m="$MARKER_END" '$0 ~ m { found = 1 } found' "$1"
}

extract_driver_region "$WF" >"$TMP/driver-region.js"
[ -s "$TMP/driver-region.js" ] || fail "extracted an EMPTY driver region from $WF — is the '$MARKER_END' marker present?"

PIPELINE_BINDINGS=$(grep -cE "= *buildReviewPipeline\('code'\)" "$TMP/driver-region.js" | tr -d ' ')
CLASSIFY_CALLS=$(grep -cE 'classifyOutcome\(' "$TMP/driver-region.js" | tr -d ' ')

[ "$PIPELINE_BINDINGS" -eq 1 ] ||
    fail "expected exactly ONE 'buildReviewPipeline('code')' binding in the driver region, found $PIPELINE_BINDINGS"
[ "$CLASSIFY_CALLS" -eq 1 ] ||
    fail "expected exactly ONE 'classifyOutcome(' call in the driver region, found $CLASSIFY_CALLS"
pass "driver region: 1 buildReviewPipeline('code') binding, 1 classifyOutcome( call"

# Document (not merely assert) WHY a whole-file grep for classifyOutcome( could
# never pass: the stamped block above the marker already declares the function.
WHOLE_FILE_CLASSIFY=$(grep -cE 'classifyOutcome\(' "$WF" | tr -d ' ')
[ "$WHOLE_FILE_CLASSIFY" -eq 2 ] ||
    fail "expected the WHOLE file to contain classifyOutcome( exactly twice (1 definition + 1 call) — got $WHOLE_FILE_CLASSIFY; the driver-region scoping assumption above may no longer hold"
pass "whole-file classifyOutcome( count is 2 (1 stamped definition + 1 driver-region call) — confirms a whole-file check could never equal 1"

# Self-test A: planted duplicate call site WITHIN the driver region must trip
# the count-based check (proves it is not vacuous).
cp "$WF" "$TMP/planted-dup.js"
awk -v m="$MARKER_END" '
    { print }
    $0 ~ m && !done { print "const _plantedDuplicate = classifyOutcome(classifierInput)"; done = 1 }
' "$WF" >"$TMP/planted-dup.js"
extract_driver_region "$TMP/planted-dup.js" >"$TMP/planted-dup-region.js"
DUP_CLASSIFY=$(grep -cE 'classifyOutcome\(' "$TMP/planted-dup-region.js" | tr -d ' ')
if [ "$DUP_CLASSIFY" -eq 1 ]; then
    fail "self-test broken — planted duplicate classifyOutcome( call site in the driver region was not detected"
fi
pass "self-test: planted duplicate classifyOutcome( call site in the driver region is detected (count=$DUP_CLASSIFY)"

# Self-test B: planted duplicate buildReviewPipeline('code') BINDING.
awk -v m="$MARKER_END" '
    { print }
    $0 ~ m && !done { print "const _plantedRunReview = buildReviewPipeline(\047code\047)"; done = 1 }
' "$WF" >"$TMP/planted-dup2.js"
extract_driver_region "$TMP/planted-dup2.js" >"$TMP/planted-dup2-region.js"
DUP_PIPELINE=$(grep -cE "= *buildReviewPipeline\('code'\)" "$TMP/planted-dup2-region.js" | tr -d ' ')
if [ "$DUP_PIPELINE" -eq 1 ]; then
    fail "self-test broken — planted duplicate buildReviewPipeline('code') binding in the driver region was not detected"
fi
pass "self-test: planted duplicate buildReviewPipeline('code') binding in the driver region is detected (count=$DUP_PIPELINE)"

# Regression guard: the real, unmutated file must PASS — proving the check does
# not fail merely because the stamped block's function definition exists above
# the marker (i.e. it is truly driver-region-scoped, not a disguised whole-file
# check that happens to still work).
if [ "$PIPELINE_BINDINGS" -ne 1 ] || [ "$CLASSIFY_CALLS" -ne 1 ]; then
    fail "regression: the real unmutated file must pass the driver-region-scoped check"
fi
pass "regression guard: the real file passes the driver-region-scoped check (not a whole-file check in disguise)"

# --- 2. HYGIENE ----------------------------------------------------------------
say "2. Hygiene: no Done: trailer literal, no Date.now(/Math.random( anywhere in the file"
if grep -nE 'Done:|Date\.now\(|Math\.random\(' "$WF" >&2; then
    fail "$WF must not contain a 'Done:' trailer literal or Date.now(/Math.random("
fi
pass "no forbidden literals in $WF"

# Self-test: each forbidden literal must be caught when planted.
for literal in 'Done:' 'Date.now(' 'Math.random('; do
    printf '\n// planted: %s\n' "$literal" >"$TMP/planted-hygiene.js"
    cat "$WF" >>"$TMP/planted-hygiene.js"
    if ! grep -qE 'Done:|Date\.now\(|Math\.random\(' "$TMP/planted-hygiene.js"; then
        fail "hygiene self-test broken — planted '$literal' was not detected"
    fi
done
pass "hygiene self-test: all three planted literals are detected"

# --- 3. BEHAVIOR ---------------------------------------------------------------
say "3. Behavior: the real driver under injected fakes — OUTCOME shape, fail-open signals, mutual exclusion, legacy paths, optional gate"

cat >"$TMP/behavior.mjs" <<'NODE_TEST'
import assert from 'node:assert/strict';
import fs from 'node:fs';

const wfPath = process.argv[2];
let src = fs.readFileSync(wfPath, 'utf8');
src = src.replace(/^export /m, '');

// Wrap the workflow script's top-level body (which uses `export`, a top-level
// `return`, and top-level `await`, none of which are legal in a plain module)
// in an async function taking the Workflow runtime's ambient globals as
// parameters, so the REAL driver logic runs unmodified under injected fakes.
const wrapperPath = '/tmp/verify-workflow-review-outcome-wrapped.mjs';
fs.writeFileSync(
  wrapperPath,
  'export default async function(args, agent, pipeline, parallel, log) {\n' + src + '\n}\n'
);
const mod = await import('file://' + wrapperPath);
const run = mod.default;

// Reference pipeline/parallel — faithful to the documented runtime contract.
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

// The standalone code-review path shells out (worktree add; the optional gate's
// phase/task update), so it resolves the `rdmBin` environment arg — optional,
// defaulting to a plain `rdm` on PATH. Every run here injects an explicit fake
// binary so its assertions stay independent of the defaulting behavior § 6c
// owns. The two LEGACY survivors-only shapes below — run({ mode: 'plan' }) and
// run({ mode: 'code' }) with no item identifiers — deliberately do NOT get it:
// they emit zero rdm invocations and never call resolveRdmBin, and that
// carve-out is asserted in both directions (see the defaulted-rdmBin section).
const RDM_BIN_ARG = '/fake/bin/rdm';

const ALL_CODE_DIMS = ['ac', 'correctness', 'tests', 'architecture', 'api-docs', 'changelog', 'security'];
const CLEAN = Object.fromEntries(ALL_CODE_DIMS.map((k) => [k, []]));

function makeAgent(opts) {
  const o = opts || {};
  const calls = [];
  const agent = async (prompt, callOpts) => {
    const label = (callOpts && callOpts.label) || '';
    calls.push({ label, prompt, opts: callOpts });
    if (label === 'diff:signals') {
      if (o.diffThrows) throw new Error('diff agent failed');
      return o.diffResult;
    }
    if (label === 'gate:persist') {
      return o.gateAck || { ok: true };
    }
    const parts = label.split(':');
    if (parts[0] === 'find') {
      const dim = parts[2];
      // The `ac` dimension in code mode returns the AC_REVIEW_SCHEMA shape
      // ({ ac, findings }), not a bare findings array — o.acTable seeds it.
      if (dim === 'ac' && o.acTable) return { ac: o.acTable, findings: (o.findings || {})[dim] || [] };
      return { findings: (o.findings || {})[dim] || [] };
    }
    if (parts[0] === 'refute') {
      const id = parts.slice(2).join(':');
      return (o.verdicts || {})[id] || { refuted: false, confidence: 90 };
    }
    throw new Error('unexpected agent label: ' + label);
  };
  return { agent, calls };
}

// ============================================================================
// 3a. Full OUTCOME shape — a clean seed (reviewed) and a blocking seed (rework).
// ============================================================================
{
  const a = makeAgent({ diffResult: { changedFiles: ['src/api/index.ts'], diffText: '+export function foo() {}' }, findings: CLEAN, verdicts: {} });
  const out = await run({ mode: 'code', rdmBin: RDM_BIN_ARG, roadmap: 'rm', phase: '1', gate: false }, a.agent, refPipeline, refParallel, () => {});
  assert.deepEqual(
    Object.keys(out).sort(),
    ['findings', 'outcome', 'phase', 'reason', 'reviewBudget', 'roadmap', 'status', 'summary', 'writesCompletion'].sort(),
    'reviewed OUTCOME has exactly the dispatch-shaped keys'
  );
  assert.equal(out.outcome, 'reviewed');
  assert.equal(out.status, 'reviewed');
  assert.equal(out.writesCompletion, true);
  assert.equal(out.reason, '');
  assert.deepEqual(out.findings, []);
  // The refutation budget is projected onto the OUTCOME by the SAME shared
  // helper dispatch-phase uses, so a bounded standalone review is legible too.
  assert.equal(typeof out.reviewBudget, 'object', 'the standalone OUTCOME carries reviewBudget');
  assert.equal(out.reviewBudget.max, 5, 'the default refutation budget is reported');
  assert.equal(out.reviewBudget.everHit, false, 'a clean review did not hit the bound');
  assert.ok(!out.summary.includes('review budget hit'), 'an unbounded review carries NO summary clause');
}
{
  const findings = { ...CLEAN, ac: [{ id: 'ac1', concern: 'ac', severity: 'blocking', confidence: 90, what_fails: 'missing test' }] };
  const verdicts = { ac1: { refuted: false, confidence: 95 } };
  const a = makeAgent({ diffResult: { changedFiles: ['src/cli/x.ts'], diffText: '' }, findings, verdicts });
  const out = await run({ mode: 'code', rdmBin: RDM_BIN_ARG, task: 'my-task', gate: false }, a.agent, refPipeline, refParallel, () => {});
  assert.deepEqual(
    Object.keys(out).sort(),
    ['findings', 'outcome', 'reason', 'reviewBudget', 'status', 'summary', 'task', 'writesCompletion'].sort(),
    'rework OUTCOME (task shape) has exactly the dispatch-shaped keys'
  );
  assert.equal(out.task, 'my-task');
  assert.equal(out.outcome, 'rework');
  assert.equal(out.status, 'in-progress');
  assert.equal(out.writesCompletion, false);
  assert.equal(out.findings.length, 1);
}
{
  // OVER BUDGET on the standalone path: with `maxRefutations: 1` a unit that
  // produced several gating findings grades only one, and the OUTCOME says so —
  // both in `reviewBudget` and, visibly, in the summary (and therefore in the
  // persisted reason for a non-clean outcome).
  const many = {
    ...CLEAN,
    correctness: [
      { id: 'g1', concern: 'correctness', severity: 'blocking', confidence: 95, what_fails: 'a' },
      { id: 'g2', concern: 'correctness', severity: 'blocking', confidence: 90, what_fails: 'b' },
      { id: 'g3', concern: 'correctness', severity: 'blocking', confidence: 85, what_fails: 'c' },
    ],
  };
  const a = makeAgent({ diffResult: { changedFiles: ['src/api/index.ts'], diffText: '' }, findings: many, verdicts: {} });
  const out = await run(
    { mode: 'code', rdmBin: RDM_BIN_ARG, roadmap: 'rm', phase: '1', gate: false, maxRefutations: 1 },
    a.agent,
    refPipeline,
    refParallel,
    () => {}
  );
  assert.equal(out.reviewBudget.max, 1, 'the caller-supplied maxRefutations reaches the pipeline');
  assert.equal(out.reviewBudget.everHit, true, 'the bound was hit');
  assert.equal(out.reviewBudget.graded, 1, 'exactly one finding was graded');
  assert.equal(out.reviewBudget.passedThroughBudget, 2, 'two were passed through for budget');
  assert.ok(
    out.summary.includes('[review budget hit: 3 produced, 1 graded, 2 ungraded]'),
    'a budget-hit standalone review is VISIBLY distinguishable in its summary'
  );
  assert.equal(out.outcome, 'rework', 'an ungraded over-budget blocker still gates');
  const refuteCalls = a.calls.filter((c) => c.label && c.label.startsWith('refute:'));
  assert.equal(refuteCalls.length, 1, 'only one refuter was dispatched');
}
console.log('3a OK: reviewed / rework OUTCOME shapes and the refutation budget verified');

// ============================================================================
// 3b. Fail-open: diff agent throws -> `signals` key OMITTED -> every dimension
// runs. A populated diff -> deriveSignals output threaded through, so a
// non-triggered dimension is skipped.
// ============================================================================
{
  const a = makeAgent({ diffThrows: true, findings: CLEAN, verdicts: {} });
  await run({ mode: 'code', rdmBin: RDM_BIN_ARG, roadmap: 'rm', phase: '2', gate: false }, a.agent, refPipeline, refParallel, () => {});
  const findLabels = a.calls.filter((c) => c.label.startsWith('find:')).map((c) => c.label);
  assert.equal(findLabels.length, ALL_CODE_DIMS.length, 'diff agent throws -> fail-open -> every code dimension runs');
}
{
  // An empty (not thrown) changedFiles array is the OTHER fail-open trigger.
  const a = makeAgent({ diffResult: { changedFiles: [], diffText: '' }, findings: CLEAN, verdicts: {} });
  await run({ mode: 'code', rdmBin: RDM_BIN_ARG, roadmap: 'rm', phase: '2b', gate: false }, a.agent, refPipeline, refParallel, () => {});
  const findLabels = a.calls.filter((c) => c.label.startsWith('find:')).map((c) => c.label);
  assert.equal(findLabels.length, ALL_CODE_DIMS.length, 'empty changedFiles -> fail-open -> every code dimension runs');
}
{
  // A populated, non-triggering diff: docs only — no code file at all, so every
  // conditional signal is a confident false (content is never even consulted)
  // -> only the two always-on dimensions (ac, correctness) should fire.
  const a = makeAgent({ diffResult: { changedFiles: ['docs/readme.md'], diffText: '' }, findings: CLEAN, verdicts: {} });
  await run({ mode: 'code', rdmBin: RDM_BIN_ARG, roadmap: 'rm', phase: '3', gate: false }, a.agent, refPipeline, refParallel, () => {});
  const findLabels = a.calls.filter((c) => c.label.startsWith('find:')).map((c) => c.label.split(':')[2]);
  assert.deepEqual(findLabels.sort(), ['ac', 'correctness'].sort(), 'a real, non-triggering diff selects only the always-on dimensions (signals threaded through, not omitted)');
}
console.log('3b OK: fail-open omits signals entirely; a real diff threads deriveSignals output through');

// ============================================================================
// 3c. Mutual exclusion: both task and roadmap+phase supplied -> throws.
// ============================================================================
{
  const a = makeAgent({ diffResult: { changedFiles: [], diffText: '' }, findings: CLEAN, verdicts: {} });
  let threw = false;
  try {
    await run({ mode: 'code', rdmBin: RDM_BIN_ARG, task: 'x', roadmap: 'rm', phase: '1' }, a.agent, refPipeline, refParallel, () => {});
  } catch (e) {
    threw = true;
  }
  assert.ok(threw, 'both task and roadmap+phase supplied must throw');
}
console.log('3c OK: mutual-exclusion guard throws');

// ============================================================================
// 3d. Legacy backward-compatible shapes: `mode` + `survivors` unchanged, plus
// the ADDITIVE `budget` field so a caller of either legacy shape can still see
// the refutation bound the pipeline applied.
// ============================================================================
{
  const a = makeAgent({ findings: { coherence: [], 'architectural-fit': [], 'unit-of-work': [] }, verdicts: {} });
  const out = await run({ mode: 'plan' }, a.agent, refPipeline, refParallel, () => {});
  assert.deepEqual(
    Object.keys(out).sort(),
    ['budget', 'mode', 'survivors'].sort(),
    'mode=plan keeps the legacy {mode, survivors} shape plus the additive budget'
  );
  assert.equal(typeof out.budget, 'object', 'the legacy plan shape carries the budget accounting');
  assert.equal(out.budget.max, 5, 'the legacy plan shape reports the default refutation budget');
  assert.equal(out.budget.hit, false, 'a clean plan review did not hit the bound');
}
{
  const a = makeAgent({ findings: CLEAN, verdicts: {} });
  const out = await run({ mode: 'code' }, a.agent, refPipeline, refParallel, () => {});
  assert.deepEqual(
    Object.keys(out).sort(),
    ['budget', 'mode', 'survivors'].sort(),
    'mode=code with no identifiers keeps the legacy {mode, survivors} shape plus the additive budget'
  );
  assert.equal(out.budget.max, 5, 'the legacy code shape reports the default refutation budget');
  const diffCalls = a.calls.filter((c) => c.label === 'diff:signals');
  assert.equal(diffCalls.length, 0, 'the legacy no-identifier path never calls the diff-signals agent');
}
console.log('3d OK: both legacy shapes unaffected, no diff agent called on the legacy path');

// ============================================================================
// 3e. Optional gate: off by default; on, persists status via a mechanical
// agent and never runs `rdm commit`.
// ============================================================================
{
  const a = makeAgent({ diffResult: { changedFiles: ['x.rs'], diffText: '' }, findings: CLEAN, verdicts: {} });
  await run({ mode: 'code', rdmBin: RDM_BIN_ARG, roadmap: 'rm', phase: '1' }, a.agent, refPipeline, refParallel, () => {});
  const gateCalls = a.calls.filter((c) => c.label === 'gate:persist');
  assert.equal(gateCalls.length, 0, 'gate omitted -> no status-persist agent call');
}
{
  const a = makeAgent({ diffResult: { changedFiles: ['x.rs'], diffText: '' }, findings: CLEAN, verdicts: {} });
  const out = await run({ mode: 'code', rdmBin: RDM_BIN_ARG, roadmap: 'rm', phase: '1', gate: true }, a.agent, refPipeline, refParallel, () => {});
  const gateCalls = a.calls.filter((c) => c.label === 'gate:persist');
  assert.equal(gateCalls.length, 1, 'gate:true -> exactly one status-persist agent call');
  assert.ok(gateCalls[0].prompt.includes('rdm phase update 1 --status ' + out.status), 'gate prompt persists the mapped status');
  assert.ok(!/^\s+\S*rdm commit/m.test(gateCalls[0].prompt), 'gate prompt never invokes rdm commit as a command');
  assert.ok(gateCalls[0].prompt.includes('Do not run'), 'gate prompt explicitly instructs against running rdm commit');
}
{
  // gate:true on an escalated-shaped seed would carry --reason, but escalated
  // is structurally unreachable from this workflow (planFindings always []),
  // so this only exercises the rework path's reason (empty) for completeness.
  const findings = { ...CLEAN, ac: [{ id: 'ac1', concern: 'ac', severity: 'blocking', confidence: 90, what_fails: 'x' }] };
  const a = makeAgent({ diffResult: { changedFiles: ['x.rs'], diffText: '' }, findings, verdicts: { ac1: { refuted: false, confidence: 95 } } });
  const out = await run({ mode: 'code', rdmBin: RDM_BIN_ARG, roadmap: 'rm', phase: '1', gate: true }, a.agent, refPipeline, refParallel, () => {});
  assert.equal(out.outcome, 'rework');
  const gateCalls = a.calls.filter((c) => c.label === 'gate:persist');
  assert.ok(gateCalls[0].prompt.includes('--status in-progress'), 'rework gate persists in-progress');
  assert.ok(!gateCalls[0].prompt.includes('--reason'), 'rework gate carries no --reason (reason is empty for rework)');
}
console.log('3e OK: gate defaults off, gate:true persists mapped status without rdm commit or Done:');

// ============================================================================
// 3f. AC-only-gap summary: a FAIL AC-table entry with an otherwise-clean
// findings sweep forces `rework` with a summary naming the real cause, not
// the misleading "no surviving findings" (mirrors the identical fix in
// dispatch-phase.mjs's buildOutcome/buildTaskOutcome).
// ============================================================================
{
  const a = makeAgent({
    diffResult: { changedFiles: ['src/api/index.ts'], diffText: '+export function foo() {}' },
    findings: CLEAN,
    acTable: [{ criterion: 'x', status: 'FAIL', evidence: 'y' }],
    verdicts: {},
  });
  const out = await run({ mode: 'code', rdmBin: RDM_BIN_ARG, roadmap: 'rm', phase: '1', gate: false }, a.agent, refPipeline, refParallel, () => {});
  assert.equal(out.outcome, 'rework', 'an AC-FAIL alone (no blocking findings) forces rework');
  assert.equal(
    out.summary,
    'code rework unresolved: unmet acceptance criteria in AC table',
    'summary names the real cause, not "no surviving findings"'
  );
}
console.log('3f OK: AC-only-gap summary names the real cause');

// ============================================================================
// 3g. args.diff HOIST — a caller-supplied diff replaces the diff:signals agent
// entirely, threads the SAME deriveSignals output the agent path would have,
// and is strictly optional: absent / malformed / empty all fall back to the
// agent, which stays byte-unchanged.
// ============================================================================
const HOIST_DIFF = { changedFiles: ['src/api/index.ts'], diffText: '+export function foo() {}' };
{
  // Supplied and shape-valid -> ZERO diff:signals calls.
  const a = makeAgent({ diffResult: HOIST_DIFF, findings: CLEAN, verdicts: {} });
  const out = await run(
    { mode: 'code', rdmBin: RDM_BIN_ARG, roadmap: 'rm', phase: '1', gate: false, diff: HOIST_DIFF },
    a.agent,
    refPipeline,
    refParallel,
    () => {}
  );
  assert.equal(a.calls.filter((c) => c.label === 'diff:signals').length, 0, 'hoisted diff -> no diff:signals agent call');
  assert.equal(out.outcome, 'reviewed', 'hoisted diff still produces the normal OUTCOME');

  // Same run WITHOUT the hoist -> exactly one diff:signals call, and a
  // deep-equal OUTCOME. The hoist changes agent count, never behaviour.
  const b = makeAgent({ diffResult: HOIST_DIFF, findings: CLEAN, verdicts: {} });
  const outNoHoist = await run({ mode: 'code', rdmBin: RDM_BIN_ARG, roadmap: 'rm', phase: '1', gate: false }, b.agent, refPipeline, refParallel, () => {});
  assert.equal(b.calls.filter((c) => c.label === 'diff:signals').length, 1, 'no hoist -> exactly one diff:signals agent call');
  assert.deepEqual(out, outNoHoist, 'OUTCOME is deep-equal with and without the hoisted diff');

  // ... and the dimension selection is identical: `api-docs` is triggered by the
  // added exported symbol on both paths, `changelog` by neither.
  const dimsHoisted = a.calls.filter((c) => c.label.startsWith('find:')).map((c) => c.label).sort();
  const dimsFetched = b.calls.filter((c) => c.label.startsWith('find:')).map((c) => c.label).sort();
  assert.deepEqual(dimsHoisted, dimsFetched, 'hoisted diff threads the same deriveSignals output as the agent path');
  assert.ok(dimsHoisted.includes('find:code:api-docs'), 'the exported-symbol content triggers api-docs on the hoisted path too');
}
for (const [name, bad] of [
  ['absent', undefined],
  ['null', null],
  ['wrong type', 'main...HEAD'],
  ['missing changedFiles', { diffText: 'x' }],
  ['changedFiles not an array', { changedFiles: 'a.rs', diffText: 'x' }],
]) {
  const a = makeAgent({ diffResult: HOIST_DIFF, findings: CLEAN, verdicts: {} });
  const args = { mode: 'code', rdmBin: RDM_BIN_ARG, roadmap: 'rm', phase: '1', gate: false };
  if (name !== 'absent') args.diff = bad;
  const out = await run(args, a.agent, refPipeline, refParallel, () => {});
  assert.equal(
    a.calls.filter((c) => c.label === 'diff:signals').length,
    1,
    'malformed hoisted diff (' + name + ') falls back to exactly one diff:signals agent call'
  );
  assert.equal(out.outcome, 'reviewed', 'fallback path (' + name + ') still produces the normal OUTCOME');
}
{
  // A shape-valid but EMPTY changedFiles hoist is accepted (it IS the agent's
  // own "no commits of its own" answer) and lands on the untouched FAIL-OPEN
  // branch: no diff:signals call, and every dimension runs.
  const a = makeAgent({ diffResult: HOIST_DIFF, findings: CLEAN, verdicts: {} });
  await run(
    { mode: 'code', rdmBin: RDM_BIN_ARG, roadmap: 'rm', phase: '1', gate: false, diff: { changedFiles: [], diffText: '' } },
    a.agent,
    refPipeline,
    refParallel,
    () => {}
  );
  assert.equal(a.calls.filter((c) => c.label === 'diff:signals').length, 0, 'empty hoisted diff -> still no agent call');
  const dims = a.calls.filter((c) => c.label.startsWith('find:')).map((c) => c.label).sort();
  assert.deepEqual(dims, ALL_CODE_DIMS.map((d) => 'find:code:' + d).sort(), 'empty hoisted diff fails OPEN — every dimension runs');
}
console.log('3g OK: args.diff hoist eliminates the agent, threads identical signals, and falls back on anything malformed');

console.log('ALL BEHAVIOR CHECKS PASSED');
NODE_TEST

if run_node "$TMP/behavior.mjs" "$WF"; then
    pass "behavior: real driver verified under injected fakes"
else
    fail "behavior checks failed against $WF"
fi

# --- 3h. HOIST FALLBACK: planted-mutation self-test ---------------------------
# The 3g fallback assertions are only meaningful if deleting the `else` branch
# that reaches the diff:signals agent actually makes them fail. Plant exactly
# that mutation (collapse the hoist to an unconditional assignment, so the
# agent path is unreachable) and require the behavior run to FAIL.
say "3h. Hoist fallback self-test: removing the diff:signals fallback branch must break the behavior run"

awk '
    index($0, "const hoistedDiff = rawArgs.diff") { print; next }
    index($0, "if (hoistedDiff && typeof hoistedDiff") {
        print "if (true) {"
        print "  diff = hoistedDiff"
        mutating = 1
        next
    }
    mutating && index($0, "}") == 1 { mutating = 0; print "}"; next }
    mutating { next }
    { print }
' "$WF" >"$TMP/hoist-mutant.js"

if cmp -s "$WF" "$TMP/hoist-mutant.js"; then
    fail "3h: planted mutation was a no-op — the hoist fallback branch was not found in $WF"
fi
if run_node "$TMP/behavior.mjs" "$TMP/hoist-mutant.js" >/dev/null 2>&1; then
    fail "3h: behavior run PASSED against a driver whose diff:signals fallback branch was deleted — the fallback assertions are vacuous"
fi
pass "3h: fallback assertions fire when the diff:signals else-branch is removed"

# --- 3i. SKILL SHIM gathers the diff --------------------------------------
# `.claude/skills/rdm-review/SKILL.md` is a LOCAL dogfood shim (its distributed
# template `rdm-core/src/templates/skill-review-{cli,mcp}.md` is NOT a Workflow
# shim yet — tracked by task convert-remaining-skill-templates-to-workflow-shims),
# so the gathering check lives here rather than in the distribution harness.
say "3i. Skill shim gathers the branch diff and passes it as args.diff"

assert_skill_gathers_diff() {
    grep -qF 'git diff --name-only main...HEAD' "$1" || return 1
    grep -qF 'git diff main...HEAD' "$1" || return 1
    grep -qF 'changedFiles' "$1" || return 1
    grep -qF 'diffText' "$1" || return 1
    grep -qF '40000' "$1" || return 1
    # Occurrence floor: the arg key must appear in BOTH invocation shapes
    # (phase and task) plus the gathering instruction.
    [ "$(grep -cF 'diff:' "$1")" -ge 2 ] || return 1
    return 0
}
assert_skill_gathers_diff "$SKILL" ||
    fail "3i: $SKILL must gather 'git diff --name-only main...HEAD' / 'git diff main...HEAD' into { changedFiles, diffText } (40000-char truncation) and pass it as args.diff"
pass "3i: skill shim gathers the diff and passes it as args.diff"

sed 's/git diff --name-only main\.\.\.HEAD/git diff --name-onlyX main...HEAD/' "$SKILL" >"$TMP/skill-diff-typo.md"
if assert_skill_gathers_diff "$TMP/skill-diff-typo.md"; then
    fail "3i: gathering detector missed a typo'd git-diff command in the shim"
fi
pass "3i: gathering detector fires on a planted typo in the shim's git-diff command"

# --- 4. SKILL SHIM -------------------------------------------------------------
say "4. Skill shim: references the workflow, retains interactive Report/Act/Gate + Done: trailer mechanism"

grep -qF 'review-refute-fix' "$SKILL" ||
    fail "$SKILL must reference the review-refute-fix workflow"
grep -qE '^### [0-9]+\. Report' "$SKILL" ||
    fail "$SKILL must retain an interactive '### N. Report' section"
grep -qE '^### [0-9]+\. Act' "$SKILL" ||
    fail "$SKILL must retain an interactive '### N. Act' section"
grep -qE '^### [0-9]+\. Gate' "$SKILL" ||
    fail "$SKILL must retain an interactive '### N. Gate' section"
grep -qF 'rdm hook done-line' "$SKILL" ||
    fail "$SKILL must retain the 'rdm hook done-line' completion-trailer mechanism"
grep -qF 'git commit --amend' "$SKILL" ||
    fail "$SKILL must retain the git commit --amend gate mechanism"
grep -qE 'gate: ?false' "$SKILL" ||
    fail "$SKILL must invoke the workflow with gate: false — it must own its own gate, not delegate to the workflow's mechanical one"

# ENVIRONMENT ARGS (§ 6): `rdmBin` now DEFAULTS to a plain `rdm` on PATH, so a
# shim that omits it degrades silently to whatever global rdm is first on PATH —
# inside this repo, the stale build the development-build rule forbids. That is
# why this check survives the contract reversal unchanged: it guards a silent
# wrong-binary failure rather than a loud one. Occurrence floor of 2 because the
# phase and task invocation shapes are separate arg lines and BOTH must carry it.
assert_skill_passes_rdmbin() {
    grep -qF 'rdmBin' "$1" || return 1
    [ "$(grep -cF 'rdmBin: "./target/debug/rdm"' "$1")" -ge 2 ] || return 1
    grep -qF 'project: "rdm"' "$1" || return 1
    return 0
}
assert_skill_passes_rdmbin "$SKILL" ||
    fail "$SKILL must pass rdmBin (in BOTH the phase and task arg shapes) and project into the review-refute-fix invocation"
pass "skill shim references the workflow and retains its interactive gate mechanism, and passes rdmBin/project"

sed 's/rdmBin/rdmBn/g' "$SKILL" >"$TMP/skill-rdmbin-typo.md"
if assert_skill_passes_rdmbin "$TMP/skill-rdmbin-typo.md"; then
    fail "4: detector missed a typo'd rdmBin arg key in the shim"
fi
pass "4: rdmBin detector fires on a planted typo in the shim"

say "4b. gen-skill-review.sh --check still passes in both modes (untouched by this hand-authored trim)"
if sh "$SKILL_GEN" --check --mode code && sh "$SKILL_GEN" --check --mode plan; then
    pass "gen-skill-review.sh --check clean in both --mode code and --mode plan"
else
    fail "gen-skill-review.sh --check failed — the shipped templates must be unaffected by this phase"
fi

# --- 5. STATIC INVARIANT: meta.phases -----------------------------------------
say "5. meta.phases lists exactly the distinct phase: tags the file emits"

# Distinct `phase: '<Name>',` literals actually emitted by agent() calls. Real
# agent-option phase names are TitleCase with a trailing comma — this excludes
# the review block's lowercase STATUS_MAPPING rows (`phase: 'reviewed'`,
# `phase: 'blocked'`), which are rdm statuses, not workflow phases. Mirrors
# scripts/verify-workflow-dispatch.sh's emitted_phases() convention.
PHASE_TAGS_USED=$(grep -oE "phase: '[A-Z][A-Za-z]*'," "$WF" | sed "s/phase: '//;s/',//" | sort -u | tr '\n' ',' | sed 's/,$//')

# Distinct `{ title: '<name>' }` entries declared in meta.phases, scoped to that
# array literal so DIMENSIONS' `title:` entries in the stamped block are excluded.
PHASE_TAGS_DECLARED=$(awk '/phases: \[/{p=1} p{print} p&&/\]/{exit}' "$WF" | grep -oE "title: '[^']+'" | sed "s/title: '//;s/'\$//" | sort -u | tr '\n' ',' | sed 's/,$//')

[ -n "$PHASE_TAGS_DECLARED" ] || fail "meta.phases not found or empty in $WF"
[ "$PHASE_TAGS_DECLARED" = "$PHASE_TAGS_USED" ] ||
    fail "meta.phases ($PHASE_TAGS_DECLARED) does not match the distinct phase: tags actually emitted ($PHASE_TAGS_USED)"
pass "meta.phases matches the distinct phase: tags emitted: $PHASE_TAGS_DECLARED"

# --- 6. PARAMETERIZATION ------------------------------------------------------
# review-refute-fix names NO particular rdm executable and NO particular rdm
# project: both arrive as RUNTIME args (`rdmBin`, `project`) and are threaded
# into every prompt that shells out. This is dispatch-phase's landed contract,
# reused — scripts/verify-workflow-dispatch.sh § 9 gates the same rules there.
# Four sub-gates:
#
#   6a — per-file literal zeroing across BOTH copies (dogfood workflow + shipped
#        template), asserted PER FILE so a half-applied edit cannot pass. Also
#        pins lib/review.mjs's counts, which this phase must NOT change.
#   6b — a DRIVEN prompt capture over the standalone phase and task paths with
#        `gate: true` and NO diff hoist, tokenizing every emitted
#        `rdm <subcommand>` and checking it against the project-agnostic
#        allow-list expressed AS DATA.
#   6c — the fail-closed `rdmBin` rule AND its documented carve-out: the
#        standalone path throws without it, while BOTH legacy survivors-only
#        shapes still succeed without it and keep their exact legacy result.
#   6d — planted-mutation self-tests for 6b.
say "6. Parameterization: no hardcoded rdm binary or project; the environment axes are runtime args"

# --- 6a. Per-file literal zeroing ---------------------------------------------
say "6a. Per-file literal zeroing (dogfood workflow + shipped template)"

# assert_no_env_literals <file> — zero occurrences of THIS repo's dev binary path
# and zero of THIS repo's project flag. Deliberately per-file: a concatenated
# stream would let a zero in one copy mask a hit in the other, which is exactly
# the half-applied-edit failure mode (the two copies are kept identical by hand).
assert_no_env_literals() {
    _f=$1
    _bin=$(grep -c 'target/debug/rdm' "$_f" || true)
    _proj=$(grep -c -- '--project rdm' "$_f" || true)
    [ "$_bin" -eq 0 ] && [ "$_proj" -eq 0 ]
}

for f in "$WF" "$WF_TEMPLATE"; do
    if assert_no_env_literals "$f"; then
        pass "6a: ${f#"$REPO_ROOT"/} carries neither 'target/debug/rdm' nor '--project rdm'"
    else
        grep -n 'target/debug/rdm' "$f" >&2 || true
        grep -n -- '--project rdm' "$f" >&2 || true
        fail "6a: $f still hardcodes this repo's rdm binary and/or project — both must be runtime args"
    fi
done

_i=0
for f in "$WF" "$WF_TEMPLATE"; do
    _i=$((_i + 1))
    cp "$f" "$TMP/env-mutant-$_i"
    printf '\n// planted: ./target/debug/rdm phase show --project rdm\n' >>"$TMP/env-mutant-$_i"
    if assert_no_env_literals "$TMP/env-mutant-$_i"; then
        fail "6a: the per-file literal check did not fire on a planted literal in $f — the gate is vacuous"
    fi
done
pass "6a: the per-file check fires independently on both planted mutants"

# NEGATIVE: lib/review.mjs is the canonical review source, stamped into THREE
# consumers. It contains ZERO binary INVOCATION sites; its single
# 'target/debug/rdm' occurrence lives in a `//!` module-doc comment describing
# the `{rdm_bin}` placeholder the skill renderer substitutes, and is not stamped
# into any consumer. Pin BOTH counts exactly: an added literal moves the count
# and fails, a removed one likewise.
REVIEW_LIB="$REPO_ROOT/.claude/workflows/lib/review.mjs"
[ -f "$REVIEW_LIB" ] || fail "6a: canonical review source not found: $REVIEW_LIB"
REVIEW_LIB_BIN=$(grep -c 'target/debug/rdm' "$REVIEW_LIB" || true)
REVIEW_LIB_PROJ=$(grep -c -- '--project rdm' "$REVIEW_LIB" || true)
[ "$REVIEW_LIB_BIN" -eq 1 ] ||
    fail "6a: lib/review.mjs must carry EXACTLY ONE 'target/debug/rdm' (the //! module-doc {rdm_bin} reference) — found $REVIEW_LIB_BIN"
[ "$REVIEW_LIB_PROJ" -eq 0 ] ||
    fail "6a: lib/review.mjs must carry ZERO '--project rdm' — found $REVIEW_LIB_PROJ"
grep -n 'target/debug/rdm' "$REVIEW_LIB" | grep -q '^[0-9]*://!' ||
    fail "6a: lib/review.mjs's single 'target/debug/rdm' occurrence must be inside a //! module-doc comment, not a live invocation"
pass "6a: lib/review.mjs is unchanged (1 doc-comment reference, 0 project literals) — the stamped source was not touched"

# --- 6b. Driven prompt capture ------------------------------------------------
say "6b. Driven prompt capture: every emitted rdm invocation honors the allow-list"

cat >"$TMP/paramz.mjs" <<'NODE_PARAMZ'
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const wfPath = process.argv[2];
const src = fs.readFileSync(wfPath, 'utf8').replace(/^export /m, '');
const wrapperPath = path.join(os.tmpdir(), 'verify-workflow-review-outcome-paramz-wrapped.mjs');
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

// The PROJECT-AGNOSTIC ALLOW-LIST, expressed as DATA — the SAME array
// verify-workflow-dispatch.sh § 9b uses (dispatch-phase's landed contract, not
// re-derived here).
const PROJECT_AGNOSTIC = ['model resolve', 'commit', 'status', 'discard'];

const ALL_CODE_DIMS = ['ac', 'correctness', 'tests', 'architecture', 'api-docs', 'changelog', 'security'];
const CLEAN = Object.fromEntries(ALL_CODE_DIMS.map((k) => [k, []]));

// A capturing fake agent. NOTE: the capture runs deliberately supply NO `diff`
// hoist — a hoist short-circuits the `diff:signals` agent, which is where the
// `worktree add` invocation lives, silently narrowing the scan. They also run
// with `gate: true`, because the `phase update` / `task update` invocation
// lives ONLY in the optional mechanical gate step (the rdm-review skill itself
// correctly passes `gate: false` and owns its own gate).
function makeCapture() {
  const prompts = [];
  const agent = async (prompt, opts) => {
    prompts.push(String(prompt));
    const label = (opts && opts.label) || '';
    if (label === 'diff:signals') return { changedFiles: ['rdm-core/src/lib.rs'], diffText: '' };
    if (label === 'gate:persist') return { ok: true };
    const parts = label.split(':');
    if (parts[0] === 'find') return { findings: [] };
    if (parts[0] === 'refute') return { refuted: false, confidence: 90 };
    throw new Error('unexpected agent label: ' + label);
  };
  return { agent, prompts };
}

const INVOCATION = /(^|[\s`])((?:[^\s`]*\/)?rdm)\s+([a-z][a-z-]*(?:\s+[a-z][a-z-]*)?)/g;

// Only COMMAND-BEARING lines are tokenized. Every command this driver emits is
// an INDENTED command line ('  <bin> worktree add …', '  ' + statusCmd).
// Flush-left prose is not an invocation — and one such line is a NEGATIVE
// directive ("Do not run `rdm commit`"), which names a bare `rdm` deliberately
// and must not be read as a hardcoded binary. The non-vacuity floors below
// prove the filter is not silently dropping real commands.
function isCommandLine(line) {
  return /^\s{2,}\S/.test(line);
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
  await run(args, c.agent, refPipeline, refParallel, () => {});
  return scan(c.prompts);
}

const BASE_PHASE = { mode: 'code', roadmap: 'rm', phase: '1', gate: true };
const BASE_TASK = { mode: 'code', task: 'my-task', gate: true };

for (const [mode, base] of [['phase', BASE_PHASE], ['task', BASE_TASK]]) {
  // --- Run A: a project IS configured.
  const withProject = await capture({ ...base, rdmBin: FAKE_BIN, project: 'demo' });
  assert.ok(withProject.length > 0, mode + ': the scan found no rdm invocations at all — it cannot pass vacuously');

  const seen = new Set();
  for (const occ of withProject) {
    assert.equal(occ.bin, FAKE_BIN, mode + ': an rdm invocation used ' + occ.bin + ' instead of the injected rdmBin: ' + occ.line);
    seen.add(occ.two);
    const agnostic = PROJECT_AGNOSTIC.includes(occ.two);
    if (agnostic) {
      assert.ok(!occ.line.includes('--project'), mode + ': project-agnostic `rdm ' + occ.two + '` must carry NO project flag: ' + occ.line);
    } else {
      assert.ok(occ.line.includes(' --project demo'), mode + ': project-scoped `rdm ' + occ.two + '` must carry " --project demo": ' + occ.line);
    }
  }

  // Non-vacuity floors: the scan must actually have reached both command shapes.
  const need = mode === 'task' ? ['worktree add', 'task update'] : ['worktree add', 'phase update'];
  for (const n of need) {
    assert.ok(seen.has(n), mode + ': expected at least one `rdm ' + n + '` occurrence, saw: ' + [...seen].join(', '));
  }

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

console.log('all review-refute-fix parameterization prompt-capture assertions passed');
NODE_PARAMZ

if run_node "$TMP/paramz.mjs" "$WF"; then
    pass "6b: every emitted rdm invocation uses the injected binary and honors the project-agnostic allow-list (phase + task, with and without a project)"
else
    fail "6b: parameterization prompt-capture assertions failed"
fi

# --- 6c. Defaulted rdmBin + the legacy-path carve-out -------------------------
say "6c. Defaulted rdmBin on the standalone path (wrong-TYPE still throws); both legacy shapes still succeed without it"

cat >"$TMP/rdmbin.mjs" <<'NODE_RDMBIN'
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const wfPath = process.argv[2];
const src = fs.readFileSync(wfPath, 'utf8').replace(/^export /m, '');
const wrapperPath = path.join(os.tmpdir(), 'verify-workflow-review-outcome-rdmbin-wrapped.mjs');
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

let agentCalls = 0;
const spy = async (prompt, opts) => {
  agentCalls++;
  const label = (opts && opts.label) || '';
  const parts = label.split(':');
  if (parts[0] === 'find') return { findings: [] };
  if (parts[0] === 'refute') return { refuted: false, confidence: 90 };
  if (label === 'diff:signals') return { changedFiles: [], diffText: '' };
  return { ok: true };
};

// (1a) The STANDALONE code-review path shells out, but an ABSENT rdmBin now
// DEFAULTS to a plain `rdm` on PATH and the run PROCEEDS. A plugin-installed
// consumer has no repo-local build path to pass; this repo's own stale-global
// hazard is handled by RDM_BIN in .mise.toml (gated by
// verify-workflow-dispatch.sh § 9c-dogfood), not by refusing to resolve.
for (const absent of [undefined, null, '', '   ', '\t']) {
  agentCalls = 0;
  const args = { mode: 'code', roadmap: 'rm', phase: '1' };
  if (absent !== undefined) args.rdmBin = absent;
  const out = await run(args, spy, refPipeline, refParallel, () => {});
  assert.equal(
    out.outcome,
    'reviewed',
    'an absent rdmBin (' + JSON.stringify(absent) + ') must default and let the standalone path reach an outcome'
  );
  assert.ok(agentCalls > 0, 'the run must actually have dispatched agents — otherwise this assertion is vacuous');
}
agentCalls = 0;
{
  const out = await run({ mode: 'code', task: 'my-task' }, spy, refPipeline, refParallel, () => {});
  assert.equal(out.outcome, 'reviewed', 'the standalone TASK path defaults rdmBin too');
  assert.equal(out.task, 'my-task', 'the task-mode OUTCOME is keyed by task');
}

// (1b) A present-but-wrong-TYPE value STILL throws, before any agent() call —
// degrading a `rdmBin: 42` typo to PATH would reintroduce the silent-wrong-
// binary hazard the absent-value default does not need.
for (const bad of [42, {}, [], true]) {
  agentCalls = 0;
  await assert.rejects(
    () => run({ mode: 'code', roadmap: 'rm', phase: '1', rdmBin: bad }, spy, refPipeline, refParallel, () => {}),
    /rdmBin/,
    'a non-string rdmBin (' + JSON.stringify(bad) + ') must still throw on the standalone path'
  );
  assert.equal(agentCalls, 0, 'the rdmBin type throw must precede every agent() call');
}

// The type error must stay actionable.
try {
  await run({ mode: 'code', roadmap: 'rm', phase: '1', rdmBin: 42 }, spy, refPipeline, refParallel, () => {});
  assert.fail('expected a throw');
} catch (e) {
  assert.match(e.message, /rdmBin must be a string/, 'the message names the type requirement');
  assert.match(e.message, /"rdm"/, 'the message names the explicit PATH sentinel');
  assert.match(e.message, /PATH/, 'the message names what omitting the arg entirely does');
}

// (2) THE CARVE-OUT, UNCHANGED and deliberately preserved through the contract
// reversal: BOTH legacy survivors-only shapes emit ZERO rdm invocations and
// never call resolveRdmBin at all. They must still succeed with NO rdmBin, and
// still return the exact legacy { mode, survivors, budget } shape — with no
// rdmBin key smuggled in by the new defaulting branch.
for (const legacy of [{ mode: 'plan' }, { mode: 'code' }]) {
  const out = await run(legacy, spy, refPipeline, refParallel, () => {});
  assert.deepEqual(
    Object.keys(out).sort(),
    ['budget', 'mode', 'survivors'].sort(),
    JSON.stringify(legacy) + ' must still return the legacy { mode, survivors, budget } shape with no rdmBin'
  );
  assert.equal(out.mode, legacy.mode, 'the legacy shape echoes its mode');
}

// (3) A hostile project name is rejected rather than escaped (it is
// interpolated into a Bash-agent prompt).
for (const hostile of ['a b', 'a;rm -rf /', '$(x)', '`x`', 'a|b']) {
  await assert.rejects(
    () => run({ mode: 'code', roadmap: 'rm', phase: '1', rdmBin: 'rdm', project: hostile }, spy, refPipeline, refParallel, () => {}),
    /project must be a plain project name/,
    'hostile project ' + JSON.stringify(hostile) + ' must be rejected'
  );
}
// The 'rdm' sentinel is accepted verbatim — an explicit PATH opt-in.
{
  const out = await run({ mode: 'code', roadmap: 'rm', phase: '1', rdmBin: 'rdm' }, spy, refPipeline, refParallel, () => {});
  assert.equal(out.outcome, 'reviewed', "the 'rdm' PATH sentinel is accepted verbatim and the run proceeds");
}

console.log('all defaulted rdmBin + legacy-carve-out assertions passed');
NODE_RDMBIN

if run_node "$TMP/rdmbin.mjs" "$WF"; then
    pass "6c: an absent rdmBin defaults to a bare 'rdm' on the standalone path (phase and task), a wrong-TYPE value still throws before any agent call, and both legacy shapes still succeed without it"
else
    fail "6c: defaulted rdmBin / legacy-carve-out assertions failed"
fi

# The guard must NOT be an existence preflight. `which -a rdm` resolves to the
# stale global build in this repo, so an existence check passes while running
# exactly the binary the development-build rule forbids. Comment lines are
# stripped first so rationale prose is not itself flagged.
assert_no_existence_preflight() {
    grep -vE '^[[:space:]]*(//|\*|/\*)' "$1" |
        grep -nE 'which +(-a +)?rdm|command -v|existsSync|accessSync|statSync' >"$TMP/preflight-hits" 2>/dev/null || true
    [ ! -s "$TMP/preflight-hits" ]
}
for f in "$WF" "$WF_TEMPLATE"; do
    if ! assert_no_existence_preflight "$f"; then
        cat "$TMP/preflight-hits" >&2
        fail "6c: $f must not implement the rdmBin guard as an existence preflight — the guard is on the ABSENCE of the argument"
    fi
done
cp "$WF" "$TMP/preflight-mutant.js"
printf "\nconst ok = existsSync(rdmBin)\n" >>"$TMP/preflight-mutant.js"
if assert_no_existence_preflight "$TMP/preflight-mutant.js"; then
    fail "6c: the existence-preflight detector missed a planted existsSync call — the gate is vacuous"
fi
pass "6c: no existence preflight in either copy; the detector fires on planted code"

# --- 6d. Planted-mutation self-tests for 6b -----------------------------------
say "6d. Planted-mutation self-tests: the allow-list assertion is not vacuous"

# (i) a builder RE-HARDCODES this repo's dev binary path.
sed "s|+ resolveRdmBin(cfg \&\& cfg.rdmBin) + ' worktree add '|+ './target/debug/rdm' + ' worktree add '|" "$WF" >"$TMP/pz-mut-bin.js"
if cmp -s "$WF" "$TMP/pz-mut-bin.js"; then
    fail "6d(i): the re-hardcoded-binary mutation did not apply — the self-test is not exercising anything"
fi
if run_node "$TMP/paramz.mjs" "$TMP/pz-mut-bin.js" >/dev/null 2>&1; then
    fail "6d(i): a re-hardcoded rdm binary was NOT detected — the binary assertion is vacuous"
fi
pass "6d(i): detector fires when a builder re-hardcodes the rdm binary"

# (ii) projectFlag returns the flag UNCONDITIONALLY -> the no-project run gains
#      stray --project occurrences, which run B must reject.
sed "s|return cfg \&\& cfg.project ? ' --project ' + cfg.project : '';|return ' --project ' + ((cfg \&\& cfg.project) \|\| 'demo');|" "$WF" >"$TMP/pz-mut-uncond.js"
if cmp -s "$WF" "$TMP/pz-mut-uncond.js"; then
    fail "6d(ii): the projectFlag mutation did not apply"
fi
if run_node "$TMP/paramz.mjs" "$TMP/pz-mut-uncond.js" >/dev/null 2>&1; then
    fail "6d(ii): an unconditional projectFlag was NOT detected — the allow-list assertion is vacuous"
fi
pass "6d(ii): detector fires when projectFlag stops honoring the configured project"

# (iii) a project-scoped builder DROPS its flag.
sed "s|' worktree add ' + ref + projectFlag(cfg)|' worktree add ' + ref|" "$WF" >"$TMP/pz-mut-drop.js"
if cmp -s "$WF" "$TMP/pz-mut-drop.js"; then
    fail "6d(iii): the dropped-flag mutation did not apply"
fi
if run_node "$TMP/paramz.mjs" "$TMP/pz-mut-drop.js" >/dev/null 2>&1; then
    fail "6d(iii): a project-scoped command that dropped its project flag was NOT detected"
fi
pass "6d(iii): detector fires when a project-scoped builder drops its project flag"

say "verify-workflow-review-outcome.sh: ALL GREEN"
