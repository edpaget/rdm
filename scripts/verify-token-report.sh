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
#      figure; a broken extractor) proving neither check is vacuous. This
#      section also gates the instrument's two REVIEW-FANOUT distributions:
#      findings-per-finder (n/min/p50/p90/max, split by mode + dimension,
#      sourced from each finder's own StructuredOutput output, never inferred
#      from refuter counts) and refuters-dispatched-per-review-unit
#      (n/min/p50/p90/max plus a recovery rate), where the unit identity is
#      parsed from the target embedded in each refuter's own prompt — never
#      `phaseTitle`/`phaseIndex`, which collapse an entire plan-review run's
#      review units into one bucket. Gated the same way: fixture comparison,
#      --check/--audit over the extended `refuterFanout` doc section, and two
#      more planted mutations (a broken unit/dimension extractor; the
#      `refutersDispatched` join falling back to the unreliable label instead
#      of the prompt-derived dimension) proving the dimension-shadow case —
#      where a finder-supplied `f.id` names a DIFFERENT real dimension than
#      the finding's own — is resolved from the prompt, not the label.
#   7. DETERMINING-FINDING RANK RECONSTRUCTION — the instrument's third
#      distribution, and the one phase 4's refutation cap lives or dies on:
#      where in a severity-then-confidence ranking does the finding that
#      actually determined the outcome sit? Gated over its own purpose-built
#      fixture tree (tests/fixtures/token-determining-rank — three sidecar
#      runs holding SEVEN attributable review units plus ONE orphan agent
#      that is deliberately not a unit), with static checks that the ranking
#      and gating rule are IMPORTED from .claude/workflows/lib/review.mjs
#      rather than reimplemented (and that no local SEVERITY_RANK /
#      CONFIDENCE_FLOOR / rankFindings / hasBlocking / survives exists), that
#      no phaseTitle/phaseIndex is read in any code path, and that no lane
#      file was modified. Behavior: a whole-block deep compare plus targeted
#      assertions that top-3 and top-5 are genuinely different figures, that
#      a non-determining unit is a distinct row from an unrecoverable one and
#      contributes to no within-top-N numerator, that each unrecoverable unit
#      carries its own exact per-unit reason without spilling onto its run
#      sibling, that a unit sharing a run with an unattributable orphan agent
#      is entirely unaffected by it (contamination is PER UNIT — there is no
#      run-wide reason, and the closed vocabulary refuses one), and that a
#      finder and a refuter on the same unit resolve to a byte-identical
#      prompt-derived unitIdent in both the code-mode and plan-mode
#      trailing-punctuation shapes. Plus --check/--audit against the fixture,
#      a corpus-free --audit of the COMMITTED figures (including the
#      supports/kills verdict, which auditRankDoc RE-DERIVES rather than
#      trusts), a prose-twin check that docs/token-baseline.md § Phase 2
#      states the window, record count, unit partition, recoverable share,
#      orphan bound, scoping rule and verdict, and SIX planted mutations —
#      an edited figure, a neutered ranking, an imputed disposition, a
#      mutated cap-verdict threshold, a removed orphan guard, and a
#      re-introduced run-wide contamination reason — each proven to flip its
#      check to FAIL.
#   8. CHANGELOG HYGIENE — the same commit that touches
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
# Resolve symlinks in the scratch root. On macOS `mktemp -d` hands back a path
# under the /var -> /private/var symlink, and every instrument here gates its
# CLI on `path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)`.
# Node realpaths `import.meta.url` but NOT `process.argv[1]`, so a script copied
# into an unresolved $TMP and run from there compares /var/... against
# /private/var/..., decides it was imported rather than invoked, and silently
# does NOTHING. Every planted-mutation self-test below would then "pass"
# vacuously: the fixture comparison fails because the mutant emitted no report
# at all, not because the mutation was caught. Resolve it once, here — and the
# self-tests in section 7 additionally assert their mutant produced a non-empty
# report, so vacuity cannot come back silently.
TMP=$(cd "$TMP" && pwd -P)
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

# --- fixture comparison: the two review-fanout distributions -----------------
cat >"$TMP/refsev-fanout-compare.mjs" <<'NODE_REFSEV_FANOUT_COMPARE'
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const [, , reportPath, expectedPath] = process.argv;
const report = JSON.parse(readFileSync(reportPath, 'utf8'));
const expected = JSON.parse(readFileSync(expectedPath, 'utf8')).refuterFanout;

const fp = report.refuterFanout.findingsPerFinder;
const expFp = expected.findingsPerFinder;
const ROW_FIELDS = ['n', 'min', 'p50', 'p90', 'max', 'refutersDispatched'];
const byKey = Object.fromEntries(fp.rows.map((r) => [r.key, r]));

assert.deepEqual(
  fp.rows.map((r) => r.key),
  expFp.rows.map((r) => r.key),
  'findingsPerFinder yields exactly the expected rows, in the expected order'
);
for (const exp of expFp.rows) {
  const act = byKey[exp.key];
  assert.ok(act, `missing findingsPerFinder row "${exp.key}"`);
  for (const f of ROW_FIELDS) {
    assert.equal(act[f], exp[f], `findingsPerFinder.${exp.key}.${f}: expected ${exp[f]}, got ${act[f]}`);
  }
}
assert.equal(fp.unreadableFinderCount, expFp.unreadableFinderCount, 'findingsPerFinder.unreadableFinderCount');
assert.equal(fp.unresolvedLabelCount, expFp.unresolvedLabelCount, 'findingsPerFinder.unresolvedLabelCount');

