#!/usr/bin/env bash
# verify-refuter-agreement.sh — hermetic gate for the refuter-agreement harness.
#
# WHAT THIS GATES
#   1  Hygiene: determinism, no hot-path coupling, --help surface, docs present.
#   2  Corpus validation: schema, floors, class/authority/provenance shares.
#  2c  Batch-group POWER under the UNIT-SCOPED key (runId|unitIdent|mode|dim):
#      two review units never merge into one group, the committed histogram and
#      all three exclusion counts, the size-1 exclusion, and the zero-spend
#      --batch-power verdict. Plus 2c-equivalence (the deliberately duplicated
#      unit-identity predicate cannot drift from phase 1's canonical rule) and
#      2c-guard (an underpowered batched arm THROWS, and --allow-underpowered
#      forces a NO MEASUREMENT banner with no decision line).
#   3  Prompt fidelity: every item regenerates through the REAL refutePrompt,
#      and every MINED prompt exceeds 401 chars (proving the transcript, not the
#      sidecar's truncated promptPreview, was the source).
#   4  Miner behavior against the hermetic mine-sidecars fixture, including all
#      SIX degradation paths (no-transcript, no-prompt, unparseable-finding,
#      unrecoverable-mode, unrecoverable-dim, no-verdict), a no-silent-drop
#      accounting identity, and the project-slug filter.
#  4b  The miner's remaining CLI surface: --severity (single and comma-set),
#      --until in both directions, --limit, --out, --help, and every
#      argument-validation error path (each an actionable named message, never
#      a stack trace).
#   5  Scorer behavior against trials-sample.json: FN/FP separation, distinct
#      denominators, ungraded bucketing, flip rate, per-class and
#      authoritative-only splits, token + tool-call columns on the FN row.
#  5c  Batched scoring: the batched prompt as a minimal delta from the real
#      refutePrompt, expansion (unknown ids dropped, omissions and crashes left
#      ungraded, dispatch cost on the first row), independent arm buckets,
#      dispatches counted by unique dispatch id, and anchoring over qualifying
#      groups only.
#   6  The no-blended-accuracy negative assertion (recursive, JSON and text).
#   7  --dry-run dispatches NOTHING; --dispatch-stub drives the full path.
#  7b  The REAL paid-dispatch parsing path, driven with zero spend:
#      parseClaudeResult / tryParseEmbeddedJson / countSessionToolUses /
#      projectSlugFor as pure functions, plus claudeDispatch against
#      PATH-shadowed fake `claude` binaries. These branches produce the verdicts
#      and token/tool-call figures the DECISION was computed from, so a
#      regression in them would silently corrupt the numbers.
#  7c  parseClaudeBatchResult, the batched sibling of that parser: unknown ids
#      dropped, non-boolean verdicts left ungraded, a missing `verdicts` array
#      read as a CRASH rather than as a clean grade.
#   8  --audit docs/token-baseline.json arithmetic.
#  8b  --audit-section refuterBatching: the corpus-power arithmetic, including
#      the closed decision vocabulary and the rule that an underpowered arm can
#      only ever carry decision "no-measurement".
#   9  Planted-mutation self-tests proving 2, 2c, 3, 4, 4b, 5, 5c, 6, and 7b are
#      not vacuous.
#  10  (removed — asserting on CHANGELOG.md content is forbidden; see CLAUDE.md)
#  11  The AC9 XOR: either a changed model binding is reflected in a new
#      verify-workflow-review.sh criterion, or the unchanged binding carries the
#      pointer comment to the decision doc. Exactly one must hold.
#
# THIS SCRIPT NEVER DISPATCHES A PAID AGENT. Section 7 proves it with a stub
# that fails the run if it is ever called under --dry-run.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

FAILURES=0
say() { printf '\n=== %s\n' "$1"; }
pass() { printf '  ok   %s\n' "$1"; }
fail() {
    printf '  FAIL %s\n' "$1" >&2
    FAILURES=$((FAILURES + 1))
    return 0
}
die() {
    printf '  FAIL %s\n' "$1" >&2
    exit 1
}

MODULE="scripts/lib/refuter-agreement.mjs"
MINER="scripts/mine-refuter-corpus.mjs"
RUNNER="scripts/run-refuter-agreement.mjs"
CORPUS="tests/fixtures/refuter-agreement/corpus.jsonl"
TRIALS="tests/fixtures/refuter-agreement/trials-sample.json"
SIDECARS="tests/fixtures/refuter-agreement/mine-sidecars"
DOC="docs/refuter-model-tiering.md"
BATCH_DOC="docs/refuter-batching.md"
BASELINE_JSON="docs/token-baseline.json"
REVIEW_LIB=".claude/workflows/lib/review.mjs"

# ---------------------------------------------------------------------------
say "1. Hygiene: determinism, no hot-path coupling, help surface, docs"

for f in "$MODULE" "$MINER" "$RUNNER"; do
    [ -f "$f" ] || die "missing $f"
    node --check "$f" >/dev/null 2>&1 || fail "$f does not parse under node --check"
done
pass "all three scripts parse"

# GREP LIVENESS. Almost every hygiene assertion below is a grep, and grep treats
# a file containing a NUL byte as BINARY: it reports no match and exits 1,
# silently turning each of those assertions into a vacuous pass. A stray NUL is
# easy to introduce inside a template literal and `node --check` accepts it, so
# assert the scripts are real text before trusting a single grep result.
BEFORE="$FAILURES"
for f in "$MODULE" "$MINER" "$RUNNER"; do
    node -e '
const b = require("fs").readFileSync(process.argv[1]);
if (b.includes(0)) {
  console.error(process.argv[1] + " contains a NUL byte at offset " + b.indexOf(0) +
    " — grep would treat it as binary and every grep-based check in this gate would pass vacuously");
  process.exit(1);
}
' "$f" || fail "$f is not grep-readable text"
    grep -q 'export' "$f" || fail "$f: a control grep for 'export' found nothing — grep cannot read this file"
done
[ "$FAILURES" = "$BEFORE" ] && pass "all three scripts are grep-readable text (no NUL byte), so the greps below are not vacuous"

# Determinism: the report must be a pure function of its inputs. A clock or an
# RNG anywhere makes two runs over the same corpus incomparable.
BEFORE="$FAILURES"
for f in "$MODULE" "$MINER" "$RUNNER"; do
    if grep -nE 'Date\.now\(|Math\.random\(' "$f" >&2; then
        fail "$f contains a forbidden nondeterministic global (a clock or an RNG)"
    fi
done
[ "$FAILURES" = "$BEFORE" ] && pass "no clock and no RNG anywhere in the harness"

# No network beyond the dispatcher the operator explicitly asks for.
BEFORE="$FAILURES"
for f in "$MODULE" "$MINER"; do
    if grep -nE "\bfetch\(|node:https?|require\('https?'\)|child_process" "$f" >&2; then
        fail "$f reaches the network or spawns a subprocess; only the runner may"
    fi
done
[ "$FAILURES" = "$BEFORE" ] && pass "the module and the miner neither reach the network nor spawn subprocesses"

# HOT-PATH COUPLING: the harness must be a strict CONSUMER of the lane. It
# imports FROM .claude/workflows/lib/review.mjs and nothing under
# .claude/workflows/ may import it back.
if grep -rn 'refuter-agreement' .claude/workflows/ >&2; then
    fail "a file under .claude/workflows/ references refuter-agreement — the harness must never be in the hot path"
fi
pass "nothing under .claude/workflows/ references refuter-agreement"
grep -q "workflows/lib/review.mjs" "$RUNNER" || fail "the runner must import the REAL refutePrompt from .claude/workflows/lib/review.mjs"
pass "the runner imports the real refutePrompt from the canonical review source"

node "$RUNNER" --help >"$TMP/help.txt" 2>&1 || fail "--help exited non-zero"
for flag in -- --corpus --tiers --replicates --dry-run --dispatch-stub --audit \
    --audit-section --shape --batch-power --min-batch-group --allow-underpowered; do
    [ "$flag" = "--" ] && continue
    grep -q -- "$flag" "$TMP/help.txt" || fail "--help does not document $flag"
done
grep -qi "COST WARNING" "$TMP/help.txt" || fail "--help must carry the paid-dispatch cost warning"
pass "--help exits 0, documents every flag, and warns about paid dispatch"

[ -f "$DOC" ] || die "missing $DOC"
grep -q '^## Refuter-agreement harness' "$DOC" || fail "$DOC has no '## Refuter-agreement harness' section"
for f in "$MODULE" "$MINER" "$RUNNER"; do
    grep -q "$f" "$DOC" || fail "$DOC does not name $f"
done
grep -q 'on-demand' "$DOC" || fail "$DOC must state the harness is on-demand only"
pass "$DOC documents the harness, names all three scripts, and states on-demand only"
grep -q 'verify-refuter-agreement.sh' CLAUDE.md || fail "CLAUDE.md has no verify-refuter-agreement.sh bullet"
pass "CLAUDE.md carries the harness bullet"

# AC7: the rdm-wf-plan-review.js model-omission question is answered explicitly.
BEFORE_AC7="$FAILURES"
grep -qE '^## The .?rdm-wf-plan-review\.js.? model-omission question' "$DOC" ||
    fail "$DOC has no '## The rdm-wf-plan-review.js model-omission question' section"
grep -q 'f4e89d7' "$DOC" || fail "$DOC must cite commit f4e89d7"
grep -q 'verify-workflow-review.sh' "$DOC" || fail "$DOC must cite scripts/verify-workflow-review.sh"
grep -q '5b-mechanical' "$DOC" || fail "$DOC must cite the 5b-mechanical criterion"
grep -q 'docs/workflow-schemas.md' "$DOC" || fail "$DOC must cite docs/workflow-schemas.md"
grep -qE '\*\*Verdict: (deliberate|oversight)\*\*' "$DOC" ||
    fail "$DOC must state an explicit '**Verdict: deliberate|oversight**' token"
[ "$FAILURES" = "$BEFORE_AC7" ] && pass "AC7 answered with its governing citations and an explicit verdict"

# AC7 code-fact re-derivation: assert the doc's premise mechanically, so it
# cannot go stale if the binding later changes.
PLAN_LIB=".claude/workflows/lib/plan-review.mjs"
if grep -q 'findModel' "$PLAN_LIB"; then
    grep -cE 'runPlanReview\(\{[^}]*findModel' "$PLAN_LIB" >/dev/null ||
        fail "$PLAN_LIB mentions findModel but no runPlanReview call site threads it"
    pass "plan-review.mjs threads findModel (the decision changed the binding)"
else
    # Matched on the CALL, not on a one-line argument-object spelling: the
    # context object legitimately grew (e.g. `maxRefutations`) and may be
    # multi-line. The premise the doc rests on is the ABSENCE of model keys,
    # which the outer `if` already covers for findModel — assert verifyModel too.
    grep -q 'runPlanReview({' "$PLAN_LIB" ||
        fail "$PLAN_LIB no longer calls runPlanReview({ ... }) — the doc's premise is stale"
    grep -q 'target: unit.target' "$PLAN_LIB" ||
        fail "$PLAN_LIB no longer threads the review target into runPlanReview — the doc's premise is stale"
    ! grep -q 'verifyModel' "$PLAN_LIB" ||
        fail "$PLAN_LIB now threads verifyModel — the doc's 'no model bindings' premise is stale"
    pass "plan-review.mjs still calls runPlanReview with no findModel/verifyModel (the doc's premise holds)"
fi
grep -q "$DOC" docs/workflow-schemas.md || fail "docs/workflow-schemas.md must cross-reference $DOC"
pass "docs/workflow-schemas.md cross-references the decision doc"

# AC8: the decision doc's structure and its cross-artifact agreement with the JSON.
BEFORE_HEADINGS="$FAILURES"
for h in '^## The question' '^## Method' '^## Decision rule' '^## Results' \
    '^### False negatives' '^### False positives' '^### Token volume' '^### Per-class' \
    '^## DECISION' '^## Limitations'; do
    grep -q "$h" "$DOC" || fail "$DOC is missing a required section matching $h"
done
[ "$FAILURES" = "$BEFORE_HEADINGS" ] && pass "$DOC carries every required section"
DOC_DECISION="$(grep -oE 'keep-opus|tier-by-severity|thread-plan-review-models' "$DOC" | head -1 || true)"
[ -n "$DOC_DECISION" ] || fail "$DOC's DECISION section carries no recognized decision token"
JSON_DECISION="$(node -e 'const j=require("./'"$BASELINE_JSON"'");process.stdout.write(String(j.refuterModelTiering&&j.refuterModelTiering.decision))')"
if [ -n "$DOC_DECISION" ] && [ "$DOC_DECISION" = "$JSON_DECISION" ]; then
    pass "the doc and $BASELINE_JSON agree on the decision token ($DOC_DECISION)"
else
    fail "decision token disagrees: $DOC says '$DOC_DECISION', $BASELINE_JSON says '$JSON_DECISION'"
fi
grep -q "refuter-model-tiering.md" docs/token-baseline.md || fail "docs/token-baseline.md must link to the decision doc"
pass "docs/token-baseline.md links to the decision doc"

# The SHAPE decision doc: same structural + cross-artifact discipline as the
# model one above. It must state its corpus-power analysis (the pre-registered
# step 1) and the correction of the superseded naive grouping key, or a reader
# cannot tell a no-measurement outcome from a missing one.
[ -f "$BATCH_DOC" ] || die "missing $BATCH_DOC"
BEFORE_BATCH_HEADINGS="$FAILURES"
for h in '^## The question' '^## Method' '^### Corpus power' '^## Decision rule' '^## Results' \
    '^### False negatives' '^### False positives' '^### Self-consistency' '^### Anchoring' \
    '^### Token volume' '^## DECISION' '^## Limitations'; do
    grep -q "$h" "$BATCH_DOC" || fail "$BATCH_DOC is missing a required section matching $h"
done
grep -qi 'superseded' "$BATCH_DOC" ||
    fail "$BATCH_DOC must record the superseded naive grouping key and why it is void"
grep -q 'runId | unitIdent | mode | dim.key' "$BATCH_DOC" ||
    fail "$BATCH_DOC must state the UNIT-SCOPED grouping key"
grep -q 'POWER: INSUFFICIENT\|POWER: SUFFICIENT' "$BATCH_DOC" ||
    fail "$BATCH_DOC must paste the verbatim --batch-power verdict line"
[ "$FAILURES" = "$BEFORE_BATCH_HEADINGS" ] && pass "$BATCH_DOC carries every required section, the unit-scoped key, and the power verdict"

BATCH_DOC_DECISION="$(grep -oE 'ship-batched|no-ship-worse-fn|no-ship-anchoring|no-measurement' "$BATCH_DOC" | head -1 || true)"
[ -n "$BATCH_DOC_DECISION" ] || fail "$BATCH_DOC's DECISION section carries no recognized decision token"
BATCH_JSON_DECISION="$(node -e 'const j=require("./'"$BASELINE_JSON"'");process.stdout.write(String(j.refuterBatching&&j.refuterBatching.decision))')"
if [ -n "$BATCH_DOC_DECISION" ] && [ "$BATCH_DOC_DECISION" = "$BATCH_JSON_DECISION" ]; then
    pass "the batching doc and $BASELINE_JSON agree on the decision token ($BATCH_DOC_DECISION)"
