#!/bin/sh
# Hermetic regression for the token-lane measurement tool
# (scripts/lib/token-report.mjs + scripts/measure-lane-tokens.mjs).
#
# This tool is the prerequisite measurement harness every later phase of the
# workflow-token-reduction roadmap depends on to substantiate a token-saving
# claim: it locates Claude Code Workflow session sidecars across every
# project-slug directory under a `--root` (including `--worktrees-`-named
# ones), joins each `wf_*.json` run's `workflowProgress[]` agent entries with
# their `subagents/workflows/<runId>/agent-*.jsonl` transcripts (deduping
# usage by `requestId` with last-write-wins semantics), and reports
# token-class-broken-out totals (output / uncached input / cache write /
# cache read) grouped by agent class, full label, model, and workflow,
# alongside an explicit, never-reconciled sidecar-totalTokens-vs-deduped-sum
# discrepancy line.
#
# This harness gates:
#
#   1. TOOL/HYGIENE GUARDS — node resolves (PATH or mise), no package.json /
#      node_modules were introduced, and neither source file falls back to
#      the real `~/.claude` home directory outside the one guarded
#      `defaultProjectsRoot()` definition (grep-based, catches an accidental
#      regression back to touching real data).
#   2. END-TO-END FIXTURE COMPARISON — the real CLI, run against a mktemp
#      scratch copy of the checked-in fixture tree, recomputes per-class /
#      per-grouping totals that match tests/fixtures/token-sidecar's hand
#      -computed expected-totals.json exactly, in both --format json and
#      --format text (the discrepancy line specifically), including the
#      per-agent-class first-request floor (floorByAgentClass): exact
#      n/min/p10/median/mean for the fetch/find/plan classes, and the
#      gate/implement/refute/stamp classes (every record sidecarOnly or
#      cached) being ABSENT from it rather than present with n:0.
#   3. DIRECT LIB BEHAVIOR — parseAgentTranscript's requestId dedupe
#      (last-write-wins, not first-write-wins or summed), a genuinely
#      unreadable transcript and a transcript with zero usable assistant
#      lines each degrading to a distinct, non-throwing sidecarOnly warning
#      rather than crashing or being silently indistinguishable from the
#      "no transcript file at all" case, locateSessionDirs walking into
#      the `--worktrees-`-named project-slug directory, and buildRecords
#      giving every cached/sidecarOnly record firstRequestTokens === null
#      (not 0, not the sidecar tokens value) while a multi-request
#      transcript's firstRequestTokens comes from its FIRST request only.
#   4. CLI ARGUMENT VALIDATION — a value-taking flag with its value omitted
#      (or immediately followed by another flag) fails loudly instead of
#      silently filtering every run out of the report.
#   5. PLANTED-MUTATION SELF-TESTS — four independent mutations (dedupe
#      collapsed into first-write-wins; the totalsDiscrepancy delta silently
#      reconciled to 0; the first-request floor read from the LAST perRequest
#      entry instead of the first; cached/sidecarOnly records' firstRequestTokens
#      set to 0 instead of null) are each applied to a scratch copy of the
#      source and proven to flip the fixture comparison from MATCH to FAIL,
#      proving the comparisons in section 2 are not vacuous.
#   6. REFUTER-SEVERITY MEASUREMENT — scripts/measure-refuter-severity.mjs,
#      the second instrument over the same library, which breaks refuter
#      spend out by the SEVERITY of the finding each refuter graded (the
#      dimension the single `refute` agent-class bucket cannot see, and the
#      evidence behind the non-gating-refutation skip). Gated against its own
#      hermetic fixture (exact per-severity agent counts, verdict tallies and
#      all four token classes, including a braced-target prompt that defeats a
#      naive extractor and a transcript-less refuter that must NOT count as
#      non-gating), its --check path, a corpus-free --audit of the COMMITTED
#      figures in docs/token-baseline.json, a pin of its NON_GATING_SEVERITIES
#      to the canonical review source, and two planted mutations (an edited doc
#      figure; a broken extractor) proving neither check is vacuous.
#   7. CHANGELOG HYGIENE — the same commit that touches
#      scripts/lib/token-report.mjs / scripts/measure-lane-tokens.mjs /
#      scripts/measure-refuter-severity.mjs also touches CHANGELOG.md, so a
#      user-facing change is never landed without its changelog entry.
#
# Node is stdlib-only (node:assert, node:fs, node:path); no package.json /
# node_modules / third-party packages anywhere. node is pinned in .mise.toml.
#
# Requires: node (via PATH or `mise exec node --`).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)

LIB="$REPO_ROOT/scripts/lib/token-report.mjs"
CLI="$REPO_ROOT/scripts/measure-lane-tokens.mjs"
FIXTURE_DIR="$REPO_ROOT/tests/fixtures/token-sidecar"
EXPECTED="$FIXTURE_DIR/expected-totals.json"

say() { printf '\n\033[1;34m==>\033[0m %s\n' "$*"; }
fail() {
    printf '\n\033[1;31m[FAIL]\033[0m %s\n' "$*" >&2
    exit 1
}
pass() { printf '\033[1;32m[ok]\033[0m %s\n' "$*"; }

[ -f "$LIB" ] || fail "source module not found: $LIB"
[ -f "$CLI" ] || fail "CLI not found: $CLI"
[ -d "$FIXTURE_DIR" ] || fail "fixture tree not found: $FIXTURE_DIR"
[ -f "$EXPECTED" ] || fail "expected-totals fixture not found: $EXPECTED"

