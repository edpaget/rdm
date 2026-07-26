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
#      --format text (the discrepancy line specifically).
#   3. DIRECT LIB BEHAVIOR — parseAgentTranscript's requestId dedupe
#      (last-write-wins, not first-write-wins or summed), a genuinely
#      unreadable transcript and a transcript with zero usable assistant
#      lines each degrading to a distinct, non-throwing sidecarOnly warning
#      rather than crashing or being silently indistinguishable from the
#      "no transcript file at all" case, and locateSessionDirs walking into
#      the `--worktrees-`-named project-slug directory.
#   4. CLI ARGUMENT VALIDATION — a value-taking flag with its value omitted
#      (or immediately followed by another flag) fails loudly instead of
#      silently filtering every run out of the report.
#   5. PLANTED-MUTATION SELF-TESTS — two independent mutations (dedupe
#      collapsed into first-write-wins; the totalsDiscrepancy delta silently
#      reconciled to 0) are each applied to a scratch copy of the source and
#      proven to flip the fixture comparison from MATCH to FAIL, proving the
#      comparison in section 2 is not vacuous.
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
  locateSessionDirs,
  agentClassFromLabel,
} from '$LIB';

// --- requestId dedupe: last-write-wins on the planted duplicate (req-A) ---
const agent1Path = '$SCRATCH_FIXTURE/-Users-edward-Projects-rdm/sess-fixture-a/subagents/workflows/wf_run001/agent-agent1.jsonl';
const t1 = parseAgentTranscript(agent1Path);
assert.equal(t1.ok, true);
assert.equal(t1.dedupedRequestCount, 2, 'expected 2 distinct requestIds (req-A, req-B), not 3 lines');
assert.equal(t1.perRequest.get('req-A').usage.outputTokens, 50, 'req-A must resolve to the LAST line\\'s output_tokens (50), not the first (10) or a sum (60)');
assert.equal(t1.perRequest.get('req-B').usage.outputTokens, 120);
console.log('dedupe: ok');

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

# ==============================================================================
say "verify-token-report.sh: ALL GREEN"