else
    fail "batching decision token disagrees: $BATCH_DOC says '$BATCH_DOC_DECISION', $BASELINE_JSON says '$BATCH_JSON_DECISION'"
fi
grep -q "refuter-batching.md" docs/token-baseline.md || fail "docs/token-baseline.md must link to the batching decision doc"
grep -q "refuter-batching.md" docs/workflow-schemas.md || fail "docs/workflow-schemas.md must cross-reference the batching decision doc"
grep -q "refuter-batching.md" "$DOC" || fail "$DOC must cross-reference its sibling shape A/B"
grep -q 'refuter-batching.md' CLAUDE.md || fail "CLAUDE.md must point at the batching decision doc"
pass "the batching doc is cross-referenced from token-baseline.md, workflow-schemas.md, the sibling doc, and CLAUDE.md"

# THE SHIP/NO-SHIP XOR. A half-landed pipeline change must not be able to
# coexist with a no-ship decision, or vice versa. The batched prompt/schema live
# in the EXPERIMENT until the pre-registered rule passes.
if [ "$BATCH_JSON_DECISION" = "ship-batched" ]; then
    grep -q 'batchRefutePrompt' "$REVIEW_LIB" ||
        fail "decision is ship-batched but $REVIEW_LIB has no batchRefutePrompt"
    grep -q 'BATCH_VERDICT_SCHEMA' "$REVIEW_LIB" ||
        fail "decision is ship-batched but $REVIEW_LIB has no BATCH_VERDICT_SCHEMA"
    pass "decision is ship-batched and the pipeline carries both batched symbols"
else
    if grep -qE 'batchRefutePrompt|BATCH_VERDICT_SCHEMA' "$REVIEW_LIB" >&2; then
        fail "decision is '$BATCH_JSON_DECISION' but $REVIEW_LIB already carries a batched symbol — a half-landed pipeline change cannot coexist with a no-ship decision"
    fi
    pass "decision is '$BATCH_JSON_DECISION' and the pipeline carries no batched symbol"
fi

# ---------------------------------------------------------------------------
say "2. Corpus validation and composition floors"

cat >"$TMP/check-corpus.mjs" <<'EOF'
import fs from 'node:fs';
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';
// argv[2] is the MODULE under test (the real one, or a planted mutant), so the
// self-tests below can re-run this exact file against a mutated module.
const {
  loadCorpus, summarizeCorpus, GROUND_TRUTH_CLASSES, AUTHORITIES, PROVENANCE_KINDS,
  MIN_CORPUS_SIZE, MIN_DIVERGENCE_CLASS_SHARE, MIN_MINED_SHARE, MIN_AUTHORITATIVE_SHARE,
  DIVERGENCE_CLASS, HISTORICAL_ONLY_SEVERITIES, AUTHORITATIVE_EVIDENCE_RE,
} = await import(pathToFileURL(process.argv[2]).href);

const text = fs.readFileSync(process.argv[3], 'utf8');
const { items, errors } = loadCorpus(text);
assert.deepEqual(errors, [], 'corpus has validation errors');
const s = summarizeCorpus(items);

assert.ok(items.length >= MIN_CORPUS_SIZE, `corpus is ${items.length}, floor is ${MIN_CORPUS_SIZE}`);
assert.equal(new Set(items.map((i) => i.id)).size, items.length, 'duplicate corpus ids');
assert.ok(s.divergenceClassShare >= MIN_DIVERGENCE_CLASS_SHARE * 100,
  `${DIVERGENCE_CLASS} share ${s.divergenceClassShare}% is below the ${MIN_DIVERGENCE_CLASS_SHARE * 100}% floor`);
assert.ok(s.minedShare >= MIN_MINED_SHARE * 100, `mined share ${s.minedShare}% below floor`);
assert.ok(s.authoritativeShare >= MIN_AUTHORITATIVE_SHARE * 100, `authoritative share ${s.authoritativeShare}% below floor`);

for (const i of items) {
  assert.ok(GROUND_TRUTH_CLASSES.includes(i.groundTruth.class), `${i.id}: bad class`);
  assert.ok(AUTHORITIES.includes(i.groundTruth.authority), `${i.id}: bad authority`);
  assert.ok(PROVENANCE_KINDS.includes(i.provenance.kind), `${i.id}: bad provenance kind`);
  assert.ok(i.groundTruth.evidence.trim().length > 0, `${i.id}: empty evidence`);
  if (i.groundTruth.authority === 'authoritative') {
    assert.ok(AUTHORITATIVE_EVIDENCE_RE.test(i.groundTruth.evidence), `${i.id}: authoritative evidence cites no artifact`);
  }
  assert.ok(/^[0-9a-f]{7,40}$/.test(i.groundTruth.adjudicatedAgainstCommit), `${i.id}: no adjudication commit`);
  if (i.provenance.kind === 'mined') {
    for (const f of ['projectSlug', 'sessionId', 'runId', 'agentId', 'workflow']) {
      assert.ok(i.provenance[f], `${i.id}: mined item missing provenance.${f}`);
    }
    assert.equal(typeof i.provenance.historicalVerdict.refuted, 'boolean', `${i.id}: no historical verdict`);
  } else {
    assert.ok(i.provenance.builtAgainstCommit, `${i.id}: constructed item missing builtAgainstCommit`);
    assert.ok(i.provenance.rationale, `${i.id}: constructed item missing rationale`);
  }
  // No corpus item may embed an absolute path outside this repo — mined
  // findings quote real source text and must never carry a foreign tree in.
  const blob = JSON.stringify(i);
  const abs = blob.match(/\/Users\/[A-Za-z0-9_.-]+\/Projects\/[A-Za-z0-9_.-]+/g) || [];
  for (const a of abs) {
    assert.ok(/\/Projects\/rdm/.test(a), `${i.id}: embeds an absolute path outside the rdm repo: ${a}`);
  }
}

// Both GATING severities and both modes must be represented.
for (const sev of ['blocking', 'concern']) assert.ok(s.bySeverity[sev] > 0, `no ${sev} items`);
for (const mode of ['code', 'plan']) assert.ok(s.byMode[mode] > 0, `no ${mode}-mode items`);

// Historical-only severities are recorded, and recorded AS historical-only.
const histOnly = items.filter((i) => HISTORICAL_ONLY_SEVERITIES.includes(i.finding.severity));
assert.ok(histOnly.length > 0, 'no historical-only (suggestion) items recorded');

console.log(JSON.stringify({ size: items.length, ...s }));
EOF
node "$TMP/check-corpus.mjs" "$REPO_ROOT/$MODULE" "$CORPUS" >"$TMP/corpus-summary.json" ||
    fail "corpus validation failed"
if [ -s "$TMP/corpus-summary.json" ]; then
    pass "corpus validates: $(node -e 'const s=require("'"$TMP"'/corpus-summary.json");console.log(s.size+" items, divergence "+s.divergenceClassShare+"%, mined "+s.minedShare+"%, authoritative "+s.authoritativeShare+"%")')"
fi

# ---------------------------------------------------------------------------
say "2c. Batch-group power under the UNIT-SCOPED key"

cat >"$TMP/check-batch-power.mjs" <<'EOF'
import fs from 'node:fs';
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';
const {
  loadCorpus, groupCorpusForBatching, batchGroupKeyFor, unitIdentOf, formatBatchPower,
  MIN_BATCH_GROUP_SIZE, MIN_QUALIFYING_BATCH_GROUPS, MIN_QUALIFYING_BATCH_ITEMS,
} = await import(pathToFileURL(process.argv[2]).href);

// --- THE REGRESSION THIS SECTION EXISTS FOR ----------------------------
// buildReviewPipeline runs ONCE PER REVIEW UNIT, so a real batch is one unit's
// findings for one dimension. Two findings from the same run, mode and
// dimension but DIFFERENT review units could never be dispatched together and
// must form TWO groups. A key without the unit identity merges them into one.
function synth(id, target, extra = {}) {
  return {
    id,
    schemaVersion: 1,
    mode: 'code',
    dim: { key: 'correctness' },
    target,
    finding: { id: 'f', concern: 'correctness', location: 'x', severity: 'concern', confidence: 90, what_fails: 'y' },
    promptSha256: '0'.repeat(64),
    promptDrift: false,
    provenance: { kind: 'mined', runId: 'wf_same', projectSlug: 'p', sessionId: 's', agentId: 'a' + id, workflow: 'w', ...extra },
    groundTruth: { defect: false, class: 'stale-fact', authority: 'judgement-call', evidence: 'e', adjudicatedAgainstCommit: 'abcdef1' },
  };
}
const twoUnits = [synth('u1', 'roadmap-a/phase-1'), synth('u2', 'roadmap-a/phase-2')];
const split = groupCorpusForBatching(twoUnits, { minGroupSize: 2 });
assert.equal(split.groupCount, 2,
  'two items sharing runId/mode/dim but differing in target MERGED into one group — the grouping key has lost the review-unit identity');
assert.notEqual(batchGroupKeyFor(twoUnits[0]), batchGroupKeyFor(twoUnits[1]), 'batchGroupKeyFor does not distinguish review units');
for (const k of [batchGroupKeyFor(twoUnits[0]), batchGroupKeyFor(twoUnits[1])]) {
  assert.equal(k.split('|').length, 4, `the batch key must be four-part runId|unitIdent|mode|dim, got "${k}"`);
}
// Same unit ⇒ one group, so the split above is a real discrimination and not a
// key that simply never merges anything.
const sameUnit = [synth('s1', 'roadmap-a/phase-1'), synth('s2', 'roadmap-a/phase-1')];
assert.equal(groupCorpusForBatching(sameUnit, { minGroupSize: 2 }).groupCount, 1, 'two findings on the SAME unit did not group together');

// --- unit identity: first line, phase 1's plausibility rule -------------
assert.equal(unitIdentOf({ target: 'phase r/p-1\n\nbody with { braces }' }), 'phase r/p-1', 'plan-mode identity is the FIRST LINE');
assert.equal(unitIdentOf({ target: '{\n "steps": []\n}' }), null, 'a JSON-shaped target must be rejected, not captured as a fake identity');
assert.equal(unitIdentOf({ target: 'a "quoted" thing' }), null, 'a double-quote-bearing identity must be rejected');
assert.equal(unitIdentOf({ target: 'x'.repeat(201) }), null, 'an implausibly long identity must be rejected');
assert.equal(unitIdentOf({ target: '' }), null, 'an empty target must be rejected');

// --- the committed corpus's actual power -------------------------------
const { items } = loadCorpus(fs.readFileSync(process.argv[3], 'utf8'));
const s = groupCorpusForBatching(items);
const expected = JSON.parse(fs.readFileSync(process.argv[4], 'utf8')).refuterBatching.corpusPower;
assert.equal(s.totalItems, expected.totalItems);
assert.equal(s.constructedExcluded, expected.constructedExcluded, 'constructed exclusion count drifted');
assert.equal(s.nonGatingExcluded, expected.nonGatingExcluded, 'non-gating exclusion count drifted');
assert.equal(s.unrecoverableUnitExcluded, expected.unrecoverableUnitExcluded, 'unrecoverable-unit exclusion count drifted');
assert.equal(s.groupableItems, expected.groupableItems);
assert.equal(s.groupCount, expected.groupCount);
assert.deepEqual(s.sizeHistogram, Object.fromEntries(Object.entries(expected.sizeHistogram).map(([k, v]) => [k, v])),
  'the committed size histogram no longer matches docs/token-baseline.json');
assert.equal(s.qualifyingGroups, expected.qualifyingGroups, 'qualifying group count drifted');
assert.equal(s.qualifyingItems, expected.qualifyingItems, 'qualifying item count drifted');
assert.equal(s.meetsMinimum, expected.meetsMinimum);

// Every exclusion plus the groupable items must account for the whole corpus —
// no silent drops.
assert.equal(s.constructedExcluded + s.nonGatingExcluded + s.unrecoverableUnitExcluded + s.groupableItems, s.totalItems,
  'the three exclusions and the groupable items do not account for every corpus item');

// The minimum, and that a size-1 group NEVER counts toward it.
assert.equal(s.minGroupSize, MIN_BATCH_GROUP_SIZE);
assert.ok(MIN_BATCH_GROUP_SIZE >= 3, 'the anchoring minimum must be at least 3');
const qualifying = s.groups.filter((g) => g.size >= s.minGroupSize);
assert.ok(qualifying.every((g) => g.size > 1), 'a size-1 group qualified');
assert.equal(qualifying.reduce((a, g) => a + g.size, 0), s.qualifyingItems, 'qualifyingItems does not match the qualifying groups');
assert.equal(s.minQualifyingGroups, MIN_QUALIFYING_BATCH_GROUPS);
assert.equal(s.minQualifyingItems, MIN_QUALIFYING_BATCH_ITEMS);

// The rendered verdict line must exist and agree with meetsMinimum.
const text = formatBatchPower(s);
assert.ok(/^POWER: (SUFFICIENT|INSUFFICIENT)$/m.test(text), 'formatBatchPower prints no POWER: verdict line');
assert.ok(text.includes('POWER: ' + (s.meetsMinimum ? 'SUFFICIENT' : 'INSUFFICIENT')), 'the POWER verdict contradicts meetsMinimum');

// Round splitting: with agentIndex on every member, a discontinuity splits.
const rounds = groupCorpusForBatching(
  [synth('r1', 'roadmap-a/phase-1', { agentIndex: 3 }), synth('r2', 'roadmap-a/phase-1', { agentIndex: 4 }), synth('r3', 'roadmap-a/phase-1', { agentIndex: 90 })],
  { minGroupSize: 2 }
);
assert.equal(rounds.groupCount, 2, 'a rework re-review far from the first round was not split off');
assert.equal(rounds.roundSplits, 1, 'the round split was not counted');

console.log(`unit-scoped grouping holds: ${s.groupCount} groups, ${s.qualifyingGroups} qualifying / ${s.qualifyingItems} items, POWER ${s.meetsMinimum ? 'SUFFICIENT' : 'INSUFFICIENT'}`);
EOF
node "$TMP/check-batch-power.mjs" "$REPO_ROOT/$MODULE" "$CORPUS" "$REPO_ROOT/$BASELINE_JSON" >"$TMP/batchpower.txt" ||
    fail "batch-group power check failed"
[ -s "$TMP/batchpower.txt" ] && pass "$(cat "$TMP/batchpower.txt")"

# --batch-power must exit 0, print the verdict, and DISPATCH NOTHING.
cat >"$TMP/exploding-power-stub.mjs" <<'EOF'
export function dispatch() {
  throw new Error('DISPATCHED-UNDER-BATCH-POWER');
}
EOF
node "$RUNNER" --batch-power --dispatch-stub "$TMP/exploding-power-stub.mjs" >"$TMP/power.txt" 2>&1 || {
    cat "$TMP/power.txt" >&2
    fail "--batch-power exited non-zero"
}
grep -qE '^POWER: (SUFFICIENT|INSUFFICIENT)$' "$TMP/power.txt" || fail "--batch-power printed no POWER: verdict line"
grep -q 'DISPATCHED-UNDER-BATCH-POWER' "$TMP/power.txt" && fail "--batch-power CALLED the dispatcher"
pass "--batch-power exits 0, prints a POWER: verdict, and provably dispatches nothing"

