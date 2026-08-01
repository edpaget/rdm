#!/usr/bin/env bash
# verify-finder-collapse.sh — hermetic gate for the collapsed-plan-finder A/B.
#
# WHAT THIS GATES
#   1  Hygiene: the three scripts parse, are grep-readable text, carry no clock
#      and no RNG, are not coupled to the lane's hot path, and document
#      themselves; docs/finder-collapse.md exists and pre-registers a rule.
#   2  Corpus validation and the pre-registered POWER floors: every committed
#      unit is schema-valid with a real plan document, the run population clears
#      the floors, buildCollapseTrials THROWS on a truncated corpus, and
#      --allow-underpowered forces a NO MEASUREMENT banner with NO decision line.
#   3  Prompt fidelity: arm A is built by the REAL exported findPrompt over the
#      REAL always-on DIMENSIONS.plan entries (never a copy), and arm B is a
#      MINIMAL DELTA from it — the real lens titles/focus verbatim, the real
#      PLAN_SEVERITY_CALIBRATION paragraph exactly once, the concern enum, and
#      the unit-of-work prohibition. Once the merge SHIPS, the instrument's arm-B
#      render must be byte-identical to review.mjs's own merged render.
#   4  Miner behavior against the hermetic mine-sidecars fixture: both recovered
#      units, all FOUR exclusion paths (unrecoverable unit identity, plan doc
#      below the floor, incomplete always-on lenses, no findings output), a
#      no-silent-drop accounting identity that holds UNDER --limit truncation as
#      well as without it, and the CLI surface.
#   5  Scorer replay: re-scoring the committed trials file reproduces the figures
#      committed in docs/token-baseline.json § planFinderCollapse exactly.
#   6  The decision rule, driven over synthetic trial sets: all six pass ->
#      ship-collapsed; one lens losing material findings -> no-ship with that
#      criterion named and criterion 6 explicitly non-dispositive; underpowered
#      -> no-measurement with the banner and no decision line.
#   7  --dry-run dispatches NOTHING (proved with a PATH-shadowed `claude` that
#      fails the run if it is ever invoked); --dispatch-stub and the REAL paid
#      parsing path (parseClaudeFinderResult / extractFindingsPayload /
#      dispatchTrial) driven with zero spend.
#   8  --audit of the committed figures with NO corpus present, and the
#      no-blended-cross-lens-rate negative in both JSON and text.
#   9  The DECISION/PIPELINE XOR: a no-ship decision and a half-landed merged
#      dimension can never coexist, in both directions.
#  10  Planted-mutation self-tests proving 2, 3, 4, 5, 6, 8 and 9 are not
#      vacuous — including reverting the miner's --limit bucket to a bare
#      `break`, which must break section 4's under-truncation identity.
#
# THIS SCRIPT NEVER DISPATCHES A PAID AGENT. Section 7 proves it.
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

MODULE="scripts/lib/finder-collapse.mjs"
MINER="scripts/mine-plan-finder-corpus.mjs"
RUNNER="scripts/run-finder-collapse.mjs"
CORPUS="tests/fixtures/finder-collapse/corpus.jsonl"
TRIALS="tests/fixtures/finder-collapse/trials-opus-r2.json"
ADJUDICATION="tests/fixtures/finder-collapse/adjudication.jsonl"
SIDECARS="tests/fixtures/finder-collapse/mine-sidecars"
DOC="docs/finder-collapse.md"
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
# silently turning each of those assertions into a vacuous pass.
BEFORE="$FAILURES"
for f in "$MODULE" "$MINER" "$RUNNER"; do
    # shellcheck disable=SC2016  # a Node program, deliberately not shell-expanded
    node -e '
const b = require("fs").readFileSync(process.argv[1]);
if (b.includes(0)) {
  console.error(process.argv[1] + " contains a NUL byte at offset " + b.indexOf(0));
  process.exit(1);
}
' "$f" || fail "$f is not grep-readable text"
    grep -q 'export' "$f" || fail "$f: a control grep for 'export' found nothing"
done
[ "$FAILURES" = "$BEFORE" ] && pass "all three scripts are grep-readable text, so the greps below are not vacuous"

# Determinism: the decision must be a pure function of its inputs. A clock or an
# RNG anywhere makes two runs over the same corpus incomparable.
BEFORE="$FAILURES"
for f in "$MODULE" "$MINER" "$RUNNER"; do
    if grep -nE 'Date\.now\(|Math\.random\(' "$f" >&2; then
        fail "$f contains a forbidden nondeterministic global (a clock or an RNG)"
    fi
done
[ "$FAILURES" = "$BEFORE" ] && pass "no clock and no RNG anywhere in the instrument"

# The dependency may only ever point ONE way: the instrument imports FROM the
# lane, never the other way round.
BEFORE="$FAILURES"
if grep -rn "finder-collapse" .claude/workflows/ >&2; then
    fail "a workflow script references the finder-collapse instrument — the lane must never depend on it"
fi
grep -q "workflows/lib/review.mjs" "$RUNNER" || fail "$RUNNER must import the REAL review lib"
if grep -nE "(from|import\()[[:space:]]*['\"][^'\"]*workflows/lib/review\.mjs" "$MODULE" >&2; then
    fail "$MODULE must take findPrompt/DIMENSIONS as INJECTED deps, never import the lane itself"
fi
[ "$FAILURES" = "$BEFORE" ] && pass "the instrument imports from the lane and the lane never imports the instrument"

# Only the runner may spawn anything, and only the module owns the dispatcher.
BEFORE="$FAILURES"
if grep -nE "\bfetch\(|node:https?|child_process" "$MINER" >&2; then
    fail "$MINER must not reach the network or spawn a subprocess"
fi
[ "$FAILURES" = "$BEFORE" ] && pass "the miner reaches neither the network nor a subprocess"

BEFORE="$FAILURES"
for f in "$MINER" "$RUNNER"; do
    node "$f" --help >"$TMP/help.txt" 2>&1 || fail "$f --help exited non-zero"
    grep -q -- '--help' "$TMP/help.txt" || fail "$f --help does not document --help"
done
node "$RUNNER" --help >"$TMP/runhelp.txt" 2>&1
for flag in --dry-run --dispatch --dispatch-stub --score --audit --replicates --corpus --adjudication --allow-underpowered --format; do
    grep -q -- "$flag" "$TMP/runhelp.txt" || fail "$RUNNER --help does not document $flag"
done
grep -qi 'COST WARNING' "$TMP/runhelp.txt" || fail "$RUNNER --help must carry a COST WARNING"
[ "$FAILURES" = "$BEFORE" ] && pass "both CLIs document their whole surface, and the runner warns about spend"

BEFORE="$FAILURES"
[ -f "$DOC" ] || fail "missing $DOC"
grep -q '## Decision rule' "$DOC" || fail "$DOC must pre-register a decision rule"
grep -q '## DECISION' "$DOC" || fail "$DOC must carry a DECISION section"
grep -qi 'never a ship' "$DOC" || fail "$DOC must state that the token criterion alone is never a ship"
grep -qi 'legitimate terminal negative' "$DOC" || fail "$DOC must name the per-lens loss as a legitimate negative"
[ "$FAILURES" = "$BEFORE" ] && pass "$DOC pre-registers a rule, names the negative outcome, and forbids a token-only ship"