# Resolve a node command: prefer PATH, fall back to the mise-pinned toolchain.
# Fail hard if node is genuinely unavailable — a silent skip would turn this
# gate into a no-op (matches the sibling verify-*.sh harnesses' convention).
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

# ==============================================================================
say "1. Tool/hygiene guards"
# ==============================================================================

run_node --check "$LIB" || fail "node --check failed on $LIB"
run_node --check "$CLI" || fail "node --check failed on $CLI"
pass "both modules parse cleanly under the pinned node"

if [ -e "$REPO_ROOT/scripts/package.json" ] || [ -e "$REPO_ROOT/scripts/node_modules" ]; then
    fail "a package.json/node_modules was introduced under scripts/ — this tool must stay stdlib-only"
fi
pass "no package.json/node_modules introduced under scripts/"

# Exactly one os.homedir() call in the library (inside defaultProjectsRoot),
# and none at all in the CLI (it goes through the library's guarded default,
# only when --root was not supplied).
HOMEDIR_LIB_COUNT=$(grep -c 'homedir()' "$LIB" || true)
[ "$HOMEDIR_LIB_COUNT" -eq 1 ] || fail "expected exactly one os.homedir() call in $LIB, found $HOMEDIR_LIB_COUNT"
grep -A3 'export function defaultProjectsRoot' "$LIB" | grep -q 'homedir()' ||
    fail "the single os.homedir() call in $LIB must live inside defaultProjectsRoot()"
if grep -q 'homedir(' "$CLI"; then
    fail "$CLI must not call os.homedir() directly — it must go through the library's defaultProjectsRoot()"
fi
if grep -qE "\\\$HOME/\\.claude" "$CLI"; then
    fail "$CLI must not hardcode a literal \$HOME/.claude path"
fi
pass "no unguarded os.homedir()/\$HOME/.claude access outside the one guarded defaultProjectsRoot() definition"

# ==============================================================================
say "2. End-to-end fixture comparison (real CLI, mktemp scratch copy of the fixture)"
# ==============================================================================

SCRATCH_FIXTURE="$TMP/fixture"
cp -R "$FIXTURE_DIR" "$SCRATCH_FIXTURE"
# Never let the scratch copy be mistaken for the checked-in ground truth.
rm -f "$SCRATCH_FIXTURE/expected-totals.json"

run_node "$CLI" --root "$SCRATCH_FIXTURE" --format json --out "$TMP/actual-report.json" ||
    fail "measure-lane-tokens.mjs failed against the fixture"
[ -s "$TMP/actual-report.json" ] || fail "CLI produced no output"

cat >"$TMP/compare.mjs" <<'NODE_COMPARE'
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const [, , reportPath, expectedPath] = process.argv;
const report = JSON.parse(readFileSync(reportPath, 'utf8'));
const expected = JSON.parse(readFileSync(expectedPath, 'utf8'));

const CLASS_FIELDS = ['agentCount', 'dedupedRequestCount', 'output', 'uncachedInput', 'cacheWrite', 'cacheRead'];

function toKeyedMap(arr) {
  const m = {};
  for (const row of arr) m[row.key] = row;
  return m;
}

function checkGroup(actualArr, expectedObj, label) {
  const actualMap = toKeyedMap(actualArr);
  for (const [key, exp] of Object.entries(expectedObj)) {
    const act = actualMap[key];
    assert.ok(act, `${label} missing expected key "${key}"`);
    for (const field of CLASS_FIELDS) {
      assert.equal(act[field], exp[field], `${label}.${key}.${field}: expected ${exp[field]}, got ${act[field]}`);
    }
  }
}

checkGroup(report.byAgentClass, expected.byAgentClass, 'byAgentClass');
checkGroup(report.byLabel, expected.byLabel, 'byLabel');
checkGroup(report.byModel, expected.byModel, 'byModel');
checkGroup(report.byWorkflow, expected.byWorkflow, 'byWorkflow');

// floorByAgentClass: exact n/min/p10/median/mean for the eligible classes,
// AND the sidecarOnly/cached-only classes must be entirely absent (not
// present with n:0) — this is the AC2/AC8 population check, not just a
// values check.
const FLOOR_FIELDS = ['n', 'minTokens', 'p10Tokens', 'medianTokens', 'meanTokens'];
const floorMap = toKeyedMap(report.floorByAgentClass);
for (const [key, exp] of Object.entries(expected.floorByAgentClass)) {
  const act = floorMap[key];
  assert.ok(act, `floorByAgentClass missing expected key "${key}"`);
  for (const field of FLOOR_FIELDS) {
    assert.equal(act[field], exp[field], `floorByAgentClass.${key}.${field}: expected ${exp[field]}, got ${act[field]}`);
  }
}
for (const key of expected.floorByAgentClassAbsentKeys ?? []) {
  assert.ok(
    !(key in floorMap),
    `floorByAgentClass must NOT contain key "${key}" (every record in that class is cached/sidecarOnly)`,
  );
}

assert.equal(report.runsConsidered, expected.runsConsidered, 'runsConsidered');
assert.equal(report.recordCount, expected.recordCount, 'recordCount');