# --- 2c-equivalence: the deliberately duplicated unit-identity predicate ----
# refuter-agreement.mjs restates measure-refuter-severity.mjs's
# isPlausibleUnitIdent rather than importing it (that module is a CLI and
# importing it would invert the dependency). The two copies must never drift.
cat >"$TMP/check-unit-equivalence.mjs" <<'EOF'
import fs from 'node:fs';
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';
const { loadCorpus, unitIdentOf } = await import(pathToFileURL(process.argv[2]).href);
const { isPlausibleUnitIdent } = await import(pathToFileURL(process.argv[3]).href);

const { items } = loadCorpus(fs.readFileSync(process.argv[4], 'utf8'));
let checked = 0;
let rejected = 0;
for (const i of items) {
  const target = String(i.target || '');
  const nl = target.indexOf('\n');
  const first = nl === -1 ? target : target.slice(0, nl);
  const mine = unitIdentOf(i) !== null;
  const canonical = isPlausibleUnitIdent(first);
  assert.equal(mine, canonical, `${i.id}: unitIdentOf says ${mine} but isPlausibleUnitIdent says ${canonical} for first line ${JSON.stringify(first.slice(0, 60))}`);
  checked += 1;
  if (!canonical) rejected += 1;
}
// The check must not pass vacuously: some committed item really is rejected.
assert.ok(rejected > 0, 'no committed item exercises the rejection branch — the equivalence check is vacuous');
for (const probe of ['', '{ "a": 1 }', 'has "quotes"', 'x'.repeat(201), 'roadmap/phase-1', 'task/some-slug']) {
  assert.equal(unitIdentOf({ target: probe }) !== null, isPlausibleUnitIdent(probe), `predicates disagree on ${JSON.stringify(probe.slice(0, 40))}`);
}
console.log(`unitIdentOf and isPlausibleUnitIdent agree on all ${checked} committed targets (${rejected} rejected) and 6 probes`);
EOF
node "$TMP/check-unit-equivalence.mjs" "$REPO_ROOT/$MODULE" "$REPO_ROOT/scripts/measure-refuter-severity.mjs" "$CORPUS" \
    >"$TMP/uniteq.txt" || fail "the duplicated unit-identity predicate has drifted from phase 1's canonical rule"
[ -s "$TMP/uniteq.txt" ] && pass "$(cat "$TMP/uniteq.txt")"

# --- 2c-guard: an underpowered batched arm can never be a passing gate ------
cat >"$TMP/check-underpowered.mjs" <<'EOF'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';
const { groupCorpusForBatching, buildBatchTrials, scoreTrials, formatReport } = await import(pathToFileURL(process.argv[2]).href);

function synth(id, target) {
  return {
    id, schemaVersion: 1, mode: 'code', dim: { key: 'correctness' }, target,
    finding: { id: 'f', concern: 'correctness', location: 'x', severity: 'concern', confidence: 90, what_fails: 'y' },
    promptSha256: '0'.repeat(64), promptDrift: false,
    provenance: { kind: 'mined', runId: 'wf_' + id, projectSlug: 'p', sessionId: 's', agentId: 'a' + id, workflow: 'w' },
    groundTruth: { defect: false, class: 'stale-fact', authority: 'judgement-call', evidence: 'e', adjudicatedAgainstCommit: 'abcdef1' },
  };
}
const singletons = Array.from({ length: 20 }, (_, i) => synth('s' + i, 'roadmap/phase-' + i));
const summary = groupCorpusForBatching(singletons);
assert.equal(summary.qualifyingGroups, 0, 'an all-singleton corpus reported a qualifying group');
assert.equal(summary.meetsMinimum, false, 'an all-singleton corpus reported SUFFICIENT power');

// THROWS by default — the mechanical form of "never a passing gate".
assert.throws(() => buildBatchTrials(summary, { tiers: ['opus'], replicates: 2 }), /UNDERPOWERED/,
  'buildBatchTrials built a batched arm from an all-singleton corpus without complaint');

// --allow-underpowered builds it, but stamps noMeasurement and forces the banner.
const plan = buildBatchTrials(summary, { tiers: ['opus'], replicates: 2, allowUnderpowered: true });
assert.equal(plan.noMeasurement, true, 'allowUnderpowered did not stamp noMeasurement');
assert.equal(plan.underpowered, true, 'allowUnderpowered did not stamp underpowered');
const report = scoreTrials(singletons, [], { baselineTier: 'opus', noMeasurement: plan.noMeasurement, decision: 'ship-batched' });
const text = formatReport(report, 'text');
assert.ok(/^NO MEASUREMENT — batched arm was underpowered$/m.test(text), 'the NO MEASUREMENT banner is missing');
assert.ok(!/^DECISION:/m.test(text), 'a decision line was printed for a no-measurement report');
console.log('an underpowered batched arm throws by default, and --allow-underpowered forces a NO MEASUREMENT banner with no decision line');
EOF
node "$TMP/check-underpowered.mjs" "$REPO_ROOT/$MODULE" >"$TMP/underpowered.txt" ||
    fail "the underpowered-arm guard failed"
[ -s "$TMP/underpowered.txt" ] && pass "$(cat "$TMP/underpowered.txt")"

# ---------------------------------------------------------------------------
say "3. Prompt fidelity: regeneration through the REAL refutePrompt"

cat >"$TMP/check-prompts.mjs" <<'EOF'
import fs from 'node:fs';
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';
const { loadCorpus, checkPromptFidelity, regeneratePrompt } = await import(pathToFileURL(process.argv[2]).href);
const { refutePrompt } = await import(pathToFileURL(process.argv[3]).href);

const { items } = loadCorpus(fs.readFileSync(process.argv[4], 'utf8'));
const drifted = [];
let minedChecked = 0;
for (const i of items) {
  const r = checkPromptFidelity(i, refutePrompt);
  if (r.drifted !== i.promptDrift) drifted.push(`${i.id}: promptDrift is ${i.promptDrift} but regeneration says ${r.drifted}`);
  if (i.provenance.kind === 'mined') {
    // The 401-char truncation is a promptPreview/resultPreview property of the
    // wf_*.json SIDECARS. A mined prompt longer than that proves the TRANSCRIPT
    // was the source.
    const len = regeneratePrompt(i, refutePrompt).length;
    assert.ok(len > 401, `${i.id}: recovered prompt is only ${len} chars — that is sidecar-preview length, not transcript length`);
    minedChecked += 1;
  }
}
assert.deepEqual(drifted, [], 'prompt drift is unrecorded');
assert.ok(minedChecked > 0, 'no mined items to check');
console.log(`${items.length} prompts regenerate as recorded; ${minedChecked} mined prompts all exceed 401 chars`);
EOF
node "$TMP/check-prompts.mjs" "$REPO_ROOT/$MODULE" "$REPO_ROOT/$REVIEW_LIB" "$CORPUS" >"$TMP/prompts.txt" ||
    fail "prompt fidelity check failed"
[ -s "$TMP/prompts.txt" ] && pass "$(cat "$TMP/prompts.txt")"

# ---------------------------------------------------------------------------
say "4. Miner behavior against the hermetic sidecar fixture"

[ -d "$SIDECARS" ] || die "missing fixture tree $SIDECARS"
node "$MINER" --root "$SIDECARS" --format json >"$TMP/mined.json" 2>"$TMP/mined.err" ||
    fail "miner exited non-zero against the fixture"

cat >"$TMP/check-miner.mjs" <<'EOF'
import fs from 'node:fs';
import assert from 'node:assert/strict';
const m = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));

// Three recoverable refuters; EVERY non-filter degradation branch the miner can
// take is exercised and COUNTED. These branches decide how many historical
// refuters make it into the corpus at all, so a regression that made one of them
// fire on healthy transcripts would silently shrink the mined majority.
assert.equal(m.recovered, 3, `expected 3 recovered, got ${m.recovered}`);
assert.equal(m.skips['unparseable-finding'], 1, 'unparseable finding not bucketed');
assert.equal(m.skips['no-verdict'], 1, 'verdictless transcript not bucketed');
assert.equal(m.skips['no-transcript'], 1, 'missing transcript not bucketed');
assert.equal(m.skips['no-prompt'], 1, 'a transcript with no initiating user turn was not bucketed');
assert.equal(m.skips['unrecoverable-mode'], 1, 'a prompt with no code/plan stance was not bucketed');
assert.equal(m.skips['unrecoverable-dim'], 1, 'a prompt with no dimension header was not bucketed');
// No skip bucket may be a silent drop: recovered + skips must account for every
// refuter record the parser found.
const skipTotal = Object.values(m.skips).reduce((a, b) => a + b, 0);
assert.equal(m.recovered + skipTotal, m.refuterRecordCount,
  `recovered(${m.recovered}) + skipped(${skipTotal}) != refuter records(${m.refuterRecordCount})`);
// The skip branches are ORDERED, and each must claim only its own transcript:
// a mode-less prompt must not be filed as unparseable-finding, and vice versa.
assert.ok(!('severity-filtered' in m.skips), 'no --severity was passed, so nothing may be severity-filtered');

const byId = Object.fromEntries(m.items.map((i) => [i.id, i]));
const ok = byId['mined-wf_mine001-refute-ok'];
assert.ok(ok, 'the recoverable code-mode refuter was not mined');
assert.equal(ok.mode, 'code');
assert.equal(ok.dim.key, 'correctness');
assert.equal(ok.finding.severity, 'blocking');
assert.equal(ok.provenance.historicalVerdict.refuted, true);
assert.equal(ok.groundTruth, null, 'the miner must NEVER assign ground truth');
assert.ok(ok.promptLength > 401, 'mined prompt is sidecar-preview length');

// The inline-brace hazard: the target is itself a pretty-printed JSON document,
// so a naive indexOf('{') finds the TARGET and a first-line read TRUNCATES it.
const plan = byId['mined-wf_mine001-refute-plan'];
assert.ok(plan, 'the implementation-plan-target refuter was not mined');
assert.equal(plan.mode, 'plan');
assert.equal(plan.dim.key, 'coherence');
assert.equal(plan.finding.id, 'f-2', 'the finding was mis-extracted from a brace-bearing target');
assert.ok(plan.target.includes('\n'), 'a multi-line target was truncated to its first line');
assert.ok(plan.target.includes('steps_per_ac'), 'the pretty-printed plan target was not recovered whole');

// A recoverable non-blocking refuter, so the --severity filter below has a real
// item to exclude rather than passing vacuously.
const concern = byId['mined-wf_mine001-refute-concern'];
assert.ok(concern, 'the recoverable concern-severity refuter was not mined');
assert.equal(concern.finding.severity, 'concern');
assert.equal(concern.dim.key, 'tests');

// The out-of-scope project slug must contribute NOTHING.
for (const i of m.items) {
  assert.ok(i.provenance.projectSlug.startsWith('-Users-edward-Projects-rdm'),
    `out-of-scope slug leaked into the corpus: ${i.provenance.projectSlug}`);
}
console.log('miner recovers all three shapes, buckets all six degradations with no silent drops, assigns no ground truth, and filters foreign slugs');
EOF
node "$TMP/check-miner.mjs" "$TMP/mined.json" >"$TMP/miner.txt" || fail "miner behavior check failed"
[ -s "$TMP/miner.txt" ] && pass "$(cat "$TMP/miner.txt")"

# The slug filter is a POSITIVE gate, not an accident of the fixture: opening it
# up must surface the foreign item.
node "$MINER" --root "$SIDECARS" --project-slug '-Users-edward-Projects-' --format json >"$TMP/mined-open.json" 2>/dev/null
# String(): node colorizes a bare numeric console.log argument, and the escape
# codes would make the comparison below fail for the wrong reason.
FOREIGN="$(node -e 'const m=require("'"$TMP"'/mined-open.json");console.log(String(m.items.filter(i=>i.provenance.projectSlug.includes("bowling")).length))')"
[ "$FOREIGN" = "1" ] || fail "widening --project-slug did not surface the out-of-scope item (got $FOREIGN); the filter may be vacuous"
pass "widening --project-slug surfaces the out-of-scope item — the default filter is load-bearing"

# ---------------------------------------------------------------------------
say "4b. The miner's remaining CLI surface: --severity, --until, --limit, --out, --help, errors"

# --severity is a real filter: it must EXCLUDE the mined concern item and file
# the exclusion under its own counted bucket, not drop it silently.
cat >"$TMP/check-miner-severity.mjs" <<'EOF'
import fs from 'node:fs';
import assert from 'node:assert/strict';
const m = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
assert.equal(m.recovered, 1, `--severity blocking should recover exactly the blocking item, got ${m.recovered}`);
assert.equal(m.items[0].finding.severity, 'blocking');
assert.equal(m.skips['severity-filtered'], 2, 'the excluded items were not bucketed as severity-filtered');
console.log('--severity excludes non-matching severities into a counted severity-filtered bucket');
EOF
node "$MINER" --root "$SIDECARS" --severity blocking --format json >"$TMP/mined-sev.json" 2>/dev/null
node "$TMP/check-miner-severity.mjs" "$TMP/mined-sev.json" >"$TMP/sev.txt" ||
    fail "--severity filter did not exclude and bucket correctly"
[ -s "$TMP/sev.txt" ] && pass "$(cat "$TMP/sev.txt")"

# A comma-separated --severity admits both, proving the filter is a set test and
# not a hardcoded equality against one value.
node "$MINER" --root "$SIDECARS" --severity blocking,concern --format json >"$TMP/mined-sev2.json" 2>/dev/null
node -e '
const fs = require("fs"), assert = require("assert");
const m = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
assert.equal(m.recovered, 3, "comma-separated --severity should recover all 3, got " + m.recovered);
assert.ok(!("severity-filtered" in m.skips), "nothing may remain severity-filtered once both severities are admitted");
' "$TMP/mined-sev2.json" || fail "comma-separated --severity did not admit both severities"
pass "--severity accepts a comma-separated set"

# --until pins the mining window. The fixture run starts 2026-07-25T09:00:00Z, so
# a cutoff before it must empty the run set and a cutoff after it must not.
node "$MINER" --root "$SIDECARS" --until 2026-07-24T00:00:00Z --format json >"$TMP/mined-before.json" 2>/dev/null
node "$MINER" --root "$SIDECARS" --until 2026-07-26T00:00:00Z --format json >"$TMP/mined-after.json" 2>/dev/null
node -e '
const fs = require("fs"), assert = require("assert");
const before = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
const after = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
assert.equal(before.recovered, 0, "--until before the run still mined it");
assert.equal(before.refuterRecordCount, 0, "--until before the run did not drop the run file");
assert.equal(after.recovered, 3, "--until after the run should mine everything, got " + after.recovered);
assert.equal(after.until, "2026-07-26T00:00:00Z", "--until was not echoed into the report");
' "$TMP/mined-before.json" "$TMP/mined-after.json" || fail "--until window filter is wrong in one direction"
pass "--until pins the mining window in both directions"