# ---------------------------------------------------------------------------
say "2. Corpus validation and the pre-registered POWER floors"

BEFORE="$FAILURES"
# shellcheck disable=SC2016  # a Node program, deliberately not shell-expanded
node --input-type=module -e '
import fs from "node:fs";
import assert from "node:assert/strict";
const m = await import("./scripts/lib/finder-collapse.mjs");
const { header, units, errors } = m.loadCorpus(fs.readFileSync(process.argv[1], "utf8"));
assert.deepEqual(errors, [], "the committed corpus must validate cleanly");
assert.ok(header && header.window && header.window.until, "the corpus must pin its selection window");
assert.ok(units.length >= m.POWER_FLOORS.minUnits, "corpus must clear the unit floor: " + units.length);
for (const u of units) {
  assert.ok(u.planDoc.length >= m.MIN_PLAN_DOC_CHARS, u.id + ": plan doc below the floor");
  assert.ok(m.PLAN_TARGET_TYPES.includes(u.targetType), u.id + ": bad target type");
  for (const lens of m.ALWAYS_ON_PLAN_LENSES) {
    assert.ok(Array.isArray(u.armA.byLens[lens]), u.id + ": missing recorded arm-A lens " + lens);
  }
  assert.equal(m.targetTypeOf(u.targetId), u.targetType, u.id + ": targetType disagrees with the identity line");
}
// The mined corpus must agree with the REAL always-on set, or arm A is a
// different experiment from the one the committed figures describe.
const { DIMENSIONS } = await import("./.claude/workflows/lib/review.mjs");
const realAlwaysOn = DIMENSIONS.plan.filter((d) => !d.when).flatMap((d) => (d.lenses ? d.lenses.map((l) => l.key) : [d.key]));
assert.deepEqual(realAlwaysOn.slice().sort(), m.ALWAYS_ON_PLAN_LENSES.slice().sort(),
  "ALWAYS_ON_PLAN_LENSES has drifted from the real when-less DIMENSIONS.plan entries");
console.log("corpus: " + units.length + " unit(s), window " + header.window.until);
' "$CORPUS" || fail "the committed corpus failed validation"
[ "$FAILURES" = "$BEFORE" ] && pass "the committed corpus is schema-valid, pins its window, and matches the real always-on set"

BEFORE="$FAILURES"
# shellcheck disable=SC2016  # a Node program, deliberately not shell-expanded
node --input-type=module -e '
import fs from "node:fs";
import assert from "node:assert/strict";
const m = await import("./scripts/lib/finder-collapse.mjs");
const { findPrompt, DIMENSIONS, PLAN_SEVERITY_CALIBRATION } = await import("./.claude/workflows/lib/review.mjs");
const corpus = m.loadCorpus(fs.readFileSync(process.argv[1], "utf8"));
const dims = DIMENSIONS.plan.filter((d) => !d.when);
const lenses = dims[0] && dims[0].lenses ? dims[0].lenses : dims;

// The committed population clears the floors.
const plan = m.buildCollapseTrials(corpus, { findPrompt, planDimensions: lenses, calibration: PLAN_SEVERITY_CALIBRATION });
assert.equal(plan.power.power, "SUFFICIENT", "the committed run population must clear the floors");
assert.equal(plan.noMeasurement, false);
assert.equal(plan.trials.length, plan.runUnits.length * (m.ALWAYS_ON_PLAN_LENSES.length + 1) * 2);
assert.ok(plan.power.targetTypesPresent.length >= m.POWER_FLOORS.minTargetTypes);

// TRUNCATED corpus -> THROWS. An A/B whose population failed the pre-registered
// floor is not a result and must never be dispatched as if it were.
const truncated = { header: corpus.header, units: corpus.units.slice(0, 3) };
assert.throws(
  () => m.buildCollapseTrials(truncated, { findPrompt, planDimensions: lenses }),
  /POWER: UNDERPOWERED/,
  "an underpowered population must THROW rather than dispatch"
);

// --allow-underpowered builds the plan but stamps noMeasurement, and the report
// then prints the banner and SUPPRESSES the decision line.
const forced = m.buildCollapseTrials(truncated, { findPrompt, planDimensions: lenses, allowUnderpowered: true });
assert.equal(forced.noMeasurement, true);
const report = m.scoreCollapse({ trials: [] }, [], { noMeasurement: true, power: forced.power });
assert.equal(report.decision, "no-measurement");
const text = m.formatReport(report, "text");
assert.ok(/NO MEASUREMENT/.test(text), "an underpowered report must print the NO MEASUREMENT banner");
assert.ok(!/^DECISION:/m.test(text), "an underpowered report must print NO decision line");
console.log("power: SUFFICIENT on the committed population; underpowered throws; --allow-underpowered suppresses the decision");
' "$CORPUS" || fail "the power gate did not behave as pre-registered"
[ "$FAILURES" = "$BEFORE" ] && pass "power floors hold, an underpowered population throws, and NO MEASUREMENT suppresses the decision line"

# ---------------------------------------------------------------------------
say "3. Prompt fidelity: arm A through the REAL findPrompt, arm B a minimal delta"

BEFORE="$FAILURES"
# shellcheck disable=SC2016  # a Node program, deliberately not shell-expanded
node --input-type=module -e '
import fs from "node:fs";
import assert from "node:assert/strict";
const m = await import("./scripts/lib/finder-collapse.mjs");
const { findPrompt, DIMENSIONS, PLAN_SEVERITY_CALIBRATION } = await import("./.claude/workflows/lib/review.mjs");
const corpus = m.loadCorpus(fs.readFileSync(process.argv[1], "utf8"));
const alwaysOn = DIMENSIONS.plan.filter((d) => !d.when);
const merged = alwaysOn.find((d) => Array.isArray(d.lenses)) || null;
const lenses = merged ? merged.lenses : alwaysOn;

// Refuses to build without the REAL prompt builder.
assert.throws(() => m.buildCollapseTrials(corpus, { planDimensions: lenses }), /REAL findPrompt/);
// Refuses a planDimensions set that is not the real always-on one.
assert.throws(
  () => m.buildCollapseTrials(corpus, { findPrompt, planDimensions: lenses.filter((d) => d.key !== "restraint") }),
  /missing the always-on lens "restraint"/
);

const plan = m.buildCollapseTrials(corpus, { findPrompt, planDimensions: lenses, calibration: PLAN_SEVERITY_CALIBRATION });
const unit = plan.runUnits[0];

// ARM A: byte-identical to the production render, per lens.
for (const dim of lenses) {
  const got = plan.prompts.get(unit.id + "|A|" + dim.key + "|r1");
  assert.equal(got, findPrompt("plan", dim, { target: unit.target }),
    "arm A must be the REAL findPrompt render for " + dim.key);
}

