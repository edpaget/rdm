#!/bin/sh
# Hermetic regression for the review-refute-fix shared workflow pipeline.
#
# `.claude/workflows/lib/review.mjs` is the single canonical review source —
# find → refute → filter → verdict → gate. Its stamped block is copied into the
# workflow-script consumers by `scripts/gen-workflow-review.sh` (the Workflow
# runtime cannot import a helper module — see docs/workflow-schemas.md
# § "Import spike"), and its `//|` spec prose is rendered into the shipped review
# skill templates by `scripts/gen-skill-review.sh`. This harness gates both
# projections so a refactor can't silently break either review lane:
#
#   1. DRIFT   — every consumer is in sync with the source block (gen --check).
#   2. HYGIENE — no forbidden nondeterministic global (Date.now / Math.random)
#                creeps into a workflow script (the runtime forbids them).
#   3. BEHAVIOR — the pure pipeline logic, driven in Node with an injected fake
#                 agent + reference pipeline/parallel (zero LLM calls):
#                   * a planted refutable finding is dropped, a planted real one
#                     survives, and a not-refuted-but-low-confidence finding is
#                     dropped by the confidence floor;
#                   * a FRESH refuter grades each finding (separate agent call —
#                     the finder never grades its own work);
#                   * the review target (context) is threaded into every prompt;
#                   * output is deterministic across runs (total-order ranking);
#                   * BOTH `code` and `plan` dimension sets work behind `mode`.
#
# Node is used only as a host to unit-test the shared module's pure logic; it is
# stdlib-only (node:assert), with no package.json / node_modules / third-party
# packages. node is pinned in .mise.toml.
#
# Requires: node (via PATH or `mise exec node --`).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)

LIB="$REPO_ROOT/.claude/workflows/lib/review.mjs"
GEN="$REPO_ROOT/scripts/gen-workflow-review.sh"
SKILL_GEN="$REPO_ROOT/scripts/gen-skill-review.sh"
TEMPLATES="$REPO_ROOT/rdm-core/src/templates"
WF_DIR="$REPO_ROOT/.claude/workflows"

# Clear rdm-related env vars inherited from the caller's shell for hermeticity.
unset RDM_ROOT RDM_PROJECT RDM_STAGE RDM_FORMAT RDM_PLAN_REPO RDM_PLAN_REPO_TOKEN RDM_PLAN_REPO_PATH 2>/dev/null || true

say() { printf '\n\033[1;34m==>\033[0m %s\n' "$*"; }
fail() {
    printf '\n\033[1;31m[FAIL]\033[0m %s\n' "$*" >&2
    exit 1
}
pass() { printf '\033[1;32m[ok]\033[0m %s\n' "$*"; }

[ -f "$LIB" ] || fail "source module not found: $LIB"
[ -f "$GEN" ] || fail "generator not found: $GEN"
[ -f "$SKILL_GEN" ] || fail "skill generator not found: $SKILL_GEN"
[ -e "$REPO_ROOT/.claude/workflows/lib/review-refute-fix.mjs" ] &&
    fail "the canonical source moved to lib/review.mjs — lib/review-refute-fix.mjs must not exist"

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

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

# --- 0. MARKER STRUCTURE ------------------------------------------------------
# The canonical source carries two marker systems. `review-spec` must nest
# STRICTLY inside the stamped block (so the spec prose rides along in every
# workflow consumer), while `review-gate-spec` must sit STRICTLY after the
# stamped block's end (it is the only place the land-time completion trailer may
# appear, and the dispatch harness forbids that literal inside a stamped region).
say "0. Marker structure: review-spec nested inside the stamped block, review-gate-spec after it"
line_of() { grep -n "$1" "$2" | head -1 | cut -d: -f1; }
BLOCK_BEGIN=$(line_of '>>> review-refute-fix:begin' "$LIB")
BLOCK_END=$(line_of '>>> review-refute-fix:end' "$LIB")
SPEC_BEGIN=$(line_of '>>> review-spec:begin' "$LIB")
SPEC_END=$(line_of '>>> review-spec:end' "$LIB")
GATE_BEGIN=$(line_of '>>> review-gate-spec:begin' "$LIB")
GATE_END=$(line_of '>>> review-gate-spec:end' "$LIB")
for v in BLOCK_BEGIN BLOCK_END SPEC_BEGIN SPEC_END GATE_BEGIN GATE_END; do
    eval "val=\$$v"
    [ -n "$val" ] || fail "missing marker in $LIB: $v"
done
[ "$BLOCK_BEGIN" -lt "$SPEC_BEGIN" ] || fail "review-spec:begin must come AFTER the stamped block's begin marker"
[ "$SPEC_BEGIN" -lt "$SPEC_END" ] || fail "review-spec markers are inverted"
[ "$SPEC_END" -lt "$BLOCK_END" ] || fail "review-spec:end must come BEFORE the stamped block's end marker"
[ "$BLOCK_END" -lt "$GATE_BEGIN" ] || fail "review-gate-spec must start AFTER the stamped block ends"
[ "$GATE_BEGIN" -lt "$GATE_END" ] || fail "review-gate-spec markers are inverted"

# The stamped region of the SOURCE must not name the land-time completion
# trailer: it is copied verbatim into dispatch-phase.js, whose AC-1 forbids it.
awk -v b=">>> review-refute-fix:begin" -v e=">>> review-refute-fix:end" '
    index($0, b) { inb = 1; next }
    index($0, e) { inb = 0 }
    inb { print }
' "$LIB" >"$TMP/source-stamped-region"
[ -s "$TMP/source-stamped-region" ] || fail "extracted an EMPTY stamped region from $LIB"
if grep -n 'Done:' "$TMP/source-stamped-region" >&2; then
    fail "the stamped region of $LIB must not contain a 'Done:' trailer literal — put it in review-gate-spec"
fi
# The gate region MUST carry it, otherwise the split is pointless.
grep -q 'Done:' "$LIB" || fail "the review-gate-spec region should document the 'Done:' trailer"
pass "marker regions nest correctly; the trailer literal lives only outside the stamped block"

# --- 1. DRIFT ----------------------------------------------------------------
say "1. Drift: every consumer is in sync with the source block"
if sh "$GEN" --check; then
    pass "gen-workflow-review.sh --check clean"
else
    fail "a workflow consumer drifted from $LIB — run scripts/gen-workflow-review.sh"
fi

# --- 1b. DRIFT DETECTOR SELF-TEST --------------------------------------------
# Prove the drift gate is not a no-op: on a hermetic scratch copy, a mutation
# inside the generated block MUST make --check fail, and regeneration MUST heal
# it. Without this, a future regression to gen-workflow-review.sh (inverted exit
# code, a consumer list that stops matching the real file) would silently pass
# whenever the real tree happens to be clean.
say "1b. Drift detector fires on planted drift (self-test)"
SCRATCH="$TMP/scratch"
mkdir -p "$SCRATCH/scripts" "$SCRATCH/.claude/workflows/lib"
cp "$GEN" "$SCRATCH/scripts/gen-workflow-review.sh"
cp "$LIB" "$SCRATCH/.claude/workflows/lib/review.mjs"
cp "$WF_DIR/review-refute-fix.js" "$SCRATCH/.claude/workflows/review-refute-fix.js"
# gen-workflow-review.sh lists every consumer; the scratch tree must carry them
# all or the scratch --check fails on a missing consumer rather than on drift.
cp "$WF_DIR/dispatch-phase.js" "$SCRATCH/.claude/workflows/dispatch-phase.js"
cp "$WF_DIR/plan-review.js" "$SCRATCH/.claude/workflows/plan-review.js"
sh "$SCRATCH/scripts/gen-workflow-review.sh" --check >/dev/null 2>&1 ||
    fail "scratch --check should pass on a clean copy"
# Mutate a line INSIDE the generated block, portably (no in-place sed).
sed 's/const CONFIDENCE_FLOOR = 70;/const CONFIDENCE_FLOOR = 999;/' \
    "$SCRATCH/.claude/workflows/review-refute-fix.js" >"$SCRATCH/mutated" &&
    mv "$SCRATCH/mutated" "$SCRATCH/.claude/workflows/review-refute-fix.js"
if sh "$SCRATCH/scripts/gen-workflow-review.sh" --check >/dev/null 2>&1; then
    fail "drift gate did NOT detect planted drift in the scratch consumer"
fi
sh "$SCRATCH/scripts/gen-workflow-review.sh" >/dev/null 2>&1
sh "$SCRATCH/scripts/gen-workflow-review.sh" --check >/dev/null 2>&1 ||
    fail "regeneration did not restore sync in the scratch consumer"
pass "drift detector fails on drift and heals on regenerate"

# --- 1c. SKILL PROJECTION -----------------------------------------------------
# The SAME canonical source projects into the shipped review skill templates via
# scripts/gen-skill-review.sh. Gate it the same way: --check on the real tree,
# then a planted-drift self-test on a scratch copy.
say "1c. Skill projection: shipped review templates are in sync with the source"
if sh "$SKILL_GEN" --check --mode code; then
    pass "gen-skill-review.sh --check --mode code clean"
else
    fail "a review skill template drifted from $LIB — run scripts/gen-skill-review.sh"
fi

