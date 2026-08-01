#!/bin/sh
# Hermetic regression for the standalone review-refute-fix workflow's full
# dispatch-shaped OUTCOME path (headless-skill-workflows roadmap).
#
# `.claude/workflows/review-refute-fix.js` grew from a survivors-only wrapper
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
#   0. BYTE-IDENTICAL COPY — `.claude/workflows/review-refute-fix.js` and
#      `rdm-core/src/templates/workflows/review-refute-fix.js` are identical
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
#      `review-refute-fix` workflow AND still carries its interactive
#      Report/Act/Gate sections and the `Done:`-trailer gate mechanism (it must
#      NOT have become a headless pass-through), and
#      `scripts/gen-skill-review.sh --check` still passes in both `--mode code`
#      and `--mode plan` (proving the untouched, separately-generated
#      `skill-review-{cli,mcp}.md` / `skill-plan-review-{cli,mcp}.md` templates
#      are unaffected by this hand-authored prose trim).
#   5. STATIC INVARIANT — `meta.phases` in review-refute-fix.js lists exactly
#      the distinct `phase:` tags the driver + the inlined review block
#      actually emit (`Find`, `Refute`, `Review`, `Gate`).
#
# Node is used only as a host to drive the pure/injectable parts of the real
# workflow script; it is stdlib-only (node:assert), no package.json /
# node_modules / third-party packages. node is pinned in .mise.toml.
#
# Requires: node (via PATH or `mise exec node --`).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)

WF="$REPO_ROOT/.claude/workflows/review-refute-fix.js"
WF_TEMPLATE="$REPO_ROOT/rdm-core/src/templates/workflows/review-refute-fix.js"
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
    pass "review-refute-fix.js and its shipped template copy are byte-identical"
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
    $0 ~ m && !done { print "const _plantedRunReview = buildReviewPipeline(\x27code\x27)"; done = 1 }
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
  const out = await run({ mode: 'code', roadmap: 'rm', phase: '1', gate: false }, a.agent, refPipeline, refParallel, () => {});
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
  const out = await run({ mode: 'code', task: 'my-task', gate: false }, a.agent, refPipeline, refParallel, () => {});
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
    { mode: 'code', roadmap: 'rm', phase: '1', gate: false, maxRefutations: 1 },
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
  await run({ mode: 'code', roadmap: 'rm', phase: '2', gate: false }, a.agent, refPipeline, refParallel, () => {});
  const findLabels = a.calls.filter((c) => c.label.startsWith('find:')).map((c) => c.label);
  assert.equal(findLabels.length, ALL_CODE_DIMS.length, 'diff agent throws -> fail-open -> every code dimension runs');
}
{
  // An empty (not thrown) changedFiles array is the OTHER fail-open trigger.
  const a = makeAgent({ diffResult: { changedFiles: [], diffText: '' }, findings: CLEAN, verdicts: {} });
  await run({ mode: 'code', roadmap: 'rm', phase: '2b', gate: false }, a.agent, refPipeline, refParallel, () => {});
  const findLabels = a.calls.filter((c) => c.label.startsWith('find:')).map((c) => c.label);
  assert.equal(findLabels.length, ALL_CODE_DIMS.length, 'empty changedFiles -> fail-open -> every code dimension runs');
}
{
  // A populated, non-triggering diff: docs only — no code file at all, so every
  // conditional signal is a confident false (content is never even consulted)
  // -> only the two always-on dimensions (ac, correctness) should fire.
  const a = makeAgent({ diffResult: { changedFiles: ['docs/readme.md'], diffText: '' }, findings: CLEAN, verdicts: {} });
  await run({ mode: 'code', roadmap: 'rm', phase: '3', gate: false }, a.agent, refPipeline, refParallel, () => {});
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
    await run({ mode: 'code', task: 'x', roadmap: 'rm', phase: '1' }, a.agent, refPipeline, refParallel, () => {});
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
  await run({ mode: 'code', roadmap: 'rm', phase: '1' }, a.agent, refPipeline, refParallel, () => {});
  const gateCalls = a.calls.filter((c) => c.label === 'gate:persist');
  assert.equal(gateCalls.length, 0, 'gate omitted -> no status-persist agent call');
}
{
  const a = makeAgent({ diffResult: { changedFiles: ['x.rs'], diffText: '' }, findings: CLEAN, verdicts: {} });
  const out = await run({ mode: 'code', roadmap: 'rm', phase: '1', gate: true }, a.agent, refPipeline, refParallel, () => {});
  const gateCalls = a.calls.filter((c) => c.label === 'gate:persist');
  assert.equal(gateCalls.length, 1, 'gate:true -> exactly one status-persist agent call');
  assert.ok(gateCalls[0].prompt.includes('rdm phase update 1 --status ' + out.status), 'gate prompt persists the mapped status');
  assert.ok(!gateCalls[0].prompt.includes('./target/debug/rdm commit'), 'gate prompt never invokes rdm commit');
  assert.ok(gateCalls[0].prompt.includes('Do not run'), 'gate prompt explicitly instructs against running rdm commit');
}
{
  // gate:true on an escalated-shaped seed would carry --reason, but escalated
  // is structurally unreachable from this workflow (planFindings always []),
  // so this only exercises the rework path's reason (empty) for completeness.
  const findings = { ...CLEAN, ac: [{ id: 'ac1', concern: 'ac', severity: 'blocking', confidence: 90, what_fails: 'x' }] };
  const a = makeAgent({ diffResult: { changedFiles: ['x.rs'], diffText: '' }, findings, verdicts: { ac1: { refuted: false, confidence: 95 } } });
  const out = await run({ mode: 'code', roadmap: 'rm', phase: '1', gate: true }, a.agent, refPipeline, refParallel, () => {});
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
  const out = await run({ mode: 'code', roadmap: 'rm', phase: '1', gate: false }, a.agent, refPipeline, refParallel, () => {});
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
    { mode: 'code', roadmap: 'rm', phase: '1', gate: false, diff: HOIST_DIFF },
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
  const outNoHoist = await run({ mode: 'code', roadmap: 'rm', phase: '1', gate: false }, b.agent, refPipeline, refParallel, () => {});
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
  const args = { mode: 'code', roadmap: 'rm', phase: '1', gate: false };
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
    { mode: 'code', roadmap: 'rm', phase: '1', gate: false, diff: { changedFiles: [], diffText: '' } },
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
pass "skill shim references the workflow and retains its interactive gate mechanism"

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

say "verify-workflow-review-outcome.sh: ALL GREEN"