// ARM B: a MINIMAL DELTA. Every lens title and focus verbatim, the real
// calibration paragraph exactly ONCE, the concern enum, the unit-of-work
// prohibition, and the shared READ-ONLY/evidence/schema lines.
const b = plan.prompts.get(unit.id + "|B|collapsed|r1");
for (const dim of lenses) {
  assert.ok(b.includes(dim.title), "arm B is missing lens title " + dim.title);
  assert.ok(b.includes(dim.focus), "arm B is missing the verbatim focus for " + dim.key);
}
assert.equal(b.split(PLAN_SEVERITY_CALIBRATION).length - 1, 1,
  "arm B must inject the plan severity calibration EXACTLY once");
assert.ok(b.includes("`concern` MUST be exactly one of: " + m.ALWAYS_ON_PLAN_LENSES.join(", ")),
  "arm B must state the concern enum");
assert.ok(b.includes("Never emit `concern: unit-of-work`"), "arm B must forbid a unit-of-work concern");
assert.ok(b.includes("You are a READ-ONLY reviewer. Do not edit any files."), "arm B must keep the READ-ONLY stance");
assert.ok(b.includes("Review target: " + unit.target + "."), "arm B must keep the Review target line");
assert.ok(b.includes("Inspect the plan document text."), "arm B must keep the plan-mode inspect hint");
assert.ok(b.includes("One strong finding beats five weak ones."), "arm B must keep the evidence instruction");
assert.ok(b.includes("Return JSON matching the FINDINGS schema"), "arm B must keep the FINDINGS schema instruction");
assert.ok(!b.includes("Your single dimension is"), "arm B must not keep the single-dimension sentence");

// ONCE SHIPPED: the instrument stops owning the arm-B prompt and renders it
// through review.mjs, so the two must be byte-identical.
if (merged) {
  const shipped = findPrompt("plan", merged, { target: unit.target });
  assert.equal(b, shipped,
    "the merge has SHIPPED, so the instrument arm-B render must be byte-identical to review.mjs findPrompt");
  console.log("prompt fidelity: arm A real, arm B byte-identical to the SHIPPED merged render");
} else {
  console.log("prompt fidelity: arm A real, arm B a minimal delta owned by the instrument (merge not shipped)");
}
' "$CORPUS" || fail "prompt fidelity failed"
[ "$FAILURES" = "$BEFORE" ] && pass "arm A is the real findPrompt render; arm B is a minimal, gated delta"

# ---------------------------------------------------------------------------
say "4. Miner behavior against the hermetic sidecar fixture"

[ -d "$SIDECARS" ] || die "missing $SIDECARS"

BEFORE="$FAILURES"
node "$MINER" --root "$SIDECARS" --project-slug -Users-edward-Projects-rdm-fixture --format json \
    >"$TMP/mined.json" 2>"$TMP/mined.err" || fail "the miner failed against the fixture"
# shellcheck disable=SC2016  # a Node program, deliberately not shell-expanded
node --input-type=module -e '
import fs from "node:fs";
import assert from "node:assert/strict";
const r = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
// TWO qualifying units, so the --limit truncation case below has a real
// boundary to cut at; a single-qualifying-unit fixture can never exercise it.
assert.equal(r.units.length, 2, "exactly two fixture units are recoverable");
const u = r.units[0];
assert.equal(u.targetType, "phase");
assert.equal(u.targetId, "phase fixture-roadmap/phase-1-complete-unit");
assert.equal(r.units[1].targetId, "phase fixture-roadmap/phase-3-second-complete-unit");
assert.ok(u.planDoc.length >= 500, "the recovered plan doc must clear the floor");
assert.ok(!u.planDoc.startsWith("phase fixture-roadmap"), "planDoc must be the BODY, not the identity line");
assert.deepEqual(Object.keys(u.armA.byLens).sort(), ["architectural-fit", "coherence", "restraint"]);
assert.equal(u.armA.byLens.coherence.length, 2);
assert.equal(u.armA.byLens.restraint.length, 1);
assert.ok(u.armAUsage.coherence.cacheRead > 0, "per-lens usage must be recorded by token class");
assert.equal(u.provenance.runId, "wf_fixture001");

// ALL FOUR exclusion paths fire, each counted.
const s = r.skips;
assert.equal(s["unrecoverable-unit-identity"], 3, "the --implementation-plan JSON shape must be rejected, not bucketed");
assert.equal(s["plan-doc-below-floor"], 3, "a fetch-status-line roadmap body must be excluded, counted PER FINDER");
assert.equal(s["incomplete-always-on-lenses"], 4, "a unit missing a lens must be excluded, counted PER FINDER");
assert.equal(s["no-findings-output"], 1, "a finder with no StructuredOutput must be excluded and counted");
assert.equal(s["not-an-always-on-lens"], 2, "each recovered unit unit-of-work finder is counted, not silently dropped");
assert.equal(s["beyond-limit"], undefined, "an unlimited run must never bucket anything as beyond-limit");

// NO SILENT DROP: every plan-finder record is either recovered as an always-on
// lens observation or counted in exactly one skip bucket. Exact, not >=.
const lensesRecovered = r.units.length * 3;
const skipped = Object.values(s).reduce((a, b) => a + b, 0);
assert.equal(lensesRecovered + skipped, r.finderRecordCount,
  "accounting identity: recovered lens records + skips must equal the plan-finder records exactly");
// The label filter is doing work: the fixture holds a refuter that must not appear.
assert.equal(r.finderRecordCount, 19, "only find:plan:* agents are considered");
console.log("miner: 2 units, 4 exclusion buckets, accounting closes");
' "$TMP/mined.json" || fail "the miner did not behave as specified against the fixture"
[ "$FAILURES" = "$BEFORE" ] && pass "the miner recovers both units, fires all four exclusion paths, and drops nothing silently"

BEFORE="$FAILURES"
# --project-slug is a real filter: a prefix that matches nothing recovers nothing.
node "$MINER" --root "$SIDECARS" --project-slug -no-such-project --format json >"$TMP/none.json" 2>/dev/null ||
    fail "the miner failed under a non-matching project slug"
# shellcheck disable=SC2016  # a Node program, deliberately not shell-expanded
node -e '
const r = require("fs").readFileSync(process.argv[1], "utf8");
if (JSON.parse(r).units.length !== 0) { console.error("--project-slug is not filtering"); process.exit(1); }
' "$TMP/none.json" || fail "--project-slug did not filter"
# --limit and --out.
node "$MINER" --root "$SIDECARS" --project-slug -Users-edward-Projects-rdm-fixture --limit 1 \
    --out "$TMP/mined.jsonl" >/dev/null 2>&1 || fail "the miner failed under --limit/--out"
[ -s "$TMP/mined.jsonl" ] || fail "--out wrote nothing"
head -1 "$TMP/mined.jsonl" | grep -q '"kind":"header"' || fail "--out must write the pinned header first"
# --limit TRUNCATES MID-LIST (the fixture holds two qualifying units), and the
# accounting identity must survive the truncation: a unit past the limit is
# classified into `beyond-limit` BEFORE any other test, so neither the boundary
# unit's always-on records nor any later unit's records vanish into no bucket.
node "$MINER" --root "$SIDECARS" --project-slug -Users-edward-Projects-rdm-fixture --limit 1 \
    --format json >"$TMP/mined-limited.json" 2>/dev/null || fail "the miner failed under --limit 1"