// AC3: the dimension-shadow refuter (finding.id = 'coherence', giving label
// refute:plan:coherence, but a prompt-embedded dim.key of 'unit-of-work')
// must land its refutersDispatched contribution on plan:unit-of-work, the
// PROMPT-resolved dimension — never on plan:coherence (which naive
// label-splitting would produce, and which has no finder row at all in this
// fixture, so under the broken behavior this count would silently drop to 0
// rather than move somewhere visibly wrong).
assert.equal(
  byKey['plan:unit-of-work'].refutersDispatched,
  1,
  'plan:unit-of-work.refutersDispatched must count the dimension-shadow refuter via its PROMPT-resolved dim (unit-of-work), not its label-derived one (coherence)'
);
assert.ok(
  !('plan:coherence' in byKey),
  'no plan:coherence finder row exists in the fixture, so the shadow refuter must not spuriously create one'
);

const unit = report.refuterFanout.refuterCountsByUnit;
const expUnit = expected.refuterCountsByUnit;
for (const f of ['n', 'min', 'p50', 'p90', 'max', 'totalRefuters', 'recoveredRefuters', 'unrecoverableRefuterCount', 'recoveryRatePercent']) {
  assert.equal(unit[f], expUnit[f], `refuterCountsByUnit.${f}: expected ${expUnit[f]}, got ${unit[f]}`);
}

console.log('refuter-fanout fixture comparison MATCH');
NODE_REFSEV_FANOUT_COMPARE

run_node "$TMP/refsev-fanout-compare.mjs" "$TMP/refsev-report.json" "$REFSEV_EXPECTED" ||
    fail "the refuter-fanout report did not match the hand-checked fixture figures"
pass "findings-per-finder and per-unit refuter-count distributions match the fixture exactly, including the dimension-shadow row"

# --- the --check and --audit paths -------------------------------------------
# readDoc/checkDoc+checkFanoutDoc/auditDoc+auditFanoutDoc now require AND
# validate BOTH doc sections (nonGatingRefutationSkip and refuterFanout) in
# one pass — these calls exercise both; the planted-mutation self-tests below
# prove the refuterFanout half specifically is not a vacuous pass-through.
run_node "$REFSEV" --root "$REFSEV_SCRATCH" --check "$REFSEV_EXPECTED" >/dev/null ||
    fail "--check failed against the fixture's own recorded figures"
run_node "$REFSEV" --audit "$REFSEV_EXPECTED" >/dev/null ||
    fail "--audit failed against the fixture's own recorded figures"
# The COMMITTED baseline figures (both nonGatingRefutationSkip AND
# refuterFanout) are gated corpus-free, so this holds on any machine — the
# sidecar-reading --check is run by hand (see the doc's gating.checkGatedBy).
run_node "$REFSEV" --audit "$BASELINE_DOC" >/dev/null ||
    fail "docs/token-baseline.json's nonGatingRefutationSkip/refuterFanout figures are not internally consistent — re-run the instrument and update the doc"
pass "--check matches the fixture, and the committed baseline figures (both sections) pass the corpus-free --audit"

# --- the --until measurement window ------------------------------------------
# `--until` is the mechanism that PINS every committed figure in
# docs/token-baseline.json § nonGatingRefutationSkip (its regenerateCommand
# passes it, and --check re-applies the recorded window). Neither the fixture
# run above nor --audit exercises it — the fixture doc deliberately records no
# window, and --audit reads no sidecars at all — so an inverted comparison or a
# dropped guard would silently change the run set behind a green harness. Gate
# it directly: the fixture run's startTime is 2026-07-25T09:00:00Z.
run_node "$REFSEV" --root "$REFSEV_SCRATCH" --until 2026-07-24T00:00:00Z --format json >"$TMP/refsev-before.json" ||
    fail "--until before the fixture run failed"
run_node "$REFSEV" --root "$REFSEV_SCRATCH" --until 2026-07-26T00:00:00Z --format json >"$TMP/refsev-after.json" ||
    fail "--until after the fixture run failed"

cat >"$TMP/refsev-window.mjs" <<'NODE_REFSEV_WINDOW'
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const [, , beforePath, afterPath, expectedPath] = process.argv;
const before = JSON.parse(readFileSync(beforePath, 'utf8'));
const after = JSON.parse(readFileSync(afterPath, 'utf8'));
const expected = JSON.parse(readFileSync(expectedPath, 'utf8')).nonGatingRefutationSkip;

// A window that ENDS BEFORE the only run excludes it entirely.
assert.equal(before.corpus.runCount, 0, '--until before the run excludes it');
assert.equal(before.corpus.agentRecordCount, 0, 'an excluded run contributes no agent records');
assert.deepEqual(before.refuteBySeverity, [], 'an excluded run contributes no severity rows');
assert.equal(before.projected.agentsNotSpawned, 0, 'nothing is projected away from an empty window');