# --limit stops after n RECOVERED records (not n scanned records).
node "$MINER" --root "$SIDECARS" --limit 1 --format json >"$TMP/mined-limit.json" 2>/dev/null
node -e '
const fs = require("fs"), assert = require("assert");
const m = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
assert.equal(m.recovered, 1, "--limit 1 recovered " + m.recovered);
assert.equal(m.items.length, 1);
' "$TMP/mined-limit.json" || fail "--limit did not cap the recovered records"
pass "--limit caps recovered records"

# --out writes JSONL to a file and leaves stdout empty; the default format is
# one JSON object per line, which is what the corpus file is made of.
node "$MINER" --root "$SIDECARS" --out "$TMP/mined.jsonl" >"$TMP/mined-out.stdout" 2>/dev/null
[ -s "$TMP/mined.jsonl" ] || fail "--out wrote no file"
[ -s "$TMP/mined-out.stdout" ] && fail "--out still wrote the body to stdout"
JSONL_LINES="$(wc -l <"$TMP/mined.jsonl" | tr -d ' ')"
[ "$JSONL_LINES" = "3" ] || fail "--out JSONL should have 3 lines, got $JSONL_LINES"
node -e '
const fs = require("fs"), assert = require("assert");
for (const line of fs.readFileSync(process.argv[1], "utf8").split("\n").filter(Boolean)) {
  const item = JSON.parse(line);
  assert.equal(item.provenance.kind, "mined");
  assert.equal(item.groundTruth, null);
}
' "$TMP/mined.jsonl" || fail "--out JSONL lines are not parseable mined candidates"
pass "--out writes parseable JSONL to disk and nothing to stdout"

# --help exits 0 and names every documented flag, so the help text cannot drift
# away from parseArgs.
node "$MINER" --help >"$TMP/miner-help.txt" 2>&1 || fail "$MINER --help exited non-zero"
for flag in --root --project-slug --until --severity --limit --out --format --min-group-size --exclude-corpus; do
    grep -q -- "$flag" "$TMP/miner-help.txt" || fail "$MINER --help does not document $flag"
done
grep -q "groundTruth: null" "$TMP/miner-help.txt" ||
    fail "$MINER --help no longer states that it assigns no ground truth"
pass "--help exits 0, documents every flag, and restates the no-ground-truth contract"

# --min-group-size buys only power-ADDING candidates. The three fixture refuters
# sit in three DIFFERENT unit-scoped groups, so a floor of 2 must empty the
# output and bucket all three — proving the filter is real, not decorative.
node "$MINER" --root "$SIDECARS" --min-group-size 2 --format json >"$TMP/mined-group.json" 2>/dev/null
node -e '
const fs = require("fs"), assert = require("assert");
const m = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
assert.equal(m.recovered, 0, "--min-group-size 2 kept " + m.recovered + " candidate(s) from an all-singleton fixture");
assert.equal(m.skips["below-min-group-size"], 3, "the filtered candidates were not bucketed");
// No silent drops: the accounting identity still holds with the new bucket.
const skipTotal = Object.values(m.skips).reduce((a, b) => a + b, 0);
assert.equal(m.recovered + skipTotal, m.refuterRecordCount, "--min-group-size introduced a silent drop");
assert.equal(m.batchGrouping.minGroupSize, 2);
assert.equal(m.batchGrouping.qualifyingGroups, 0, "an all-singleton fixture reported a qualifying group");
' "$TMP/mined-group.json" || fail "--min-group-size did not filter to unit-scoped groups"
pass "--min-group-size emits only unit-scoped groups at or above the floor, bucketing the rest with no silent drop"

# --exclude-corpus makes a re-mine APPEND rather than duplicate.
node -e '
const fs = require("fs");
const m = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
fs.writeFileSync(process.argv[2], JSON.stringify({ id: m.items[0].id }) + "\n");
' "$TMP/mined.json" "$TMP/already.jsonl"
node "$MINER" --root "$SIDECARS" --exclude-corpus "$TMP/already.jsonl" --format json >"$TMP/mined-excl.json" 2>/dev/null
node -e '
const fs = require("fs"), assert = require("assert");
const all = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
const excl = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
assert.equal(excl.recovered, all.recovered - 1, "--exclude-corpus dropped " + (all.recovered - excl.recovered) + " candidate(s), expected 1");
assert.equal(excl.skips["already-adjudicated"], 1, "the excluded candidate was not bucketed as already-adjudicated");
assert.ok(!excl.items.some((i) => i.id === all.items[0].id), "the already-adjudicated id was still emitted");
const skipTotal = Object.values(excl.skips).reduce((a, b) => a + b, 0);
assert.equal(excl.recovered + skipTotal, excl.refuterRecordCount, "--exclude-corpus introduced a silent drop");
' "$TMP/mined.json" "$TMP/mined-excl.json" || fail "--exclude-corpus did not drop an already-adjudicated id"
pass "--exclude-corpus drops an already-adjudicated id into its own counted bucket"

# provenance.agentIndex is what lets a REWORK re-review be split off a
# first-round batch instead of silently inflating its apparent size.
node -e '
const fs = require("fs"), assert = require("assert");
const m = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
for (const i of m.items) {
  assert.ok(Number.isInteger(i.provenance.agentIndex),
    i.id + ": newly mined items must carry provenance.agentIndex so a rework round can be split from a first-round batch");
}
' "$TMP/mined.json" || fail "newly mined items carry no provenance.agentIndex"
pass "newly mined items carry provenance.agentIndex"

# Every argument-validation failure must be an ACTIONABLE named message, not a
# stack trace — the miner is run by hand and a raw throw is unreadable.
miner_arg_error() {
    local label="$1"
    shift
    if node "$MINER" "$@" >/dev/null 2>"$TMP/miner-err.txt"; then
        fail "$label: miner exited 0 on a malformed argument"
        return 0
    fi
    grep -qE -- "$label" "$TMP/miner-err.txt" ||
        fail "$label: error message does not name the offending flag/value: $(head -1 "$TMP/miner-err.txt")"
    if grep -q '^    at ' "$TMP/miner-err.txt"; then
        fail "$label: a raw stack trace leaked instead of an actionable message"
    fi
    return 0
}
miner_arg_error '--until' --root "$SIDECARS" --until not-a-date
miner_arg_error '--limit' --root "$SIDECARS" --limit 0
miner_arg_error '--limit' --root "$SIDECARS" --limit abc
miner_arg_error '--format' --root "$SIDECARS" --format yaml
miner_arg_error '--root' --root
miner_arg_error 'unrecognized' --nonsense
pass "every argument-validation failure is an actionable named error, never a stack trace"

# ---------------------------------------------------------------------------
say "5. Scorer behavior against the recorded sample trials"

cat >"$TMP/check-scorer.mjs" <<'EOF'
import fs from 'node:fs';
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';
const { loadCorpus, scoreTrials, formatReport, TOKEN_CLASSES } = await import(pathToFileURL(process.argv[2]).href);

const { items } = loadCorpus(fs.readFileSync(process.argv[3], 'utf8'));
const sample = JSON.parse(fs.readFileSync(process.argv[4], 'utf8'));
const r = scoreTrials(items, sample.trials, { baselineTier: sample.baselineTier });
const tier = Object.fromEntries(r.tiers.map((t) => [t.tier, t]));

// --- FALSE NEGATIVES: defect-truth trials only -------------------------
assert.equal(tier.opus.all.falseNegatives, 1, 'opus FN count');
assert.equal(tier.opus.all.defectTrials, 4, 'opus FN denominator');
assert.equal(tier.opus.all.falseNegativeRate, 25, 'opus FN rate');
assert.equal(tier.sonnet.all.falseNegatives, 2, 'sonnet FN count');
assert.equal(tier.sonnet.all.falseNegativeRate, 50, 'sonnet FN rate');

// --- FALSE POSITIVES: non-defect-truth trials only, a DIFFERENT denominator.
assert.equal(tier.opus.all.falsePositives, 2, 'opus FP count');
assert.equal(tier.opus.all.nonDefectTrials, 5, 'opus FP denominator');
assert.equal(tier.opus.all.falsePositiveRate, 40, 'opus FP rate');
// The two denominators genuinely differ (4 vs 5), so a pooled rate would be a
// DIFFERENT number — which is exactly why they must never be pooled.
assert.notEqual(tier.opus.all.defectTrials, tier.opus.all.nonDefectTrials,
  'the FN and FP denominators must differ in this fixture, or the separation is untested');

// --- UNGRADED belongs to neither rate ----------------------------------
assert.equal(tier.opus.all.ungraded, 1, 'opus ungraded count');
assert.equal(tier.opus.all.defectTrials + tier.opus.all.nonDefectTrials + tier.opus.all.ungraded,
  tier.opus.all.trials, 'ungraded trials leaked into a rate denominator');

// --- authoritative / judgement-call partition --------------------------
for (const t of ['opus', 'sonnet']) {
  for (const f of ['trials', 'ungraded', 'defectTrials', 'nonDefectTrials', 'falseNegatives', 'falsePositives']) {
    assert.equal(tier[t].authoritativeOnly[f] + tier[t].judgementCallOnly[f], tier[t].all[f],
      `${t}.${f}: authoritativeOnly + judgementCallOnly must partition all`);
  }
}
assert.equal(tier.opus.authoritativeOnly.falsePositives, 0, 'opus authoritative FP');
assert.equal(tier.opus.judgementCallOnly.falsePositives, 2, 'opus judgement-call FP');
assert.equal(tier.sonnet.authoritativeOnly.falsePositives, 2, 'sonnet authoritative FP');

// --- self-consistency ---------------------------------------------------
assert.equal(tier.opus.selfConsistency.replicateFlips, 1, 'opus flips');
assert.equal(tier.opus.selfConsistency.flipRate, 25, 'opus flip rate');
assert.equal(tier.sonnet.selfConsistency.replicateFlips, 1, 'sonnet flips');

// --- historical-only severities never reach a rate ----------------------
assert.equal(r.historicalOnlyTrials, 4, 'historical-only trials not excluded');
for (const t of ['opus', 'sonnet']) {
  assert.equal(tier[t].historicalOnlyTrials, 2, `${t} historical-only count`);
  for (const row of tier[t].byClass) {
    assert.ok(row.class !== 'suggestion', 'a suggestion severity became a ground-truth class');
  }
}

// --- per-class breakdown ------------------------------------------------
const opusClasses = Object.fromEntries(tier.opus.byClass.map((c) => [c.class, c]));
assert.equal(opusClasses['real-defect'].falseNegatives, 1);
assert.equal(opusClasses['mechanically-true-not-a-defect'].falsePositives, 2);
const sonnetClasses = Object.fromEntries(tier.sonnet.byClass.map((c) => [c.class, c]));
assert.equal(sonnetClasses['mechanically-true-not-a-defect'].falsePositives, 3);

// --- token + tool-call columns -----------------------------------------
for (const t of ['opus', 'sonnet']) {
  for (const c of TOKEN_CLASSES) assert.equal(typeof tier[t].cost[c], 'number', `${t}.cost.${c} missing`);
  assert.equal(typeof tier[t].cost.meanTokensPerTrial, 'number');
  assert.equal(typeof tier[t].cost.meanToolCallsPerTrial, 'number');
}
// Hand-summed: opus output = 11*1000 + 1*500 = 11500 over 12 dispatched trials.
assert.equal(tier.opus.cost.output, 11500, 'opus output tokens');
assert.equal(tier.opus.cost.dispatchedTrials, 12, 'opus dispatched trials');
assert.equal(tier.sonnet.cost.output, 24000, 'sonnet output tokens');
// tokenDelta present on the non-baseline tier, absent on the baseline.
assert.equal(tier.opus.tokenDelta, null, 'the baseline tier must carry tokenDelta: null');
assert.ok(tier.sonnet.tokenDelta && typeof tier.sonnet.tokenDelta.delta === 'number', 'sonnet tokenDelta missing');

// --- the FN row carries the cost columns on the SAME line ---------------
const text = formatReport(r, 'text');
const fnBlock = text.split('## FALSE POSITIVES')[0];
const opusRow = fnBlock.split('\n').find((l) => l.startsWith('| opus |'));
assert.ok(opusRow, 'no opus row in the FALSE NEGATIVES table');
assert.ok(/25\.0%/.test(opusRow), 'the FN row does not carry the FN rate');
assert.ok(/11,500/.test(opusRow), 'the FN row does not carry the output-token column');
assert.ok(/13\.6/.test(opusRow), 'the FN row does not carry the mean-tool-calls column');

// --- required prose -----------------------------------------------------
assert.ok(/ships a defect/i.test(text), 'the FN block does not spell out its consequence');
assert.ok(/costs a rework round/i.test(text), 'the FP block does not spell out its consequence');
assert.ok(/PRICE-PER-TOKEN, not token VOLUME/i.test(text), 'the volume-not-price caveat is missing');
assert.ok(/DELIBERATELY WEIGHTED/i.test(text), 'the weighted-corpus caveat is missing');

console.log('scorer separates FN/FP over distinct denominators, buckets ungraded, partitions by authority, tracks flips, and puts cost on the FN row');
EOF
node "$TMP/check-scorer.mjs" "$REPO_ROOT/$MODULE" "$CORPUS" "$TRIALS" >"$TMP/scorer.txt" ||
    fail "scorer behavior check failed"
[ -s "$TMP/scorer.txt" ] && pass "$(cat "$TMP/scorer.txt")"

# ---------------------------------------------------------------------------
say "5c. Batched scoring: expansion, arm buckets, token attribution, anchoring"

cat >"$TMP/check-batch-scoring.mjs" <<'EOF'
import fs from 'node:fs';
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';
const {
  loadCorpus, groupCorpusForBatching, buildBatchTrials, expandBatchResults, buildBatchPrompt,
  scoreTrials, scoreAnchoring, formatReport, findBlendedAccuracyKeys, bucketKeyFor,
} = await import(pathToFileURL(process.argv[2]).href);
const { refutePrompt } = await import(pathToFileURL(process.argv[3]).href);

const { items } = loadCorpus(fs.readFileSync(process.argv[4], 'utf8'));
const power = groupCorpusForBatching(items);
// The committed corpus is underpowered by design; this section is about the
// SCORING mechanics, so it builds the arm explicitly under the guard.
const plan = buildBatchTrials(power, { tiers: ['opus'], replicates: 2, allowUnderpowered: true });
assert.ok(plan.trials.length > 0, 'no batched trials were built from the qualifying group');
const group = plan.groups[0];
assert.ok(group.size >= power.minGroupSize, 'the batched arm was built from a below-floor group');

