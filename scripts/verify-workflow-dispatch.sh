#!/bin/sh
# Hermetic regression for the dispatch-phase keystone workflow.
#
# dispatch-phase (`.claude/workflows/dispatch-phase.js`) is the per-phase unit of
# autonomous execution: a deterministic 4-stage pipeline
#   Plan → PlanReview → Implement → CodeReview → OUTCOME
# that returns { roadmap, phase, outcome, summary, findings } with
# outcome ∈ { reviewed, rework, escalated } and NEVER emits a land-time
# completion directive. Its pure decision core lives once in
# `.claude/workflows/lib/dispatch-phase.mjs` and is copied BYTE-IDENTICAL into the
# workflow script (the Workflow runtime cannot import a helper module — see
# docs/workflow-schemas.md § "Import spike"). This harness gates three things:
#
#   1. BEHAVIOR — the pure decision logic, driven in Node with fabricated ranked
#                 finding arrays (zero LLM calls): all three outcome branches
#                 (reviewed / rework / escalated), tier-scaling, the bounded
#                 one-revise/one-rework loops reaching a terminal, determinism,
#                 and that no OUTCOME ever carries a `Done:` directive.
#   2. BLOCK DRIFT — the `dispatch-outcome` region is byte-identical between the
#                 lib source of truth and the stamped workflow script (with a
#                 planted-mutation self-test proving the gate is not a no-op).
#   3. STATIC INVARIANTS — grep-based assertions on the workflow source for the
#                 ACs that are structural rather than runtime (no `Done:` line;
#                 no `isolation:` worktree flag but a `worktree add` prompt; both
#                 stamped review markers present; no import/require/nested
#                 workflow() call; distinct planner/implementer/fetch agent labels;
#                 the implementer prompt seeded from phase body + plan doc only,
#                 never the plan-review findings; no unbounded loop construct).
#
# NOTE ON THE DETERMINISTIC MODEL: the pipeline cannot classify a code finding's
# *nature* (the FINDING schema has severity but no fixable/decision flag), so a
# code defect that survives the one bounded rework resolves to `rework`, and
# genuine decisions surface earlier at the plan gate as `escalated`. That is why
# the code stage yields only reviewed|rework here.
#
# Node is used only as a host to unit-test the pure module; it is stdlib-only
# (node:assert), with no package.json / node_modules / third-party packages.
# node is pinned in .mise.toml.
#
# Requires: node (via PATH or `mise exec node --`).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)

LIB="$REPO_ROOT/.claude/workflows/lib/dispatch-phase.mjs"
WF="$REPO_ROOT/.claude/workflows/dispatch-phase.js"

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

# Parse a workflow script under MODULE semantics and fail on a SyntaxError.
#
# A naive `node --check "$WF"` does NOT work: the file mixes `export` with a
# top-level `return`, so plain --check would false-fail a VALID workflow on the
# legal top-level return (and, on the broken file, does not reliably surface the
# redeclaration). This transform strips the leading `export` and wraps the body
# in an async function so top-level `return`/`await` are legal, while keeping the
# top-level `const meta`/`let meta` in ONE shared scope so a redeclaration is
# still a SyntaxError. Empirically: exit 1 on a duplicate top-level `meta`, exit
# 0 on the valid file.
parse_workflow() {
    {
        echo '(async function(){'
        sed 's/^export //' "$1"
        echo '})'
    } |
        run_node --check --input-type=module -
}

# Distinct `phase: '<name>',` literals the workflow actually emits (driver agent()
# calls + the inlined review block), one per line, sorted-unique. The alpha-name +
# trailing-comma shape matches only real agent-option keys, excluding prose like
# the `phase: '<stem-or-number>'` placeholder in the module's doc comment.
emitted_phases() {
    grep -oE "phase: '[A-Za-z]+'," "$1" | sed "s/phase: '//;s/',//" | sort -u
}