SKSCRATCH="$TMP/skill-scratch"
mkdir -p "$SKSCRATCH/scripts" "$SKSCRATCH/.claude/workflows/lib" "$SKSCRATCH/rdm-core/src/templates"
cp "$SKILL_GEN" "$SKSCRATCH/scripts/gen-skill-review.sh"
cp "$LIB" "$SKSCRATCH/.claude/workflows/lib/review.mjs"
for t in skill-review-cli.md skill-review-mcp.md skill-plan-review-cli.md skill-plan-review-mcp.md; do
    cp "$TEMPLATES/$t" "$SKSCRATCH/rdm-core/src/templates/$t"
done
sh "$SKSCRATCH/scripts/gen-skill-review.sh" --check --mode code >/dev/null 2>&1 ||
    fail "scratch skill --check should pass on a clean copy"
# Mutate one `//|` prose line in the scratch SOURCE: --check must fail, and a
# regenerate must heal it.
sed 's/below \*\*70\*\*/below **999**/' "$SKSCRATCH/.claude/workflows/lib/review.mjs" >"$SKSCRATCH/mut" &&
    mv "$SKSCRATCH/mut" "$SKSCRATCH/.claude/workflows/lib/review.mjs"
if sh "$SKSCRATCH/scripts/gen-skill-review.sh" --check --mode code >/dev/null 2>&1; then
    fail 'skill drift gate did NOT detect a planted //| prose change'
fi
sh "$SKSCRATCH/scripts/gen-skill-review.sh" --mode code >/dev/null 2>&1
sh "$SKSCRATCH/scripts/gen-skill-review.sh" --check --mode code >/dev/null 2>&1 ||
    fail "regeneration did not restore skill sync in the scratch tree"
pass "skill drift detector fails on planted prose drift and heals on regenerate"

# The SAME generator renders the plan-review skills from the SAME source, via
# the per-line `//|plan|` mode tag. Gate it exactly like the code mode: --check
# on the real tree, then a planted-drift/heal self-test on the scratch copy.
if sh "$SKILL_GEN" --check --mode plan; then
    pass "gen-skill-review.sh --check --mode plan clean"
else
    fail "a plan-review skill template drifted from $LIB — run scripts/gen-skill-review.sh --mode plan"
fi

# The scratch source was mutated above (code-mode prose), so restore it before
# the plan self-test, then plant drift on a `//|plan|` line specifically.
cp "$LIB" "$SKSCRATCH/.claude/workflows/lib/review.mjs"
for t in skill-review-cli.md skill-review-mcp.md skill-plan-review-cli.md skill-plan-review-mcp.md; do
    cp "$TEMPLATES/$t" "$SKSCRATCH/rdm-core/src/templates/$t"
done
sh "$SKSCRATCH/scripts/gen-skill-review.sh" --check --mode plan >/dev/null 2>&1 ||
    fail "scratch plan --check should pass on a clean copy"
sed 's/\*\*Plan review dimensions:\*\*/**Plan review DIMENSIONS(mutated):**/' \
    "$SKSCRATCH/.claude/workflows/lib/review.mjs" >"$SKSCRATCH/pmut" &&
    mv "$SKSCRATCH/pmut" "$SKSCRATCH/.claude/workflows/lib/review.mjs"
grep -q 'Plan review DIMENSIONS(mutated)' "$SKSCRATCH/.claude/workflows/lib/review.mjs" ||
    fail "plan-mode mutation setup did not actually mutate a //|plan| prose line"
if sh "$SKSCRATCH/scripts/gen-skill-review.sh" --check --mode plan >/dev/null 2>&1; then
    fail 'plan drift gate did NOT detect a planted //|plan| prose change'
fi
# A plan-only mutation must NOT perturb the code render — proof the tags isolate.
sh "$SKSCRATCH/scripts/gen-skill-review.sh" --check --mode code >/dev/null 2>&1 ||
    fail "a //|plan|-only mutation leaked into the code render — the mode tags do not isolate"
sh "$SKSCRATCH/scripts/gen-skill-review.sh" --mode plan >/dev/null 2>&1
sh "$SKSCRATCH/scripts/gen-skill-review.sh" --check --mode plan >/dev/null 2>&1 ||
    fail "regeneration did not restore plan-skill sync in the scratch tree"
sh "$SKSCRATCH/scripts/gen-skill-review.sh" --mode bogus >/dev/null 2>&1 &&
    fail "an unknown --mode must be rejected"
pass "plan drift detector fires on planted //|plan| drift, isolates from code, and heals"

# The generated region is shared byte-for-byte between the cli and mcp
# templates, so it must be identical in both and free of template placeholders.
extract_spec_region() {
    awk '
        index($0, "<!-- rdm:review-spec:begin") { inr = 1; next }
        index($0, "<!-- rdm:review-spec:end") { inr = 0 }
        inr { print }
    ' "$1"
}
extract_spec_region "$TEMPLATES/skill-review-cli.md" >"$TMP/spec-cli"
extract_spec_region "$TEMPLATES/skill-review-mcp.md" >"$TMP/spec-mcp"
[ -s "$TMP/spec-cli" ] || fail "the generated spec region in skill-review-cli.md is EMPTY"
diff -u "$TMP/spec-cli" "$TMP/spec-mcp" >/dev/null 2>&1 ||
    fail "the generated spec region differs between the cli and mcp review templates"
if grep -nE '\{proj_flag\}|\{proj_param\}|\{t_[a-z_]+\}|\{principles\}' "$TMP/spec-cli" >&2; then
    fail "a template placeholder leaked into the shared generated review spec"
fi
# Prose <-> DIMENSIONS consistency: every code dimension key must be named in the
# rendered fleet, and the retired verdict vocabulary must be gone everywhere.
for key in ac correctness tests architecture api-docs changelog security; do
    grep -q "\*\*$key\*\*" "$TMP/spec-cli" ||
        fail "the rendered review spec does not document the '$key' dimension"
done
for word in reviewed rework escalated; do
    grep -q "$word" "$TMP/spec-cli" || fail "the rendered review spec is missing the '$word' outcome"
done
for t in skill-review-cli.md skill-review-mcp.md; do
    if grep -n 'PASS WITH CONCERNS' "$TEMPLATES/$t" >&2; then
        fail "$t still uses the retired PASS WITH CONCERNS verdict"
    fi
    if grep -n 'tasks have no .blocked. status' "$TEMPLATES/$t" >&2; then
        fail "$t still claims tasks have no blocked status"
    fi
    grep -q 'rdm hook done-line' "$TEMPLATES/$t" ||
        fail "$t must source the completion trailer from 'rdm hook done-line'"
done
pass "shared spec region is byte-identical, placeholder-free, and documents all seven dimensions"

# --- 1d. PLAN SPEC PROJECTION -------------------------------------------------
# The plan render is produced by the same emitter from the same regions, so it
# gets the same battery — plus mode-isolation greps in BOTH directions, which
# are the detector for a mistagged (or untagged) prose line leaking across.
say "1d. Plan spec region: rendered, isolated from the code render, and gate-preserving"
extract_spec_region "$TEMPLATES/skill-plan-review-cli.md" >"$TMP/plan-spec-cli"
extract_spec_region "$TEMPLATES/skill-plan-review-mcp.md" >"$TMP/plan-spec-mcp"
[ -s "$TMP/plan-spec-cli" ] || fail "the generated spec region in skill-plan-review-cli.md is EMPTY"
diff -u "$TMP/plan-spec-cli" "$TMP/plan-spec-mcp" >/dev/null 2>&1 ||
    fail "the generated spec region differs between the cli and mcp plan-review templates"
if grep -nE '\{proj_flag\}|\{proj_param\}|\{t_[a-z_]+\}|\{principles\}' "$TMP/plan-spec-cli" >&2; then
    fail "a template placeholder leaked into the shared generated plan-review spec"
fi
for key in coherence architectural-fit unit-of-work; do
    grep -q "\*\*$key\*\*" "$TMP/plan-spec-cli" ||
        fail "the rendered plan spec does not document the '$key' dimension"
done
grep -q '\*trigger: the target is a phase\.\*' "$TMP/plan-spec-cli" ||
    fail "the rendered plan spec must gate unit-of-work on the phase target type"
for word in reviewed rework escalated; do
    grep -q "$word" "$TMP/plan-spec-cli" || fail "the rendered plan spec is missing the '$word' outcome"
done
grep -q 'needs-plan-review' "$TMP/plan-spec-cli" ||
    fail "the rendered plan spec must document the needs-plan-review gate"
grep -q 'no gate at all' "$TMP/plan-spec-cli" ||
    fail "the rendered plan spec must carry the --implementation-plan no-gate carve-out"
grep -q 'gate each phase \*\*individually\*\*' "$TMP/plan-spec-cli" ||
    fail "the rendered plan spec must carry per-phase --roadmap gating"

# Mode isolation, both directions. A code-only line left untagged would ship
# into the plan skill (and vice versa); these greps are the detector.
for bad in '\*\*ac\*\*' '\*\*changelog\*\*' '\*\*security\*\*' 'rdm hook done-line' 'AC table' 'AC FAIL'; do
    if grep -nE "$bad" "$TMP/plan-spec-cli" >&2; then
        fail "code-only prose ($bad) leaked into the generated plan spec — tag it //|code|"
    fi
done
for bad in 'needs-plan-review' '\*\*unit-of-work\*\*'; do
    if grep -nE "$bad" "$TMP/spec-cli" >&2; then
        fail "plan-only prose ($bad) leaked into the generated code spec — tag it //|plan|"
    fi