// A window that ends AFTER it keeps everything — same figures as no window at
// all, so the filter cannot be silently dropping records inside its range.
assert.equal(after.corpus.runCount, 1, '--until after the run includes it');
assert.equal(after.corpus.agentRecordCount, 11, 'the whole run is included');
assert.deepEqual(
  after.refuteBySeverity.map((r) => [r.key, r.agentCount, r.output]),
  expected.refuteBySeverity.map((r) => [r.key, r.agentCount, r.output]),
  'a window that covers the run reproduces the unwindowed figures exactly'
);
assert.equal(after.projected.agentsNotSpawned, expected.projected.agentsNotSpawned, 'projected drop unchanged');

console.log('refuter-severity --until window behaves correctly at both edges');
NODE_REFSEV_WINDOW
run_node "$TMP/refsev-window.mjs" "$TMP/refsev-before.json" "$TMP/refsev-after.json" "$REFSEV_EXPECTED" ||
    fail "the --until measurement window did not behave correctly"

# An unparseable window must fail loudly rather than silently measuring
# everything (or nothing).
if run_node "$REFSEV" --root "$REFSEV_SCRATCH" --until not-a-date --format json >/dev/null 2>&1; then
    fail "--until with an unparseable date must fail, not silently ignore the window"
fi

# A doc that RECORDS a window must have it honored by --check, and --until must
# override it. Build both cases from the fixture doc.
run_node -e '
const fs = require("node:fs");
const doc = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
doc.nonGatingRefutationSkip.measurementWindow = { until: "2026-07-24T00:00:00Z" };
fs.writeFileSync(process.argv[2], JSON.stringify(doc, null, 2));
' "$REFSEV_EXPECTED" "$TMP/refsev-windowed-doc.json"
# The doc's figures describe the FULL fixture, but its recorded window excludes
# the only run — so --check must FAIL, proving the window was applied.
if run_node "$REFSEV" --root "$REFSEV_SCRATCH" --check "$TMP/refsev-windowed-doc.json" >/dev/null 2>&1; then
    fail "--check ignored the window recorded in the doc's measurementWindow"
fi
# ... and an explicit --until must override the doc's window, restoring the match.
run_node "$REFSEV" --root "$REFSEV_SCRATCH" --until 2026-07-26T00:00:00Z --check "$TMP/refsev-windowed-doc.json" >/dev/null ||
    fail "--until did not override the window recorded in the doc"
pass "--until pins the run set at both edges, fails loudly on a bad date, and composes correctly with --check"

# Planted mutation: invert the window comparison. The section above must flip.
# The mutant scratch tree carries a pristine token-report.mjs beside it, since
# the instrument imports ./lib/token-report.mjs relative to its own path.
mkdir -p "$TMP/refsev-mut/lib" "$TMP/.claude/workflows/lib"
cp "$LIB" "$TMP/refsev-mut/lib/token-report.mjs"
# The instrument also imports the canonical ranking/gating rule from
# ../.claude/workflows/lib/review.mjs (relative to its own path), so a mutant
# living at $TMP/refsev-mut/ needs a pristine copy at $TMP/.claude/... . Copied,
# never mutated: review.mjs is read-only to this whole measurement.
cp "$REVIEW_LIB" "$TMP/.claude/workflows/lib/review.mjs"
sed 's/rf.run.startTimeMs <= untilMs/rf.run.startTimeMs >= untilMs/' \
    "$REFSEV" >"$TMP/refsev-mut/measure-refuter-severity-window.mjs"
if diff -q "$REFSEV" "$TMP/refsev-mut/measure-refuter-severity-window.mjs" >/dev/null 2>&1; then
    fail "self-test setup: the window mutation did not change the source — sed pattern did not match"
fi
run_node "$TMP/refsev-mut/measure-refuter-severity-window.mjs" --root "$REFSEV_SCRATCH" --until 2026-07-24T00:00:00Z --format json \
    >"$TMP/refsev-mut-before.json" 2>/dev/null || true
run_node "$TMP/refsev-mut/measure-refuter-severity-window.mjs" --root "$REFSEV_SCRATCH" --until 2026-07-26T00:00:00Z --format json \
    >"$TMP/refsev-mut-after.json" 2>/dev/null || true
if run_node "$TMP/refsev-window.mjs" "$TMP/refsev-mut-before.json" "$TMP/refsev-mut-after.json" "$REFSEV_EXPECTED" >/dev/null 2>&1; then
    fail "planted-mutation self-test FAILED TO CATCH: an inverted --until comparison still passed the window checks"
fi
pass "planted-mutation self-test: inverting the --until comparison flips the window checks to FAIL"

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

# (c) A broken unit/dimension extractor must flip the review-fanout comparison.
# Force the header-marker search to miss, so extractRefuterContext can never
# resolve a dimension or unit identity for any refuter.
sed "s/const HEADER_MARKER = 'A prior reviewer raised this ';/const HEADER_MARKER = 'NEVER MATCHES THIS MARKER';/" \
    "$REFSEV" >"$TMP/refsev-mut/measure-refuter-severity-context.mjs"
if diff -q "$REFSEV" "$TMP/refsev-mut/measure-refuter-severity-context.mjs" >/dev/null 2>&1; then
    fail "self-test setup: the context-extractor mutation did not change the source — sed pattern did not match"