# shellcheck disable=SC2016  # a Node program, deliberately not shell-expanded
node --input-type=module -e '
import fs from "node:fs";
import assert from "node:assert/strict";
const r = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
assert.equal(r.units.length, 1, "--limit 1 must recover exactly one unit");
assert.equal(r.units[0].targetId, "phase fixture-roadmap/phase-1-complete-unit",
  "--limit must truncate in the deterministic sort order, keeping the first unit");
assert.equal(r.finderRecordCount, 19, "finderRecordCount is limit-independent (it is a Pass-1 figure)");
assert.ok(r.skips["beyond-limit"] > 0, "the truncated units must land in the beyond-limit bucket");
const skipped = Object.values(r.skips).reduce((a, b) => a + b, 0);
assert.equal(r.units.length * 3 + skipped, r.finderRecordCount,
  "accounting identity must hold UNDER --limit too: nothing may be dropped by truncation");
console.log("miner --limit: truncates mid-list and still closes the accounting identity");
' "$TMP/mined-limited.json" || fail "--limit truncation lost records from the accounting identity"
# --until in both directions.
node "$MINER" --root "$SIDECARS" --project-slug -Users-edward-Projects-rdm-fixture \
    --until 2026-01-01T00:00:00Z --format json >"$TMP/before.json" 2>/dev/null || fail "the miner failed under --until"
# shellcheck disable=SC2016  # a Node program, deliberately not shell-expanded
node -e '
if (JSON.parse(require("fs").readFileSync(process.argv[1], "utf8")).units.length !== 0) {
  console.error("--until did not exclude a later run"); process.exit(1);
}' "$TMP/before.json" || fail "--until did not exclude"
# Argument validation is an actionable named message, never a stack trace.
for bad in "--limit 0" "--limit abc" "--format yaml" "--nope"; do
    # shellcheck disable=SC2086
    if node "$MINER" --root "$SIDECARS" $bad >/dev/null 2>"$TMP/argerr.txt"; then
        fail "the miner accepted the invalid argument set: $bad"
    elif grep -q 'at ' "$TMP/argerr.txt" && grep -q 'node:internal' "$TMP/argerr.txt"; then
        fail "the miner surfaced a stack trace for: $bad"
    fi
done
[ "$FAILURES" = "$BEFORE" ] && pass "the miner's CLI surface (slug filter, --limit, --out, --until, arg validation) behaves"

# ---------------------------------------------------------------------------
say "5. Scorer replay: the committed trials reproduce the committed figures"

BEFORE="$FAILURES"
[ -f "$TRIALS" ] || fail "missing $TRIALS — the saved run must be replayable with zero spend"
[ -f "$ADJUDICATION" ] || fail "missing $ADJUDICATION"
if [ -f "$TRIALS" ] && [ -f "$ADJUDICATION" ]; then
    node --input-type=module -e '
import fs from "node:fs";
import assert from "node:assert/strict";
const m = await import("./scripts/lib/finder-collapse.mjs");
const run = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
const { rows, errors } = m.loadAdjudication(fs.readFileSync(process.argv[2], "utf8"));
assert.deepEqual(errors, [], "the committed adjudication must validate cleanly");
const committed = JSON.parse(fs.readFileSync(process.argv[3], "utf8")).planFinderCollapse;
assert.ok(committed, "docs/token-baseline.json is missing its planFinderCollapse section");

const report = m.scoreCollapse(run, rows, { noMeasurement: run.noMeasurement === true, power: run.power });
assert.equal(report.decision, committed.decision, "the committed decision must be the scorer output");
for (const lens of m.ALWAYS_ON_PLAN_LENSES) {
  for (const arm of ["armA", "armB"]) {
    assert.equal(report.perLens[lens][arm].findings, committed.perLens[lens][arm].findings,
      "committed perLens." + lens + "." + arm + ".findings disagrees with the replay");
    assert.deepEqual(report.perLens[lens][arm].severity, committed.perLens[lens][arm].severity,
      "committed severity distribution for " + lens + "/" + arm + " disagrees with the replay");
  }
  assert.equal(report.material[lens].A.count, committed.material[lens].armA,
    "committed material armA for " + lens + " disagrees");
  assert.equal(report.material[lens].B.count, committed.material[lens].armB,
    "committed material armB for " + lens + " disagrees");
}
assert.equal(report.attribution.total, committed.attribution.total);
assert.equal(report.attribution.valid, committed.attribution.valid);
for (const arm of ["A", "B"]) {
  for (const c of m.TOKEN_CLASSES) {
    assert.equal(report.tokens.byArm[arm][c], committed.tokens.byArm[arm][c],
      "committed token class " + c + " for arm " + arm + " disagrees with the replay");
  }
}
assert.deepEqual(report.criteria.map((c) => [c.id, c.pass]), committed.criteria.map((c) => [c.id, c.pass]),
  "the committed criteria table disagrees with the replay");
// Adjudication covers the pre-registered replicate for BOTH arms.
const arms = new Set(rows.filter((r) => r.replicate === m.DECISION_RULE.adjudicatedReplicate).map((r) => r.arm));
assert.deepEqual([...arms].sort(), ["A", "B"], "adjudication must cover both arms of the adjudicated replicate");
console.log("replay: decision " + report.decision + " reproduces every committed figure");
' "$TRIALS" "$ADJUDICATION" "$BASELINE_JSON" || fail "the committed figures do not replay"
fi
[ "$FAILURES" = "$BEFORE" ] && pass "re-scoring the committed trials reproduces the committed planFinderCollapse figures"

# ---------------------------------------------------------------------------
say "6. The pre-registered decision rule, over synthetic trial sets"

BEFORE="$FAILURES"
# shellcheck disable=SC2016  # a Node program, deliberately not shell-expanded
node --input-type=module -e '
import assert from "node:assert/strict";
const m = await import("./scripts/lib/finder-collapse.mjs");
const LENSES = m.ALWAYS_ON_PLAN_LENSES;
const F = (id, concern, sev) => ({ id, concern, severity: sev, confidence: 90, what_fails: id });
const usage = (n) => ({ output: n, uncachedInput: 0, cacheWrite: 0, cacheRead: n * 9 });