done

# The retired vocabulary may survive ONLY inside the generated block, and only
# as the explicit "PASS/PWC collapse to reviewed" mapping note. The
# hand-authored prose must speak the new vocabulary exclusively.
for t in skill-plan-review-cli.md skill-plan-review-mcp.md; do
    awk 'index($0, "<!-- rdm:review-spec:begin") { exit } { print }' "$TEMPLATES/$t" >"$TMP/plan-hand"
    for retired in 'PASS WITH CONCERNS' 'REWORK'; do
        if grep -n "$retired" "$TMP/plan-hand" >&2; then
            fail "$t still uses the retired $retired verdict in its hand-authored prose"
        fi
    done
    grep -q 'find → refute → filter → verdict → act → gate' "$TMP/plan-hand" ||
        fail "$t must describe the canonical find → refute → filter → verdict → act → gate pipeline"
done
pass "plan spec region is byte-identical, placeholder-free, mode-isolated, and gate-preserving"

# --- 1e. NO SECOND MECHANISM --------------------------------------------------
# The plan surface must reuse the ONE generator and the ONE dimension table.
say "1e. No second mechanism: one generator, exactly two dimension modes"
for f in "$REPO_ROOT"/scripts/gen-plan-review*; do
    [ -e "$f" ] || continue
    fail "a second plan-review generator exists ($f) — --mode plan is the sole renderer"