fi
if run_node "$TMP/refsev-mut/measure-refuter-severity-context.mjs" --root "$REFSEV_SCRATCH" --format json >"$TMP/refsev-mutant-fanout-report.json" 2>/dev/null; then
    if run_node "$TMP/refsev-fanout-compare.mjs" "$TMP/refsev-mutant-fanout-report.json" "$REFSEV_EXPECTED" >/dev/null 2>&1; then
        fail "planted-mutation self-test FAILED TO CATCH: a broken unit/dimension extractor still matched the fixture's refuterFanout figures"
    fi
fi
pass "planted-mutation self-test: a broken unit/dimension extractor flips the refuter-fanout comparison to FAIL"

# (d) AC3: the refutersDispatched join must use the refuter's PROMPT-derived
# dimension (r.dimKey), never its unreliable label segment. Mutate the join to
# fall back to label-splitting and confirm the dimension-shadow refuter lands
# on the wrong (nonexistent) row instead of plan:unit-of-work.
sed "s/mode + ':' + r.dimKey/mode + ':' + labelParts[2]/" \
    "$REFSEV" >"$TMP/refsev-mut/measure-refuter-severity-labeljoin.mjs"
if diff -q "$REFSEV" "$TMP/refsev-mut/measure-refuter-severity-labeljoin.mjs" >/dev/null 2>&1; then
    fail "self-test setup: the label-join mutation did not change the source — sed pattern did not match"
fi
if run_node "$TMP/refsev-mut/measure-refuter-severity-labeljoin.mjs" --root "$REFSEV_SCRATCH" --format json >"$TMP/refsev-mutant-labeljoin-report.json" 2>/dev/null; then
    if run_node "$TMP/refsev-fanout-compare.mjs" "$TMP/refsev-mutant-labeljoin-report.json" "$REFSEV_EXPECTED" >/dev/null 2>&1; then
        fail "planted-mutation self-test FAILED TO CATCH: joining refutersDispatched on the label instead of the prompt-derived dimension still matched the fixture (the dimension-shadow row should have flipped to the wrong bucket)"
    fi
fi
pass "planted-mutation self-test: joining refutersDispatched on the label (not the prompt-derived dimension) flips the dimension-shadow assertion to FAIL"

# ==============================================================================
say "7. Determining-finding rank reconstruction (determiningFindingRank)"
# ==============================================================================
# Phase 4's refutation cap lives or dies on WHERE in a ranked candidate list the
# finding that determined the outcome sits. This section gates that
# reconstruction the same way section 6 gates the fanout distributions, over a
# PURPOSE-BUILT fixture tree (tests/fixtures/token-determining-rank) rather than
# section 6's — whose per-severity and fanout numbers are hand-checked and must
# not be perturbed by new agents.
#
# The tree seeds EIGHT entities across THREE wf_*.json sidecars: SEVEN
# attributable review units plus ONE orphan agent that is deliberately NOT a
# unit. wf_rank001 (clean): A determining at rank 1, B determining at rank 4
# with its findings emitted OUT of ranked order, C non-determining, F a
# plan-mode multi-line target proving finder/refuter unit-key agreement.
# wf_rank002 (mixed): G determining at rank 2, sharing its run with orphan agent
# E. wf_rank003 (unrecoverable): D via unknown-disposition-above-determining and
# H via dimension-coverage-gap, siblings so neither reason spills onto the other.
# Multiple runs are the only way to exercise run-boundary behavior at all, since
# a sidecar's runId is its own filename stem.

RANK_FIXTURE="$REPO_ROOT/tests/fixtures/token-determining-rank"
RANK_EXPECTED="$RANK_FIXTURE/expected-determiningFindingRank.json"
BASELINE_MD="$REPO_ROOT/docs/token-baseline.md"

[ -d "$RANK_FIXTURE" ] || fail "determining-rank fixture not found: $RANK_FIXTURE"
[ -f "$RANK_EXPECTED" ] || fail "determining-rank expected-figures fixture not found: $RANK_EXPECTED"
[ -f "$BASELINE_MD" ] || fail "token baseline prose doc not found: $BASELINE_MD"

# --- static checks: the rule is IMPORTED, and no forbidden key is read --------
# AC2: the ranking and gating rule must come from the canonical review source.
grep -q "from '\.\./\.claude/workflows/lib/review\.mjs'" "$REFSEV" ||
    fail "$REFSEV must import the ranking/gating rule from .claude/workflows/lib/review.mjs, not reimplement it"
for sym in survives rankFindings hasBlocking; do
    grep -qE "^[[:space:]]*$sym,?\$" "$REFSEV" ||
        fail "$REFSEV must import \`$sym\` from .claude/workflows/lib/review.mjs"
done
# ...and must define no local copy of any of it. Comment lines are stripped
# first: the module header DOCUMENTS why the import is mandatory, and a naive
# grep would fire on that documentation instead of on real code.
REFSEV_CODE="$TMP/refsev-code-only.txt"
grep -vE '^[[:space:]]*(//|\*|/\*)' "$REFSEV" >"$REFSEV_CODE"
if grep -qE 'function (rankFindings|hasBlocking|survives)\b' "$REFSEV_CODE"; then
    fail "$REFSEV defines a local rankFindings/hasBlocking/survives — the imported rule must be the only one"