assert.equal(
  report.totalsDiscrepancy.sidecarTotalTokens,
  expected.totalsDiscrepancy.sidecarTotalTokens,
  'totalsDiscrepancy.sidecarTotalTokens',
);
assert.equal(
  report.totalsDiscrepancy.dedupedTotalTokens,
  expected.totalsDiscrepancy.dedupedTotalTokens,
  'totalsDiscrepancy.dedupedTotalTokens',
);
assert.equal(report.totalsDiscrepancy.delta, expected.totalsDiscrepancy.delta, 'totalsDiscrepancy.delta');

console.log('MATCH');
NODE_COMPARE

run_node "$TMP/compare.mjs" "$TMP/actual-report.json" "$EXPECTED" || fail "recomputed totals mismatch expected-totals.json"
pass "recomputed per-class / per-grouping totals match expected-totals.json exactly (--format json)"

# AC4: the 'refute' agent-class bucket must actually roll up 2+ distinct
# full labels (proving the rollup collapses siblings, not a no-op alias) and
# by-model/by-workflow must both surface entries from BOTH runs.
run_node -e "
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const report = JSON.parse(readFileSync('$TMP/actual-report.json', 'utf8'));
const refuteLabels = report.byLabel.filter((r) => r.key.startsWith('refute:'));
assert.ok(refuteLabels.length >= 2, 'expected >=2 distinct refute: labels, got ' + refuteLabels.length);
const refuteClass = report.byAgentClass.find((r) => r.key === 'refute');
assert.ok(refuteClass, 'no refute agent-class bucket found');
const summedAgentCount = refuteLabels.reduce((s, r) => s + r.agentCount, 0);
assert.equal(refuteClass.agentCount, summedAgentCount, 'refute class agentCount does not equal the sum of its labels');
const workflowKeys = report.byWorkflow.map((r) => r.key).sort();
assert.deepEqual(workflowKeys, ['autopilot', 'dispatch-phase'], 'expected both run001 (dispatch-phase) and run002 (autopilot) workflows present');
const modelKeys = report.byModel.map((r) => r.key).sort();
assert.ok(modelKeys.includes('claude-sonnet-5') && modelKeys.includes('haiku'), 'expected models from both runs present');
console.log('AC4 grouping checks: ok');
" || fail "AC4 grouping assertions failed"
pass "agent-class rollup collapses siblings; by-model/by-workflow surface entries from both runs"

# AC6: the --worktrees- named project-slug directory's run is present.
run_node -e "
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const report = JSON.parse(readFileSync('$TMP/actual-report.json', 'utf8'));
const autopilot = report.byWorkflow.find((r) => r.key === 'autopilot');
assert.ok(autopilot, 'no autopilot (worktree-run) entry in byWorkflow');
assert.ok(autopilot.agentCount > 0, 'autopilot entry has zero agentCount');
console.log('AC6 worktree-locator check: ok');
" || fail "worktree project-slug directory run missing from aggregated output"
pass "the --worktrees- project-slug directory's run is present in the aggregated output"

# Text format: assert the named discrepancy line is present, unconditionally,
# with the exact hand-computed figures.
run_node "$CLI" --root "$SCRATCH_FIXTURE" --format text >"$TMP/actual-report.txt" ||
    fail "measure-lane-tokens.mjs (text format) failed against the fixture"
grep -q 'Sidecar totalTokens vs deduped-sum discrepancy: 1020 (sidecar=9200, deduped=8180)' "$TMP/actual-report.txt" ||
    fail "text-format report is missing the exact expected discrepancy line"
pass "--format text carries the exact named discrepancy line"

grep -q -- '-- Per-agent-class first-request floor --' "$TMP/actual-report.txt" ||
    fail "text-format report is missing the '-- Per-agent-class first-request floor --' heading"
grep -q 'fetch  n=1 min=800 p10=800 median=800 mean=800' "$TMP/actual-report.txt" ||
    fail "text-format report is missing the exact expected fetch floor line"
pass "--format text carries the per-agent-class first-request floor section with the exact fetch line"

# ==============================================================================
say "3. Direct lib behavior: dedupe, no-throw degradation, worktree locator"
# ==============================================================================

cat >"$TMP/lib-behavior.mjs" <<NODE_TEST
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {
  parseAgentTranscript,
  buildRecords,
  findWorkflowRunFiles,
  locateSessionDirs,
  agentClassFromLabel,
  floorByAgentClass,
} from '$LIB';