# Distinct `{ title: '<name>' }` entries declared in the `meta.phases` array. The
# awk window is scoped to the meta.phases array (from `phases: [` to its closing
# `],`) so DIMENSIONS' `title:` literals in the stamped review block are excluded.
declared_phases() {
    awk '/phases: \[/{p=1} p{print} p&&/^  \],?$/{exit}' "$1" |
        grep -oE "title: '[^']+'" | sed "s/title: '//;s/'\$//" | sort -u
}

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

# --- 1. BEHAVIOR -------------------------------------------------------------
say "1. Behavior: all three outcome branches, tier-scaling, bounded loops, determinism"

cat >"$TMP/test.mjs" <<'NODE_TEST'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const libPath = process.argv[2];
const mod = await import(pathToFileURL(libPath).href);
const { hasBlocking, summarizeFindings, classifyOutcome, buildOutcome } = mod;

// Fabricated ranked findings (already in most-severe-first order per severity).
const B = (id) => ({ id, concern: 'x', severity: 'blocking', confidence: 90, what_fails: id });
const C = (id) => ({ id, concern: 'x', severity: 'concern', confidence: 90, what_fails: id });

const SHAPE = ['findings', 'outcome', 'phase', 'roadmap', 'summary'];

// ============================================================================
// Happy path FIRST — clean plan + clean code → reviewed, full OUTCOME shape.
// ============================================================================
const rev = buildOutcome({
  roadmap: 'rm',
  phase: 'phase-1-foo',
  planFindings: [],
  codeFindings: [],
  codeFindingsAfterRework: [],
  tier: 'medium',
});
assert.equal(rev.outcome, 'reviewed', 'clean plan + clean code → reviewed');
assert.equal(rev.roadmap, 'rm', 'roadmap echoed into OUTCOME');
assert.equal(rev.phase, 'phase-1-foo', 'phase echoed into OUTCOME');
assert.ok(Array.isArray(rev.findings), 'findings is an array');
assert.equal(typeof rev.summary, 'string', 'summary is a string');
assert.deepEqual(Object.keys(rev).sort(), SHAPE, 'OUTCOME is exactly {roadmap,phase,outcome,summary,findings}');

// ============================================================================
// Failure-branch fixtures.
// ============================================================================
// Blocking code fixed by the ONE bounded rework → reviewed.
const revRework = buildOutcome({
  roadmap: 'rm',
  phase: 'p',
  planFindings: [],
  codeFindings: [B('bug')],
  codeFindingsAfterRework: [],
  tier: 'medium',
});
assert.equal(revRework.outcome, 'reviewed', 'blocking code fixed by the one rework → reviewed');

// Blocking code surviving the one rework → rework (budget exhausted).
const rw = buildOutcome({
  roadmap: 'rm',
  phase: 'p',
  planFindings: [],
  codeFindings: [B('bug')],
  codeFindingsAfterRework: [B('bug')],
  tier: 'medium',
});
assert.equal(rw.outcome, 'rework', 'blocking code surviving rework → rework');
assert.deepEqual(rw.findings.map((f) => f.id), ['bug'], 'rework surfaces the post-rework code findings');

// Blocking plan → escalated regardless of the code arrays (never implements).
const esc = buildOutcome({
  roadmap: 'rm',
  phase: 'p',
  planFindings: [B('vague')],
  codeFindings: [B('bug')],
  codeFindingsAfterRework: [],
  tier: 'medium',
});
assert.equal(esc.outcome, 'escalated', 'blocking plan → escalated regardless of code');
assert.deepEqual(esc.findings.map((f) => f.id), ['vague'], 'escalated surfaces the plan findings');

// fetchError short-circuits to escalated.
const fe = buildOutcome({ roadmap: 'rm', phase: 'p', fetchError: true });
assert.equal(fe.outcome, 'escalated', 'fetchError → escalated');
assert.equal(fe.summary, 'phase fetch failed', 'fetchError summary is fixed');
assert.deepEqual(fe.findings, [], 'fetchError findings empty');
assert.deepEqual(Object.keys(fe).sort(), SHAPE, 'fetchError OUTCOME keeps the full shape');