done
MODE_KEYS=$(run_node -e '
  import("file://" + process.argv[1]).then((m) => {
    console.log(Object.keys(m.DIMENSIONS).join(","));
  });
' "$LIB")
[ "$MODE_KEYS" = "code,plan" ] ||
    fail "review.mjs must declare exactly the two DIMENSIONS modes code,plan (got: $MODE_KEYS)"
pass "one generator, one dimension table with exactly the code and plan modes"

# --- 1f. NO DANGLING GENERATOR REFERENCES IN SHIPPED TEMPLATES ----------------
# `.claude/workflows/lib/review.mjs` and `scripts/gen-skill-review.sh` are
# dogfood-only tooling (see the `distribute-workflow-lane` roadmap, Phase 1's
# landed decision recorded in its commit message: "lib/*.mjs is deliberately
# not shipped since no regeneration script travels downstream to consume
# it") — a consumer repo never has them. No shipped skill template may
# instruct the reader to edit or run either path.
say "1f. No dangling generator references in shipped templates"
if grep -lE 'scripts/gen-skill-review\.sh|\.claude/workflows/lib/review\.mjs' "$TEMPLATES"/*.md; then
    fail "a shipped template references scripts/gen-skill-review.sh or .claude/workflows/lib/review.mjs — these do not exist in a consumer repo"
fi
# Self-test: the grep MUST catch a planted dangling reference.
printf 'edit .claude/workflows/lib/review.mjs and run scripts/gen-skill-review.sh\n' >"$SCRATCH/planted-dangling.md"
if ! grep -lE 'scripts/gen-skill-review\.sh|\.claude/workflows/lib/review\.mjs' "$SCRATCH/planted-dangling.md" >/dev/null 2>&1; then
    fail "dangling-reference grep did NOT catch a planted reference — the detector is broken"
fi
pass "no shipped template references the dogfood-only generator or its source module"

# --- 2. HYGIENE --------------------------------------------------------------
say "2. Hygiene: no forbidden nondeterministic global in workflow scripts"
if grep -nE 'Date\.now\(|Math\.random\(' "$WF_DIR"/*.js "$WF_DIR"/lib/*.mjs 2>/dev/null; then
    fail "found Date.now( / Math.random( in a workflow script — the runtime forbids them"
fi
# Self-test: the grep MUST catch a planted violation (guards against a glob that
# silently matches zero files, turning the check into a no-op).
printf 'const x = Date.now();\n' >"$SCRATCH/planted.js"
if ! grep -nE 'Date\.now\(|Math\.random\(' "$SCRATCH/planted.js" >/dev/null 2>&1; then
    fail "hygiene grep did NOT catch a planted Date.now() — the detector is broken"
fi
pass "no forbidden globals present; detector catches a planted one"

# --- 3. BEHAVIOR -------------------------------------------------------------
say "3. Behavior: find -> refute -> filter, both modes, deterministic"

cat >"$TMP/test.mjs" <<'NODE_TEST'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const libPath = process.argv[2];
const mod = await import(pathToFileURL(libPath).href);
const {
  buildReviewPipeline,
  DIMENSIONS,
  SIGNAL_KEYS,
  OUTCOMES,
  statusFor,
  writesCompletion,
  selectDimensions,
  deriveSignals,
  classifyOutcome,
  findPrompt,
  survives,
  rankFindings,
  CONFIDENCE_FLOOR,
} = mod;

// --- reference pipeline/parallel: faithful to the real Workflow runtime -------
// Both are order-preserving (Promise.all). Their ERROR semantics mirror the
// documented runtime contract, so the module's degraded-path behavior is
// actually exercised here:
//   parallel — "a thunk that throws resolves to null in the result array".
//   pipeline — "a stage that throws drops that item to null, skipping the rest".
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
          return null; // a throwing stage drops this item to null
        }
      }
      return acc;
    })
  );
}

// --- recording fake agent: returns planted data keyed by label, logs calls ----
// label is `find:<mode>:<dimKey>` or `refute:<mode>:<findingId>`.
function makeSpyAgent(plantFindings, plantVerdicts) {
  const calls = [];
  async function agent(prompt, opts) {
    const label = (opts && opts.label) || '';
    calls.push({ label, prompt });
    const parts = label.split(':');
    if (parts[0] === 'find') {
      return { findings: plantFindings[parts[2]] || [] };
    }
    if (parts[0] === 'refute') {
      const findingId = parts.slice(2).join(':');
      return plantVerdicts[findingId] || { refuted: false, confidence: 90 };
    }
    throw new Error('unexpected agent label: ' + label);
  }
  return { agent, calls };
}

function deps(spy) {
  return { agent: spy.agent, pipeline: refPipeline, parallel: refParallel, log: () => {} };
}

const CTX = { target: 'phase widget/phase-1-foo' };

// ============================================================================
// Pure unit checks — the survival rule and the total ordering.
// ============================================================================
assert.equal(CONFIDENCE_FLOOR, 70, 'confidence floor is 70');
assert.equal(survives({ confidence: 70 }, { refuted: false }), true, 'exactly at floor survives');
assert.equal(survives({ confidence: 69 }, { refuted: false }), false, 'below floor dropped');
assert.equal(survives({ confidence: 100 }, { refuted: true }), false, 'refuted dropped regardless of confidence');

const ranked = rankFindings([
  { id: 'b', severity: 'concern', confidence: 80 },
  { id: 'a', severity: 'concern', confidence: 80 },
  { id: 'c', severity: 'blocking', confidence: 10 },
  { id: 'd', severity: 'suggestion', confidence: 99 },
  { id: 'e', severity: 'concern', confidence: 90 },
]);
assert.deepEqual(
  ranked.map((f) => f.id),
  ['c', 'e', 'a', 'b', 'd'],
  'total order: blocking first; concerns by confidence desc then id; suggestion last'
);

// ============================================================================
// CODE mode — refutable dropped, low-confidence dropped, real survives.
// ============================================================================
// Only `correctness` plants findings; every other dimension comes back clean.
// With no `context.signals` the pipeline runs ALL code dimensions (fail-open).
const CODE_DIM_KEYS = DIMENSIONS.code.map((d) => d.key);
const codeFindings = {
  correctness: [
    { id: 'real-bug', concern: 'correctness', severity: 'blocking', confidence: 90, what_fails: 'off-by-one' },
    { id: 'false-alarm', concern: 'correctness', severity: 'concern', confidence: 85, what_fails: 'looks wrong but is fine' },
    { id: 'low-conf', concern: 'correctness', severity: 'suggestion', confidence: 50, what_fails: 'possible nit' },
  ],
};
const codeVerdicts = {
  'real-bug': { refuted: false, confidence: 95 },
  'false-alarm': { refuted: true, confidence: 88 }, // refuter kills it
  'low-conf': { refuted: false, confidence: 40 }, // NOT refuted, but finding confidence < floor
};

const spy = makeSpyAgent(codeFindings, codeVerdicts);
const out = await buildReviewPipeline('code', deps(spy))(CTX);

assert.deepEqual(out.map((f) => f.id), ['real-bug'], 'code: only the real, un-refuted, high-confidence finding survives');

const findCalls = spy.calls.filter((c) => c.label.startsWith('find:'));
const refuteCalls = spy.calls.filter((c) => c.label.startsWith('refute:'));
assert.equal(findCalls.length, CODE_DIM_KEYS.length, 'one finder per code dimension');
assert.equal(refuteCalls.length, 3, 'a fresh refuter per finding');
assert.equal(
  new Set(findCalls.map((c) => c.label)).size,
  CODE_DIM_KEYS.length,
  'finder labels are distinct per dimension'
);
// A fresh refuter grades each finding: every refuter call is distinctly labelled
// (no collisions even on duplicate ids), and no refuter reuses a finder's prompt.
assert.equal(new Set(refuteCalls.map((c) => c.label)).size, refuteCalls.length, 'refuter labels are unique per finding');
const findPromptSet = new Set(findCalls.map((c) => c.prompt));
assert.ok(refuteCalls.every((c) => !findPromptSet.has(c.prompt)), 'refuter prompts differ from finder prompts');
assert.ok(findCalls.every((c) => c.prompt.includes(CTX.target)), 'context.target threaded into finder prompts');
assert.ok(refuteCalls.every((c) => c.prompt.includes(CTX.target)), 'context.target threaded into refuter prompts');

// OUTCOME shape sanity (matches the FINDING contract in docs/workflow-schemas.md).
assert.ok(Array.isArray(out), 'OUTCOME is an array');
for (const f of out) {
  assert.equal(typeof f.id, 'string');
  assert.ok(['blocking', 'concern', 'suggestion'].includes(f.severity));
  assert.equal(typeof f.confidence, 'number');
}

// Determinism: two independent runs produce byte-identical output.
const outA = await buildReviewPipeline('code', deps(makeSpyAgent(codeFindings, codeVerdicts)))(CTX);
const outB = await buildReviewPipeline('code', deps(makeSpyAgent(codeFindings, codeVerdicts)))(CTX);
assert.equal(JSON.stringify(outA), JSON.stringify(outB), 'code review output is deterministic across runs');

// ============================================================================
// Resilience — a single agent failure degrades gracefully, never crashes.
// ============================================================================
// A finder that throws drops ONLY its dimension (the runtime pipeline sends a
// thrown stage to null); the other dimensions still complete. Here the only
// planted findings live in `correctness`, so a thrown `correctness` finder
// yields zero survivors WITHOUT rejecting the whole review.
{
  const spyF = makeSpyAgent(codeFindings, codeVerdicts);
  const base = spyF.agent;
  spyF.agent = async (prompt, opts) => {
    if (opts && opts.label === 'find:code:correctness') throw new Error('boom finder');
    return base(prompt, opts);
  };
  const rOut = await buildReviewPipeline('code', deps(spyF))(CTX);
  assert.deepEqual(rOut, [], 'a thrown finder drops its dimension; others survive; no crash');
}

// A refuter that throws must NOT silently drop the finding — a crash is not proof
// of refutation. The finding is kept as un-refuted and survives if confidence ≥
// floor (locks in the module's refuter-error .catch).
{
  const realOnly = {
    correctness: [{ id: 'infra', concern: 'correctness', severity: 'blocking', confidence: 90, what_fails: 'real bug' }],
  };
  const spyR = makeSpyAgent(realOnly, {});
  const base = spyR.agent;
  spyR.agent = async (prompt, opts) => {
    if (opts && opts.label.startsWith('refute:')) throw new Error('boom refuter');
    return base(prompt, opts);
  };
  const rOut = await buildReviewPipeline('code', deps(spyR))(CTX);
  assert.deepEqual(rOut.map((f) => f.id), ['infra'], 'a refuter crash keeps the finding un-refuted, not silently dropped');
}

// ============================================================================
// PLAN mode — same battery on the plan dimension set (thin variation).
// ============================================================================
assert.deepEqual(
  DIMENSIONS.plan.map((d) => d.key),
  ['coherence', 'architectural-fit', 'unit-of-work'],
  'plan dimension set'
);

const planFindings = {
  coherence: [
    { id: 'vague-step', concern: 'coherence', severity: 'blocking', confidence: 88, what_fails: 'step 3 is ambiguous' },
    { id: 'nonissue', concern: 'coherence', severity: 'concern', confidence: 80, what_fails: 'reads odd but is fine' },
    { id: 'weak-plan', concern: 'coherence', severity: 'suggestion', confidence: 50, what_fails: 'minor plan nit' },
  ],
  'architectural-fit': [],
  'unit-of-work': [],
};
const planVerdicts = {
  'vague-step': { refuted: false, confidence: 90 },
  nonissue: { refuted: true, confidence: 85 }, // refuter kills it
  'weak-plan': { refuted: false, confidence: 40 }, // NOT refuted, but below floor
};

const pspy = makeSpyAgent(planFindings, planVerdicts);
const pout = await buildReviewPipeline('plan', deps(pspy))(CTX);

// refutable dropped, below-floor dropped, real survives — full parity with code mode.
assert.deepEqual(pout.map((f) => f.id), ['vague-step'], 'plan: refutable dropped, below-floor dropped, real survives');
const pFind = pspy.calls.filter((c) => c.label.startsWith('find:'));
const pRefute = pspy.calls.filter((c) => c.label.startsWith('refute:'));
assert.equal(pFind.length, 3, 'one finder per plan dimension');
assert.equal(pRefute.length, 3, 'a fresh refuter per plan finding');
assert.ok(pFind.every((c) => c.label.startsWith('find:plan:')), 'plan finders labelled by dimension');
assert.ok(pFind.every((c) => c.prompt.includes(CTX.target)), 'context.target threaded into plan finder prompts');

// Plan-mode determinism parity with code mode.
const poutA = await buildReviewPipeline('plan', deps(makeSpyAgent(planFindings, planVerdicts)))(CTX);
const poutB = await buildReviewPipeline('plan', deps(makeSpyAgent(planFindings, planVerdicts)))(CTX);
assert.equal(JSON.stringify(poutA), JSON.stringify(poutB), 'plan review output is deterministic across runs');

// Unknown mode is rejected, not silently empty.
assert.throws(() => buildReviewPipeline('bogus', deps(spy)), /unknown review mode/, 'unknown mode throws');

// ============================================================================
// AC2 — code-mode findPrompt output is byte-exact against a baseline captured
// BEFORE the plan-severity-calibration change. Any difference (including a
// single stray byte) means the calibration work leaked into code-mode prompts.
// ============================================================================
const CODE_PROMPT_BASELINE = {
  ac: 'You are a READ-ONLY reviewer. Do not edit any files.\nReview target: phase widget/phase-1-foo.\nInspect the implementation diff (use git log / git diff in the worktree).\nYour single dimension is AC compliance (ac). For each acceptance criterion in the target, rate PASS / FAIL / PARTIAL with evidence (file:line, test name). Flag any criterion that is unmet, ambiguous, or untestable.\nReport only findings you can back with concrete evidence. One strong finding beats five weak ones.\nReturn JSON matching the FINDINGS schema: a `findings` array, each with id, concern, location, severity (blocking|concern|suggestion), confidence (0-100), what_fails, why, recommendation.\nReturn an empty `findings` array if the dimension is clean.',
  correctness: 'You are a READ-ONLY reviewer. Do not edit any files.\nReview target: phase widget/phase-1-foo.\nInspect the implementation diff (use git log / git diff in the worktree).\nYour single dimension is Correctness & error handling (correctness). Logic bugs, edge cases, race conditions, and error paths. In rdm-core, errors must be hand-written matchable enums (no anyhow / type erasure); in rdm-cli / rdm-server, anyhow with .context(). User-facing CLI errors must be actionable.\nReport only findings you can back with concrete evidence. One strong finding beats five weak ones.\nReturn JSON matching the FINDINGS schema: a `findings` array, each with id, concern, location, severity (blocking|concern|suggestion), confidence (0-100), what_fails, why, recommendation.\nReturn an empty `findings` array if the dimension is clean.',
  tests: 'You are a READ-ONLY reviewer. Do not edit any files.\nReview target: phase widget/phase-1-foo.\nInspect the implementation diff (use git log / git diff in the worktree).\nYour single dimension is Tests (tests). Do tests exist and cover the key behaviors and edge cases? Was TDD followed? Are there untested branches or newly added logic with no test?\nReport only findings you can back with concrete evidence. One strong finding beats five weak ones.\nReturn JSON matching the FINDINGS schema: a `findings` array, each with id, concern, location, severity (blocking|concern|suggestion), confidence (0-100), what_fails, why, recommendation.\nReturn an empty `findings` array if the dimension is clean.',
  architecture: 'You are a READ-ONLY reviewer. Do not edit any files.\nReview target: phase widget/phase-1-foo.\nInspect the implementation diff (use git log / git diff in the worktree).\nYour single dimension is Architecture (architecture). Does logic live in rdm-core with cli/server as thin layers? No duplicated logic across interfaces? Correct core/cli/server separation and conventional-commit scope discipline.\nReport only findings you can back with concrete evidence. One strong finding beats five weak ones.\nReturn JSON matching the FINDINGS schema: a `findings` array, each with id, concern, location, severity (blocking|concern|suggestion), confidence (0-100), what_fails, why, recommendation.\nReturn an empty `findings` array if the dimension is clean.',
};
// Scoped to the dimensions that existed when the baseline was captured; the
// dimensions added later (api-docs, changelog, security) are covered by the
// coverage-parity assertion below instead.
for (const dim of DIMENSIONS.code.filter((d) => CODE_PROMPT_BASELINE[d.key])) {
  assert.equal(
    findPrompt('code', dim, CTX),
    CODE_PROMPT_BASELINE[dim.key],
    'code-mode findPrompt("' + dim.key + '") must stay byte-identical to the pre-calibration baseline'
  );
}
console.log('AC2: code-mode findPrompt output is byte-exact against the pre-calibration baseline');

// ============================================================================
// AC1 — every plan-mode findPrompt output carries the plan-stage severity
// calibration contract (blocking = goal/approach/scope/architectural-constraint
// violation; a proposed-code/shell defect is a concern, not a gate). Exported
// as a function so the scratch mutation self-test (shell section 4) can run
// the identical check against a mutated copy of the module.
// ============================================================================
const PLAN_CALIBRATION_KEYPHRASES = [
  'blocking` means the goal, approach, or scope is wrong, or the plan violates a stated architectural constraint',
  'concern` that rides along as an implementation note for the implementing agent',
];

function assertPlanCalibrationPresent(m) {
  for (const dim of m.DIMENSIONS.plan) {
    const prompt = m.findPrompt('plan', dim, CTX);
    for (const phrase of PLAN_CALIBRATION_KEYPHRASES) {
      assert.ok(
        prompt.includes(phrase),
        'plan-mode findPrompt("' + dim.key + '") is missing calibration keyphrase: ' + phrase
      );
    }
  }
}
assertPlanCalibrationPresent(mod);
console.log('AC1: every plan-mode findPrompt output carries the severity-calibration keyphrases');

// The pre-existing "empty or ambiguous plan is itself blocking" coherence rule
// must survive the calibration edit untouched — not deleted, not overwritten.
assert.ok(
  DIMENSIONS.plan
    .find((d) => d.key === 'coherence')
    .focus.includes('An empty or ambiguous plan is itself a blocking finding'),
  "coherence's pre-existing empty/ambiguous-plan rule must still be present"
);

// ============================================================================
// AC4 — an architectural-violation finding (the review-verify tier-downgrade
// class) still comes back `blocking` on the first pass, ranked ahead of an
// implementation-detail nit that must NOT be `blocking`. Proves the
// calibration prompt text does not, and structurally cannot, cause the
// pipeline itself to downgrade or drop a legitimate architectural blocker —
// paired in one buildReviewPipeline('plan', ...) call per the plan.
// ============================================================================
const calibrationFindings = {
  coherence: [],
  'architectural-fit': [
    {
      id: 'tier-downgrade',
      concern: 'architectural-fit',
      severity: 'blocking',
      confidence: 92,
      what_fails: 'The plan silently downgrades the review tier on failure, violating the stated model-tier binding contract.',
    },
  ],
  'unit-of-work': [
    {
      id: 'impl-nit',
      concern: 'unit-of-work',
      severity: 'concern',
      confidence: 85,
      what_fails: 'Off-by-one in the loop bound of the proposed pseudo-code snippet.',
    },
  ],
};
const calibrationVerdicts = {
  'tier-downgrade': { refuted: false, confidence: 95 },
  'impl-nit': { refuted: false, confidence: 88 },
};
const cspy = makeSpyAgent(calibrationFindings, calibrationVerdicts);
const cout = await buildReviewPipeline('plan', deps(cspy))(CTX);
assert.deepEqual(
  cout.map((f) => f.id),
  ['tier-downgrade', 'impl-nit'],
  'both the architectural-violation finding and the implementation-detail nit survive refutation/floor'
);
assert.equal(
  cout.find((f) => f.id === 'tier-downgrade').severity,
  'blocking',
  'the architectural-violation (tier-downgrade class) finding still yields blocking'
);
assert.notEqual(
  cout.find((f) => f.id === 'impl-nit').severity,
  'blocking',
  'the implementation-detail nit is not blocking'
);
assert.equal(
  cout[0].id,
  'tier-downgrade',
  'the blocking architectural finding ranks ahead of the concern-severity nit'
);
console.log('AC4: an architectural-violation finding still yields blocking, ranked ahead of an implementation nit');

// ============================================================================
// Model threading + the null-agent loud-failure guard.
//
// An unknown model id makes agent() RESOLVE to null (docs/workflow-schemas.md
// § "agent() options spike"). Without a guard, a null finder is laundered into
// [] by the refute stage's `(found && …) || []` and the review reports CLEAN.
// ============================================================================
function nullAgent() {
  const calls = [];
  return {
    calls,
    agent: async (prompt, opts) => {
      calls.push({ label: (opts && opts.label) || '', model: opts && opts.model });
      return null;
    },
  };
}

// (a) With an explicit findModel, an all-null finder sweep must REJECT, never
//     resolve to a clean review.
const nspy = nullAgent();
await assert.rejects(
  buildReviewPipeline('code', deps(nspy))({ ...CTX, findModel: 'bogus-model', verifyModel: 'bogus-model' }),
  /every code dimension finder failed|returned null with model/,
  'all-null finders with an explicit model must fail loudly, not report clean'
);

// (b) The models are actually threaded onto the agent() options.
assert.ok(nspy.calls.length > 0, 'finders were dispatched');
assert.ok(
  nspy.calls.every((c) => c.model === 'bogus-model'),
  'findModel is threaded onto every finder agent() call'
);

// (c) WITHOUT a model (the standalone consumer), today's behavior is preserved:
//     null finders degrade to an empty review rather than throwing.
const nspy2 = nullAgent();
const degraded = await buildReviewPipeline('code', deps(nspy2))(CTX);
assert.deepEqual(degraded, [], 'no-model callers keep the pre-existing lenient behavior');
assert.ok(
  nspy2.calls.every((c) => c.model === undefined),
  'no model key is invented when the caller supplies none'
);

// (d) A genuinely clean review (findings: []) must NOT trip the guard.
const cleanSpy = makeSpyAgent({}, {});
const cleanOut = await buildReviewPipeline('code', deps(cleanSpy))({ ...CTX, findModel: 'haiku', verifyModel: 'opus' });
assert.deepEqual(cleanOut, [], 'a real clean review still returns [] with models set');

// ============================================================================
// AC4 — dimension coverage parity + `when` triggers over diff shape AND target
// type, with the fail-open contract of selectDimensions.
// ============================================================================
assert.deepEqual(
  DIMENSIONS.code.map((d) => d.key).sort(),
  ['ac', 'api-docs', 'architecture', 'changelog', 'correctness', 'security', 'tests'],
  'code dimension set is exactly the pre-existing skill fleet plus security — no coverage regression'
);
assert.deepEqual(
  DIMENSIONS.code.filter((d) => !d.when).map((d) => d.key),
  ['ac', 'correctness'],
  'ac and correctness are the always-on dimensions'
);

// (a) Omitted / null / undefined signals → EVERY dimension. This is the
//     regression test for the `d.when(signals || {})` bug, which would have
//     returned only ac + correctness exactly when the caller knew least.
for (const call of [() => selectDimensions('code'), () => selectDimensions('code', null), () => selectDimensions('code', undefined)]) {
  assert.deepEqual(
    call().map((d) => d.key),
    DIMENSIONS.code.map((d) => d.key),
    'omitted signals must fail OPEN to all dimensions'
  );
}

// (b) An explicit `{}` means "computed, nothing triggered" — a DIFFERENT path.
assert.deepEqual(
  selectDimensions('code', {}).map((d) => d.key),
  ['ac', 'correctness'],
  'an explicit empty signals object selects only the always-on dimensions'
);

// (c) A docs-only change derived from deriveSignals trips nothing.
const docsSignals = deriveSignals({ targetType: 'phase', changedFiles: ['docs/workflow-schemas.md'] });
assert.deepEqual(
  selectDimensions('code', docsSignals).map((d) => d.key),
  ['ac', 'correctness'],
  'a docs-only change runs only the always-on dimensions'
);

// (d) Each trigger fires on its own signal and only on its own signal.
const TRIGGER_MATRIX = [
  ['securitySurface', 'security'],
  ['hasUnsafe', 'security'],
  ['publicApiChanged', 'api-docs'],
  ['userFacing', 'changelog'],
  ['changesLogic', 'tests'],
  ['missingTests', 'tests'],
  ['multiModule', 'architecture'],
];
for (const [signal, dim] of TRIGGER_MATRIX) {
  const on = selectDimensions('code', { [signal]: true }).map((d) => d.key);
  assert.ok(on.includes(dim), signal + ': true must include the ' + dim + ' dimension');
  const off = selectDimensions('code', { [signal]: false }).map((d) => d.key);
  assert.ok(!off.includes(dim), signal + ': false must exclude the ' + dim + ' dimension');
}

// (e) Target type is a first-class signal: plan-mode unit-of-work is phases-only.
assert.ok(
  selectDimensions('plan', { targetType: 'phase' }).map((d) => d.key).includes('unit-of-work'),
  'plan mode on a phase includes unit-of-work'
);
assert.ok(
  !selectDimensions('plan', { targetType: 'task' }).map((d) => d.key).includes('unit-of-work'),
  'plan mode on a task excludes unit-of-work'
);
assert.deepEqual(
  selectDimensions('plan', { targetType: 'task' }).map((d) => d.key),
  ['coherence', 'architectural-fit'],
  'coherence and architectural-fit stay always-on in plan mode'
);

// (f) deriveSignals is deterministic and FULLY populated (never a partial object).
const derivedInput = {
  targetType: 'phase',
  changedFiles: ['rdm-core/src/hook.rs', 'rdm-cli/src/commands/hook.rs', 'rdm-cli/tests/cli_hook.rs'],
  diffText: '+pub fn format_done_directive() {}\n+    let out = std::process::Command::new("git");\n',
};
const d1 = deriveSignals(derivedInput);
const d2 = deriveSignals(derivedInput);
assert.equal(JSON.stringify(d1), JSON.stringify(d2), 'deriveSignals is deterministic across invocations');
for (const key of SIGNAL_KEYS) {
  assert.equal(typeof d1[key], 'boolean', 'deriveSignals must set ' + key + ' to an explicit boolean');
}
assert.equal(d1.targetType, 'phase', 'deriveSignals carries the target type through');
assert.equal(d1.publicApiChanged, true, 'a `+pub` line under rdm-core/src trips publicApiChanged');
assert.equal(d1.userFacing, true, 'an rdm-cli path trips userFacing');
assert.equal(d1.multiModule, true, 'files in three directories trip multiModule');
assert.equal(d1.missingTests, false, 'a changed test file clears missingTests');
assert.equal(d1.securitySurface, true, 'std::process in the diff trips securitySurface');
assert.equal(d1.hasUnsafe, false, 'no added `unsafe` line means hasUnsafe stays false');
assert.equal(
  deriveSignals({ changedFiles: ['rdm-core/src/model.rs'], diffText: '+    unsafe { ptr.read() }\n' }).hasUnsafe,
  true,
  'an added `unsafe` line trips hasUnsafe'
);
assert.deepEqual(deriveSignals(), {
  targetType: null,
  changedFiles: [],
  changesLogic: false,
  missingTests: false,
  multiModule: false,
  publicApiChanged: false,
  userFacing: false,
  securitySurface: false,
  hasUnsafe: false,
}, 'deriveSignals with no input is fully populated and all-false');

// (g) Unknown mode throws in both entry points; the always-on set makes an empty
//     selection unreachable, but the guard is asserted structurally.
assert.throws(() => selectDimensions('bogus', {}), /unknown review mode/, 'selectDimensions rejects an unknown mode');
assert.throws(() => selectDimensions('bogus'), /unknown review mode/, 'selectDimensions rejects an unknown mode with no signals');

// (h) The pipeline actually honours the selection: an explicit `{}` narrows the
//     fleet to the always-on dimensions.
{
  const narrowSpy = makeSpyAgent(codeFindings, codeVerdicts);
  await buildReviewPipeline('code', deps(narrowSpy))({ ...CTX, signals: {} });
  const labels = narrowSpy.calls.filter((c) => c.label.startsWith('find:')).map((c) => c.label);
  assert.deepEqual(labels.sort(), ['find:code:ac', 'find:code:correctness'], 'context.signals narrows the dispatched fleet');
}
console.log('AC4: dimension coverage parity, `when` triggers, and the selectDimensions fail-open contract hold');

// ============================================================================
// AC1/AC2 — classifyOutcome in its new home, and the outcome→status mapping.
// ============================================================================
assert.deepEqual(OUTCOMES, ['reviewed', 'rework', 'escalated'], 'the canonical outcome vocabulary');

const BLOCKER = [{ id: 'x', severity: 'blocking', confidence: 90, what_fails: 'boom' }];
assert.equal(classifyOutcome({ planFindings: BLOCKER }), 'escalated', 'a blocking plan finding escalates');
assert.equal(
  classifyOutcome({ planFindings: [], codeReviews: [BLOCKER, BLOCKER] }),
  'rework',
  'a blocking finding on the LAST code round yields rework'
);
assert.equal(classifyOutcome({ planFindings: [], codeReviews: [[]] }), 'reviewed', 'a clean review yields reviewed');
assert.equal(
  classifyOutcome({ planFindings: [], codeFindings: BLOCKER, maxRework: 0 }),
  'rework',
  'budget-0 with a blocking first pass yields rework, never a laundered reviewed'
);

assert.equal(statusFor('reviewed', 'phase'), 'reviewed');
assert.equal(statusFor('reviewed', 'task'), 'reviewed');
assert.equal(statusFor('rework', 'phase'), 'in-progress');
assert.equal(statusFor('rework', 'task'), 'in-progress');
assert.equal(statusFor('escalated', 'phase'), 'blocked');
assert.equal(statusFor('escalated', 'task'), 'blocked', 'an escalated TASK is blocked, not downgraded to in-progress');
assert.throws(() => statusFor('PASS', 'phase'), /unknown outcome/, 'a retired verdict word throws');
assert.throws(() => statusFor('reviewed', 'roadmap'), /unknown item kind/, 'an unknown item kind throws');
assert.equal(writesCompletion('reviewed'), true, 'only a clean review writes the completion trailer');
assert.equal(writesCompletion('rework'), false);
assert.equal(writesCompletion('escalated'), false);
assert.throws(() => writesCompletion('BLOCKED'), /unknown outcome/);
console.log('AC1/AC2: classifyOutcome truth table and the outcome->status mapping hold');

// ============================================================================
// The mode-dispatched gate policy. `code` must be the SAME table statusFor /
// writesCompletion already read (re-expressed, not forked); `plan` must
// reproduce today's PASS/PWC -> clear, REWORK -> leave outcome under the new
// vocabulary, and must never persist an rdm status.
// ============================================================================
const { GATE_POLICY, gateFor } = mod;
assert.deepEqual(Object.keys(GATE_POLICY), ['code', 'plan'], 'exactly two gate modes');
assert.equal(GATE_POLICY.code, mod.STATUS_MAPPING, 'STATUS_MAPPING IS GATE_POLICY.code — one table, not a fork');

// code rows: today's behaviour, plus an explicit "clears nothing" flag.
assert.equal(gateFor('code', 'reviewed').status, 'reviewed');
assert.equal(gateFor('code', 'reviewed').writesCompletion, true);
assert.equal(gateFor('code', 'reviewed').clearsPlanReviewTag, false, 'the code gate never touches the plan-review tag');
assert.equal(gateFor('code', 'rework').clearsPlanReviewTag, false);
assert.equal(gateFor('code', 'escalated').clearsPlanReviewTag, false);
assert.equal(gateFor('code', 'escalated').reasonPrefix, '[code]');

// plan rows: reviewed clears the tag, rework/escalated leave it, status is a
// literal null (never undefined — a caller must not persist an empty status).
assert.equal(gateFor('plan', 'reviewed').clearsPlanReviewTag, true, 'plan reviewed clears needs-plan-review');
assert.equal(gateFor('plan', 'rework').clearsPlanReviewTag, false, 'plan rework leaves needs-plan-review');
assert.equal(gateFor('plan', 'escalated').clearsPlanReviewTag, false, 'plan escalated leaves needs-plan-review');
assert.equal(gateFor('plan', 'escalated').reasonPrefix, '[plan]');
for (const outcome of OUTCOMES) {
  const row = gateFor('plan', outcome);
  assert.ok('status' in row, 'plan row must declare status explicitly: ' + outcome);
  assert.strictEqual(row.status, null, 'a plan review never persists an rdm status: ' + outcome);
  assert.equal(row.writesCompletion, false, 'a plan review never writes the completion directive: ' + outcome);
}

// Unknown keys throw rather than returning a partial/undefined row.
assert.throws(() => gateFor('bogus', 'reviewed'), /unknown gate mode/, 'an unknown gate mode throws');
assert.throws(() => gateFor('plan', 'bogus'), /unknown outcome/, 'an unknown outcome throws');
assert.throws(() => gateFor('code', 'PASS'), /unknown outcome/, 'a retired verdict word throws');

// selectDimensions gates unit-of-work through the ONE target-type predicate.
for (const t of ['roadmap', 'task', 'implementation-plan']) {
  assert.ok(
    !selectDimensions('plan', { targetType: t }).map((d) => d.key).includes('unit-of-work'),
    'plan mode on a ' + t + ' target excludes unit-of-work'
  );
}
assert.ok(
  selectDimensions('plan', { targetType: 'phase' }).map((d) => d.key).includes('unit-of-work'),
  'plan mode on a phase target includes unit-of-work'
);
console.log('gate policy: mode-dispatched, code re-expressed not forked, plan preserves the tag-gate outcome');

console.log('all review-refute-fix behavior assertions passed');
NODE_TEST

if run_node "$TMP/test.mjs" "$LIB"; then
    pass "find -> refute -> filter behavior verified (code + plan, deterministic)"
else
    fail "review-refute-fix behavior assertions failed"
fi

# --- 4. PLAN CALIBRATION MUTATION SELF-TEST -----------------------------------
# Prove the AC1 presence check (embedded in section 3's test.mjs) is not
# vacuous: on a hermetic scratch copy of the lib, strip the
# PLAN_SEVERITY_CALIBRATION constant declaration and its single injection line
# inside findPrompt, then re-run the identical presence assertion against the
# mutated copy and require it to THROW. Mirrors 1b's SCRATCH-only isolation —
# never touches $LIB. A failed/partial strip must still leave valid, importable
# JS (whole-statement removals only), so a parse error can't be mistaken for a
# passing self-test.
say "4. Plan calibration mutation self-test (proves the AC1 check would catch a regression)"
mkdir -p "$SCRATCH/.claude/workflows/lib"
cp "$LIB" "$SCRATCH/.claude/workflows/lib/review.mjs"

# Remove the `const PLAN_SEVERITY_CALIBRATION = ... ;` declaration (spans the
# `const NAME =` line through the line ending in `;`) and the one line that
# pushes it into the prompt. Both are whole-statement removals, so the mutated
# file stays syntactically valid — leaves the block-comment prose above it in
# place, which is harmless.
awk '
    /^const PLAN_SEVERITY_CALIBRATION =$/ { skip = 1 }
    skip && /;$/ { skip = 0; next }
    skip { next }
    /lines\.push\(PLAN_SEVERITY_CALIBRATION\);/ { next }
    { print }
' "$SCRATCH/.claude/workflows/lib/review.mjs" >"$SCRATCH/mutated-lib.mjs"
mv "$SCRATCH/mutated-lib.mjs" "$SCRATCH/.claude/workflows/lib/review.mjs"

if grep -q 'PLAN_SEVERITY_CALIBRATION' "$SCRATCH/.claude/workflows/lib/review.mjs"; then
    fail "mutation setup did not fully strip PLAN_SEVERITY_CALIBRATION from the scratch copy"
fi

cat >"$TMP/mutation-test.mjs" <<'NODE_MUTATION_TEST'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const mutatedLibPath = process.argv[2];
const mod = await import(pathToFileURL(mutatedLibPath).href); // must still parse/import cleanly

const CTX = { target: 'phase widget/phase-1-foo' };
const PLAN_CALIBRATION_KEYPHRASES = [
  'blocking` means the goal, approach, or scope is wrong, or the plan violates a stated architectural constraint',
  'concern` that rides along as an implementation note for the implementing agent',
];

function assertPlanCalibrationPresent(m) {
  for (const dim of m.DIMENSIONS.plan) {
    const prompt = m.findPrompt('plan', dim, CTX);
    for (const phrase of PLAN_CALIBRATION_KEYPHRASES) {
      assert.ok(prompt.includes(phrase), 'missing calibration keyphrase in "' + dim.key + '": ' + phrase);
    }
  }
}

assert.throws(
  () => assertPlanCalibrationPresent(mod),
  'the presence check must FAIL against a mutated copy with the calibration text stripped — the check is vacuous otherwise'
);

console.log('mutation self-test passed: presence check correctly fails on stripped calibration text');
NODE_MUTATION_TEST

if run_node "$TMP/mutation-test.mjs" "$SCRATCH/.claude/workflows/lib/review.mjs"; then
    pass "calibration presence check fires on planted removal (self-test proves it is not vacuous)"
else
    fail "mutation self-test did not behave as expected — either the mutated file failed to import, or the presence check did not fail on stripped calibration text"
fi

# --- 5. PLAN-STANDALONE PATH -------------------------------------------------
# The plan-review.js standalone workflow reuses buildReviewPipeline('plan') and
# GATE_POLICY.plan with NO new review logic, and adds three pure consolidation
# helpers to the stamped block: stripNonPhaseUnitOfWork (phase-only unit-of-work
# scoping), filterPlanReviewTag (sibling-preserving tag read-filter-write), and
# classifyPlanOutcome (reviewed|rework|escalated). Drive them in Node, then grep
# plan-review.js for the structural invariants (four target types, parallel()
# fan-out, pipeline/gate reuse, per-unit strip, implementation-plan carve-outs).
say "5. Plan-standalone path: consolidation helpers + plan-review.js structure"

PLAN_REVIEW="$WF_DIR/plan-review.js"
[ -f "$PLAN_REVIEW" ] || fail "plan-review.js not found: $PLAN_REVIEW"

cat >"$TMP/plan-test.mjs" <<'NODE_PLAN_TEST'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const libPath = process.argv[2];
const mod = await import(pathToFileURL(libPath).href);
const { stripNonPhaseUnitOfWork, filterPlanReviewTag, classifyPlanOutcome, gateFor, hasBlocking } = mod;

// Export presence — the harness (and plan-review.js's stamped copy) needs all three.
for (const name of ['stripNonPhaseUnitOfWork', 'filterPlanReviewTag', 'classifyPlanOutcome']) {
  assert.equal(typeof mod[name], 'function', name + ' must be exported from review.mjs');
}

const uow = { id: 'u', concern: 'unit-of-work', severity: 'blocking', confidence: 90, what_fails: 'phase too big' };
const coh = { id: 'c', concern: 'coherence', severity: 'blocking', confidence: 90, what_fails: 'ambiguous step' };
const arch = { id: 'a', concern: 'architectural-fit', severity: 'blocking', confidence: 92, what_fails: 'violates constraint' };
const nit = { id: 'n', concern: 'coherence', severity: 'concern', confidence: 80, what_fails: 'minor' };

// --- stripNonPhaseUnitOfWork: phase keeps unit-of-work; every other unit drops it.
assert.deepEqual(stripNonPhaseUnitOfWork([uow, coh], 'phase').map((f) => f.id), ['u', 'c'], 'phase keeps unit-of-work');
for (const t of ['task', 'roadmap', 'implementation-plan']) {
  assert.deepEqual(
    stripNonPhaseUnitOfWork([uow, coh], t).map((f) => f.id),
    ['c'],
    'a ' + t + ' unit drops unit-of-work survivors'
  );
}
// Order-preserving: a non-uow finding before AND after a uow one keeps its order.
assert.deepEqual(
  stripNonPhaseUnitOfWork([coh, uow, nit], 'task').map((f) => f.id),
  ['c', 'n'],
  'strip is order-preserving'
);
// Idempotent.
const once = stripNonPhaseUnitOfWork([coh, uow], 'roadmap');
assert.deepEqual(stripNonPhaseUnitOfWork(once, 'roadmap'), once, 'strip is idempotent');
assert.deepEqual(stripNonPhaseUnitOfWork([], 'phase'), [], 'strip on empty is empty');

// --- filterPlanReviewTag: sibling preserved, only-tag -> empty, idempotent no-op.
assert.deepEqual(filterPlanReviewTag(['needs-plan-review', 'depends-unlanded']), ['depends-unlanded'], 'sibling preserved');
assert.deepEqual(filterPlanReviewTag(['depends-unlanded', 'needs-plan-review']), ['depends-unlanded'], 'order preserved on filter');
assert.deepEqual(filterPlanReviewTag(['needs-plan-review']), [], 'only tag -> empty list');
assert.deepEqual(filterPlanReviewTag(['a', 'b']), ['a', 'b'], 'idempotent no-op when tag absent');
assert.deepEqual(filterPlanReviewTag(filterPlanReviewTag(['needs-plan-review', 'x'])), ['x'], 'idempotent under re-application');
assert.deepEqual(filterPlanReviewTag([]), [], 'empty tag list stays empty');

// --- classifyPlanOutcome: reviewed | rework | escalated.
assert.equal(classifyPlanOutcome([]), 'reviewed', 'no findings -> reviewed');
assert.equal(classifyPlanOutcome([nit]), 'reviewed', 'concern-only -> reviewed');
assert.equal(classifyPlanOutcome([coh]), 'rework', 'a blocking coherence finding -> rework (fixable rewrite)');
assert.equal(classifyPlanOutcome([arch]), 'escalated', 'a blocking architectural-fit finding -> escalated (human decision)');
// An empty/ambiguous plan surfaces as a blocking coherence survivor -> rework, not escalated.
assert.equal(
  classifyPlanOutcome([{ id: 'empty', concern: 'coherence', severity: 'blocking', confidence: 95, what_fails: 'plan is empty' }]),
  'rework',
  'an empty/ambiguous plan (blocking coherence) is rework'
);

// --- Per-unit INDEPENDENT gate planning (AC-1): a seeded roadmap where one phase
//     reworks and the rest pass. The tag is cleared on every reviewed unit
//     (phase-B, phase-C, and the roadmap body) and LEFT on the reworked phase-A.
const seededUnits = [
  { id: 'roadmap-body', targetType: 'roadmap', tags: ['needs-plan-review'], survivors: [] },
  { id: 'phase-A', targetType: 'phase', tags: ['needs-plan-review', 'depends-unlanded'], survivors: [coh] },
  { id: 'phase-B', targetType: 'phase', tags: ['needs-plan-review'], survivors: [] },
  { id: 'phase-C', targetType: 'phase', tags: ['needs-plan-review'], survivors: [nit] },
];
const gatePlan = seededUnits.map((u) => {
  const stripped = stripNonPhaseUnitOfWork(u.survivors, u.targetType);
  const outcome = classifyPlanOutcome(stripped);
  const gate = gateFor('plan', outcome);
  // The gate NEVER persists an rdm status, whatever the outcome.
  assert.strictEqual(gate.status, null, 'plan gate persists no rdm status for ' + u.id);
  const remaining = gate.clearsPlanReviewTag ? filterPlanReviewTag(u.tags) : u.tags;
  return { id: u.id, outcome, cleared: gate.clearsPlanReviewTag, remaining };
});
const byId = Object.fromEntries(gatePlan.map((g) => [g.id, g]));
assert.equal(byId['phase-A'].outcome, 'rework', 'phase-A reworks');
assert.equal(byId['phase-A'].cleared, false, 'phase-A keeps needs-plan-review');
assert.deepEqual(byId['phase-A'].remaining, ['needs-plan-review', 'depends-unlanded'], 'phase-A tags untouched');
for (const id of ['roadmap-body', 'phase-B', 'phase-C']) {
  assert.equal(byId[id].outcome, 'reviewed', id + ' is reviewed');
  assert.equal(byId[id].cleared, true, id + ' clears needs-plan-review');
  assert.deepEqual(byId[id].remaining, [], id + ' tag list is emptied (needs-plan-review was the only tag)');
}

// --- --implementation-plan is a NO-GATE, no-status path. Even a blocking finding
//     yields an outcome + findings only; the gate row persists no status and the
//     unit drops unit-of-work like every non-phase unit.
{
  const stripped = stripNonPhaseUnitOfWork([uow, coh], 'implementation-plan');
  assert.deepEqual(stripped.map((f) => f.id), ['c'], 'implementation-plan drops unit-of-work');
  const gate = gateFor('plan', classifyPlanOutcome(stripped));
  assert.strictEqual(gate.status, null, 'implementation-plan gate persists no status');
  assert.equal(gate.writesCompletion, false, 'implementation-plan never writes a completion directive');
}

console.log('plan-standalone helper + gate-planning assertions passed');
NODE_PLAN_TEST

if run_node "$TMP/plan-test.mjs" "$LIB"; then
    pass "stripNonPhaseUnitOfWork / filterPlanReviewTag / classifyPlanOutcome + per-unit gate planning verified"
else
    fail "plan-standalone helper assertions failed"
fi

# --- 5b. plan-review.js STRUCTURE (static greps) -----------------------------
say "5b. plan-review.js parses four target types, fans out, and reuses the core"
grep -q "buildReviewPipeline('plan')" "$PLAN_REVIEW" ||
    fail "plan-review.js must call buildReviewPipeline('plan')"
grep -qE "gateFor\('plan'|GATE_POLICY\.plan" "$PLAN_REVIEW" ||
    fail "plan-review.js must gate through gateFor('plan', …) / GATE_POLICY.plan"
grep -q 'stripNonPhaseUnitOfWork' "$PLAN_REVIEW" ||
    fail "plan-review.js must apply stripNonPhaseUnitOfWork per unit"
grep -q 'filterPlanReviewTag' "$PLAN_REVIEW" ||
    fail "plan-review.js must clear the tag via filterPlanReviewTag"
grep -q 'classifyPlanOutcome' "$PLAN_REVIEW" ||
    fail "plan-review.js must classify each outcome via classifyPlanOutcome"
grep -qE '\bparallel\(' "$PLAN_REVIEW" ||
    fail "plan-review.js must fan out per-phase via parallel()"
# The three flag target forms are all parsed...
for form in '--task' '--roadmap' '--implementation-plan'; do
    grep -q -- "$form" "$PLAN_REVIEW" ||
        fail "plan-review.js does not parse the '$form' target form"
done
# ...and the fourth (positional `<slug> [phase]`) resolves to the phase/roadmap kinds.
grep -q "kind = 'phase'" "$PLAN_REVIEW" ||
    fail "plan-review.js must resolve a positional <slug> phase target"
grep -q "kind = 'roadmap'" "$PLAN_REVIEW" ||
    fail "plan-review.js must resolve the roadmap target"

# The act half AND the gate are carved out for --implementation-plan behind an
# explicit `if (kind !== 'implementation-plan')` guard, so a static reader (and
# this grep) can confirm no rdm update/create/commit is reachable in that branch.
IMPL_GUARDS=$(grep -c "kind !== 'implementation-plan'" "$PLAN_REVIEW")
[ "$IMPL_GUARDS" -ge 2 ] ||
    fail "plan-review.js must guard BOTH act and gate with 'if (kind !== \"implementation-plan\")' (found $IMPL_GUARDS)"

# The driver must not RE-DECLARE the pipeline internals (it consumes the stamped
# block), and must not thread a signals object into the pipeline (the deferral).
DRIVER=$(awk '/>>> review-refute-fix:end/{p=1;next} p' "$PLAN_REVIEW")
if printf '%s\n' "$DRIVER" | grep -nE 'function findPrompt|function refutePrompt|const DIMENSIONS ='; then
    fail "plan-review.js driver re-declares pipeline internals — it must consume the stamped block"
fi
if printf '%s\n' "$DRIVER" | grep -nE 'signals:'; then
    fail "plan-review.js must NOT thread a signals object into the pipeline (unit-of-work scoping is consumer-side)"
fi
# The hygiene grep (section 2) already covers plan-review.js via workflows/*.js;
# re-assert here that it carries no forbidden nondeterministic global.
if grep -nE 'Date\.now\(|Math\.random\(' "$PLAN_REVIEW" >&2; then
    fail "plan-review.js contains a forbidden nondeterministic global"
fi
pass "plan-review.js parses four targets, fans out, reuses the core, and carves out implementation-plan"

# --- 5c. SKILL SHIM (AC-5) ---------------------------------------------------
# The local dogfood SKILL.md is a thin shim over plan-review.js. Its hand-authored
# prose (above the generated review-spec marker) must reference the workflow, keep
# the canonical pipeline phrase, and speak only the new outcome vocabulary — the
# retired PASS WITH CONCERNS / REWORK words survive ONLY inside the generated
# region (as the collapse-mapping note), never in the hand-authored prose.
say "5c. rdm-plan-review SKILL.md is a thin shim over plan-review.js"
SKILL_MD="$REPO_ROOT/.claude/skills/rdm-plan-review/SKILL.md"
[ -f "$SKILL_MD" ] || fail "SKILL.md not found: $SKILL_MD"
grep -q 'plan-review.js' "$SKILL_MD" || fail "SKILL.md must reference the plan-review.js Workflow"
grep -q '<!-- rdm:review-spec:begin' "$SKILL_MD" || fail "SKILL.md must keep the generated review-spec begin marker"
grep -q '<!-- rdm:review-spec:end' "$SKILL_MD" || fail "SKILL.md must keep the generated review-spec end marker"
# Hand-authored prose = everything BEFORE the generated region begins.
awk 'index($0, "<!-- rdm:review-spec:begin") { exit } { print }' "$SKILL_MD" >"$TMP/skill-hand"
grep -q 'find → refute → filter → verdict → act → gate' "$TMP/skill-hand" ||
    fail "SKILL.md hand-authored prose must keep the canonical pipeline phrase"
for retired in 'PASS WITH CONCERNS' 'REWORK'; do
    if grep -n "$retired" "$TMP/skill-hand" >&2; then
        fail "SKILL.md hand-authored prose still uses the retired '$retired' vocabulary"
    fi
done
pass "SKILL.md is a thin shim: references the workflow, keeps the pipeline phrase and markers, drops retired vocab"

# --- 6. PLAN HELPER MUTATION SELF-TESTS (non-vacuity) ------------------------
# Prove the AC-1 phase-scoping and AC-2 tag-filter checks are not vacuous: on a
# scratch copy of the lib, break each helper to a pass-through and require the
# corresponding assertion to THROW; then heal by restoring. Mirrors section 4's
# SCRATCH-only isolation — never touches $LIB.
say "6. Plan helper mutation self-tests (prove the AC-1/AC-2 checks would catch a regression)"
PLANMUT="$TMP/plan-mut"
mkdir -p "$PLANMUT/.claude/workflows/lib"

# (a) stripNonPhaseUnitOfWork -> pass-through: the phase-scoping assertion must fail.
sed 's/^function stripNonPhaseUnitOfWork(survivors, targetType) {/function stripNonPhaseUnitOfWork(survivors, targetType) { return Array.isArray(survivors) ? survivors.slice() : []; \/\/ MUTANT/' \
    "$LIB" >"$PLANMUT/.claude/workflows/lib/review.mjs"
grep -q 'MUTANT' "$PLANMUT/.claude/workflows/lib/review.mjs" ||
    fail "strip mutation setup did not inject the pass-through"

cat >"$TMP/plan-strip-mut-test.mjs" <<'NODE_STRIP_MUT'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';
const mod = await import(pathToFileURL(process.argv[2]).href); // must still import
const uow = { id: 'u', concern: 'unit-of-work', severity: 'blocking', confidence: 90 };
const coh = { id: 'c', concern: 'coherence', severity: 'blocking', confidence: 90 };
assert.throws(
  () => assert.deepEqual(mod.stripNonPhaseUnitOfWork([uow, coh], 'task').map((f) => f.id), ['c']),
  'a pass-through stripNonPhaseUnitOfWork must FAIL the phase-scoping check — else it is vacuous'
);
console.log('strip mutation self-test passed');
NODE_STRIP_MUT
run_node "$TMP/plan-strip-mut-test.mjs" "$PLANMUT/.claude/workflows/lib/review.mjs" ||
    fail "strip mutation self-test did not behave as expected"

# (b) filterPlanReviewTag -> pass-through: the tag-filter assertion must fail.
sed 's/^function filterPlanReviewTag(tags) {/function filterPlanReviewTag(tags) { return Array.isArray(tags) ? tags.slice() : []; \/\/ MUTANT/' \
    "$LIB" >"$PLANMUT/.claude/workflows/lib/review.mjs"
grep -q 'MUTANT' "$PLANMUT/.claude/workflows/lib/review.mjs" ||
    fail "filterPlanReviewTag mutation setup did not inject the pass-through"

cat >"$TMP/plan-tag-mut-test.mjs" <<'NODE_TAG_MUT'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';
const mod = await import(pathToFileURL(process.argv[2]).href); // must still import
assert.throws(
  () => assert.deepEqual(mod.filterPlanReviewTag(['needs-plan-review', 'depends-unlanded']), ['depends-unlanded']),
  'a pass-through filterPlanReviewTag must FAIL the sibling-preservation check — else it is vacuous'
);
console.log('tag-filter mutation self-test passed');
NODE_TAG_MUT
run_node "$TMP/plan-tag-mut-test.mjs" "$PLANMUT/.claude/workflows/lib/review.mjs" ||
    fail "tag-filter mutation self-test did not behave as expected"

pass "plan helper checks fire on planted pass-through mutations (proven non-vacuous)"

say "verify-workflow-review.sh: ALL GREEN"