// --- requestId dedupe: last-write-wins on the planted duplicate (req-A) ---
const agent1Path = '$SCRATCH_FIXTURE/-Users-edward-Projects-rdm/sess-fixture-a/subagents/workflows/wf_run001/agent-agent1.jsonl';
const t1 = parseAgentTranscript(agent1Path);
assert.equal(t1.ok, true);
assert.equal(t1.dedupedRequestCount, 2, 'expected 2 distinct requestIds (req-A, req-B), not 3 lines');
assert.equal(t1.perRequest.get('req-A').usage.outputTokens, 50, 'req-A must resolve to the LAST line\\'s output_tokens (50), not the first (10) or a sum (60)');
assert.equal(t1.perRequest.get('req-B').usage.outputTokens, 120);
console.log('dedupe: ok');

// --- firstRequestTokens: first-request floor field on real fixture records ---
const fixtureSessions = locateSessionDirs('$SCRATCH_FIXTURE');
const fixtureRunFiles = findWorkflowRunFiles(fixtureSessions);
const fixtureRecords = buildRecords(fixtureRunFiles);

const agent1Record = fixtureRecords.find((r) => r.agentId === 'agent1');
assert.equal(
  agent1Record.firstRequestTokens,
  800,
  'agent1 (fetch:task-meta) firstRequestTokens must be req-A\\'s 100+500+200=800, not req-B\\'s 380 and not the two-request sum 1180',
);

const cachedRecord = fixtureRecords.find((r) => r.agentId === 'agent5');
assert.equal(cachedRecord.cached, true);
assert.equal(cachedRecord.firstRequestTokens, null, 'cached:true record (stamp:in-progress/agent5) must have firstRequestTokens === null, not 0');

const sidecarOnlyIds = ['agent4', 'agent-fc', 'agent-ra1', 'agent-rc1'];
for (const id of sidecarOnlyIds) {
  const rec = fixtureRecords.find((r) => r.agentId === id);
  assert.ok(rec, 'expected a fixture record for agentId ' + id);
  assert.equal(rec.sidecarOnly, true, id + ' must be a sidecarOnly record');
  assert.equal(rec.firstRequestTokens, null, id + ' (sidecarOnly) must have firstRequestTokens === null, not 0 and not its sidecar tokens value');
}
console.log('firstRequestTokens (measured, cached, sidecarOnly): ok');

// --- floorByAgentClass: right population, right classes absent ---
const fixtureFloor = floorByAgentClass(fixtureRecords);
const fixtureFloorMap = Object.fromEntries(fixtureFloor.map((r) => [r.key, r]));
assert.equal(fixtureFloorMap.fetch.n, 1);
assert.equal(fixtureFloorMap.fetch.medianTokens, 800);
assert.equal(fixtureFloorMap.find.n, 1);
assert.equal(fixtureFloorMap.find.medianTokens, 3040);
assert.equal(fixtureFloorMap.plan.n, 1);
assert.equal(fixtureFloorMap.plan.medianTokens, 360);
for (const absentKey of ['gate', 'implement', 'refute', 'stamp']) {
  assert.ok(!(absentKey in fixtureFloorMap), 'floorByAgentClass must not contain "' + absentKey + '" (every record in that class is cached/sidecarOnly)');
}
console.log('floorByAgentClass population: ok');

// --- percentile interpolation: the fixture gives every agentClass n=1,
// which short-circuits percentile() at 'if (n === 1) return sorted[0]'
// before the linear-interpolation arithmetic ever runs. Feed
// floorByAgentClass a synthetic n=4 class so p10/median both land on a
// non-integer rank and exercise the interpolation branch directly, with
// hand-computed expectations:
//   sorted = [10, 20, 30, 40]
//   p10:    idx = (4-1)*0.1 = 0.3  -> lower=0 upper=1 weight=0.3
//           -> 10 + (20-10)*0.3 = 13
//   median: idx = (4-1)*0.5 = 1.5  -> lower=1 upper=2 weight=0.5
//           -> 20 + (30-20)*0.5 = 25
const syntheticRecords = [10, 20, 30, 40].map((v, i) => ({
  agentClass: 'synthtest',
  agentId: 'synth' + i,
  firstRequestTokens: v,
}));
const syntheticFloor = floorByAgentClass(syntheticRecords);
const synthMap = Object.fromEntries(syntheticFloor.map((r) => [r.key, r]));
assert.equal(synthMap.synthtest.n, 4);
assert.equal(synthMap.synthtest.minTokens, 10);
assert.equal(synthMap.synthtest.p10Tokens, 13, 'p10 must be linearly interpolated (10 + (20-10)*0.3 = 13), not floored/ceiled to 10 or 20');
assert.equal(synthMap.synthtest.medianTokens, 25, 'median must be linearly interpolated (20 + (30-20)*0.5 = 25), not floored/ceiled to 20 or 30');
assert.equal(synthMap.synthtest.meanTokens, 25);
console.log('percentile interpolation (multi-record class): ok');

// --- agentClassFromLabel edge case: no colon at all ---
assert.equal(agentClassFromLabel('malformed'), 'malformed');
assert.equal(agentClassFromLabel(''), '');
console.log('agentClassFromLabel edge cases: ok');

// --- locateSessionDirs walks into the --worktrees- named project-slug dir ---
const sessions = locateSessionDirs('$SCRATCH_FIXTURE');
const worktreeSessions = sessions.filter((s) => s.projectSlug.includes('--worktrees-'));
assert.ok(worktreeSessions.length >= 1, 'expected at least one session under a --worktrees- project-slug dir');
console.log('locateSessionDirs worktree walk: ok');

// --- buildRecords: a genuinely unreadable transcript degrades to sidecarOnly
//     with a distinct warning, rather than throwing or being silently
//     indistinguishable from "no transcript file at all" (cf-1) ---
const scratchSessionDir = '$TMP/synthetic-session';
const unreadableDir = path.join(scratchSessionDir, 'subagents', 'workflows', 'run-synth');
fs.mkdirSync(unreadableDir, { recursive: true });
const unreadablePath = path.join(unreadableDir, 'agent-unreadable.jsonl');
fs.writeFileSync(unreadablePath, '{"type":"assistant","requestId":"req-x","message":{"model":"m","usage":{"output_tokens":1}}}\\n');
fs.chmodSync(unreadablePath, 0o000);

const emptyPath = path.join(unreadableDir, 'agent-empty.jsonl');
fs.writeFileSync(emptyPath, '{"type":"user","message":{"role":"user","content":"no assistant lines here"}}\\n');

const syntheticRunFiles = [
  {
    projectSlug: 'synthetic',
    sessionId: 'sess-synth',
    sessionDir: scratchSessionDir,
    run: {
      runId: 'run-synth',
      workflowName: 'synthetic-wf',
      agents: [
        { label: 'find:unreadable', agentId: 'unreadable', model: 'm', tokens: 77 },
        { label: 'find:empty', agentId: 'empty', model: 'm', tokens: 55 },
      ],
    },
  },
];

let readErrorWarned = false;
let emptyTranscriptWarned = false;
const warnings = [];
const records = buildRecords(syntheticRunFiles, {
  warn: (msg) => {
    warnings.push(msg);
    if (msg.includes('agent-unreadable') || (msg.includes('unreadable') && msg.includes('failed to read'))) readErrorWarned = true;
    if (msg.includes('empty') && msg.includes('empty transcript')) emptyTranscriptWarned = true;
  },
});

assert.equal(records.length, 2);
const unreadableRecord = records.find((r) => r.agentId === 'unreadable');
const emptyRecord = records.find((r) => r.agentId === 'empty');
assert.equal(unreadableRecord.sidecarOnly, true, 'unreadable transcript must degrade to sidecarOnly, not throw');
assert.equal(unreadableRecord.output, 77, 'sidecarOnly fallback must use the sidecar tokens scalar');
assert.equal(emptyRecord.sidecarOnly, true, 'empty transcript must degrade to sidecarOnly, not throw');
assert.equal(emptyRecord.output, 55);
assert.ok(readErrorWarned, 'expected a distinct warning for the unreadable-transcript case, got: ' + JSON.stringify(warnings));
assert.ok(emptyTranscriptWarned, 'expected a distinct warning for the empty-transcript case, got: ' + JSON.stringify(warnings));
assert.notEqual(warnings[0], warnings[1], 'the two degraded-fallback cases must produce DISTINCT warning messages');
console.log('no-throw degradation (unreadable + empty transcript, distinct warnings): ok');

console.log('ALL LIB BEHAVIOR ASSERTIONS PASSED');
NODE_TEST

run_node "$TMP/lib-behavior.mjs" || fail "direct lib behavior assertions failed"
pass "dedupe, edge cases, worktree walk, and no-throw degradation all behave correctly"

# ==============================================================================
say "4. CLI argument validation (cf-2): an omitted flag value fails loudly"
# ==============================================================================

if run_node "$CLI" --root "$SCRATCH_FIXTURE" --workflow >"$TMP/omitted-value.out" 2>&1; then
    fail "CLI must fail when --workflow's value is omitted, not silently filter every run out"
fi
grep -qi 'requires a value' "$TMP/omitted-value.out" || fail "omitted-value error is not actionable: $(cat "$TMP/omitted-value.out")"
pass "an omitted flag value fails loudly with an actionable message"

if run_node "$CLI" --root "$SCRATCH_FIXTURE" --since --format json >"$TMP/flag-as-value.out" 2>&1; then
    fail "CLI must not silently swallow a following flag as another flag's value"
fi
grep -qi 'requires a value' "$TMP/flag-as-value.out" || fail "flag-as-value error is not actionable: $(cat "$TMP/flag-as-value.out")"
pass "a flag immediately following a value-taking flag is rejected, not silently consumed"

# --out to a nonexistent parent directory fails actionably (edge case).
if run_node "$CLI" --root "$SCRATCH_FIXTURE" --out "$TMP/does-not-exist/out.json" >"$TMP/bad-out.out" 2>&1; then
    fail "CLI must fail when --out's parent directory does not exist"
fi
grep -qi 'does not exist' "$TMP/bad-out.out" || fail "--out error is not actionable: $(cat "$TMP/bad-out.out")"
pass "--out to a missing parent directory fails actionably"

# ==============================================================================
say "5. Planted-mutation self-tests (dedupe removal; discrepancy reconciliation)"
# ==============================================================================

# Build a scratch copy of the scripts tree (lib + CLI) so mutations never
# touch the real checked-in source.
MUT1="$TMP/mutant-dedupe"
mkdir -p "$MUT1/lib"
cp "$CLI" "$MUT1/measure-lane-tokens.mjs"
sed 's/perRequest\.set(requestId, {/if (perRequest.has(requestId)) continue; perRequest.set(requestId, {/' \
    "$LIB" >"$MUT1/lib/token-report.mjs"
if diff -q "$LIB" "$MUT1/lib/token-report.mjs" >/dev/null 2>&1; then
    fail "self-test setup: mutation 1 (first-write-wins) did not change the source — sed pattern did not match"
fi

if run_node "$MUT1/measure-lane-tokens.mjs" --root "$SCRATCH_FIXTURE" --format json --out "$TMP/mutant1-report.json" >"$TMP/mutant1.log" 2>&1; then
    if run_node "$TMP/compare.mjs" "$TMP/mutant1-report.json" "$EXPECTED" >/dev/null 2>&1; then
        fail "planted-mutation self-test 1 FAILED TO CATCH: removing requestId dedupe (first-write-wins) still matched expected-totals.json"
    fi
fi
pass "planted-mutation self-test 1: collapsing dedupe to first-write-wins flips the fixture comparison to FAIL"

MUT2="$TMP/mutant-discrepancy"
mkdir -p "$MUT2/lib"
cp "$CLI" "$MUT2/measure-lane-tokens.mjs"
sed 's/delta: sidecarTotalTokens - dedupedTotalTokens,/delta: 0,/' "$LIB" >"$MUT2/lib/token-report.mjs"
if diff -q "$LIB" "$MUT2/lib/token-report.mjs" >/dev/null 2>&1; then
    fail "self-test setup: mutation 2 (silent reconciliation) did not change the source — sed pattern did not match"
fi

if run_node "$MUT2/measure-lane-tokens.mjs" --root "$SCRATCH_FIXTURE" --format json --out "$TMP/mutant2-report.json" >"$TMP/mutant2.log" 2>&1; then
    if run_node "$TMP/compare.mjs" "$TMP/mutant2-report.json" "$EXPECTED" >/dev/null 2>&1; then
        fail "planted-mutation self-test 2 FAILED TO CATCH: silently reconciling the discrepancy delta to 0 still matched expected-totals.json"
    fi
fi
pass "planted-mutation self-test 2: silently reconciling the discrepancy delta flips the fixture comparison to FAIL"

MUT3="$TMP/mutant-floor-last"
mkdir -p "$MUT3/lib"
cp "$CLI" "$MUT3/measure-lane-tokens.mjs"
sed 's/transcript\.perRequest\.values()\.next()\.value/[...transcript.perRequest.values()].at(-1)/' \
    "$LIB" >"$MUT3/lib/token-report.mjs"
if diff -q "$LIB" "$MUT3/lib/token-report.mjs" >/dev/null 2>&1; then
    fail "self-test setup: mutation 3 (last-request floor) did not change the source — sed pattern did not match"
fi

if run_node "$MUT3/measure-lane-tokens.mjs" --root "$SCRATCH_FIXTURE" --format json --out "$TMP/mutant3-report.json" >"$TMP/mutant3.log" 2>&1; then
    if run_node "$TMP/compare.mjs" "$TMP/mutant3-report.json" "$EXPECTED" >/dev/null 2>&1; then
        fail "planted-mutation self-test 3 FAILED TO CATCH: reading the first-request floor from the LAST perRequest entry instead of the first still matched expected-totals.json"
    fi
fi
pass "planted-mutation self-test 3: reading the floor from the last (not first) transcript request flips the fixture comparison to FAIL"

MUT4="$TMP/mutant-floor-zero"
mkdir -p "$MUT4/lib"
cp "$CLI" "$MUT4/measure-lane-tokens.mjs"
sed 's/firstRequestTokens: null,/firstRequestTokens: 0,/' "$LIB" >"$MUT4/lib/token-report.mjs"
if diff -q "$LIB" "$MUT4/lib/token-report.mjs" >/dev/null 2>&1; then
    fail "self-test setup: mutation 4 (cached/sidecarOnly exclusion removed) did not change the source — sed pattern did not match"
fi
# Both the cached branch and the sidecarOnly branch share the literal
# "firstRequestTokens: null," text, so the plain (non-`g`) sed above already
# mutates both occurrences (each on its own line) — confirm that here rather
# than assuming it.
NULL_COUNT_ORIG=$(grep -c 'firstRequestTokens: null,' "$LIB" || true)
NULL_COUNT_MUT=$(grep -c 'firstRequestTokens: null,' "$MUT4/lib/token-report.mjs" || true)
[ "$NULL_COUNT_ORIG" -eq 2 ] || fail "self-test setup: expected exactly 2 'firstRequestTokens: null,' occurrences in $LIB (cached + sidecarOnly branches), found $NULL_COUNT_ORIG"
[ "$NULL_COUNT_MUT" -eq 0 ] || fail "self-test setup: mutation 4 left $NULL_COUNT_MUT 'firstRequestTokens: null,' occurrence(s) unmutated"

if run_node "$MUT4/measure-lane-tokens.mjs" --root "$SCRATCH_FIXTURE" --format json --out "$TMP/mutant4-report.json" >"$TMP/mutant4.log" 2>&1; then
    if run_node "$TMP/compare.mjs" "$TMP/mutant4-report.json" "$EXPECTED" >/dev/null 2>&1; then
        fail "planted-mutation self-test 4 FAILED TO CATCH: giving cached/sidecarOnly records firstRequestTokens: 0 instead of null still matched expected-totals.json"
    fi
fi
pass "planted-mutation self-test 4: removing the cached/sidecarOnly firstRequestTokens exclusion flips the fixture comparison to FAIL"

# ==============================================================================
say "6. Refuter-severity measurement (scripts/measure-refuter-severity.mjs)"
# ==============================================================================
# The phase-6 "stop refuting findings that cannot change the outcome" change is
# justified by a MEASURED per-severity breakdown of refuter spend, which the
# per-agent-class report above cannot see (it has one undifferentiated `refute`
# bucket). This section gates that second instrument the same way section 2
# gates the first: a hermetic fixture with hand-checkable numbers, its --check
# path, a corpus-free --audit of the COMMITTED figures in docs/token-baseline.json,
# and planted mutations proving neither check is vacuous.

REFSEV="$REPO_ROOT/scripts/measure-refuter-severity.mjs"
REFSEV_FIXTURE="$REPO_ROOT/tests/fixtures/token-refuter-severity"
REFSEV_EXPECTED="$REFSEV_FIXTURE/expected-nonGatingRefutationSkip.json"
BASELINE_DOC="$REPO_ROOT/docs/token-baseline.json"

[ -f "$REFSEV" ] || fail "refuter-severity instrument not found: $REFSEV"
[ -d "$REFSEV_FIXTURE" ] || fail "refuter-severity fixture not found: $REFSEV_FIXTURE"
[ -f "$REFSEV_EXPECTED" ] || fail "refuter-severity expected-figures fixture not found: $REFSEV_EXPECTED"
[ -f "$BASELINE_DOC" ] || fail "token baseline doc not found: $BASELINE_DOC"

run_node --check "$REFSEV" || fail "node --check failed on $REFSEV"
if grep -q 'homedir(' "$REFSEV"; then
    fail "$REFSEV must not call os.homedir() directly — it must go through the library's defaultProjectsRoot()"
fi
# Determinism: the same corpus in must give the same numbers out.
# Strip comment lines first: the module header DOCUMENTS this rule, and a naive
# grep would fire on the documentation instead of on real code.
if grep -vE '^[[:space:]]*(//|\*|/\*)' "$REFSEV" | grep -qE 'Date\.now\(|Math\.random\('; then
    fail "$REFSEV must be deterministic — no Date.now()/Math.random()"
fi
pass "the instrument parses, is deterministic, and has no unguarded home-directory access"

# The skip set is duplicated across two runtimes that cannot import each other
# (the Workflow lib and this Node script), so pin them to each other here rather
# than trusting the copy.
REVIEW_LIB="$REPO_ROOT/.claude/workflows/lib/review.mjs"
[ -f "$REVIEW_LIB" ] || fail "canonical review source not found: $REVIEW_LIB"
LIB_NONGATING=$(grep -F 'const NON_GATING_SEVERITIES =' "$REVIEW_LIB" | head -1 | sed 's/.*= //; s/;$//')
SCRIPT_NONGATING=$(grep -F 'const NON_GATING_SEVERITIES =' "$REFSEV" | head -1 | sed 's/.*= //; s/;$//')
[ -n "$LIB_NONGATING" ] || fail "could not read NON_GATING_SEVERITIES from $REVIEW_LIB"
[ "$LIB_NONGATING" = "$SCRIPT_NONGATING" ] ||
    fail "NON_GATING_SEVERITIES drifted: $REVIEW_LIB has $LIB_NONGATING, $REFSEV has $SCRIPT_NONGATING"
pass "the instrument's NON_GATING_SEVERITIES matches the canonical review source ($LIB_NONGATING)"

# --- fixture comparison: exact per-severity agent counts and token classes ----
REFSEV_SCRATCH="$TMP/refsev-fixture"
cp -R "$REFSEV_FIXTURE" "$REFSEV_SCRATCH"
rm -f "$REFSEV_SCRATCH/expected-nonGatingRefutationSkip.json"

run_node "$REFSEV" --root "$REFSEV_SCRATCH" --format json >"$TMP/refsev-report.json" ||
    fail "measure-refuter-severity.mjs failed against the fixture"

cat >"$TMP/refsev-compare.mjs" <<'NODE_REFSEV_COMPARE'
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const [, , reportPath, expectedPath] = process.argv;
const report = JSON.parse(readFileSync(reportPath, 'utf8'));
const expected = JSON.parse(readFileSync(expectedPath, 'utf8')).nonGatingRefutationSkip;

const ROW_FIELDS = ['agentCount', 'graded', 'refuted', 'refutedRate', 'output', 'uncachedInput', 'cacheWrite', 'cacheRead'];
const byKey = Object.fromEntries(report.refuteBySeverity.map((r) => [r.key, r]));

assert.deepEqual(
  report.refuteBySeverity.map((r) => r.key),
  expected.refuteBySeverity.map((r) => r.key),
  'the fixture yields exactly the expected severity rows, in the expected order'
);
for (const exp of expected.refuteBySeverity) {
  const act = byKey[exp.key];
  assert.ok(act, `missing severity row "${exp.key}"`);
  for (const f of ROW_FIELDS) {
    assert.equal(act[f], exp[f], `refuteBySeverity.${exp.key}.${f}: expected ${exp[f]}, got ${act[f]}`);
  }
}

// The finding whose refuter prompt embeds a TARGET containing braces (the
// --implementation-plan shape) must still resolve — it is the extraction edge
// case that defeated the ad hoc first-pass parser.
assert.equal(byKey.blocking.agentCount, 1, 'the braced-target refuter resolved to its real severity, not "unparseable"');
assert.ok(!('unrecoverable:unparseable' in byKey), 'no fixture refuter falls into the unparseable bucket');

// A refuter with no transcript at all is reported as unrecoverable and is NEVER
// counted toward the projected drop — an unknown severity is not a non-gating one.
assert.equal(byKey['unrecoverable:no-transcript'].agentCount, 1, 'the transcript-less refuter is bucketed as unrecoverable');
assert.equal(byKey['unrecoverable:no-transcript'].graded, 0, 'an unrecoverable refuter grades nothing');

for (const f of ['agentCount', 'output', 'uncachedInput', 'cacheWrite', 'cacheRead']) {
  assert.equal(report.refuteTotals[f], expected.refuteTotals[f], `refuteTotals.${f}`);
  assert.equal(report.laneTotals[f], expected.laneTotals[f], `laneTotals.${f}`);
}
for (const f of ['agentsNotSpawned', 'allTokens', 'freshTokens', 'percentOfRefuteAgents', 'percentOfRefuteTokens', 'percentOfLaneTokens']) {
  assert.equal(report.projected[f], expected.projected[f], `projected.${f}`);
}
assert.deepEqual(report.projected.severities, ['suggestion'], 'only `suggestion` is projected away');

console.log('refuter-severity fixture comparison MATCH');
NODE_REFSEV_COMPARE

run_node "$TMP/refsev-compare.mjs" "$TMP/refsev-report.json" "$REFSEV_EXPECTED" ||
    fail "the refuter-severity report did not match the hand-checked fixture figures"
pass "per-severity agent counts, verdict tallies and all four token classes match the fixture exactly"

# --- the --check and --audit paths -------------------------------------------
run_node "$REFSEV" --root "$REFSEV_SCRATCH" --check "$REFSEV_EXPECTED" >/dev/null ||
    fail "--check failed against the fixture's own recorded figures"
run_node "$REFSEV" --audit "$REFSEV_EXPECTED" >/dev/null ||
    fail "--audit failed against the fixture's own recorded figures"
# The COMMITTED baseline figures are gated corpus-free, so this holds on any
# machine — the sidecar-reading --check is run by hand (see the doc's
# gating.checkGatedBy).
run_node "$REFSEV" --audit "$BASELINE_DOC" >/dev/null ||
    fail "docs/token-baseline.json's nonGatingRefutationSkip figures are not internally consistent — re-run the instrument and update the doc"
pass "--check matches the fixture, and the committed baseline figures pass the corpus-free --audit"

# --- planted mutations: neither check may be vacuous --------------------------
# (a) A doc figure edited by hand must fail BOTH --check and --audit.
sed 's/"agentsNotSpawned": 1,/"agentsNotSpawned": 2,/' "$REFSEV_EXPECTED" >"$TMP/refsev-doc-mutant.json"
if diff -q "$REFSEV_EXPECTED" "$TMP/refsev-doc-mutant.json" >/dev/null 2>&1; then
    fail "self-test setup: the doc mutation did not change the fixture figures"
fi
if run_node "$REFSEV" --root "$REFSEV_SCRATCH" --check "$TMP/refsev-doc-mutant.json" >/dev/null 2>&1; then
    fail "planted-mutation self-test FAILED TO CATCH: --check accepted a hand-edited projected.agentsNotSpawned"
fi
if run_node "$REFSEV" --audit "$TMP/refsev-doc-mutant.json" >/dev/null 2>&1; then
    fail "planted-mutation self-test FAILED TO CATCH: --audit accepted a projected figure its own rows do not derive"
fi
pass "planted-mutation self-test: an edited doc figure flips both --check and --audit to FAIL"

# (b) A broken severity extractor must flip the fixture comparison.
mkdir -p "$TMP/refsev-mut/lib"
cp "$LIB" "$TMP/refsev-mut/lib/token-report.mjs"
# Force the sentinel search to miss, so every finding becomes unparseable.
sed "s/const sentinel = '\\\\nStart from the stance:';/const sentinel = '\\\\nNEVER MATCHES THIS SENTINEL:';/" \
    "$REFSEV" >"$TMP/refsev-mut/measure-refuter-severity.mjs"
if diff -q "$REFSEV" "$TMP/refsev-mut/measure-refuter-severity.mjs" >/dev/null 2>&1; then
    fail "self-test setup: the extractor mutation did not change the source — sed pattern did not match"
fi
if run_node "$TMP/refsev-mut/measure-refuter-severity.mjs" --root "$REFSEV_SCRATCH" --format json >"$TMP/refsev-mutant-report.json" 2>/dev/null; then
    if run_node "$TMP/refsev-compare.mjs" "$TMP/refsev-mutant-report.json" "$REFSEV_EXPECTED" >/dev/null 2>&1; then
        fail "planted-mutation self-test FAILED TO CATCH: a broken finding extractor still matched the fixture figures"
    fi
fi
pass "planted-mutation self-test: a broken severity extractor flips the fixture comparison to FAIL"

# ==============================================================================
say "7. Changelog hygiene: the code change is staged/committed alongside CHANGELOG.md"
# ==============================================================================

if git -C "$REPO_ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    STAGED_FILES=$(git -C "$REPO_ROOT" diff --cached --name-only 2>/dev/null || true)
    LIB_REL="scripts/lib/token-report.mjs"
    CLI_REL="scripts/measure-lane-tokens.mjs"
    REFSEV_REL="scripts/measure-refuter-severity.mjs"
    if printf '%s\n' "$STAGED_FILES" | grep -qx "$LIB_REL\|$CLI_REL\|$REFSEV_REL"; then
        printf '%s\n' "$STAGED_FILES" | grep -qx 'CHANGELOG.md' ||
            fail "a token-measurement script (token-report.mjs / measure-lane-tokens.mjs / measure-refuter-severity.mjs) is staged without a corresponding CHANGELOG.md update in the same change"
        pass "CHANGELOG.md is staged alongside the token-report code change"
    else
        pass "no staged token-report code change to check for a changelog entry (skipping — nothing staged right now)"
    fi
else
    pass "not inside a git work tree — skipping changelog-staged check"
fi

# ==============================================================================
say "verify-token-report.sh: ALL GREEN"