// ============================================================================
// classifyOutcome directly — bounded loops reach a terminal (AC-4). The
// classifier consumes ONLY first-pass + one-rework arrays, so no state it sees
// can loop; every combination lands on a terminal value.
// ============================================================================
assert.equal(classifyOutcome({ planFindings: [], codeFindings: [], codeFindingsAfterRework: [], tier: 'medium' }), 'reviewed');
assert.equal(classifyOutcome({ planFindings: [], codeFindings: [B('x')], codeFindingsAfterRework: [], tier: 'medium' }), 'reviewed');
assert.equal(classifyOutcome({ planFindings: [], codeFindings: [B('x')], codeFindingsAfterRework: [B('x')], tier: 'medium' }), 'rework');
assert.equal(classifyOutcome({ planFindings: [B('x')], codeFindings: [], codeFindingsAfterRework: [], tier: 'medium' }), 'escalated');
for (const cf of [[], [B('a')]]) {
  for (const cr of [[], [B('b')]]) {
    const o = classifyOutcome({ planFindings: [], codeFindings: cf, codeFindingsAfterRework: cr, tier: 'medium' });
    assert.ok(['reviewed', 'rework'].includes(o), 'code stage is terminal — only reviewed|rework');
  }
}

// ============================================================================
// Tier-scaling — a surviving `concern` is blocking at `large`, not below it.
// ============================================================================
assert.equal(hasBlocking([C('c')], 'large'), true, 'large: concern counts as blocking');
assert.equal(hasBlocking([C('c')], 'medium'), false, 'medium: concern does not block');
assert.equal(hasBlocking([B('b')], 'medium'), true, 'medium: blocking still blocks');
assert.equal(
  classifyOutcome({ planFindings: [C('c')], codeFindings: [], codeFindingsAfterRework: [], tier: 'large' }),
  'escalated',
  'plan concern escalates at large'
);
assert.notEqual(
  classifyOutcome({ planFindings: [C('c')], codeFindings: [], codeFindingsAfterRework: [], tier: 'medium' }),
  'escalated',
  'plan concern does NOT escalate at medium (one-directional tightening)'
);

// ============================================================================
// Determinism — two buildOutcome calls on the same input are byte-identical.
// ============================================================================
const detInput = {
  roadmap: 'rm',
  phase: 'p',
  planFindings: [],
  codeFindings: [B('a'), C('b')],
  codeFindingsAfterRework: [B('a')],
  tier: 'medium',
};
assert.equal(
  JSON.stringify(buildOutcome(detInput)),
  JSON.stringify(buildOutcome(detInput)),
  'buildOutcome is deterministic across runs'
);

// summarizeFindings is a deterministic one-liner off the top-ranked finding.
assert.equal(summarizeFindings([]), 'no surviving findings');
assert.ok(summarizeFindings([B('bug')]).includes('blocking'), 'summary names the top severity');

// ============================================================================
// No OUTCOME ever carries a land-time completion (`Done:`) directive.
// ============================================================================
for (const o of [rev, revRework, rw, esc, fe]) {
  assert.ok(!JSON.stringify(o).includes('Done:'), 'OUTCOME never contains a Done: directive');
}

console.log('all dispatch-phase behavior assertions passed');
NODE_TEST

if run_node "$TMP/test.mjs" "$LIB"; then
    pass "outcome classification verified (reviewed/rework/escalated, tier-scaling, bounded, deterministic)"
else
    fail "dispatch-phase behavior assertions failed"
fi

# --- 2. BLOCK DRIFT GATE -----------------------------------------------------
say "2. Block drift: the dispatch-outcome region is byte-identical (lib vs workflow)"