// --- the batched prompt is a MINIMAL delta from the real refutePrompt ---
const members = group.ids.map((id) => items.find((i) => i.id === id));
const keyed = members.map((m) => ({ refute_id: m.id, ...m.finding }));
const batchPrompt = buildBatchPrompt(members[0].mode, { key: group.dim }, keyed, { target: members[0].target });
const single = refutePrompt(members[0].mode, { key: group.dim }, members[0].finding, { target: members[0].target });
assert.ok(batchPrompt.startsWith('You are a READ-ONLY refuter. Do not edit any files.'), 'the batched prompt changed the READ-ONLY stance');
assert.ok(batchPrompt.includes('this is NOT a real issue unless the ' + (members[0].mode === 'code' ? 'code' : 'plan') + ' proves otherwise'),
  'the batched prompt changed the stance sentence — that is a confound, not a shape change');
assert.ok(single.includes('this is NOT a real issue unless the ' + (members[0].mode === 'code' ? 'code' : 'plan') + ' proves otherwise'),
  'the single-finding stance sentence moved; the two prompts are no longer a minimal delta');
for (const m of members) assert.ok(batchPrompt.includes(JSON.stringify(m.id)), `the batch prompt omits refute_id ${m.id}`);
assert.ok(/verdicts/.test(batchPrompt), 'the batched prompt does not ask for a verdicts array');

// --- expansion: unknown ids dropped, omissions ungraded, cost on row 1 --
const ids = group.ids;
const dispatched = [
  {
    dispatchId: 'd1', groupKey: group.key, tier: 'opus', replicate: 1, corpusIds: ids,
    verdicts: [
      { id: ids[0], refuted: true, confidence: 90 },
      { id: 'zz-not-in-this-batch', refuted: true, confidence: 90 },
      // ids[1] deliberately OMITTED
      { id: ids[2], refuted: false, confidence: 80 },
    ],
    usage: { output: 100, uncachedInput: 200, cacheWrite: 300, cacheRead: 400 },
    toolCalls: 11,
  },
  // A CRASHED dispatch: no verdicts array at all.
  { dispatchId: 'd2', groupKey: group.key, tier: 'opus', replicate: 2, corpusIds: ids, verdicts: null, error: 'boom', usage: {}, toolCalls: 0 },
];
const expanded = expandBatchResults(dispatched);
assert.equal(expanded.rows.length, ids.length * 2, 'expansion did not produce one row per id per dispatch');
assert.deepEqual(expanded.unknownVerdictIds, ['d1:zz-not-in-this-batch'], 'the unknown-id verdict was not dropped and recorded');
assert.ok(expanded.omittedIds.includes('d1:' + ids[1]), 'the omitted id was not recorded');
const r0 = expanded.rows.find((r) => r.dispatchId === 'd1' && r.corpusId === ids[0]);
const r1 = expanded.rows.find((r) => r.dispatchId === 'd1' && r.corpusId === ids[1]);
const r2 = expanded.rows.find((r) => r.dispatchId === 'd1' && r.corpusId === ids[2]);
assert.equal(r0.verdict.refuted, true);
assert.equal(r1.verdict, null, 'an OMITTED id must stay ungraded — never coerced to refuted:false, which would inflate the FP rate');
assert.equal(r2.verdict.refuted, false);
for (const r of expanded.rows.filter((x) => x.dispatchId === 'd2')) {
  assert.equal(r.verdict, null, 'a crashed batch must leave every id ungraded');
}
// Cost lands entirely on the FIRST row, so totalTokens stays exact while
// dispatches stay countable.
assert.equal(r0.usage.output, 100);
assert.equal(r0.toolCalls, 11);
assert.deepEqual(r1.usage, {});
assert.equal(r2.toolCalls, 0);
assert.equal(r0.positionInBatch, 1);
assert.equal(r2.positionInBatch, 3);

// --- arm bucketing: two shapes, one report, independent denominators ---
assert.equal(bucketKeyFor({ tier: 'opus' }), 'opus', 'a trial with no arm must bucket under its bare tier');
assert.equal(bucketKeyFor({ tier: 'opus', arm: 'batched' }), 'opus|batched');
const perFindingRows = ids.flatMap((id, i) =>
  [1, 2].map((rep) => ({
    trialId: id + '|opus|' + rep, corpusId: id, tier: 'opus', replicate: rep,
    verdict: { refuted: i === 0, confidence: 90 },
    usage: { output: 100, uncachedInput: 200, cacheWrite: 300, cacheRead: 400 }, toolCalls: 11,
  }))
);
const report = scoreTrials(items, perFindingRows.concat(expanded.rows), { baselineTier: 'opus' });
const byBucket = Object.fromEntries(report.tiers.map((t) => [t.bucket, t]));
assert.ok(byBucket.opus, 'the per-finding arm did not bucket under "opus"');
assert.ok(byBucket['opus|batched'], 'the batched arm did not bucket under "opus|batched"');
assert.equal(byBucket.opus.arm, null);
assert.equal(byBucket['opus|batched'].arm, 'batched');

// DISPATCHES counted by unique dispatchId, NOT by expanded rows.
assert.equal(byBucket['opus|batched'].cost.dispatches, 2, 'expanded rows were counted as separate dispatches, diluting the per-dispatch figure');
assert.equal(byBucket.opus.cost.dispatches, perFindingRows.length, 'the per-finding arm should count one dispatch per row');
assert.equal(byBucket['opus|batched'].cost.totalTokens, 1000, 'batched totalTokens must equal the one dispatch that reported usage');
assert.equal(byBucket['opus|batched'].cost.meanTokensPerDispatch, 500);
assert.equal(
  byBucket['opus|batched'].cost.meanTokensPerGradedFinding,
  Math.round((1000 / byBucket['opus|batched'].cost.gradedFindings) * 10) / 10,
  'meanTokensPerGradedFinding must divide by graded findings, not by dispatches'
);
assert.ok(byBucket['opus|batched'].cost.gradedFindings > byBucket['opus|batched'].cost.dispatches,
  'the batched arm must grade more findings than it makes dispatches, or the token argument is untested');

// FN and FP stay on their own denominators, per arm, and are never blended.
for (const b of [byBucket.opus, byBucket['opus|batched']]) {
  assert.equal(b.all.defectTrials + b.all.nonDefectTrials + b.all.ungraded, b.all.trials, 'ungraded leaked into a rate denominator');
  for (const f of ['trials', 'ungraded', 'defectTrials', 'nonDefectTrials', 'falseNegatives', 'falsePositives']) {
    assert.equal(b.authoritativeOnly[f] + b.judgementCallOnly[f], b.all[f], `${b.bucket}.${f}: authority split must partition all`);
  }
  assert.equal(typeof b.selfConsistency.replicatePairs, 'number');
}
// An OMITTED id is ungraded, never a false positive.
assert.ok(byBucket['opus|batched'].all.ungraded > 0, 'the omitted and crashed ids did not land in the ungraded bucket');

// --- anchoring, over qualifying groups only ----------------------------
const anchoring = scoreAnchoring(plan.groups, { 'per-finding': perFindingRows, batched: expanded.rows }, { minGroupSize: power.minGroupSize });
assert.equal(anchoring.minGroupSize, power.minGroupSize);
assert.equal(anchoring.qualifyingGroups, plan.groups.length, 'the anchoring denominators are not visible');
assert.equal(anchoring.arms.length, 2, 'anchoring must report BOTH arms over the same group set');
for (const a of anchoring.arms) {
  assert.equal(typeof a.allSameVerdictShare === 'number' || a.allSameVerdictShare === null, true);
  assert.ok(a.refutationRateByPosition.byPosition.every((p) => p.position >= 1));
  assert.equal(typeof a.refutationRateByPosition.risesAfterFirst, 'boolean');
}
// A size-1 group can exhibit no anchoring and must be excluded entirely.
const singletonOnly = scoreAnchoring([{ key: 'k', size: 1, ids: ['x'] }], { batched: [] }, { minGroupSize: 3 });
assert.equal(singletonOnly.qualifyingGroups, 0, 'a size-1 group reached the anchoring measurement');