// Build a synthetic run: `armBPerLens` findings per lens in arm B, 3 in arm A.
function mkRun(armBPerLens, tokensB) {
  const trials = [];
  for (let u = 0; u < 8; u++) {
    for (let r = 1; r <= 2; r++) {
      for (const lens of LENSES) {
        trials.push({ trialId: `u${u}|A|${lens}|r${r}`, unitId: `u${u}`, arm: "A", lens, replicate: r,
          findings: [0, 1, 2].map((i) => F(`a-${lens}-${i}`, lens, i === 0 ? "blocking" : "concern")),
          usage: usage(10000), error: null });
      }
      const bf = [];
      for (const lens of LENSES) {
        for (let i = 0; i < armBPerLens[lens]; i++) bf.push(F(`b-${lens}-${i}`, lens, i === 0 ? "blocking" : "concern"));
      }
      trials.push({ trialId: `u${u}|B|collapsed|r${r}`, unitId: `u${u}`, arm: "B", lens: null, replicate: r,
        findings: bf, usage: usage(tokensB), error: null });
    }
  }
  return { trials };
}
function mkAdj(run, materialPerLens) {
  const rows = [];
  const seen = {};
  for (const t of run.trials) {
    if (t.replicate !== 1) continue;
    for (const f of t.findings) {
      const lens = t.arm === "A" ? t.lens : f.concern;
      const k = `${t.unitId}|${t.arm}|${lens}`;
      seen[k] = (seen[k] || 0) + 1;
      rows.push({ unitId: t.unitId, arm: t.arm, replicate: 1, lens, findingId: f.id,
        material: seen[k] <= materialPerLens[t.arm][lens], matchedInOtherArm: false,
        matchedFindingId: null, rationale: "synthetic", adjudicatedAgainstCommit: "0000000" });
    }
  }
  return rows;
}
const power = { power: "SUFFICIENT", reasons: [] };

// (a) ALL SIX PASS -> ship-collapsed.
{
  const per = { coherence: 3, "architectural-fit": 3, restraint: 3 };
  const run = mkRun(per, 2000);
  const adj = mkAdj(run, { A: { coherence: 1, "architectural-fit": 1, restraint: 1 },
                           B: { coherence: 1, "architectural-fit": 1, restraint: 1 } });
  const rep = m.scoreCollapse(run, adj, { power });
  assert.deepEqual(rep.criteria.filter((c) => !c.pass).map((c) => c.id), [], "every criterion should pass: " +
    JSON.stringify(rep.criteria.filter((c) => !c.pass)));
  assert.equal(rep.decision, "ship-collapsed");
}

// (b) ONE lens loses 3 material findings -> no-ship, naming criterion 2, with
//     criterion 6 (tokens) still passing and explicitly non-dispositive.
{
  const per = { coherence: 3, "architectural-fit": 3, restraint: 3 };
  const run = mkRun(per, 2000);
  const adj = mkAdj(run, { A: { coherence: 3, "architectural-fit": 1, restraint: 1 },
                           B: { coherence: 0, "architectural-fit": 1, restraint: 1 } });
  const rep = m.scoreCollapse(run, adj, { power });
  assert.equal(rep.decision, "no-ship");
  const failed = rep.criteria.filter((c) => !c.pass).map((c) => c.id);
  assert.ok(failed.includes(2), "criterion 2 must be the named failure, got " + JSON.stringify(failed));
  assert.equal(rep.criteria.find((c) => c.id === 6).pass, true, "the token criterion still passes");
  assert.equal(rep.criteria.find((c) => c.id === 2).observed.coherence.pass, false, "the failing LENS is named");
  assert.equal(rep.criteria.find((c) => c.id === 2).observed.restraint.pass, true, "healthy lenses stay passing");
  const text = m.formatReport(rep, "text");
  assert.ok(/DECISION: no-ship/.test(text), "a no-ship report prints its decision");
  assert.ok(/NEVER a ship on its own/.test(text), "the report must restate that tokens alone are not a ship");
}

// (c) UNDERPOWERED -> no-measurement, banner, NO decision line.
{
  const run = mkRun({ coherence: 3, "architectural-fit": 3, restraint: 3 }, 2000);
  const rep = m.scoreCollapse(run, [], { noMeasurement: true, power: { power: "UNDERPOWERED", reasons: ["synthetic"] } });
  assert.equal(rep.decision, "no-measurement");
  const text = m.formatReport(rep, "text");
  assert.ok(/NO MEASUREMENT/.test(text));
  assert.ok(!/^DECISION:/m.test(text));
}

// A hallucinated unit-of-work concern is COUNTED as invalid attribution and is
// never remapped onto a plausible lens.
{
  const run = mkRun({ coherence: 1, "architectural-fit": 1, restraint: 1 }, 2000);
  for (const t of run.trials) {
    if (t.arm === "B") t.findings.push(F("b-uow", "unit-of-work", "blocking"), F("b-junk", "not-a-lens", "concern"));
  }
  const rep = m.scoreCollapse(run, [], { power });
  assert.equal(rep.attribution.claimedUnitOfWork, 16, "a claimed unit-of-work concern must be counted");
  assert.ok(rep.attribution.validity < 1, "invalid attributions must lower the validity rate");
  assert.equal(rep.perLens.coherence.armB.findings, 16, "an invalid concern must NOT be folded into a lens");
}
console.log("decision rule: ship / no-ship / no-measurement all reachable and correctly named");
' || fail "the decision rule did not behave as pre-registered"
[ "$FAILURES" = "$BEFORE" ] && pass "all three decisions are reachable; a per-lens loss names its criterion and tokens never carry it"

# ---------------------------------------------------------------------------
say "7. --dry-run spends nothing; the real dispatch/parse path drives with zero spend"

BEFORE="$FAILURES"
mkdir -p "$TMP/fakebin"
cat >"$TMP/fakebin/claude" <<'FAKE'
#!/usr/bin/env bash
echo "verify-finder-collapse.sh: --dry-run invoked a PAID dispatch" >&2
touch "$SPEND_SENTINEL"
exit 1
FAKE
chmod +x "$TMP/fakebin/claude"
SPEND_SENTINEL="$TMP/spent" PATH="$TMP/fakebin:$PATH" node "$RUNNER" --dry-run >"$TMP/dry.txt" 2>&1 ||
    fail "--dry-run exited non-zero"
[ -e "$TMP/spent" ] && fail "--dry-run dispatched a paid agent"
grep -q 'Nothing dispatched' "$TMP/dry.txt" || fail "--dry-run must say it dispatched nothing"
grep -q 'POWER: ' "$TMP/dry.txt" || fail "--dry-run must report the power verdict"
[ "$FAILURES" = "$BEFORE" ] && pass "--dry-run reports the plan and spends nothing"

BEFORE="$FAILURES"
cat >"$TMP/stub.mjs" <<'STUB'
// A dispatcher stub: returns a deterministic, shape-correct result per trial.
export function dispatch(trial) {
  const findings =
    trial.arm === 'A'
      ? [{ id: 'a-' + trial.lens, concern: trial.lens, severity: 'concern', confidence: 80, what_fails: 'x' }]
      : [{ id: 'b-1', concern: 'coherence', severity: 'blocking', confidence: 90, what_fails: 'x' }];
  return Promise.resolve({ findings, usage: { output: 1, uncachedInput: 1, cacheWrite: 1, cacheRead: 1 }, toolCalls: 0 });
}
STUB
SPEND_SENTINEL="$TMP/spent2" PATH="$TMP/fakebin:$PATH" node "$RUNNER" --dispatch-stub "$TMP/stub.mjs" \
    --out "$TMP/stubrun.json" >"$TMP/stub.txt" 2>/dev/null || fail "--dispatch-stub failed"