# Extract the region strictly BETWEEN the marker lines (exclusive), matched only
# on the "// >>> " comment token so an incidental in-block mention cannot truncate.
extract_block() {
    awk '
        index($0, ">>> dispatch-outcome:begin") { infence = 1; next }
        index($0, ">>> dispatch-outcome:end") { infence = 0 }
        infence { print }
    ' "$1"
}

blocks_equal() {
    extract_block "$1" >"$TMP/_a" 2>/dev/null
    extract_block "$2" >"$TMP/_b" 2>/dev/null
    [ -s "$TMP/_a" ] && diff -q "$TMP/_a" "$TMP/_b" >/dev/null 2>&1
}

extract_block "$LIB" >"$TMP/lib-block"
[ -s "$TMP/lib-block" ] || fail "no dispatch-outcome block found between markers in $LIB"
extract_block "$WF" >"$TMP/wf-block"
[ -s "$TMP/wf-block" ] || fail "no dispatch-outcome block found between markers in $WF"

if diff -u "$TMP/lib-block" "$TMP/wf-block" >/dev/null 2>&1; then
    pass "dispatch-outcome block matches byte-for-byte between lib and workflow"
else
    printf '\n' >&2
    diff -u "$TMP/lib-block" "$TMP/wf-block" >&2 || true
    fail "dispatch-outcome block DRIFTED — copy the lib block verbatim into $WF"
fi

# Self-test: prove the byte-equality gate is not a no-op. On scratch copies, a
# mutation inside the workflow's block MUST break equality; a restore MUST heal it.
say "2b. Block drift detector fires on planted drift (self-test)"
cp "$LIB" "$TMP/lib.scratch"
cp "$WF" "$TMP/wf.scratch"
blocks_equal "$TMP/lib.scratch" "$TMP/wf.scratch" || fail "scratch copies should match before mutation"
sed 's/no surviving findings/planted drift/' "$TMP/wf.scratch" >"$TMP/wf.mut" && mv "$TMP/wf.mut" "$TMP/wf.scratch"
if blocks_equal "$TMP/lib.scratch" "$TMP/wf.scratch"; then
    fail "byte-equality gate did NOT detect a planted mutation inside the block"
fi
cp "$WF" "$TMP/wf.scratch"
blocks_equal "$TMP/lib.scratch" "$TMP/wf.scratch" || fail "restore did not heal the byte-equality gate"
pass "drift detector fails on a planted mutation and heals on restore"

# --- 3. STATIC INVARIANTS ----------------------------------------------------
say "3. Static invariants on the workflow source (AC-1 / AC-2 / AC-3 / AC-5)"

# AC-1: the workflow source contains NO land-time completion (`Done:`) directive.
if grep -n 'Done:' "$WF" >/dev/null 2>&1; then
    grep -n 'Done:' "$WF" >&2 || true
    fail "AC-1: dispatch-phase.js must not contain a 'Done:' line (land-time only)"
fi
printf 'Done: rm/phase-1-x\n' >"$TMP/planted-done.js"
grep -q 'Done:' "$TMP/planted-done.js" || fail "AC-1 detector broken — grep 'Done:' missed a planted directive"
pass "AC-1: no 'Done:' directive present; detector catches a planted one"

# AC-5: shared per-roadmap worktree entered via `rdm worktree add` in a prompt;
# `isolation:` (the isolation:'worktree' agent option) must NOT be used.
if grep -n 'isolation' "$WF" >/dev/null 2>&1; then
    grep -n 'isolation' "$WF" >&2 || true
    fail "AC-5: dispatch-phase.js must not use isolation:'worktree' — enter the shared worktree via Bash"
fi
grep -q 'worktree add' "$WF" || fail "AC-5: expected a './target/debug/rdm worktree add' instruction in an agent prompt"
printf "  isolation: 'worktree',\n" >"$TMP/planted-iso.js"
grep -q 'isolation' "$TMP/planted-iso.js" || fail "AC-5 detector broken — grep 'isolation' missed a planted flag"
pass "AC-5: shared 'worktree add' prompt present; no isolation flag; detector catches a planted one"