fi
if grep -qE '\b(SEVERITY_RANK|CONFIDENCE_FLOOR)\b' "$REFSEV_CODE"; then
    fail "$REFSEV must not carry a local severity table or confidence floor — both live in review.mjs"
fi
# AC3: the review-unit key is prompt-derived. phaseTitle/phaseIndex collapse a
# whole plan-review run into one unit and must not appear in any code path.
if grep -qE '\b(phaseTitle|phaseIndex)\b' "$REFSEV_CODE"; then
    fail "$REFSEV reads phaseTitle/phaseIndex outside a comment — the unit key must come from the prompt-embedded target only"
fi
# AC9: this measurement imports review.mjs read-only. A modified lane file would
# be a behavior change, which this phase must not make.
if git -C "$REPO_ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    DIRTY_LANE=$(git -C "$REPO_ROOT" status --porcelain -- .claude/workflows .claude/skills rdm-core/src/templates 2>/dev/null || true)
    [ -z "$DIRTY_LANE" ] ||
        fail "the determining-rank measurement must not modify any lane file, but these are dirty:
$DIRTY_LANE"
fi
pass "the rule is imported from review.mjs (no local ranking/gating/severity copy), no phaseTitle/phaseIndex is read, and no lane file is modified"

# --- fixture comparison: the whole determiningFindingRank block ---------------
RANK_SCRATCH="$TMP/rank-fixture"
cp -R "$RANK_FIXTURE" "$RANK_SCRATCH"
rm -f "$RANK_SCRATCH/expected-determiningFindingRank.json"

run_node "$REFSEV" --root "$RANK_SCRATCH" --format json >"$TMP/rank-report.json" ||
    fail "measure-refuter-severity.mjs failed against the determining-rank fixture"

cat >"$TMP/rank-compare.mjs" <<'NODE_RANK_COMPARE'
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const [, , reportPath, expectedPath] = process.argv;
const raw = readFileSync(reportPath, 'utf8');
// Non-vacuity floor: a mutant that silently produces NOTHING must not be
// mistaken for a mutant whose figures were caught by the comparison below.
assert.ok(raw.trim().length > 0, 'the instrument produced no report at all');
const got = JSON.parse(raw).determiningFindingRank;
const exp = JSON.parse(readFileSync(expectedPath, 'utf8')).determiningFindingRank;
assert.ok(got, 'the report carries no determiningFindingRank block');

// Whole-block deep compare — every figure, not a sampled few.
assert.deepEqual(got, exp, 'the determiningFindingRank block must match the hand-checked fixture exactly');

// --- targeted assertions beyond the deep compare --------------------------
// Seven attributable units (A, B, C, D, F, G, H); orphan agent E is NOT one.
assert.equal(got.units.total, 7, 'exactly the seven attributable units, and no phantom unit for orphan agent E');
assert.equal(got.units.determining, 3, 'A (rank 1), G (rank 2), B (rank 4)');
assert.equal(got.units.nonDetermining, 2, 'C and F resolved fully with nothing gating');
assert.equal(got.units.unrecoverable, 2, 'D and H');
assert.equal(got.units.recoverable, got.units.determining + got.units.nonDetermining, 'recoverable is the partition');
assert.equal(
  got.units.determining + got.units.nonDetermining + got.units.unrecoverable,
  got.units.total,
  'the three statuses partition the units exactly'
);
assert.equal(got.units.recoverableSharePercent, Math.round((5 / 7) * 1000) / 10, 'recoverable share is 5/7');

// AC1: the rank distribution, and top-3 vs top-5 as genuinely different numbers.
assert.deepEqual(
  got.rankHistogram,
  [{ rank: 1, count: 1 }, { rank: 2, count: 1 }, { rank: 4, count: 1 }],
  'ranks 1, 2 and 4, one unit each'
);
assert.equal(got.rankHistogram.reduce((n, r) => n + r.count, 0), got.units.determining, 'the histogram sums to determining');
const top = Object.fromEntries(got.withinTop.map((w) => [w.n, w]));
assert.equal(top[3].count, 2, 'A and G are within top 3');
assert.equal(top[5].count, 3, 'B (rank 4) joins them within top 5');
assert.ok(top[3].count < top[5].count, 'top-3 and top-5 must be different figures, not the same number printed twice');
for (const w of got.withinTop) {
  assert.equal(w.percentOfDetermining, Math.round((w.count / got.units.determining) * 1000) / 10, `withinTop[${w.n}] % of determining`);
  assert.equal(w.percentOfRecoverable, Math.round((w.count / got.units.recoverable) * 1000) / 10, `withinTop[${w.n}] % of recoverable`);
}

// AC4: non-determining (C, F) is a distinct row from unrecoverable, and
// contributes to no within-top-N numerator.
assert.ok(got.units.nonDetermining > 0, 'the non-determining row is populated');
assert.equal(
  top[5].count + got.units.nonDetermining + got.units.unrecoverable,
  got.units.total,
  'non-determining and unrecoverable units are outside every within-top-N numerator'
);