[ -e "$TMP/spent2" ] && fail "--dispatch-stub dispatched a paid agent"
grep -q 'DECISION:' "$TMP/stub.txt" || fail "--dispatch-stub must produce a scored report"
# shellcheck disable=SC2016  # a Node program, deliberately not shell-expanded
node -e '
const r = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"));
if (r.trials.length !== 64) { console.error("expected 64 trials, got " + r.trials.length); process.exit(1); }
if (r.trials.some((t) => t.findings === null)) { console.error("stub results must parse"); process.exit(1); }
' "$TMP/stubrun.json" || fail "--dispatch-stub produced an unusable run file"
[ "$FAILURES" = "$BEFORE" ] && pass "--dispatch-stub drives the real trial/score path with no spend"

BEFORE="$FAILURES"
# shellcheck disable=SC2016  # a Node program, deliberately not shell-expanded
node --input-type=module -e '
import assert from "node:assert/strict";
const m = await import("./scripts/lib/finder-collapse.mjs");

// parseClaudeFinderResult: the REAL paid-dispatch parsing path.
const viaTool = m.parseClaudeFinderResult({
  usage: { output_tokens: 5, input_tokens: 6, cache_creation_input_tokens: 7, cache_read_input_tokens: 8 },
  messages: [{ message: { content: [{ type: "tool_use", name: "StructuredOutput", input: { findings: [{ id: "x" }] } }] } }],
});
assert.deepEqual(viaTool.usage, { output: 5, uncachedInput: 6, cacheWrite: 7, cacheRead: 8 });
assert.equal(viaTool.findings.length, 1);
assert.equal(viaTool.error, null);

// A prose/fenced body still parses.
const viaProse = m.parseClaudeFinderResult({
  result: "Here are my findings:\n\n```json\n{\"findings\": [{\"id\": \"y\", \"concern\": \"coherence\"}]}\n```\n",
});
assert.equal(viaProse.findings.length, 1);

// NO findings array anywhere -> ERROR, never coerced to [] (which would read as
// "this arm found the document clean" and manufacture a dilution result).
const none = m.parseClaudeFinderResult({ result: "I could not comply." });
assert.equal(none.findings, null);
assert.ok(/no `findings` array/.test(none.error));

// dispatchTrial against injected spawn fakes — the real branch structure, zero spend.
const enoent = () => Promise.resolve({ status: null, stdout: "", stderr: "", error: Object.assign(new Error("x"), { code: "ENOENT" }) });
await assert.rejects(() => m.dispatchTrial({ model: "opus" }, "p", { spawnImpl: enoent }), /binary was not found on PATH/);
const nonzero = await m.dispatchTrial({ model: "opus" }, "p", {
  spawnImpl: () => Promise.resolve({ status: 2, stdout: "", stderr: "boom", error: null }),
});
assert.equal(nonzero.findings, null);
assert.ok(/exited 2/.test(nonzero.error));
const nonjson = await m.dispatchTrial({ model: "opus" }, "p", {
  spawnImpl: () => Promise.resolve({ status: 0, stdout: "not json", stderr: "", error: null }),
});
assert.equal(nonjson.findings, null);
const good = await m.dispatchTrial({ model: "opus" }, "p", {
  spawnImpl: (cmd, argv, opts) => {
    assert.equal(cmd, "claude");
    assert.deepEqual(argv, ["-p", "--model", "opus", "--output-format", "json"]);
    assert.equal(opts.input, "p");
    return Promise.resolve({ status: 0, stdout: JSON.stringify({ result: "{\"findings\": []}", usage: {} }), stderr: "", error: null });
  },
});
assert.deepEqual(good.findings, []);

// runCollapseTrials preserves TRIAL-PLAN ORDER regardless of completion order.
const trials = [1, 2, 3, 4].map((i) => ({ trialId: "t" + i, unitId: "u", arm: "A", lens: "coherence", replicate: 1 }));
const prompts = new Map(trials.map((t) => [t.trialId, "p"]));
const delay = { t1: 30, t2: 1, t3: 20, t4: 2 };
const out = await m.runCollapseTrials(trials, prompts, (t) =>
  new Promise((res) => setTimeout(() => res({ findings: [], usage: {} }), delay[t.trialId])), { concurrency: 4 });
assert.deepEqual(out.map((r) => r.trialId), ["t1", "t2", "t3", "t4"], "results must be in trial-plan order");
console.log("dispatch/parse: every real branch driven with zero spend");
' || fail "the real dispatch/parse path did not behave"
[ "$FAILURES" = "$BEFORE" ] && pass "the paid-dispatch parsing path and the concurrent runner are driven with zero spend"

# ---------------------------------------------------------------------------
say "8. --audit of the committed figures, with no corpus present"

BEFORE="$FAILURES"
# Run it from a scratch tree with NO corpus, NO trials and NO adjudication, so
# the committed figures are provably gateable in CI without the measurement inputs.
AUDIT_TREE="$TMP/audit"
mkdir -p "$AUDIT_TREE/scripts/lib" "$AUDIT_TREE/docs" "$AUDIT_TREE/.claude/workflows/lib"
cp "$MODULE" "$AUDIT_TREE/scripts/lib/"
cp "$RUNNER" "$AUDIT_TREE/scripts/"
cp "$BASELINE_JSON" "$AUDIT_TREE/docs/"
cp "$REVIEW_LIB" "$AUDIT_TREE/.claude/workflows/lib/"
(cd "$AUDIT_TREE" && node scripts/run-finder-collapse.mjs --audit docs/token-baseline.json) >"$TMP/audit.txt" 2>&1 ||
    fail "--audit of the committed planFinderCollapse figures failed"
grep -q 'internally consistent' "$TMP/audit.txt" || fail "--audit did not report the figures consistent"
[ "$FAILURES" = "$BEFORE" ] && pass "the committed figures audit clean with no corpus, trials or adjudication present"

BEFORE="$FAILURES"
# shellcheck disable=SC2016  # a Node program, deliberately not shell-expanded
node --input-type=module -e '
import fs from "node:fs";
import assert from "node:assert/strict";
const m = await import("./scripts/lib/finder-collapse.mjs");
const doc = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
// NO BLENDED CROSS-LENS RATE, recursively, over the committed section AND over
// a live report — a single blended recall is exactly what would hide an extinct
// lens behind two healthy ones.
assert.deepEqual(m.findBlendedLensKeys(doc.planFinderCollapse), [],
  "the committed section carries a blended cross-lens rate key");
// The detector is not vacuous.
assert.deepEqual(m.findBlendedLensKeys({ overallRecall: 0.9 }), ["$.overallRecall"]);
assert.deepEqual(m.findBlendedLensKeys({ perLens: { coherence: { recall: 0.9 } } }), []);
// A pre-registered THRESHOLD is not a blended observation: criterion 2 applies
// it per lens by construction. Everything outside decisionRule/rule is measured
// output and must still be lens-scoped.
assert.deepEqual(m.findBlendedLensKeys({ decisionRule: { maxPerLensMaterialLossShare: 0.15 } }), []);
assert.deepEqual(m.findBlendedLensKeys({ observed: { maxPerLensMaterialLossShare: 0.15 } }), ["$.observed.maxPerLensMaterialLossShare"]);
console.log("no blended cross-lens rate key, and the detector fires on a planted one");
' "$BASELINE_JSON" || fail "the blended-rate negative failed"
[ "$FAILURES" = "$BEFORE" ] && pass "no blended cross-lens rate anywhere in the committed figures"