# AC-3: both stamped review markers present; no import/require/nested workflow().
grep -q '>>> review-refute-fix:begin' "$WF" || fail "AC-3: missing review-refute-fix:begin marker"
grep -q '>>> review-refute-fix:end' "$WF" || fail "AC-3: missing review-refute-fix:end marker"
grep -q '>>> dispatch-outcome:begin' "$WF" || fail "AC-3: missing dispatch-outcome:begin marker"
grep -q '>>> dispatch-outcome:end' "$WF" || fail "AC-3: missing dispatch-outcome:end marker"
if grep -nE '(^|[^A-Za-z_])import[ (]' "$WF" >/dev/null 2>&1; then
    grep -nE '(^|[^A-Za-z_])import[ (]' "$WF" >&2 || true
    fail "AC-3: dispatch-phase.js must not import (the runtime forbids it — sharing is by stamped copy)"
fi
if grep -nE '(^|[^A-Za-z_])require\(' "$WF" >/dev/null 2>&1; then
    fail "AC-3: dispatch-phase.js must not require() (the runtime forbids it)"
fi
if grep -n 'workflow(' "$WF" >/dev/null 2>&1; then
    grep -n 'workflow(' "$WF" >&2 || true
    fail "AC-3: dispatch-phase.js must not nest a workflow() call — both review gates are inline copies"
fi
printf "import x from 'y'\n" >"$TMP/planted-import.js"
grep -qE '(^|[^A-Za-z_])import[ (]' "$TMP/planted-import.js" || fail "AC-3 import detector broken"
printf "const x = require('y')\n" >"$TMP/planted-req.js"
grep -qE '(^|[^A-Za-z_])require\(' "$TMP/planted-req.js" || fail "AC-3 require detector broken"
printf "const r = workflow('sub')\n" >"$TMP/planted-wf.js"
grep -q 'workflow(' "$TMP/planted-wf.js" || fail "AC-3 workflow() detector broken"
pass "AC-3: both review markers present; no import/require/nested workflow(); detectors catch planted ones"

# AC-2: planner, plan-reviewer, and implementer are SEPARATE agent() calls —
# distinct labels, with plan and implement labels that differ.
NLABELS=$(grep -oE "label: '[^']+'" "$WF" | sort -u | wc -l | tr -d ' ')
[ "$NLABELS" -ge 3 ] || fail "AC-2: expected >=3 distinct agent labels, found $NLABELS"
grep -q "label: 'fetch:" "$WF" || fail "AC-2: missing a fetch agent label"
grep -q "label: 'plan:" "$WF" || fail "AC-2: missing a planner agent label"
grep -q "label: 'implement:" "$WF" || fail "AC-2: missing an implementer agent label"
# plan vs implement labels must be distinct label namespaces.
if grep -oE "label: '[^']+'" "$WF" | grep -q "label: 'plan:.*implement"; then
    fail "AC-2: planner and implementer must use separate labels"
fi
pass "AC-2: $NLABELS distinct agent labels; separate fetch/plan/implement calls"

# AC-2 (seeding nuance / plan-review strengthening): the implementer prompt is
# built from the phase body + approved plan doc ONLY — it must never interpolate
# the plan-review findings/transcript (`planFindings`). Code-review findings on
# the rework pass are a different variable and ARE allowed.
awk '/^function buildImplementPrompt\(/{p=1} p{print} p&&/^}/{exit}' "$WF" >"$TMP/impl-fn"
[ -s "$TMP/impl-fn" ] || fail "could not extract buildImplementPrompt from $WF"
grep -q 'phaseBody' "$TMP/impl-fn" || fail "AC-2: implementer prompt must be seeded from phaseBody"
grep -q 'planDocText' "$TMP/impl-fn" || fail "AC-2: implementer prompt must be seeded from the approved plan doc"
if grep -q 'planFindings' "$TMP/impl-fn"; then
    fail "AC-2: implementer prompt must NOT interpolate plan-review findings (planFindings)"