// --- still no blended accuracy, including inside the anchoring block ---
const withAnchoring = scoreTrials(items, perFindingRows.concat(expanded.rows), { baselineTier: 'opus', anchoring });
assert.deepEqual(findBlendedAccuracyKeys(withAnchoring), [], 'a blended-accuracy key entered the report');
assert.deepEqual(findBlendedAccuracyKeys(JSON.parse(formatReport(withAnchoring, 'json'))), []);
const text = formatReport(withAnchoring, 'text');
assert.ok(/## ANCHORING/.test(text), 'the ANCHORING block is not rendered');
assert.ok(/TOKENS PER GRADED FINDING/.test(text), 'the tokens-per-graded-finding table is not rendered for a two-arm report');
for (const line of text.split('\n')) {
  assert.ok(!/(accuracy|overall correct|combined rate)[^.\n]*\d/i.test(line), `the text report prints a blended figure: ${line}`);
}

console.log('batched scoring: unknown ids dropped, omissions ungraded, crash ungraded, cost on the first row, arms bucketed independently, anchoring over qualifying groups only');
EOF
node "$TMP/check-batch-scoring.mjs" "$REPO_ROOT/$MODULE" "$REPO_ROOT/$REVIEW_LIB" "$CORPUS" >"$TMP/batchscoring.txt" ||
    fail "batched scoring check failed"
[ -s "$TMP/batchscoring.txt" ] && pass "$(cat "$TMP/batchscoring.txt")"

# ---------------------------------------------------------------------------
say "6. No blended accuracy anywhere — the mechanical form of 'never averaged'"

cat >"$TMP/check-blended.mjs" <<'EOF'
import fs from 'node:fs';
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';
const { loadCorpus, scoreTrials, formatReport, findBlendedAccuracyKeys } = await import(pathToFileURL(process.argv[2]).href);

const { items } = loadCorpus(fs.readFileSync(process.argv[3], 'utf8'));
const sample = JSON.parse(fs.readFileSync(process.argv[4], 'utf8'));
const r = scoreTrials(items, sample.trials, { baselineTier: sample.baselineTier });

// Recursive: no key matching /accuracy|overallCorrect|combinedRate/i at ANY depth.
const bad = findBlendedAccuracyKeys(r);
assert.deepEqual(bad, [], `the report carries blended-accuracy key(s): ${bad.join(', ')}`);
// And the round-tripped JSON too, in case a renderer adds one.
assert.deepEqual(findBlendedAccuracyKeys(JSON.parse(formatReport(r, 'json'))), []);
// The text render must not print a bare blended percentage either.
// The caveat sentence explaining WHY there is no accuracy number is allowed;
// an actual blended FIGURE is not. Reject accuracy/overall-correct/combined
// wording that sits next to a number.
const text = formatReport(r, 'text');
for (const line of text.split('\n')) {
  assert.ok(!/(accuracy|overall correct|combined rate)[^.\n]*\d/i.test(line),
    `the text report prints a blended figure: ${line}`);
}
console.log('no blended accuracy field or figure anywhere in the report');
EOF
node "$TMP/check-blended.mjs" "$REPO_ROOT/$MODULE" "$CORPUS" "$TRIALS" >"$TMP/blended.txt" ||
    fail "blended-accuracy negative assertion failed"
[ -s "$TMP/blended.txt" ] && pass "$(cat "$TMP/blended.txt")"

# ---------------------------------------------------------------------------
say "7. --dry-run dispatches NOTHING; --dispatch-stub drives the full path"

cat >"$TMP/exploding-stub.mjs" <<'EOF'
// If this is ever called, the run dispatched when it promised not to.
export function dispatch() {
  throw new Error('DISPATCHED-UNDER-DRY-RUN');
}
EOF
node "$RUNNER" --tiers opus,sonnet --replicates 2 --limit 3 --dry-run \
    --dispatch-stub "$TMP/exploding-stub.mjs" >"$TMP/dry.txt" 2>&1 ||
    fail "--dry-run exited non-zero"
grep -q 'Nothing dispatched' "$TMP/dry.txt" || fail "--dry-run did not report that it dispatched nothing"
grep -q 'DISPATCHED-UNDER-DRY-RUN' "$TMP/dry.txt" && fail "--dry-run CALLED the dispatcher"
grep -qE '= 12 trial\(s\)' "$TMP/dry.txt" || fail "--dry-run did not plan 3 items x 2 tiers x 2 replicates = 12 trials"
pass "--dry-run plans 12 trials and provably calls no dispatcher"

cat >"$TMP/fake-stub.mjs" <<'EOF'
// Deterministic fake: opus refutes, sonnet keeps. No clock, no RNG, no network.
export function dispatch(trial, prompt) {
  if (typeof prompt !== 'string' || prompt.length === 0) throw new Error('dispatcher got no prompt');
  const refuted = trial.tier === 'opus';
  return {
    verdict: { refuted, confidence: 90, rationale: 'fake' },
    usage: { output: 10, uncachedInput: 20, cacheWrite: 5, cacheRead: 100 },
    toolCalls: trial.tier === 'opus' ? 15 : 8,
  };
}
EOF
node "$RUNNER" --tiers opus,sonnet --replicates 2 --limit 4 --label stubbed \
    --dispatch-stub "$TMP/fake-stub.mjs" --out "$TMP/stub-results.json" --format json \
    >"$TMP/stub.txt" 2>"$TMP/stub.err" || fail "--dispatch-stub run exited non-zero"
node -e '
const r = require(process.argv[1]);
const assert = require("node:assert/strict");
assert.equal(r.trials.length, 16, "expected 16 trials");
assert.equal(r.label, "stubbed");
assert.ok(r.corpusSha256 && /^[0-9a-f]{64}$/.test(r.corpusSha256), "no corpus sha recorded");
assert.equal(r.baselineTier, "opus");
const opus = r.report.tiers.find(t => t.tier === "opus");
assert.equal(opus.cost.meanToolCallsPerTrial, 15, "tool calls not threaded through the stub path");
console.log("stub run produced " + r.trials.length + " scored trials with cost columns");
' "$TMP/stub-results.json" >"$TMP/stubchk.txt" || fail "stubbed run produced an unusable results file"
[ -s "$TMP/stubchk.txt" ] && pass "$(cat "$TMP/stubchk.txt")"

# --score-only must re-score a saved file without dispatching.
node "$RUNNER" --score-only "$TMP/stub-results.json" --format text >"$TMP/scoreonly.txt" 2>&1 ||
    fail "--score-only exited non-zero"
grep -q 'FALSE NEGATIVES' "$TMP/scoreonly.txt" || fail "--score-only did not render a report"
pass "--score-only re-scores a saved results file with no dispatch"

# An illegal tier must fail AT PLAN TIME, before any dispatch.
if node "$RUNNER" --tiers not-a-real-tier --dry-run >"$TMP/badtier.txt" 2>&1; then
    fail "an unknown --tiers value was accepted; a whole run would return null verdicts"
fi
grep -q 'not a tier alias' "$TMP/badtier.txt" || fail "the unknown-tier error is not actionable"
pass "an unknown --tiers value fails at plan time with an actionable error"

# --shape batched must refuse an underpowered corpus AT PLAN TIME, before any
# dispatch — the prohibition is mechanical, not advisory prose.
if node "$RUNNER" --shape batched --tiers opus --dry-run \
    --dispatch-stub "$TMP/exploding-stub.mjs" >"$TMP/shape-underpowered.txt" 2>&1; then
    fail "--shape batched built an arm from the underpowered committed corpus without --allow-underpowered"
fi
grep -q 'UNDERPOWERED' "$TMP/shape-underpowered.txt" || fail "the underpowered refusal is not actionable"
grep -q 'DISPATCHED-UNDER-DRY-RUN' "$TMP/shape-underpowered.txt" && fail "the underpowered refusal still dispatched"
pass "--shape batched refuses an underpowered corpus at plan time, before any dispatch"

# --shape both, driven end to end through injected dispatchers: two arms, one
# report, over the SAME item set, with the NO MEASUREMENT banner in place.
cat >"$TMP/two-arm-stub.mjs" <<'EOF'
// Deterministic fakes: the per-finding arm keeps everything; the batched arm
// refutes the first id of each batch and OMITS the last, so the expansion and
// omission paths are both exercised. No clock, no RNG, no network.
export function dispatch(trial, prompt) {
  if (typeof prompt !== 'string' || prompt.length === 0) throw new Error('per-finding dispatcher got no prompt');
  return {
    verdict: { refuted: false, confidence: 88, rationale: 'fake' },
    usage: { output: 10, uncachedInput: 20, cacheWrite: 5, cacheRead: 100 },
    toolCalls: 9,
  };
}
export function dispatchBatch(trial, prompt) {
  if (typeof prompt !== 'string' || prompt.length === 0) throw new Error('batched dispatcher got no prompt');
  for (const id of trial.corpusIds) {
    if (!prompt.includes(id)) throw new Error('the batched prompt omits refute_id ' + id);
  }
  const graded = trial.corpusIds.slice(0, -1);
  return {
    verdicts: graded.map((id, i) => ({ id, refuted: i === 0, confidence: 88, rationale: 'fake' })),
    unknownVerdictIds: [],
    usage: { output: 30, uncachedInput: 60, cacheWrite: 15, cacheRead: 300 },
    toolCalls: 12,
  };
}
EOF
node "$RUNNER" --shape both --tiers opus --replicates 2 --allow-underpowered --label two-arm \
    --dispatch-stub "$TMP/two-arm-stub.mjs" --out "$TMP/two-arm.json" --format text \
    >"$TMP/two-arm.txt" 2>"$TMP/two-arm.err" || {
    cat "$TMP/two-arm.err" >&2
    fail "--shape both exited non-zero under injected dispatchers"
}
grep -q 'NO MEASUREMENT — batched arm was underpowered' "$TMP/two-arm.txt" ||
    fail "--allow-underpowered did not force the NO MEASUREMENT banner"
grep -qE '^DECISION:' "$TMP/two-arm.txt" && fail "a no-measurement report printed a decision line"
node -e '
const fs = require("fs"), assert = require("node:assert/strict");
const r = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
assert.equal(r.shape, "both");
assert.equal(r.noMeasurement, true, "the results payload was not stamped noMeasurement");
const buckets = Object.fromEntries(r.report.tiers.map((t) => [t.bucket, t]));
assert.ok(buckets.opus, "no per-finding arm bucket");
assert.ok(buckets["opus|batched"], "no batched arm bucket");
// The per-finding arm is scored over EXACTLY the items the batched arm covers.
const perFindingIds = new Set(r.trials.filter((t) => !t.arm).map((t) => t.corpusId));
const batchedIds = new Set(r.trials.filter((t) => t.arm === "batched").map((t) => t.corpusId));
assert.deepEqual([...perFindingIds].sort(), [...batchedIds].sort(),
  "the two arms were scored over different item sets — they are not comparable");
assert.ok(batchedIds.size >= 3, "the batched arm covered fewer than the minimum group size");
// Dispatches, not rows.
assert.ok(buckets["opus|batched"].cost.dispatches < buckets["opus|batched"].cost.gradedFindings + buckets["opus|batched"].historicalOnlyTrials,
  "the batched arm made as many dispatches as it graded findings — the shape did not batch");
// The deliberately omitted id is ungraded, never a false positive.
assert.ok(buckets["opus|batched"].all.ungraded > 0, "the omitted id did not land in the ungraded bucket");
assert.ok(r.omittedVerdictIds.length > 0, "omitted ids were not recorded");
// Anchoring is present and scoped to qualifying groups.
assert.ok(r.report.anchoring && r.report.anchoring.arms.length === 2, "the anchoring block is missing an arm");
assert.equal(r.report.anchoring.minGroupSize, 3);
' "$TMP/two-arm.json" || fail "--shape both produced an unusable two-arm results file"
pass "--shape both drives two arms over the same item set into one report, stamped NO MEASUREMENT with no decision line"

# ---------------------------------------------------------------------------
say "7b. The REAL paid-dispatch parsing path, driven with zero spend"

# Sections 1-7 drive the harness through --dry-run and --dispatch-stub, which
# BYPASS claudeDispatch/parseClaudeResult entirely. But those are the branches
# that produced the verdicts and the token/tool-call figures behind this phase's
# DECISION (computed from a real, non-stubbed two-tier run). A regression in
# them — picking the FIRST StructuredOutput instead of the last, coercing a
# non-boolean `refuted` to false, miscounting tool_use blocks — would silently
# corrupt the FN/FP rates with nothing to catch it. parseClaudeResult is a pure
# function of a response body, so it is unit-testable with no subprocess at all;
# claudeDispatch is driven against PATH-shadowed fake `claude` binaries so the
# spawn/exit/non-JSON/ENOENT branches are covered without a single paid dispatch.

# Fake `claude` binaries. PATH is REPLACED (not prepended) for each case, so the
# real `claude` can never be reached even if it is installed on this machine.
mkdir -p "$TMP/bin-ok" "$TMP/bin-fail" "$TMP/bin-garbage" "$TMP/bin-empty"

cat >"$TMP/bin-ok/claude" <<'EOF'
#!/bin/sh
# Echo argv and the stdin byte count back through the verdict so the test can
# prove the prompt really reached the process and --model carried the tier.
n=$(wc -c | tr -d ' ')
printf '{"session_id":"sess-fake","usage":{"output_tokens":7,"input_tokens":8,"cache_creation_input_tokens":9,"cache_read_input_tokens":10},"messages":[{"message":{"content":[{"type":"tool_use","name":"StructuredOutput","input":{"refuted":true,"confidence":91,"rationale":"argv=%s stdin=%s"}}]}}]}\n' "$*" "$n"
EOF

cat >"$TMP/bin-fail/claude" <<'EOF'
#!/bin/sh
echo "Invalid API key - please run /login" >&2
exit 3
EOF

cat >"$TMP/bin-garbage/claude" <<'EOF'
#!/bin/sh
echo "Rate limit reached. Try again later."
EOF

chmod +x "$TMP/bin-ok/claude" "$TMP/bin-fail/claude" "$TMP/bin-garbage/claude"

# bin-ok's fake needs `wc`/`tr`, but PATH is replaced wholesale rather than
# prepended so a real `claude` stays unreachable. Symlink just those two in.
for u in wc tr; do
    ln -sf "$(command -v "$u")" "$TMP/bin-ok/$u" || die "could not stage $u for the fake claude"
done

cat >"$TMP/check-dispatch.mjs" <<'EOF'
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const [, , runnerPath, modulePath, tmp] = process.argv;
const {
  parseClaudeResult,
  tryParseEmbeddedJson,
  countSessionToolUses,
  projectSlugFor,
  claudeDispatch,
} = await import(pathToFileURL(runnerPath).href);
const { TOKEN_CLASSES } = await import(pathToFileURL(modulePath).href);

const structured = (refuted, confidence, rationale) => ({
  type: 'tool_use',
  name: 'StructuredOutput',
  input: { refuted, confidence, rationale },
});
const turn = (...blocks) => ({ message: { content: blocks } });

// --- A. The canonical shape: a StructuredOutput tool_use + a usage block ----
{
  const r = parseClaudeResult({
    usage: {
      output_tokens: 111,
      input_tokens: 222,
      cache_creation_input_tokens: 333,
      cache_read_input_tokens: 444,
    },
    messages: [
      turn({ type: 'text', text: 'reading the cited file' }, { type: 'tool_use', name: 'Read', input: {} }),
      turn({ type: 'tool_use', name: 'Grep', input: {} }),
      turn(structured(true, 88, 'governed by a documented exception')),
    ],
  });
  assert.equal(r.verdict.refuted, true, 'A: refuted');
  assert.equal(r.verdict.confidence, 88, 'A: confidence');
  assert.equal(r.verdict.rationale, 'governed by a documented exception', 'A: rationale');
  assert.equal(r.error, null, 'A: a graded trial must record no error');
  assert.equal(r.usage.output, 111, 'A: output tokens');
  assert.equal(r.usage.uncachedInput, 222, 'A: uncached input tokens');
  assert.equal(r.usage.cacheWrite, 333, 'A: cache-write tokens');
  assert.equal(r.usage.cacheRead, 444, 'A: cache-read tokens');
  // The tool-call column counts EVERY tool_use, not just the verdict block —
  // 13-20 calls for Opus vs 7-9 for Sonnet is the divergence signal.
  assert.equal(r.toolCalls, 3, 'A: every tool_use block must be counted');
}

// --- B. Several StructuredOutput blocks: the LAST one must win -------------
// A refuter that revises its verdict emits more than one. Picking the first
// would record a stale answer and silently corrupt the FN/FP rates.
{
  const r = parseClaudeResult({
    messages: [
      turn(structured(false, 10, 'first pass — keep it')),
      turn({ type: 'tool_use', name: 'Read', input: {} }),
      turn(structured(true, 99, 'on reflection, refuted')),
    ],
  });
  assert.equal(r.verdict.refuted, true, 'B: the LAST StructuredOutput must win');
  assert.equal(r.verdict.confidence, 99, 'B: the last block s confidence must win');
  assert.equal(r.toolCalls, 3, 'B: tool-call count');
}

// --- C. No tool_use at all: `result` is a bare JSON string ------------------
{
  const r = parseClaudeResult({
    usage: { output_tokens: 5 },
    result: '{"refuted": false, "confidence": 65, "rationale": "the defect is real"}',
  });
  assert.equal(r.verdict.refuted, false, 'C: bare-JSON result must parse');
  assert.equal(r.verdict.confidence, 65, 'C: confidence');
  assert.equal(r.toolCalls, 0, 'C: no tool_use blocks means no tool calls');
}

// --- D. `result` wraps JSON in prose and a fence: tryParseEmbeddedJson ------
{
  const r = parseClaudeResult({
    result: 'Here is my verdict:\n\n```json\n{"refuted": true, "confidence": 70, "rationale": "the cited symbol does not exist"}\n```\nDone.',
  });
  assert.equal(r.verdict.refuted, true, 'D: fenced/prose-wrapped JSON must still parse');
  assert.equal(r.verdict.rationale, 'the cited symbol does not exist', 'D: rationale');
}

// --- E. Neither shape present: ungraded, NEVER coerced to false -------------
// Coercing a missing verdict to `refuted: false` would count it as a KEPT
// finding and silently inflate the false-positive rate.
{
  const r = parseClaudeResult({ result: 'I was unable to reach a conclusion.', usage: {} });
  assert.equal(r.verdict, null, 'E: an unparseable answer must be ungraded');
  assert.match(r.error, /no boolean `refuted`/, 'E: the error must name the missing field');
}

// --- F. A well-formed response with a NON-BOOLEAN `refuted` is ungraded -----
{
  for (const bad of ['true', 1, null, undefined, {}]) {
    const viaResult = parseClaudeResult({ result: JSON.stringify({ refuted: bad, confidence: 90 }) });
    assert.equal(viaResult.verdict, null, `F: result-shaped refuted=${JSON.stringify(bad)} must be ungraded`);
    const viaTool = parseClaudeResult({ messages: [turn(structured(bad, 90, 'x'))] });
    assert.equal(viaTool.verdict, null, `F: tool-shaped refuted=${JSON.stringify(bad)} must be ungraded`);
  }
}

// --- G. A missing `usage` object must zero all four classes, never throw ----
{
  const r = parseClaudeResult({ result: '{"refuted":true}' });
  for (const c of TOKEN_CLASSES) {
    assert.equal(r.usage[c], 0, `G: ${c} must default to 0 when usage is absent`);
  }
  assert.equal(r.verdict.confidence, null, 'G: a missing confidence stays null, not 0');
  assert.equal(r.verdict.rationale, null, 'G: a missing rationale stays null');
  // A completely empty body must degrade, not throw.
  const empty = parseClaudeResult({});
  assert.equal(empty.verdict, null, 'G: an empty body is ungraded');
  assert.equal(empty.toolCalls, 0, 'G: an empty body has no tool calls');
}

// --- H. num_tool_uses is the FALLBACK, never an override -------------------
{
  const fallback = parseClaudeResult({ result: '{"refuted":true}', num_tool_uses: 13 });
  assert.equal(fallback.toolCalls, 13, 'H: num_tool_uses must fill in when no blocks were seen');
  const observed = parseClaudeResult({
    messages: [turn({ type: 'tool_use', name: 'Read', input: {} }, structured(true, 50, 'x'))],
    num_tool_uses: 999,
  });
  assert.equal(observed.toolCalls, 2, 'H: an observed block count must not be overwritten by num_tool_uses');
}

// --- I. tryParseEmbeddedJson is brace-balanced and string-aware ------------
{
  assert.equal(tryParseEmbeddedJson('no braces here'), null, 'I: no brace');
  assert.equal(tryParseEmbeddedJson('{"refuted": true'), null, 'I: unbalanced');
  assert.equal(tryParseEmbeddedJson(42), null, 'I: non-string');
  // Braces INSIDE a string must not close the span early.
  const s = tryParseEmbeddedJson('prefix {"rationale":"a } and a {","refuted":true} suffix');
  assert.equal(s.refuted, true, 'I: braces inside a JSON string must not terminate the span');
  // Nested objects must be spanned whole.
  const n = tryParseEmbeddedJson('x {"refuted":false,"meta":{"a":{"b":1}}} y');
  assert.equal(n.refuted, false, 'I: nested objects');
}

// --- J. projectSlugFor mirrors Claude Code's project-directory naming ------
{
  assert.equal(projectSlugFor('/Users/edward/Projects/rdm'), '-Users-edward-Projects-rdm', 'J: plain path');
  assert.equal(
    projectSlugFor('/Users/edward/Projects/rdm__worktrees/roadmap-x'),
    '-Users-edward-Projects-rdm--worktrees-roadmap-x',
    'J: worktree path'
  );
}

// --- K. countSessionToolUses reads a real transcript ------------------------
{
  const root = path.join(tmp, 'projects-fixture');
  const cwd = '/Users/edward/Projects/rdm';
  const dir = path.join(root, projectSlugFor(cwd));
  fs.mkdirSync(dir, { recursive: true });
  const lines = [
    JSON.stringify({ type: 'user', message: { content: 'the refute prompt' } }),
    // An assistant turn interleaving text and tool_use — only the tool_use counts.
    JSON.stringify({
      type: 'assistant',
      message: { content: [{ type: 'text', text: 'looking' }, { type: 'tool_use', name: 'Read' }, { type: 'tool_use', name: 'Grep' }] },
    }),
    // A USER turn carrying tool_result blocks must NOT be counted as tool calls.
    JSON.stringify({ type: 'user', message: { content: [{ type: 'tool_result' }, { type: 'tool_use', name: 'NotACall' }] } }),
    '',
    'this line is not JSON at all',
    // An assistant turn whose content is a bare string must be skipped, not throw.
    JSON.stringify({ type: 'assistant', message: { content: 'plain text answer' } }),
    JSON.stringify({ type: 'assistant', message: { content: [{ type: 'tool_use', name: 'StructuredOutput' }] } }),
  ];
  fs.writeFileSync(path.join(dir, 'sess-abc.jsonl'), lines.join('\n') + '\n');
  assert.equal(countSessionToolUses('sess-abc', cwd, root), 3, 'K: only assistant tool_use blocks count');
  assert.equal(countSessionToolUses('sess-missing', cwd, root), null, 'K: a missing transcript returns null, not 0');
  assert.equal(countSessionToolUses('sess-abc', '/nope', root), null, 'K: an unknown project slug returns null');
}

// --- L. claudeDispatch against PATH-SHADOWED fakes (zero paid dispatch) ----
{
  const trial = { trialId: 't1', corpusId: 'c1', tier: 'sonnet', replicate: 0 };
  const prompt = 'X'.repeat(500);

  process.env.PATH = path.join(tmp, 'bin-ok');
  const ok = await claudeDispatch(trial, prompt, { cwd: tmp });
  assert.equal(ok.verdict.refuted, true, 'L: a well-formed response parses');
  assert.equal(ok.usage.cacheRead, 10, 'L: usage threads through the spawn path');
  assert.equal(ok.toolCalls, 1, 'L: tool calls thread through the spawn path');
  assert.match(ok.verdict.rationale, /--model sonnet/, 'L: the tier must be passed as --model');
  assert.match(ok.verdict.rationale, /--output-format json/, 'L: --output-format json must be requested');
  assert.match(ok.verdict.rationale, new RegExp('stdin=' + prompt.length + '\\b'), 'L: the prompt must arrive on stdin');

  process.env.PATH = path.join(tmp, 'bin-fail');
  const failed = await claudeDispatch(trial, prompt, { cwd: tmp });
  assert.equal(failed.verdict, null, 'L: a non-zero exit is ungraded, not refuted:false');
  assert.match(failed.error, /exited 3/, 'L: the exit status must be reported');
  assert.match(failed.error, /Invalid API key/, 'L: stderr must be surfaced actionably');

  process.env.PATH = path.join(tmp, 'bin-garbage');
  const garbage = await claudeDispatch(trial, prompt, { cwd: tmp });
  assert.equal(garbage.verdict, null, 'L: a non-JSON body is ungraded');
  assert.match(garbage.error, /non-JSON body/, 'L: a non-JSON body must be named as such');

  // A MISSING binary is a setup error, not a per-trial failure: it must THROW
  // so a long run aborts immediately instead of returning N null verdicts.
  process.env.PATH = path.join(tmp, 'bin-empty');
  let threw = null;
  try {
    await claudeDispatch(trial, prompt, { cwd: tmp });
  } catch (err) {
    threw = err;
  }
  assert.ok(threw, 'L: a missing `claude` binary must throw, not return a null verdict');
  assert.match(threw.message, /`claude` binary was not found/, 'L: the ENOENT error must name the binary');
  assert.match(threw.message, /--dry-run|--dispatch-stub/, 'L: the ENOENT error must name the no-spend escape hatches');
}

console.log('real-dispatch parsing: A-L all pass with zero paid dispatches');
EOF

node "$TMP/check-dispatch.mjs" "$REPO_ROOT/$RUNNER" "$REPO_ROOT/$MODULE" "$TMP" >"$TMP/dispatch.txt" 2>&1 ||
    {
        cat "$TMP/dispatch.txt" >&2
        fail "the real-dispatch parsing path failed its unit checks"
    }
[ -s "$TMP/dispatch.txt" ] && pass "$(tail -1 "$TMP/dispatch.txt")"

# 7c. parseClaudeBatchResult — the batched sibling of the parser above, and the
# branch that would decide a batched A/B's verdicts. Pure function of a response
# body, so it needs no subprocess and no spend.
cat >"$TMP/check-batch-parse.mjs" <<'EOF'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';
const { parseClaudeBatchResult } = await import(pathToFileURL(process.argv[2]).href);

const usage = { output_tokens: 7, input_tokens: 8, cache_creation_input_tokens: 9, cache_read_input_tokens: 10 };
const so = (verdicts) => ({
  usage,
  messages: [{ message: { content: [{ type: 'tool_use', name: 'StructuredOutput', input: { verdicts } }] } }],
});

// A. Happy path: token classes mapped, tool calls counted, verdicts normalized.
const a = parseClaudeBatchResult(so([{ id: 'x', refuted: true, confidence: 90, rationale: 'r' }]), ['x', 'y']);
assert.deepEqual(a.usage, { output: 7, uncachedInput: 8, cacheWrite: 9, cacheRead: 10 });
assert.equal(a.toolCalls, 1);
assert.deepEqual(a.verdicts, [{ id: 'x', refuted: true, confidence: 90, rationale: 'r' }]);
assert.deepEqual(a.unknownVerdictIds, []);
assert.equal(a.error, null);

// B. An UNKNOWN id is dropped and recorded — never applied to any finding.
const b = parseClaudeBatchResult(so([{ id: 'zz', refuted: true, confidence: 90 }]), ['x']);
assert.deepEqual(b.verdicts, [], 'a verdict for an id outside the dispatch reached the output');
assert.deepEqual(b.unknownVerdictIds, ['zz']);

// C. A NON-BOOLEAN refuted stays ungraded rather than being coerced to false,
// which would silently inflate the false-positive rate.
const c = parseClaudeBatchResult(so([{ id: 'x', refuted: 'yes', confidence: 90 }]), ['x']);
assert.equal(c.verdicts[0].refuted, null, 'a non-boolean `refuted` was coerced');

// D. NO verdicts array at all is a CRASH, not "every finding omitted".
for (const body of [{ usage, messages: [] }, { usage, result: 'I could not decide.' }, {}]) {
  const d = parseClaudeBatchResult(body, ['x']);
  assert.equal(d.verdicts, null, 'a response with no verdicts array was not treated as a crash');
  assert.ok(d.error, 'the crash carries no error string');
}

// E. The `result`-string shape, bare and prose-wrapped.
const e1 = parseClaudeBatchResult({ usage, result: JSON.stringify({ verdicts: [{ id: 'x', refuted: false, confidence: 50 }] }) }, ['x']);
assert.equal(e1.verdicts[0].refuted, false);
const e2 = parseClaudeBatchResult({ usage, result: 'Here you go:\n```json\n{"verdicts":[{"id":"x","refuted":true,"confidence":60}]}\n```' }, ['x']);
assert.equal(e2.verdicts[0].refuted, true, 'a fenced verdicts array did not parse');

// F. num_tool_uses fallback, matching the single-finding parser.
assert.equal(parseClaudeBatchResult({ usage, num_tool_uses: 5, result: '{"verdicts":[]}' }, ['x']).toolCalls, 5);

console.log('parseClaudeBatchResult: unknown ids dropped, non-boolean verdicts left ungraded, a missing verdicts array read as a crash, both result-string shapes parsed');
EOF
node "$TMP/check-batch-parse.mjs" "$REPO_ROOT/$RUNNER" >"$TMP/batchparse.txt" 2>&1 || {
    cat "$TMP/batchparse.txt" >&2
    fail "the batched paid-dispatch parsing path failed its unit checks"
}
[ -s "$TMP/batchparse.txt" ] && pass "$(tail -1 "$TMP/batchparse.txt")"

# ---------------------------------------------------------------------------
say "8. --audit: corpus-free arithmetic audit of the committed figures"

node "$RUNNER" --audit "$BASELINE_JSON" >"$TMP/audit.txt" 2>&1 || {
    cat "$TMP/audit.txt" >&2
    fail "--audit failed against $BASELINE_JSON"
}
grep -q 'OK' "$TMP/audit.txt" && pass "$BASELINE_JSON § refuterModelTiering is internally consistent"

# Self-test: a planted off-by-one in a COPY must make --audit fail.
node -e '
const fs = require("fs");
const j = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
j.refuterModelTiering.tiers[0].cost.totalTokens += 1;
fs.writeFileSync(process.argv[2], JSON.stringify(j, null, 2));
' "$BASELINE_JSON" "$TMP/baseline-mutant.json"
if node "$RUNNER" --audit "$TMP/baseline-mutant.json" >/dev/null 2>&1; then
    fail "--audit missed a planted off-by-one in cost.totalTokens"
fi
pass "--audit fires on a planted off-by-one (the gate is not vacuous)"

# ---------------------------------------------------------------------------
say "8b. --audit-section refuterBatching: the corpus-power arithmetic"

node "$RUNNER" --audit "$BASELINE_JSON" --audit-section refuterBatching >"$TMP/audit-batch.txt" 2>&1 || {
    cat "$TMP/audit-batch.txt" >&2
    fail "--audit-section refuterBatching failed against $BASELINE_JSON"
}
grep -q 'OK' "$TMP/audit-batch.txt" && pass "$BASELINE_JSON § refuterBatching is internally consistent"

# The audit must fire on each way the power figures can be made to lie.
batch_audit_mutation() {
    local label="$1"
    local mutate="$2"
    node -e "
const fs = require('fs');
const j = JSON.parse(fs.readFileSync(process.argv[1], 'utf8'));
const s = j.refuterBatching;
$mutate
fs.writeFileSync(process.argv[2], JSON.stringify(j, null, 2));
" "$BASELINE_JSON" "$TMP/batch-mutant.json"
    if node "$RUNNER" --audit "$TMP/batch-mutant.json" --audit-section refuterBatching >/dev/null 2>&1; then
        fail "--audit-section refuterBatching missed: $label"
    else
        pass "--audit-section refuterBatching fires on: $label"
    fi
}
batch_audit_mutation "an exclusion count that no longer accounts for the corpus" \
    's.corpusPower.unrecoverableUnitExcluded += 1;'
batch_audit_mutation "a qualifying count that disagrees with the size histogram" \
    's.corpusPower.qualifyingGroups += 1;'
batch_audit_mutation "folding a size-1 group into the qualifying items" \
    's.corpusPower.qualifyingItems += 24;'
batch_audit_mutation "a meetsMinimum flag that contradicts the derived population" \
    's.corpusPower.meetsMinimum = true;'
batch_audit_mutation "an underpowered arm carrying a real decision token" \
    "s.decision = 'ship-batched';"
batch_audit_mutation "a decision token outside the closed set" \
    "s.decision = 'looks-fine';"
batch_audit_mutation "a per-mode histogram that no longer partitions the overall one" \
    "s.corpusPower.sizeHistogramByMode.plan['3'] = 1;"

# ---------------------------------------------------------------------------
say "9. Planted-mutation self-tests (sections 2, 3, 4, 4b, 5, 6, 7b are not vacuous)"

# 9a. Relabelling the divergence-class items must fail section 2's share floor.
node -e '
const fs = require("fs");
const out = fs.readFileSync(process.argv[1], "utf8").split("\n").filter(Boolean).map((l) => {
  const i = JSON.parse(l);
  if (i.groundTruth.class === "mechanically-true-not-a-defect") i.groundTruth.class = "style-preference";
  return JSON.stringify(i);
});
fs.writeFileSync(process.argv[2], out.join("\n") + "\n");
' "$CORPUS" "$TMP/corpus-relabelled.jsonl"
if node "$TMP/check-corpus.mjs" "$REPO_ROOT/$MODULE" "$TMP/corpus-relabelled.jsonl" >/dev/null 2>&1; then
    fail "section 2 missed a corpus whose divergence class was relabelled away"
fi
pass "section 2 fires when the divergence class is relabelled away"

# 9b. A bogus authority value must fail validation.
node -e '
const fs = require("fs");
const lines = fs.readFileSync(process.argv[1], "utf8").split("\n").filter(Boolean);
const i = JSON.parse(lines[0]);
i.groundTruth.authority = "probably";
lines[0] = JSON.stringify(i);
fs.writeFileSync(process.argv[2], lines.join("\n") + "\n");
' "$CORPUS" "$TMP/corpus-bad-authority.jsonl"
if node "$TMP/check-corpus.mjs" "$REPO_ROOT/$MODULE" "$TMP/corpus-bad-authority.jsonl" >/dev/null 2>&1; then
    fail "section 2 accepted an illegal groundTruth.authority"
fi
pass "section 2 fires on an illegal groundTruth.authority"

# 9c. Stripping the citation from an authoritative item's evidence must fail.
node -e '
const fs = require("fs");
const lines = fs.readFileSync(process.argv[1], "utf8").split("\n").filter(Boolean);
for (let n = 0; n < lines.length; n++) {
  const i = JSON.parse(lines[n]);
  if (i.groundTruth.authority !== "authoritative") continue;
  i.groundTruth.evidence = "it is obviously not a bug";
  lines[n] = JSON.stringify(i);
  break;
}
fs.writeFileSync(process.argv[2], lines.join("\n") + "\n");
' "$CORPUS" "$TMP/corpus-uncited.jsonl"
if node "$TMP/check-corpus.mjs" "$REPO_ROOT/$MODULE" "$TMP/corpus-uncited.jsonl" >/dev/null 2>&1; then
    fail "section 2 accepted an authoritative item whose evidence cites no artifact"
fi
pass "section 2 fires when an authoritative item's evidence cites no artifact"

# 9d. Mutating a recorded promptSha256 must be caught by section 3.
node -e '
const fs = require("fs");
const lines = fs.readFileSync(process.argv[1], "utf8").split("\n").filter(Boolean);
const i = JSON.parse(lines[0]);
i.promptSha256 = "0".repeat(64);
lines[0] = JSON.stringify(i);
fs.writeFileSync(process.argv[2], lines.join("\n") + "\n");
' "$CORPUS" "$TMP/corpus-bad-sha.jsonl"
if node "$TMP/check-prompts.mjs" "$REPO_ROOT/$MODULE" "$REPO_ROOT/$REVIEW_LIB" "$TMP/corpus-bad-sha.jsonl" >/dev/null 2>&1; then
    fail "section 3 missed an unrecorded prompt drift"
fi
pass "section 3 fires when a recorded promptSha256 no longer regenerates"

# 9e. Swapping the FN and FP denominators must be caught by section 5.
sed 's/set.defectTrials += 1;/set.nonDefectTrials += 1;/' "$MODULE" >"$TMP/module-swapped.mjs"
if ! diff -q "$MODULE" "$TMP/module-swapped.mjs" >/dev/null; then
    if node "$TMP/check-scorer.mjs" "$TMP/module-swapped.mjs" "$CORPUS" "$TRIALS" >/dev/null 2>&1; then
        fail "section 5 missed a swapped FN/FP denominator"
    fi
    pass "section 5 fires when the FN and FP denominators are swapped"
else
    fail "the FN/FP denominator mutation did not apply — the self-test is vacuous"
fi

# 9f. Adding a blended accuracy field must be caught by section 6.
sed 's/    caveats: CAVEATS,/    accuracy: 0.5,\n    caveats: CAVEATS,/' "$MODULE" >"$TMP/module-accuracy.mjs"
if ! diff -q "$MODULE" "$TMP/module-accuracy.mjs" >/dev/null; then
    if node "$TMP/check-blended.mjs" "$TMP/module-accuracy.mjs" "$CORPUS" "$TRIALS" >/dev/null 2>&1; then
        fail "section 6 missed a planted blended-accuracy field"
    fi
    pass "section 6 fires on a planted blended-accuracy field"
else
    fail "the blended-accuracy mutation did not apply — the self-test is vacuous"
fi

# 9g. Mutating a fixture's finding JSON must change the mined output.
mkdir -p "$TMP/sidecar-mutant"
cp -R "$SIDECARS/." "$TMP/sidecar-mutant/"
MUT_T="$TMP/sidecar-mutant/-Users-edward-Projects-rdm/sess-mine/subagents/workflows/wf_mine001/agent-refute-ok.jsonl"
sed 's/\\"severity\\": \\"blocking\\"/\\"severity\\": \\"concern\\"/' "$MUT_T" >"$MUT_T.new" && mv "$MUT_T.new" "$MUT_T"
node "$MINER" --root "$TMP/sidecar-mutant" --format json >"$TMP/mined-mutant.json" 2>/dev/null
MUT_SEV="$(node -e 'const m=require("'"$TMP"'/mined-mutant.json");const i=m.items.find(x=>x.id==="mined-wf_mine001-refute-ok");console.log(i?i.finding.severity:"MISSING")')"
[ "$MUT_SEV" = "concern" ] ||
    fail "mutating the fixture's finding JSON did not change the mined severity (got $MUT_SEV) — the miner may not be reading the transcript"
pass "mutating a fixture finding changes the mined output — the miner really reads the transcript"

# 9h-9j. Section 7b must fire on the three regressions it exists to catch.
# The mutants live beside a symlinked `lib/` so the runner's relative import of
# ./lib/refuter-agreement.mjs still resolves out of the scratch directory.
mkdir -p "$TMP/mutscripts"
# -n matters: without it, a re-link against an existing dir symlink would create
# the new link INSIDE the real scripts/lib directory.
ln -sfn "$REPO_ROOT/scripts/lib" "$TMP/mutscripts/lib"

run_dispatch_mutant() {
    node "$TMP/check-dispatch.mjs" "$1" "$REPO_ROOT/$MODULE" "$TMP" >/dev/null 2>&1
}

# 9h. Taking the FIRST StructuredOutput instead of the last records a stale verdict.
sed "s/if (block.name === 'StructuredOutput'/if (verdict === null \&\& block.name === 'StructuredOutput'/" \
    "$RUNNER" >"$TMP/mutscripts/runner-first-wins.mjs"
if diff -q "$RUNNER" "$TMP/mutscripts/runner-first-wins.mjs" >/dev/null; then
    fail "the first-StructuredOutput-wins mutation did not apply — the self-test is vacuous"
elif run_dispatch_mutant "$TMP/mutscripts/runner-first-wins.mjs"; then
    fail "section 7b missed a parser that takes the FIRST StructuredOutput instead of the last"
else
    pass "section 7b fires when the parser takes the first StructuredOutput instead of the last"
fi

# 9i. Coercing a non-boolean `refuted` silently inflates the false-positive rate.
sed "s/typeof block.input.refuted === 'boolean'/block.input.refuted !== undefined/" \
    "$RUNNER" >"$TMP/mutscripts/runner-coerce.mjs"
if diff -q "$RUNNER" "$TMP/mutscripts/runner-coerce.mjs" >/dev/null; then
    fail "the non-boolean-coercion mutation did not apply — the self-test is vacuous"
elif run_dispatch_mutant "$TMP/mutscripts/runner-coerce.mjs"; then
    fail "section 7b missed a parser that accepts a non-boolean \`refuted\`"
else
    pass "section 7b fires when a non-boolean \`refuted\` is accepted instead of bucketed ungraded"
fi

# 9j. Counting only the verdict block undercounts the tool-call cost column.
sed 's/^        toolCalls += 1;$/        if (block.name === "StructuredOutput") toolCalls += 1;/' \
    "$RUNNER" >"$TMP/mutscripts/runner-miscount.mjs"
if diff -q "$RUNNER" "$TMP/mutscripts/runner-miscount.mjs" >/dev/null; then
    fail "the tool-call miscount mutation did not apply — the self-test is vacuous"
elif run_dispatch_mutant "$TMP/mutscripts/runner-miscount.mjs"; then
    fail "section 7b missed a parser that miscounts tool_use blocks"
else
    pass "section 7b fires when tool_use blocks are miscounted"
fi

# 9k-9l. Sections 4 and 4b must fire on the miner regressions they exist to
# catch. The mutants reuse the scratch dir 9h-9j set up (which already carries
# the symlinked `lib/`) and additionally need `measure-refuter-severity.mjs`,
# whose brace-matched extractor the miner imports relatively.
ln -sfn "$REPO_ROOT/scripts/measure-refuter-severity.mjs" "$TMP/mutscripts/measure-refuter-severity.mjs"

# 9k. Dropping the unrecoverable-mode guard lets a mode-less prompt through as a
# record with `mode: null`, which would silently poison the corpus.
grep -v "reason: 'unrecoverable-mode'" "$MINER" >"$TMP/mutscripts/miner-no-mode-guard.mjs"
if diff -q "$MINER" "$TMP/mutscripts/miner-no-mode-guard.mjs" >/dev/null; then
    fail "the unrecoverable-mode-guard mutation did not apply — the self-test is vacuous"
else
    node "$TMP/mutscripts/miner-no-mode-guard.mjs" --root "$SIDECARS" --format json \
        >"$TMP/mined-nomodeguard.json" 2>/dev/null || true
    if node "$TMP/check-miner.mjs" "$TMP/mined-nomodeguard.json" >/dev/null 2>&1; then
        fail "section 4 missed a miner that emits records with an unrecoverable mode"
    else
        pass "section 4 fires when the unrecoverable-mode guard is dropped"
    fi
fi

# 9l. Making the --severity filter inert must be caught by section 4b.
sed 's/if (options.severities.indexOf(sev) === -1) {/if (false) {/' "$MINER" \
    >"$TMP/mutscripts/miner-inert-severity.mjs"
if diff -q "$MINER" "$TMP/mutscripts/miner-inert-severity.mjs" >/dev/null; then
    fail "the inert---severity mutation did not apply — the self-test is vacuous"
else
    node "$TMP/mutscripts/miner-inert-severity.mjs" --root "$SIDECARS" --severity blocking --format json \
        >"$TMP/mined-inertsev.json" 2>/dev/null || true
    if node "$TMP/check-miner-severity.mjs" "$TMP/mined-inertsev.json" >/dev/null 2>&1; then
        fail "section 4b missed an inert --severity filter"
    else
        pass "section 4b fires when the --severity filter is made inert"
    fi
fi

# 9m-9r. Sections 2c, 5c and 6 must fire on the batching regressions they exist
# to catch. Each mutation is applied to a COPY of the module, never the real one.
batching_mutation() {
    local label="$1"
    local checker="$2"
    local mutant="$3"
    shift 3
    if diff -q "$MODULE" "$mutant" >/dev/null; then
        fail "the $label mutation did not apply — the self-test is vacuous"
        return 0
    fi
    if node "$checker" "$mutant" "$@" >/dev/null 2>&1; then
        fail "the batching gate missed: $label"
    else
        pass "the batching gate fires on: $label"
    fi
}

# 9m. Dropping the UNIT IDENTITY from the grouping key — the exact regression
# section 2c exists for. It must collapse two review units into one group.
# (`@` delimiter: the key template is full of `|`.)
sed 's@const unitIdent = unitIdentOf(item);@const unitIdent = unitIdentOf(item) === null ? null : "";@' \
    "$MODULE" >"$TMP/module-run-scoped.mjs"
batching_mutation "a grouping key that has lost the review-unit identity" \
    "$TMP/check-batch-power.mjs" "$TMP/module-run-scoped.mjs" "$CORPUS" "$REPO_ROOT/$BASELINE_JSON"

# 9n. Letting a singleton-dominated population report SUFFICIENT.
sed 's/meetsMinimum: qualifyingGroups >= minQualifyingGroups && qualifyingItems >= minQualifyingItems,/meetsMinimum: true,/' \
    "$MODULE" >"$TMP/module-always-sufficient.mjs"
batching_mutation "a size-1-dominated population reporting SUFFICIENT" \
    "$TMP/check-batch-power.mjs" "$TMP/module-always-sufficient.mjs" "$CORPUS" "$REPO_ROOT/$BASELINE_JSON"

# 9o. Folding the excluded (constructed / unrecoverable-unit) items back into
# the grouped population instead of counting them out.
sed 's/if (item \&\& item.provenance \&\& item.provenance.kind === .constructed.) {/if (false) {/' \
    "$MODULE" >"$TMP/module-keeps-constructed.mjs"
batching_mutation "constructed items folded back into the grouped population" \
    "$TMP/check-batch-power.mjs" "$TMP/module-keeps-constructed.mjs" "$CORPUS" "$REPO_ROOT/$BASELINE_JSON"

# 9p. Coercing an OMITTED id to `refuted: false` — silently inflates the FP rate.
sed 's/      const verdict = byId.get(String(corpusId)) || null;/      const verdict = byId.get(String(corpusId)) || { refuted: false, confidence: null, rationale: null };/' \
    "$MODULE" >"$TMP/module-omit-as-kept.mjs"
batching_mutation "an omitted id coerced to refuted:false" \
    "$TMP/check-batch-scoring.mjs" "$TMP/module-omit-as-kept.mjs" "$REPO_ROOT/$REVIEW_LIB" "$CORPUS"

# 9q. Counting expanded ROWS as dispatches — understates the per-dispatch tokens
# that the whole batching token argument rests on.
sed 's/    b.dispatchIds.add(t.dispatchId === undefined || t.dispatchId === null ? t.trialId : t.dispatchId);/    b.dispatchIds.add(t.trialId);/' \
    "$MODULE" >"$TMP/module-row-dispatches.mjs"
batching_mutation "expanded rows counted as separate dispatches" \
    "$TMP/check-batch-scoring.mjs" "$TMP/module-row-dispatches.mjs" "$REPO_ROOT/$REVIEW_LIB" "$CORPUS"

# 9r. Blending FN and FP into one number inside the ANCHORING block — section 6's
# recursive key scan covers the whole report object, including that block.
sed 's@^        dispatchesConsidered: considered,@        combinedRate: 0.5,\
        dispatchesConsidered: considered,@' "$MODULE" >"$TMP/module-anchor-blended.mjs"
batching_mutation "a blended accuracy field inside the ANCHORING block" \
    "$TMP/check-batch-scoring.mjs" "$TMP/module-anchor-blended.mjs" "$REPO_ROOT/$REVIEW_LIB" "$CORPUS"

# 9s-9t. Section 7c must fire on the two ways the batched parser can lie.
# 9s. Accepting a verdict for an id the dispatch never contained MISATTRIBUTES
# it to whatever finding happens to share that position.
sed 's@    if (expected.size \&\& !expected.has(id)) {@    if (false) {@' \
    "$RUNNER" >"$TMP/mutscripts/runner-accepts-unknown.mjs"
if diff -q "$RUNNER" "$TMP/mutscripts/runner-accepts-unknown.mjs" >/dev/null; then
    fail "the unknown-id-acceptance mutation did not apply — the self-test is vacuous"
elif node "$TMP/check-batch-parse.mjs" "$TMP/mutscripts/runner-accepts-unknown.mjs" >/dev/null 2>&1; then
    fail "section 7c missed a batched parser that accepts a verdict for an unknown id"
else
    pass "section 7c fires when the batched parser accepts an unknown-id verdict"
fi

# 9t. Reading a missing `verdicts` array as an empty one turns a model
# misconfiguration into a silently clean grade.
sed 's@^    return { verdicts: null, unknownVerdictIds: .*$@    return { verdicts: [], unknownVerdictIds: [], usage, toolCalls, error: null };@' \
    "$RUNNER" >"$TMP/mutscripts/runner-empty-verdicts.mjs"
if diff -q "$RUNNER" "$TMP/mutscripts/runner-empty-verdicts.mjs" >/dev/null; then
    fail "the missing-verdicts-array mutation did not apply — the self-test is vacuous"
elif node "$TMP/check-batch-parse.mjs" "$TMP/mutscripts/runner-empty-verdicts.mjs" >/dev/null 2>&1; then
    fail "section 7c missed a batched parser that reads a missing verdicts array as a clean grade"
else
    pass "section 7c fires when a missing verdicts array is read as a clean grade rather than a crash"
fi

# ---------------------------------------------------------------------------
# Section 10 (CHANGELOG hygiene) is REMOVED. It asserted CHANGELOG.md's
# [Unreleased] section mentioned this harness and named both decision docs — a
# release time-bomb rather than a code gate, since prepare-release.yml empties
# [Unreleased] into a versioned section on every release (it went red on main
# with v0.18.1). CLAUDE.md now categorically forbids asserting on CHANGELOG.md
# content. The section number is left as a gap so 11's numbering is stable.

# ---------------------------------------------------------------------------
say "11. AC9 XOR: a changed binding is gated, or the unchanged one is recorded"

# Exactly ONE of these must hold, so neither a silent binding change nor a
# silently-dropped decision record can pass.
BINDING_CHANGED=0
grep -q 'findModel' "$PLAN_LIB" && BINDING_CHANGED=1
GATE_HAS_MODELS=0
grep -q '5b-models' scripts/verify-workflow-review.sh && GATE_HAS_MODELS=1
GATE_HAS_POINTER=0
grep -q 'refuter-model-tiering.md' scripts/verify-workflow-review.sh && GATE_HAS_POINTER=1

if [ "$BINDING_CHANGED" = "1" ]; then
    [ "$GATE_HAS_MODELS" = "1" ] ||
        fail "plan-review.mjs threads findModel but scripts/verify-workflow-review.sh has no 5b-models criterion"
    pass "the changed judgment-site binding is gated by a 5b-models criterion"
else
    [ "$GATE_HAS_POINTER" = "1" ] ||
        fail "no binding changed, but scripts/verify-workflow-review.sh carries no pointer to $DOC"
    [ "$GATE_HAS_MODELS" = "0" ] ||
        fail "scripts/verify-workflow-review.sh asserts a 5b-models binding that plan-review.mjs does not have"
    pass "no binding changed, and scripts/verify-workflow-review.sh points at the decision doc"
fi

# ---------------------------------------------------------------------------
printf '\n'
if [ "$FAILURES" -ne 0 ]; then
    printf 'verify-refuter-agreement: %d check(s) FAILED\n' "$FAILURES" >&2
    exit 1
fi
printf 'verify-refuter-agreement: all checks passed\n'