# ---------------------------------------------------------------------------
say "9. The DECISION/PIPELINE XOR"

BEFORE="$FAILURES"
# shellcheck disable=SC2016  # a Node program, deliberately not shell-expanded
DECISION="$(node -e '
const d = JSON.parse(require("fs").readFileSync("docs/token-baseline.json", "utf8"));
process.stdout.write(String((d.planFinderCollapse || {}).decision));
')"
case "$DECISION" in
    ship-collapsed | no-ship | no-measurement) ;;
    *) fail "docs/token-baseline.json planFinderCollapse.decision is \"$DECISION\" — the XOR needs a recorded decision" ;;
esac
xor_check() {
    # $1 = decision, $2 = path to a review.mjs to inspect
    local decision="$1" lib="$2" bad=0 sym
    if [ "$decision" = "ship-collapsed" ]; then
        for sym in 'PLAN_LENSES' 'attributeConcern' 'lensDimFor'; do
            grep -q "$sym" "$lib" || bad=1
        done
    else
        for sym in 'PLAN_LENSES' 'attributeConcern' 'lensDimFor'; do
            if grep -q "$sym" "$lib"; then bad=1; fi
        done
        # shellcheck disable=SC2016  # a Node program, deliberately not shell-expanded
        node --input-type=module -e '
import assert from "node:assert/strict";
const { DIMENSIONS } = await import(process.argv[1]);
assert.deepEqual(DIMENSIONS.plan.map((d) => d.key), ["coherence", "architectural-fit", "unit-of-work", "restraint"]);
assert.ok(!DIMENSIONS.plan.some((d) => Array.isArray(d.lenses)), "no merged dimension may exist");
' "file://$(cd "$(dirname "$lib")" && pwd)/$(basename "$lib")" >/dev/null 2>&1 || bad=1
    fi
    return "$bad"
}
xor_check "$DECISION" "$REPO_ROOT/$REVIEW_LIB" ||
    fail "decision is \"$DECISION\" but $REVIEW_LIB's shape disagrees — a half-landed pipeline cannot coexist with the recorded decision"
[ "$FAILURES" = "$BEFORE" ] && pass "the recorded decision (\"$DECISION\") and the real DIMENSIONS.plan shape agree"

# Both directions of the XOR are non-vacuous: plant the opposite shape and the
# same check must fire.
BEFORE="$FAILURES"
XOR_TREE="$TMP/xor"
mkdir -p "$XOR_TREE"
sed 's/^const DIMENSIONS = {$/const PLAN_LENSES = []; \/\/ MUTANT\nconst DIMENSIONS = {/' "$REVIEW_LIB" >"$XOR_TREE/planted-ship.mjs"
grep -q 'MUTANT' "$XOR_TREE/planted-ship.mjs" || fail "the XOR self-test could not plant its mutation"
if xor_check "no-ship" "$XOR_TREE/planted-ship.mjs"; then
    fail "the XOR check passed a no-ship decision against a lib carrying PLAN_LENSES (vacuous)"
fi
if xor_check "ship-collapsed" "$REPO_ROOT/$REVIEW_LIB" && [ "$DECISION" != "ship-collapsed" ]; then
    fail "the XOR check passed a ship-collapsed decision against the unmerged lib (vacuous)"
fi
[ "$FAILURES" = "$BEFORE" ] && pass "the XOR fires in both directions (planted-mutation self-test)"

# ---------------------------------------------------------------------------
say "10. Planted-mutation self-tests (prove sections 2-8 are not vacuous)"

MUT_BEFORE="$FAILURES"
MUT="$TMP/mut"
mkdir -p "$MUT/scripts/lib" "$MUT/scripts" "$MUT/.claude/workflows/lib" "$MUT/docs"
cp -R tests "$MUT/tests"
cp "$MODULE" "$MUT/scripts/lib/finder-collapse.mjs"
cp "$REVIEW_LIB" "$MUT/.claude/workflows/lib/review.mjs"
cp "$MINER" "$MUT/scripts/"
cp "$RUNNER" "$MUT/scripts/"
cp "$BASELINE_JSON" "$MUT/docs/"