fi
pass "AC-2: implementer seeded from phase body + plan doc only, not the plan-review transcript"

# AC-4 (driver-level reinforcement): no unbounded loop construct wraps the
# revise/rework agent() calls — they are single bounded if-guards, not loops.
if grep -nE '(^|[^A-Za-z_])(for|while)[[:space:]]*\(' "$WF" >/dev/null 2>&1; then
    grep -nE '(^|[^A-Za-z_])(for|while)[[:space:]]*\(' "$WF" >&2 || true
    fail "AC-4: dispatch-phase.js must contain no loop construct (revise/rework are single bounded if-guards)"
fi
printf 'for (let i = 0; i < 3; i++) {}\n' >"$TMP/planted-loop.js"
grep -qE '(^|[^A-Za-z_])(for|while)[[:space:]]*\(' "$TMP/planted-loop.js" || fail "AC-4 loop detector broken"
pass "AC-4: no unbounded loop construct; detector catches a planted one"

# meta.phases must list EXACTLY the distinct `phase:` values the driver + the
# inlined review block emit — no emitted-but-undeclared phase (e.g. Find/Refute
# from the stamped block) and no declared-but-never-emitted phase (e.g. a stale
# CodeReview). Compare the two sorted-unique sets.
DECLARED_PHASES=$(declared_phases "$WF")
EMITTED_PHASES=$(emitted_phases "$WF")
if [ "$DECLARED_PHASES" = "$EMITTED_PHASES" ]; then
    pass "meta.phases lists exactly the emitted phase: literals ($(echo "$EMITTED_PHASES" | tr '\n' ' '))"
else
    printf 'declared (meta.phases): %s\n' "$(echo "$DECLARED_PHASES" | tr '\n' ' ')" >&2
    printf 'emitted   (phase: ...): %s\n' "$(echo "$EMITTED_PHASES" | tr '\n' ' ')" >&2
    fail "meta.phases drift: declared phases != emitted phase: literals (add/remove entries to match)"
fi
# Self-test: a planted emitted-but-undeclared phase MUST break the check.
sed "s/phase: 'Plan',/phase: 'Ghost',/" "$WF" >"$TMP/wf.phase.scratch"
if [ "$(declared_phases "$TMP/wf.phase.scratch")" = "$(emitted_phases "$TMP/wf.phase.scratch")" ]; then
    fail "meta.phases consistency check did NOT catch a planted undeclared phase"
fi
pass "meta.phases consistency detector catches a planted undeclared phase"

# --- 4. MODULE PARSE ---------------------------------------------------------
say "4. Module parse: dispatch-phase.js loads under module semantics (no SyntaxError)"

if parse_workflow "$WF" >/dev/null 2>&1; then
    pass "dispatch-phase.js parses under module semantics (top-level meta declared once)"
else
    parse_workflow "$WF" >&2 || true
    fail "dispatch-phase.js does NOT parse — fix the SyntaxError (e.g. a duplicate top-level 'meta')"
fi

# Self-test: prove the parse gate is non-vacuous. Injecting a redeclared top-level
# `meta` into a scratch copy MUST make the gate FAIL; the unmodified file PASSES.
say "4b. Parse gate fires on a planted syntax error (self-test)"
cp "$WF" "$TMP/wf.parse.scratch"
printf '\nlet meta = null\n' >>"$TMP/wf.parse.scratch"
if parse_workflow "$TMP/wf.parse.scratch" >/dev/null 2>&1; then
    fail "parse gate did NOT catch a planted duplicate top-level 'meta' declaration"
fi
if parse_workflow "$WF" >/dev/null 2>&1; then
    pass "parse gate fails on a planted syntax error and passes the unmodified file"
else
    fail "parse gate regressed on the unmodified file after the self-test"
fi

say "verify-workflow-dispatch.sh: ALL GREEN"