// AC5: each unrecoverable unit carries its own exact, per-unit reason, and the
// two reasons in this run do not spill onto each other.
const reasons = Object.fromEntries(got.unrecoverableByReason.map((r) => [r.reason, r.count]));
assert.equal(reasons['unknown-disposition-above-determining'], 1, 'unit D: a gating candidate with no refuter, ranked above the would-be determining one');
assert.equal(reasons['dimension-coverage-gap'], 1, 'unit H: a correctness refuter with no correctness finder');
assert.equal(Object.keys(reasons).length, 2, 'exactly those two reasons');
assert.equal(
  got.unrecoverableByReason.reduce((n, r) => n + r.count, 0),
  got.units.unrecoverable,
  'the reason rows sum to units.unrecoverable'
);
// The run-wide rule is REJECTED: no such reason may exist at all.
assert.ok(!('orphan-agent-in-run' in reasons), 'contamination is per unit — there is no run-wide orphan reason');

// AC5 regression for the orphan finding: unit G shares wf_rank002 with orphan
// agent E and must be entirely unaffected by it.
assert.ok(
  got.rankHistogram.some((r) => r.rank === 2 && r.count === 1),
  'unit G stayed determining at rank 2 despite sharing its run with an unattributable orphan agent'
);
assert.ok(got.orphanAgents.finders + got.orphanAgents.refuters >= 1, 'orphan agent E is counted as an orphan');
assert.equal(got.orphanAgents.finders, 1, 'exactly one orphan finder (E)');
assert.equal(got.orphanAgents.runsAffected, 1, 'the orphan sits in exactly one run');

// candidateSetSize is what tells phase 4 whether a top-N cap would even bind.
assert.equal(got.candidateSetSize.n, got.units.recoverable, 'candidate sizes are reported over recoverable units');
assert.ok(got.candidateSetSize.max >= 1, 'at least one unit has candidates');

// acTableGapUnits is its own diagnostic, never folded into non-determining.
assert.equal(got.acTableGapUnits, 1, 'unit B seeds a FAIL row in its ac table');

// largeTier: widening the blocker set can only move a determining finding
// EARLIER in the same ranking, so it is monotone against the default tier.
assert.ok(got.largeTier.units.determining >= got.units.determining, 'largeTier determines at least as many units');
const largeTop = Object.fromEntries(got.largeTier.withinTop.map((w) => [w.n, w]));
for (const n of [3, 5]) {
  assert.ok(largeTop[n].count >= top[n].count, `largeTier within-top-${n} is never below the default tier's`);
}
assert.ok(
  got.largeTier.rankSummary.max <= Math.max(...got.largeTier.rankHistogram.map((r) => r.rank)),
  'largeTier rank summary agrees with its own histogram'
);

// AC7: the verdict is DERIVED from the figures, not asserted beside them.
assert.ok(['supports-cap', 'kills-cap', 'inconclusive'].includes(got.capVerdict.verdict), 'the verdict is in the closed vocabulary');
assert.equal(got.capVerdict.inputs.determining, got.units.determining, 'the verdict reads the same determining count');
assert.equal(got.capVerdict.inputs.recoverableSharePercent, got.units.recoverableSharePercent, 'and the same recoverable share');

console.log('determining-rank fixture comparison MATCH');
NODE_RANK_COMPARE

run_node "$TMP/rank-compare.mjs" "$TMP/rank-report.json" "$RANK_EXPECTED" ||
    fail "the determining-rank report did not match the hand-checked fixture figures"
pass "the rank reconstruction matches the fixture exactly, including the non-determining, unrecoverable and orphan-non-contamination cases"

# --format text is the default the instrument is invoked with by hand, and it
# reaches fields --format json never touches — a renderer that indexes a moved
# key throws only here. Render it and require the new tables to be present.
run_node "$REFSEV" --root "$RANK_SCRATCH" --format text >"$TMP/rank-report.txt" ||
    fail "--format text crashed against the determining-rank fixture"
for needle in 'CANDIDATE list' 'recoverable share' 'unrecoverable reason' 'Orphan agents' 'Cap verdict'; do
    grep -qF "$needle" "$TMP/rank-report.txt" ||
        fail "--format text is missing the \"$needle\" section of the determining-rank report"
done
pass "--format text renders the rank histogram, unit partition, unrecoverable reasons, orphan diagnostic and verdict without throwing"

# --- AC3: finder and refuter must resolve to the BYTE-IDENTICAL unit key ------
cat >"$TMP/rank-unitkey.mjs" <<'NODE_RANK_UNITKEY'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const [, , dir, instrument] = process.argv;
const { readFinderTranscript, readRefuterTranscript } = await import(pathToFileURL(instrument).href);
// Unit F: plan mode, MULTI-line target — the identity is its first line and the
// `.` findPrompt appends sits on the body's last line, so it must NOT be
// stripped. Unit A: code mode, single-line target — the `.` sits on the SAME
// line and IS stripped. Both must round-trip identically on both sides.
const f = readFinderTranscript(dir + '/wf_rank001/agent-ff-coh.jsonl');
const rf = readRefuterTranscript(dir + '/wf_rank001/agent-rf-c1.jsonl');
assert.equal(f.unitIdent, 'phase widget/phase-6-zeta', 'plan-mode finder identity is the first line, un-stripped');
assert.equal(rf.unitIdent, f.unitIdent, 'the plan-mode finder and refuter resolve to a byte-identical unitIdent');