# CONTROL: the unmutated copy passes the probe below.
probe() {
    # Drives the assertions the mutations target, against $MUT.
    # shellcheck disable=SC2016  # a Node program, deliberately not shell-expanded
    (cd "$MUT" && node --input-type=module -e '
import fs from "node:fs";
import assert from "node:assert/strict";
const m = await import("./scripts/lib/finder-collapse.mjs");
const { findPrompt, DIMENSIONS, PLAN_SEVERITY_CALIBRATION } = await import("./.claude/workflows/lib/review.mjs");
const corpus = m.loadCorpus(fs.readFileSync("tests/fixtures/finder-collapse/corpus.jsonl", "utf8"));
assert.deepEqual(corpus.errors, []);
const lenses = DIMENSIONS.plan.filter((d) => !d.when);

// (i) the plan-doc floor actually excludes a degenerate target
assert.ok(m.MIN_PLAN_DOC_CHARS >= 500);
assert.ok(!m.validateUnit({ ...corpus.units[0], planDoc: "Successfully fetched roadmap x." }).ok);

// (ii) the UNIT floor specifically: a population spanning two target types and
//      clearing every other floor is still UNDERPOWERED below minUnits.
const twoTypesTooFew = corpus.units.filter((u) => u.targetType === "task").concat(
  corpus.units.filter((u) => u.targetType === "phase").slice(0, 2)
);
const shortPower = m.assessPower(twoTypesTooFew, { replicates: 2 });
assert.equal(shortPower.power, "UNDERPOWERED", "3 units must be below the unit floor");
assert.ok(shortPower.reasons.some((r) => /review unit/.test(r)), "the UNIT floor must be the stated reason");
assert.throws(() => m.buildCollapseTrials({ header: corpus.header, units: twoTypesTooFew },
  { findPrompt, planDimensions: lenses }), /UNDERPOWERED/);

// (iii) selectRunUnits is deterministic and STRATIFIED. Driven over a synthetic
//      list where the scarce target type sorts LAST by id, so a round-robin that
//      degenerates into by-id order is observable.
const synth = [];
for (let i = 0; i < 12; i++) synth.push({ id: "a-phase-" + i, targetType: "phase" });
synth.push({ id: "z-task-1", targetType: "task" });
assert.deepEqual(
  m.selectRunUnits(synth, 4).map((u) => u.targetType).sort(),
  ["phase", "phase", "phase", "task"],
  "round-robin must reach the scarce target type rather than taking the first 4 by id"
);
const a = m.selectRunUnits(corpus.units, 8).map((u) => u.id);
const b = m.selectRunUnits(corpus.units.slice().reverse(), 8).map((u) => u.id);
assert.deepEqual(a, b, "selectRunUnits must not depend on input order");

// (iv) arm B injects the calibration exactly once and states the enum
const p = m.buildCollapsedPlanPrompt(lenses, { target: "phase a/b\n\nbody", calibration: PLAN_SEVERITY_CALIBRATION });
assert.equal(p.split(PLAN_SEVERITY_CALIBRATION).length - 1, 1);
assert.ok(p.includes("Never emit `concern: unit-of-work`"));

// (v) a blended cross-lens rate is detected: the committed section is clean AND
//     a planted one is caught (the positive is what makes this non-vacuous).
assert.deepEqual(m.findBlendedLensKeys(JSON.parse(fs.readFileSync("docs/token-baseline.json", "utf8")).planFinderCollapse), []);
assert.deepEqual(m.findBlendedLensKeys({ observed: { overallRecall: 0.9 } }), ["$.observed.overallRecall"],
  "the blended-rate detector must fire on a planted cross-lens rate");

// (vi) the audit rejects a decision that disagrees with its own criteria table
const doc = JSON.parse(fs.readFileSync("docs/token-baseline.json", "utf8")).planFinderCollapse;
assert.ok(m.auditCollapseDoc(doc).ok, JSON.stringify(m.auditCollapseDoc(doc).errors));
assert.ok(!m.auditCollapseDoc({ ...doc, decision: "ship-collapsed" }).ok,
  "the audit must reject ship-collapsed against a failing criteria table");
' >/dev/null 2>&1)
}
if probe; then
    pass "10-control: the unmutated copy passes the mutation probe"
else
    fail "10-control: the unmutated copy FAILS the mutation probe — every mutation below is vacuous"
fi

mutate_and_expect_fail() {
    local tag="$1" desc="$2" fn="$3"
    cp "$MODULE" "$MUT/scripts/lib/finder-collapse.mjs"
    "$fn" || die "10-$tag: could not plant the mutation ($desc)"
    if probe; then
        fail "10-$tag: the probe still passed after $desc — that assertion group is vacuous"
    fi
    cp "$MODULE" "$MUT/scripts/lib/finder-collapse.mjs"
}

mut_floor() {
    sed 's/^export const MIN_PLAN_DOC_CHARS = 500;$/export const MIN_PLAN_DOC_CHARS = 0; \/\/ MUTANT/' \
        "$MODULE" >"$MUT/scripts/lib/finder-collapse.mjs"
    grep -q 'MUTANT' "$MUT/scripts/lib/finder-collapse.mjs"
}
mutate_and_expect_fail i 'dropping the plan-document floor to 0' mut_floor

mut_power() {
    sed 's/^  minUnits: 8,$/  minUnits: 1, \/\/ MUTANT/' "$MODULE" >"$MUT/scripts/lib/finder-collapse.mjs"
    grep -q 'MUTANT' "$MUT/scripts/lib/finder-collapse.mjs"
}
mutate_and_expect_fail ii 'dropping the unit floor to 1' mut_power

mut_select() {
    sed 's/^  const buckets = PLAN_TARGET_TYPES.map((t) => list.filter((u) => u.targetType === t));$/  const buckets = [list]; \/\/ MUTANT/' \
        "$MODULE" >"$MUT/scripts/lib/finder-collapse.mjs"
    grep -q 'MUTANT' "$MUT/scripts/lib/finder-collapse.mjs"
}
mutate_and_expect_fail iii 'removing the stratification from selectRunUnits' mut_select

mut_calibration() {
    sed 's/^  if (ctx.calibration) lines.push(ctx.calibration);$/  if (ctx.calibration) { lines.push(ctx.calibration); lines.push(ctx.calibration); } \/\/ MUTANT/' \
        "$MODULE" >"$MUT/scripts/lib/finder-collapse.mjs"
    grep -q 'MUTANT' "$MUT/scripts/lib/finder-collapse.mjs"
}
mutate_and_expect_fail iv 'injecting the severity calibration twice into arm B' mut_calibration

mut_uow() {
    sed "s/^    'Never emit \`concern: ' +\$/    'Emit whatever concern you like: ' + \/\/ MUTANT/" \
        "$MODULE" >"$MUT/scripts/lib/finder-collapse.mjs"
    grep -q 'MUTANT' "$MUT/scripts/lib/finder-collapse.mjs"
}
mutate_and_expect_fail v 'dropping the unit-of-work prohibition from arm B' mut_uow

mut_blend() {
    sed 's/^  const RATE_KEY = \/(recall|accuracy|materialShare|lossShare)\/i;$/  const RATE_KEY = \/__never__\/; \/\/ MUTANT/' \
        "$MODULE" >"$MUT/scripts/lib/finder-collapse.mjs"
    grep -q 'MUTANT' "$MUT/scripts/lib/finder-collapse.mjs"
}
mutate_and_expect_fail vi 'blinding the blended-cross-lens-rate detector' mut_blend

mut_audit() {
    sed "s/^      errors.push('decision is ship-collapsed but not every criterion passed');\$/      void 0; \/\/ MUTANT/" \
        "$MODULE" >"$MUT/scripts/lib/finder-collapse.mjs"
    grep -q 'MUTANT' "$MUT/scripts/lib/finder-collapse.mjs"
}
mutate_and_expect_fail vii 'letting the audit accept a ship decision its criteria table contradicts' mut_audit

# (viii) The MINER's --limit accounting, which section 4 asserts. Reverting the
# pre-classification `beyond-limit` bucket to a bare `break` — the shape that
# abandoned the boundary unit's always-on records AND every later unit into no
# bucket at all, while finderRecordCount kept reporting them — must break the
# under-truncation accounting identity. Without this, section 4's --limit
# assertion could be passing vacuously.
miner_limit_identity_holds() {
    node "$1" --root "$SIDECARS" --project-slug -Users-edward-Projects-rdm-fixture --limit 1 \
        --format json >"$TMP/mut-limited.json" 2>/dev/null || return 1
    # shellcheck disable=SC2016  # a Node program, deliberately not shell-expanded
    node --input-type=module -e '
import fs from "node:fs";
const r = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
const skipped = Object.values(r.skips).reduce((a, b) => a + b, 0);
process.exit(r.units.length * 3 + skipped === r.finderRecordCount ? 0 : 1);
' "$TMP/mut-limited.json"
}
sed "s|^      bumpUnit('beyond-limit');\$|      break; // MUTANT|" "$MINER" >"$MUT/scripts/mine-plan-finder-corpus.mjs"
grep -q 'MUTANT' "$MUT/scripts/mine-plan-finder-corpus.mjs" ||
    die "10-viii: could not plant the miner --limit mutation"
if miner_limit_identity_holds "$MUT/scripts/mine-plan-finder-corpus.mjs"; then
    fail "10-viii: the accounting identity still closed after --limit was reverted to a bare break"
fi
cp "$MINER" "$MUT/scripts/mine-plan-finder-corpus.mjs"
miner_limit_identity_holds "$MINER" ||
    fail "10-viii-control: the unmutated miner does not close the identity under --limit"

[ "$FAILURES" = "$MUT_BEFORE" ] &&
    pass "10: all eight mutations flip an assertion and the controls pass — sections 2-8 are non-vacuous"

# ---------------------------------------------------------------------------
if [ "$FAILURES" -ne 0 ]; then
    printf '\nverify-finder-collapse.sh: %d FAILURE(S)\n' "$FAILURES" >&2
    exit 1
fi
say "verify-finder-collapse.sh: ALL GREEN"