const a = readFinderTranscript(dir + '/wf_rank001/agent-fa-corr.jsonl');
const ra = readRefuterTranscript(dir + '/wf_rank001/agent-ra-b1.jsonl');
assert.equal(a.unitIdent, 'widget/phase-1-alpha', 'code-mode finder identity has its trailing `.` stripped');
assert.equal(ra.unitIdent, a.unitIdent, 'the code-mode finder and refuter resolve to a byte-identical unitIdent');

// The orphan: an --implementation-plan-shaped pretty-printed-JSON target is
// REJECTED rather than captured as a fake identity.
const e = readFinderTranscript(dir + '/wf_rank002/agent-fe-orphan.jsonl');
assert.equal(e.unitIdent, null, 'a JSON-shaped target yields no unit identity at all');

console.log('finder/refuter unit-key agreement OK');
NODE_RANK_UNITKEY
run_node "$TMP/rank-unitkey.mjs" \
    "$RANK_SCRATCH/-Users-edward-Projects-rdm/sess-rank/subagents/workflows" "$REFSEV" ||
    fail "the finder and refuter did not resolve to the same prompt-derived unit key"
pass "a finder and a refuter on the same unit resolve to a byte-identical unitIdent (both the code-mode and plan-mode trailing-punctuation shapes), and a JSON target resolves to none"

# --- the --check and --audit paths -------------------------------------------
run_node "$REFSEV" --root "$RANK_SCRATCH" --check "$RANK_EXPECTED" >/dev/null ||
    fail "--check failed against the determining-rank fixture's own recorded figures"
run_node "$REFSEV" --audit "$RANK_EXPECTED" >/dev/null ||
    fail "--audit failed against the determining-rank fixture's own recorded figures"
# The COMMITTED figures, gated corpus-free so this holds on any machine —
# including capVerdict, which auditRankDoc RE-DERIVES from the doc's own numbers.
run_node "$REFSEV" --audit "$BASELINE_DOC" >/dev/null ||
    fail "docs/token-baseline.json's determiningFindingRank figures are not internally consistent — re-run the instrument and update the doc"
pass "--check matches the fixture and --audit passes corpus-free on both the fixture and the COMMITTED baseline figures"

# --- AC6: the prose twin actually reads the committed figures -----------------
grep -q '^## Phase 2: rank of the determining finding' "$BASELINE_MD" ||
    fail "$BASELINE_MD has no '## Phase 2: rank of the determining finding' section"
run_node -e '
const fs = require("node:fs");
const doc = JSON.parse(fs.readFileSync(process.argv[1], "utf8")).determiningFindingRank;
const md = fs.readFileSync(process.argv[2], "utf8");
const start = md.indexOf("## Phase 2: rank of the determining finding");
const rest = md.slice(start);
const section = rest.slice(0, rest.indexOf("\n## ", 1) === -1 ? undefined : rest.indexOf("\n## ", 1));
const need = [
  doc.measurementWindow.until,
  String(doc.measurementWindow.runCount),
  doc.measurementWindow.agentRecordCount.toLocaleString("en-US"),
  String(doc.units.total),
  String(doc.units.recoverable),
  String(doc.units.determining),
  String(doc.units.unrecoverable),
  doc.units.recoverableSharePercent + " %",
  doc.capVerdict.verdict === "supports-cap" ? "SUPPORTS a cap" : doc.capVerdict.verdict,
  "contamination is per unit, never run-wide",
  String(doc.orphanAgents.finders),
];
const missing = need.filter((s) => section.indexOf(s) === -1);
if (missing.length) {
  console.error("docs/token-baseline.md § Phase 2 does not read the committed figures. Missing: " + JSON.stringify(missing));
  process.exit(1);
}
' "$BASELINE_DOC" "$BASELINE_MD" ||
    fail "the prose section must state the corpus window, record count, unit counts, recoverable share, orphan bound, the per-unit scoping rule, and the verdict"
pass "docs/token-baseline.md § Phase 2 states the window, record count, unit partition, recoverable share, scoping rule and verdict from the JSON twin"

# --- planted mutations: no part of this section may be vacuous ----------------
# Each follows the section-5/6 idiom: mutate a scratch copy, PROVE the mutation
# changed the file, then require the corresponding check to flip to FAIL. The
# comparer additionally refuses an empty report, so a mutant that fails to run
# can never masquerade as a mutant that was caught.
rank_mutant() {
    # $1 = name, $2 = sed expression
    sed "$2" "$REFSEV" >"$TMP/refsev-mut/rank-$1.mjs"
    if diff -q "$REFSEV" "$TMP/refsev-mut/rank-$1.mjs" >/dev/null 2>&1; then
        fail "self-test setup: the $1 mutation did not change the source — sed pattern did not match"
    fi
    run_node "$TMP/refsev-mut/rank-$1.mjs" --root "$RANK_SCRATCH" --format json >"$TMP/rank-mutant-$1.json" 2>/dev/null || true
    [ -s "$TMP/rank-mutant-$1.json" ] ||
        fail "self-test setup: the $1 mutant produced NO report, so the comparison below would flip for the wrong reason"
    if run_node "$TMP/rank-compare.mjs" "$TMP/rank-mutant-$1.json" "$RANK_EXPECTED" >/dev/null 2>&1; then
        fail "planted-mutation self-test FAILED TO CATCH: the $1 mutation still matched the fixture figures"
    fi
}

# (a) A hand-edited committed figure must fail BOTH --check and --audit.
sed 's/"determining": 3,/"determining": 4,/' "$RANK_EXPECTED" >"$TMP/rank-doc-mutant.json"
if diff -q "$RANK_EXPECTED" "$TMP/rank-doc-mutant.json" >/dev/null 2>&1; then
    fail "self-test setup: the doc mutation did not change the fixture figures"
fi
if run_node "$REFSEV" --root "$RANK_SCRATCH" --check "$TMP/rank-doc-mutant.json" >/dev/null 2>&1; then
    fail "planted-mutation self-test FAILED TO CATCH: --check accepted a hand-edited units.determining"
fi
if run_node "$REFSEV" --audit "$TMP/rank-doc-mutant.json" >/dev/null 2>&1; then
    fail "planted-mutation self-test FAILED TO CATCH: --audit accepted a units.determining its own partition does not add up to"
fi
pass "planted-mutation self-test (a): an edited determining-rank figure flips both --check and --audit to FAIL"

# (b) Neuter the IMPORTED ranking. Unit B's findings are emitted out of ranked
# order, so an identity ordering reports it at rank 1 instead of rank 4 —
# proving the imported rankFindings is load-bearing, not decorative.
rank_mutant ranking 's/const ranked = rankFindings(/const ranked = ((x) => x)(/'
pass "planted-mutation self-test (b): replacing the imported rankFindings with an identity ordering flips unit B's rank-4 to FAIL"

# (c) Impute "not refuted" for an unknown disposition instead of marking the
# unit unrecoverable. Unit D then reports a rank rather than a reason.
rank_mutant impute "s/c.disposition === 'unknown'/false/"
pass "planted-mutation self-test (c): imputing a verdict for an unknown disposition flips unit D's unrecoverable row to FAIL"

# (d) Mutate a CAP_VERDICT_RULE threshold. The COMMITTED verdict must flip,
# proving the supports/kills conclusion is re-derived rather than asserted.
sed 's/minDeterminingUnits: 20,/minDeterminingUnits: 2000,/' "$REFSEV" >"$TMP/refsev-mut/rank-capverdict.mjs"
if diff -q "$REFSEV" "$TMP/refsev-mut/rank-capverdict.mjs" >/dev/null 2>&1; then
    fail "self-test setup: the cap-verdict threshold mutation did not change the source — sed pattern did not match"
fi
if run_node "$TMP/refsev-mut/rank-capverdict.mjs" --audit "$BASELINE_DOC" >/dev/null 2>&1; then
    fail "planted-mutation self-test FAILED TO CATCH: --audit accepted the committed verdict under a mutated threshold, so the verdict is not derived"
fi
pass "planted-mutation self-test (d): mutating a CAP_VERDICT_RULE threshold flips the committed --audit to FAIL"

# (e) Remove the orphan guard, so an --implementation-plan-shaped JSON target is
# captured as a fake unit identity instead of being counted as an orphan. Unit
# count goes to 8 and orphanAgents.finders to 0 — the direct regression for
# "an unattributable agent must produce NO unit row".
rank_mutant orphan 's/isPlausibleUnitIdent(candidate) ? candidate : null/candidate/'
pass "planted-mutation self-test (e): capturing an unresolvable JSON target as a fake unit identity flips the orphan/unit-count assertions to FAIL"

# (f) Re-introduce the REJECTED run-wide contamination reason in the doc. The
# closed vocabulary must refuse it outright, so the scoping decision is
# harness-enforced rather than merely documented.
run_node -e '
const fs = require("node:fs");
const doc = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
const s = doc.determiningFindingRank;
s.unrecoverableByReason = [...s.unrecoverableByReason, { reason: "orphan-agent-in-run", count: 1 }];
s.units.unrecoverable += 1;
s.units.total += 1;
s.units.recoverableSharePercent = Math.round((s.units.recoverable / s.units.total) * 1000) / 10;
fs.writeFileSync(process.argv[2], JSON.stringify(doc, null, 2));
' "$RANK_EXPECTED" "$TMP/rank-runwide-doc.json"
if diff -q "$RANK_EXPECTED" "$TMP/rank-runwide-doc.json" >/dev/null 2>&1; then
    fail "self-test setup: the run-wide-reason mutation did not change the doc"
fi
if run_node "$REFSEV" --audit "$TMP/rank-runwide-doc.json" >/dev/null 2>&1; then
    fail "planted-mutation self-test FAILED TO CATCH: --audit accepted an 'orphan-agent-in-run' reason — the run-wide rule is REJECTED and must not be representable"
fi
pass "planted-mutation self-test (f): a run-wide 'orphan-agent-in-run' reason is refused by the closed vocabulary"

# ==============================================================================
say "8. Changelog hygiene: the code change is staged/committed alongside CHANGELOG.md"
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
