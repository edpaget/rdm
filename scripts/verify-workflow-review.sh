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
PLAN_LIB="$REPO_ROOT/.claude/workflows/lib/plan-review.mjs"
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
for key in coherence architectural-fit unit-of-work restraint; do
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
for bad in 'needs-plan-review' '\*\*unit-of-work\*\*' '\*\*restraint\*\*'; do
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

# --- 1g. LOCAL DOGFOOD SKILL PROJECTION ---------------------------------------
# The SAME canonical source ALSO projects into the two LOCAL dogfood skill
# copies — `.claude/skills/rdm-review/SKILL.md` and
# `.claude/skills/rdm-plan-review/SKILL.md` — via
# `scripts/gen-skill-review.sh --target local`. Nothing else re-stamps these;
# without this section a hand-patch (like the plan-mode drift this very phase
# discovered) can silently recur. Gated the same way as the shipped templates:
# --check on the real tree, a scratch drift/heal self-test per local target, a
# non-vacuity + target-isolation self-test for the find-refute-verdict
# local-code-override, and a {rdm_bin} substitution hygiene grep with its own
# non-vacuity self-test.
say "1g. Local dogfood skill projection: rdm-review and rdm-plan-review stay in sync"

LOCAL_SKILLS="$REPO_ROOT/.claude/skills"

if sh "$SKILL_GEN" --check --target local --mode code; then
    pass "gen-skill-review.sh --check --target local --mode code clean"
else
    fail "the local rdm-review skill drifted from $LIB — run scripts/gen-skill-review.sh --target local --mode code"
fi
if sh "$SKILL_GEN" --check --target local --mode plan; then
    pass "gen-skill-review.sh --check --target local --mode plan clean"
else
    fail "the local rdm-plan-review skill drifted from $LIB — run scripts/gen-skill-review.sh --target local --mode plan"
fi

# Fresh scratch tree carrying everything both local targets need, plus the
# shipped templates (needed for the target-isolation self-test below).
LOCALSCRATCH="$TMP/local-skill-scratch"
mkdir -p "$LOCALSCRATCH/scripts" "$LOCALSCRATCH/.claude/workflows/lib" \
    "$LOCALSCRATCH/rdm-core/src/templates" "$LOCALSCRATCH/.claude/skills/rdm-review" \
    "$LOCALSCRATCH/.claude/skills/rdm-plan-review"
cp "$SKILL_GEN" "$LOCALSCRATCH/scripts/gen-skill-review.sh"

# An unknown --target must be rejected, mirroring the existing --mode bogus
# negative test. Run the SCRATCH copy, not "$SKILL_GEN": this is the harness's
# only write-capable (no --check) invocation of the generator, so if the guard
# ever regressed, pointing it at the real tree would let it write there. Assert
# on the message too, so the test proves the guard fired rather than merely
# that the exit status was nonzero.
target_bogus_err="$(
    sh "$LOCALSCRATCH/scripts/gen-skill-review.sh" --target bogus 2>&1 >/dev/null
)" && fail "an unknown --target must be rejected"
printf '%s' "$target_bogus_err" | grep -q 'unknown target: bogus' ||
    fail "an unknown --target must fail with an actionable 'unknown target' message, got: $target_bogus_err"
pass "an unknown --target is rejected with an actionable message"

reset_localscratch_source() {
    cp "$LIB" "$LOCALSCRATCH/.claude/workflows/lib/review.mjs"
}
reset_localscratch_consumers() {
    cp "$LOCAL_SKILLS/rdm-review/SKILL.md" "$LOCALSCRATCH/.claude/skills/rdm-review/SKILL.md"
    cp "$LOCAL_SKILLS/rdm-plan-review/SKILL.md" "$LOCALSCRATCH/.claude/skills/rdm-plan-review/SKILL.md"
    for t in skill-review-cli.md skill-review-mcp.md skill-plan-review-cli.md skill-plan-review-mcp.md; do
        cp "$TEMPLATES/$t" "$LOCALSCRATCH/rdm-core/src/templates/$t"
    done
}
reset_localscratch_source
reset_localscratch_consumers

sh "$LOCALSCRATCH/scripts/gen-skill-review.sh" --check --target local --mode code >/dev/null 2>&1 ||
    fail "scratch local/code --check should pass on a clean copy"
sh "$LOCALSCRATCH/scripts/gen-skill-review.sh" --check --target local --mode plan >/dev/null 2>&1 ||
    fail "scratch local/plan --check should pass on a clean copy"

# Drift+heal self-test, target=local mode=code: corrupt the CONSUMER's
# generated region directly (this is the file a stray hand-patch would touch —
# the discovery that motivated this phase — so the detector must catch drift
# on the consumer side, not just the source side already covered by 1c/1d).
sed 's/\*\*Drop\*\* any finding a refuter refuted/**DROP** ANY FINDING A REFUTER REFUTED (mutated)/' \
    "$LOCALSCRATCH/.claude/skills/rdm-review/SKILL.md" >"$LOCALSCRATCH/mut-review" &&
    mv "$LOCALSCRATCH/mut-review" "$LOCALSCRATCH/.claude/skills/rdm-review/SKILL.md"
if sh "$LOCALSCRATCH/scripts/gen-skill-review.sh" --check --target local --mode code >/dev/null 2>&1; then
    fail "local rdm-review drift gate did NOT detect a planted consumer-side edit"
fi
sh "$LOCALSCRATCH/scripts/gen-skill-review.sh" --target local --mode code >/dev/null 2>&1
sh "$LOCALSCRATCH/scripts/gen-skill-review.sh" --check --target local --mode code >/dev/null 2>&1 ||
    fail "regeneration did not restore sync in the local rdm-review scratch copy"
pass "local rdm-review (target=local mode=code) drift detector fires on a consumer-side edit and heals"

# Drift+heal self-test, target=local mode=plan.
reset_localscratch_consumers
sed 's/Internal consistency and completeness/INTERNAL CONSISTENCY AND COMPLETENESS (mutated)/' \
    "$LOCALSCRATCH/.claude/skills/rdm-plan-review/SKILL.md" >"$LOCALSCRATCH/mut-plan" &&
    mv "$LOCALSCRATCH/mut-plan" "$LOCALSCRATCH/.claude/skills/rdm-plan-review/SKILL.md"
if sh "$LOCALSCRATCH/scripts/gen-skill-review.sh" --check --target local --mode plan >/dev/null 2>&1; then
    fail "local rdm-plan-review drift gate did NOT detect a planted consumer-side edit"
fi
sh "$LOCALSCRATCH/scripts/gen-skill-review.sh" --target local --mode plan >/dev/null 2>&1
sh "$LOCALSCRATCH/scripts/gen-skill-review.sh" --check --target local --mode plan >/dev/null 2>&1 ||
    fail "regeneration did not restore sync in the local rdm-plan-review scratch copy"
pass "local rdm-plan-review (target=local mode=plan) drift detector fires on a consumer-side edit and heals"

reset_localscratch_consumers

# Non-vacuity + target-isolation: mutate a distinguishing sentence INSIDE the
# find-refute-verdict:local-code-override block in a scratch SOURCE copy. The
# target=local mode=code render must change; the target=shipped mode=code
# render from the SAME mutated source must NOT (the override must never leak
# into the shipped template).
cp "$LOCAL_SKILLS/rdm-review/SKILL.md" "$LOCALSCRATCH/baseline-local-review.md"
sh "$LOCALSCRATCH/scripts/gen-skill-review.sh" --target shipped --mode code >/dev/null 2>&1
cp "$LOCALSCRATCH/rdm-core/src/templates/skill-review-cli.md" "$LOCALSCRATCH/baseline-shipped-review.md"

sed 's/are now performed deterministically/ARE NOW PERFORMED DETERMINISTICALLY (mutated)/' \
    "$LOCALSCRATCH/.claude/workflows/lib/review.mjs" >"$LOCALSCRATCH/mut-src" &&
    mv "$LOCALSCRATCH/mut-src" "$LOCALSCRATCH/.claude/workflows/lib/review.mjs"
grep -q 'ARE NOW PERFORMED DETERMINISTICALLY (mutated)' "$LOCALSCRATCH/.claude/workflows/lib/review.mjs" ||
    fail "override non-vacuity mutation setup did not actually mutate the local-code-override block"

sh "$LOCALSCRATCH/scripts/gen-skill-review.sh" --target local --mode code >/dev/null 2>&1
if diff -q "$LOCALSCRATCH/.claude/skills/rdm-review/SKILL.md" "$LOCALSCRATCH/baseline-local-review.md" >/dev/null 2>&1; then
    fail "mutating the local-code-override block did not change the local/code render — the override is not actually consumed"
fi
grep -q 'ARE NOW PERFORMED DETERMINISTICALLY (mutated)' "$LOCALSCRATCH/.claude/skills/rdm-review/SKILL.md" ||
    fail "the local/code render did not pick up the mutated override text"

sh "$LOCALSCRATCH/scripts/gen-skill-review.sh" --target shipped --mode code >/dev/null 2>&1
diff -u "$LOCALSCRATCH/rdm-core/src/templates/skill-review-cli.md" "$LOCALSCRATCH/baseline-shipped-review.md" >/dev/null 2>&1 ||
    fail "the local-code-override mutation LEAKED into the shipped/code render — target isolation is broken"
pass "the find-refute-verdict local-code-override is consumed by target=local mode=code and isolated from target=shipped"

# Restore the scratch source and consumers before the {rdm_bin} hygiene pass.
reset_localscratch_source
reset_localscratch_consumers

# {rdm_bin} hygiene: every freshly generated output, across both targets and
# both modes, must never carry an unsubstituted {rdm_bin} literal, and must
# resolve to the RIGHT binary per target.
sh "$LOCALSCRATCH/scripts/gen-skill-review.sh" --target local --mode code >/dev/null 2>&1
sh "$LOCALSCRATCH/scripts/gen-skill-review.sh" --target local --mode plan >/dev/null 2>&1
sh "$LOCALSCRATCH/scripts/gen-skill-review.sh" --target shipped --mode code >/dev/null 2>&1
sh "$LOCALSCRATCH/scripts/gen-skill-review.sh" --target shipped --mode plan >/dev/null 2>&1
if grep -rn '{rdm_bin}' "$LOCALSCRATCH/.claude/skills" "$LOCALSCRATCH/rdm-core/src/templates" >&2; then
    fail "an unsubstituted {rdm_bin} literal survived generation"
fi
grep -q './target/debug/rdm hook done-line' "$LOCALSCRATCH/.claude/skills/rdm-review/SKILL.md" ||
    fail "the local/code render must contain './target/debug/rdm hook done-line'"
if grep -n '\./target/debug/rdm hook done-line' "$LOCALSCRATCH/rdm-core/src/templates/skill-review-cli.md" >&2; then
    fail "the shipped/code render must use a bare 'rdm hook done-line', never './target/debug/rdm'"
fi
grep -q 'rdm hook done-line' "$LOCALSCRATCH/rdm-core/src/templates/skill-review-cli.md" ||
    fail "the shipped/code render must contain the bare 'rdm hook done-line' example"
pass "{rdm_bin} resolves per target (rdm vs ./target/debug/rdm) with no leftover placeholder"

# Self-test: the {rdm_bin} leftover-placeholder check must not be vacuous —
# disable the substitution step in a scratch copy of the generator and confirm
# it now fires.
grep -v 'sed -i.bak "s/{rdm_bin}' "$SKILL_GEN" >"$LOCALSCRATCH/scripts/gen-skill-review-nosub.sh"
chmod +x "$LOCALSCRATCH/scripts/gen-skill-review-nosub.sh"
if sh "$LOCALSCRATCH/scripts/gen-skill-review-nosub.sh" --target local --mode code 2>"$LOCALSCRATCH/nosub-err"; then
    fail "the {rdm_bin} leftover-placeholder check did not fire when substitution was disabled — it is vacuous"
fi
grep -q 'unsubstituted {rdm_bin}' "$LOCALSCRATCH/nosub-err" ||
    fail "disabling {rdm_bin} substitution did not produce the expected leftover-placeholder error"
pass "the {rdm_bin} hygiene check is not vacuous — it fires when substitution is disabled"

# Direct regression assertion (AC4): the phase's reported gap — a missing
# `restraint` dimension and missing severity-calibration paragraph in the
# LOCAL rdm-plan-review skill — must stay closed.
grep -q 'restraint' "$LOCAL_SKILLS/rdm-plan-review/SKILL.md" ||
    fail "the local rdm-plan-review skill is missing the 'restraint' dimension"
grep -q 'Plan-stage severity calibration' "$LOCAL_SKILLS/rdm-plan-review/SKILL.md" ||
    fail "the local rdm-plan-review skill is missing the 'Plan-stage severity calibration' paragraph"
pass "the local rdm-plan-review skill carries the restraint dimension and severity-calibration paragraph"

# 1e (NO SECOND MECHANISM) already covers the invariant this section depends
# on — one generator, one dimension table — and needed no change for --target
# to be added, so it is not re-asserted here.

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

# --- 2b. AGENT-CONTEXT-TRIM GUARDS -------------------------------------------
# Two guards recording decisions from the agentType/effort options spike
# (docs/workflow-schemas.md § "agentType / effort options spike"). The spike has
# now been RUN via the Workflow tool (wf_2bea58b9-38f), and it moved both guards'
# rationales without moving either verdict:
#   (i)  effort IS honored at the call site (reversing the earlier
#        definition-side negative), so this guard is now a SCOPE boundary, not a
#        statement that the option is inert.
#   (ii) agentType did not resolve from inside a Workflow run AT ALL, and an
#        unresolvable one RAISES (now observed, not inferred).
# No call site was edited, so both guards stay live.
say "2b. Agent-context-trim guards (agentType / effort options spike)"

# (i) No call site may pass `effort:`. READ THIS BEFORE "FIXING" IT: the option
#     is NOT inert. The verification channel is the top-level `effort` field on
#     each `assistant` transcript record, and the two routes disagree:
#       - DECLARED in an agent definition -> ran at "high" (not honored)
#       - agent(prompt, {effort:'low'}) from a Workflow run -> recorded "low"
#         (spike case E: the first "low" record in a 156384-record corpus)
#     So this guard encodes the phase body's scope rule ("do not thread effort:
#     anywhere"), written before that reversal was known — not a claim that the
#     key does nothing. Lifting it is owned by
#     `finish-agent-type-effort-spike-and-thread-mechanical-sites` scope item 5,
#     which must also establish that low effort does not degrade mechanical
#     transcription fidelity. `spike-agent-type.js` is the one file allowed to
#     contain it — probing the option is its entire purpose.
if grep -nE '(^|[^A-Za-z-])effort:' "$WF_DIR"/*.js "$WF_DIR"/lib/*.mjs 2>/dev/null |
    grep -v '/spike-agent-type\.js:'; then
    fail "a workflow script passes effort: — threading it is out of scope until finish-agent-type-effort-spike-and-thread-mechanical-sites lands (NB: effort:'low' IS honored at the call site; this guard is a scope boundary, not an inertness claim — see docs/workflow-schemas.md § agentType / effort options spike)"
fi
printf 'await agent(P, { label: "x", effort: %s })\n' "'low'" >"$SCRATCH/planted-effort.js"
if ! grep -nE '(^|[^A-Za-z-])effort:' "$SCRATCH/planted-effort.js" >/dev/null 2>&1; then
    fail "effort guard did NOT catch a planted effort: key — the detector is broken"
fi
pass "no workflow call site passes effort:; detector catches a planted one"

# (ii) No DISTRIBUTED workflow copy may reference an agentType. `agent_config.rs`
#      emits skills and workflows only — there is no `.claude/agents/` emission
#      surface — and an unresolvable agentType is a RAISED error in the runtime
#      (`agent({agentType}): agent type '...' not found`), not the silent null
#      an unknown `model` id produces. That raise is now OBSERVED, not inferred:
#      spike cases B, C and F plus a retry probe all threw it. Threading one into
#      a shipped template would hard-fail every downstream lane on first
#      dispatch. Lift this guard only together with
#      `ship-mechanical-agent-type-downstream`.
#
#      SCOPE, deliberate: this greps only "$TEMPLATES", not $WF_DIR. The local
#      workflows DO carry agentType — see §2c, which asserts exactly which of
#      their call sites carry it. Their definition lives in this same repo at
#      the path the runtime searches, so a local agentType resolves; a
#      downstream tree receives no .claude/agents/ at all, which is what makes
#      the distributed case different in kind rather than in degree.
if grep -nE 'agentType' "$TEMPLATES"/workflows/*.js 2>/dev/null; then
    fail "a distributed workflow template references agentType, but rdm agent-config emits no .claude/agents/ definitions — an unresolvable agentType RAISES in the runtime and would break every downstream lane"
fi
mkdir -p "$SCRATCH/planted-tpl"
printf 'await agent(P, { label: "x", agentType: %s })\n' "'rdm-mechanical'" >"$SCRATCH/planted-tpl/x.js"
if ! grep -nE 'agentType' "$SCRATCH/planted-tpl"/*.js >/dev/null 2>&1; then
    fail "distributed-agentType guard did NOT catch a planted agentType — the detector is broken"
fi
# Anti-vacuity: the real glob must match at least one file, or the guard above
# passes for the wrong reason.
TPL_WF_COUNT=$(find "$TEMPLATES/workflows" -name '*.js' | wc -l | tr -d ' ')
[ "$TPL_WF_COUNT" -ge 1 ] ||
    fail "no distributed workflow templates matched — the agentType guard would pass vacuously"
pass "no distributed workflow template references agentType ($TPL_WF_COUNT scanned); detector catches a planted one"

# --- 2c. MECHANICAL agentType THREADING (bidirectional) ----------------------
# The local-only workflows thread `agentType: 'rdm-mechanical'` at their
# MECHANICAL call sites — the ones that run one command and transcribe its
# output — and must never thread it at a JUDGMENT site, whose whole task is the
# reasoning a trimmed transcribe-only agent cannot do.
#
# Asserted in BOTH directions, each with a planted-mutation self-test, plus a
# completeness sweep so a future mechanical site cannot ship untrimmed.
#
# LABEL-MATCHING HAZARD: these labels are substrings of one another. `act:`
# also matches the mechanical `act:round-note:`, and `estimate:` matches both
# the judgment `estimate:rate:` and the mechanical `estimate:list`/`write`/
# `tier`. Every pattern below therefore matches the label's OPENING QUOTE plus
# its exact prefix, never a bare substring.
say "2c. Mechanical agentType threading (bidirectional)"

# has_agent_type <file> <label-prefix> — true when the agent() options object
# whose `label:` starts with the given prefix also carries agentType within the
# next 4 lines. The call sites are formatted one option per line, so a fixed
# window is sufficient and keeps this a grep rather than a JS parser.
has_agent_type() {
    grep -A4 -F "label: '$2" "$1" 2>/dev/null | grep -q "agentType: 'rdm-mechanical'"
}

# (i) POSITIVE — every mechanical site carries it, as `<file>|<label-prefix>`
#     records. POSIX sh has no arrays, so the list is a newline-separated here-doc
#     consumed by `while read`, matching this harness's existing style.
MECHANICAL_SITES=$(
    cat <<'SITES'
document.js|model:mechanical'
document.js|fetch:roadmap-meta'
document.js|gather:' +
document.js|write:draft'
backlog.js|model:mechanical'
backlog.js|fetch:report'
estimate.js|model:mechanical'
estimate.js|estimate:list'
estimate.js|estimate:write:' +
estimate.js|estimate:tier:' +
plan-review.js|model:mechanical'
plan-review.js|fetch:roadmap'
plan-review.js|fetch:' + kind
plan-review.js|fetch:wontfix'
plan-review.js|gate:clear-tag:' +
lib/plan-review.mjs|fetch:roadmap'
lib/plan-review.mjs|fetch:' + kind
lib/plan-review.mjs|fetch:wontfix'
lib/plan-review.mjs|gate:clear-tag:' +
SITES
)
MECH_EXPECTED=$(printf '%s\n' "$MECHANICAL_SITES" | grep -c .)
printf '%s\n' "$MECHANICAL_SITES" | while IFS= read -r site; do
    [ -n "$site" ] || continue
    f=${site%%|*}
    lbl=${site#*|}
    [ -f "$WF_DIR/$f" ] || fail "2c: expected workflow file $f is missing — the mechanical-site list is stale"
    has_agent_type "$WF_DIR/$f" "$lbl" ||
        fail "2c: mechanical call site '$lbl' in $f does NOT carry agentType: 'rdm-mechanical' — every mechanical site must be trimmed (see docs/mechanical-agent-inventory.md § 'The threadable surface, enumerated')"
done || exit 1
# Self-test: strip the key from one site in a scratch copy; the check must fail.
mkdir -p "$SCRATCH/2c"
sed "s/ *agentType: 'rdm-mechanical',//" "$WF_DIR/document.js" >"$SCRATCH/2c/stripped.js"
if has_agent_type "$SCRATCH/2c/stripped.js" "write:draft'"; then
    fail "2c: positive detector did NOT notice a stripped agentType — it is vacuous"
fi
pass "all $MECH_EXPECTED mechanical call sites carry agentType; detector catches a stripped one"

# (ii) NEGATIVE — no judgment site carries it. These agents reason; a
#      transcribe-only definition with a two-tool allowlist would break them.
JUDGMENT_LABELS="find: refute: plan:author plan:revise implement: synthesize:draft analyze: estimate:rate: act:' act:code"
JUDGMENT_COUNT=0
for lbl in $JUDGMENT_LABELS; do
    JUDGMENT_COUNT=$((JUDGMENT_COUNT + 1))
    for f in "$WF_DIR"/*.js "$WF_DIR"/lib/*.mjs; do
        case "$f" in */spike-agent-type.js) continue ;; esac
        if has_agent_type "$f" "$lbl"; then
            fail "2c: JUDGMENT call site '$lbl' in $(basename "$f") carries agentType — rdm-mechanical is a transcribe-only agent and would break it"
        fi
    done
done
# Self-test: plant agentType on a judgment-shaped site; the check must catch it.
mkdir -p "$SCRATCH/2c"
printf "await _agent(P, {\n  label: 'find:' + mode,\n  phase: 'Find',\n  agentType: 'rdm-mechanical',\n})\n" >"$SCRATCH/2c/planted-judgment.js"
if ! has_agent_type "$SCRATCH/2c/planted-judgment.js" "find:"; then
    fail "2c: negative detector did NOT catch a planted judgment-site agentType — it is vacuous"
fi
pass "no judgment call site carries agentType ($JUDGMENT_COUNT labels swept); detector catches a planted one"

# (iii) COMPLETENESS. Two separate checks, because they catch different things
#       and an earlier version of this section conflated them:
#         (a) every agentType VALUE is our definition — a live grep;
#         (b) the threaded COUNT matches BOTH the asserted list AND a
#             tree-derived expectation.
#       (b)'s second half is the one that matters. Comparing THREADED against
#       the static MECHANICAL_SITES list alone cannot detect a NEW mechanical
#       site shipped without the key: both sides stay at their old value and the
#       equality holds. So derive the expectation from the tree instead —
#       every mechanical site except the four `model:mechanical` bootstraps pins
#       `model: mechanicalModel` / `model: _mechanicalModel` (verified: no
#       judgment site pins either), and the bootstraps carry no pin because they
#       are what resolves that model. Hence THREADED must equal PINNED + 4.
#       Add an unthreaded mechanical site and PINNED rises while THREADED does
#       not, so the check fires.
# spike-agent-type.js is excluded throughout: probing an unknown id is its
# entire purpose, so it deliberately carries agentType values that resolve to
# nothing.
STRAY=$(grep -rnoE "agentType: *'[^']*'" "$WF_DIR"/*.js "$WF_DIR"/lib/*.mjs 2>/dev/null |
    grep -v '/spike-agent-type\.js:' |
    grep -v "agentType: 'rdm-mechanical'" | sort -u || true)
if [ -n "$STRAY" ]; then
    printf '%s\n' "$STRAY"
    fail "2c: a workflow references an agentType other than 'rdm-mechanical' — only that definition exists in .claude/agents/"
fi

# count_threaded <dir> / count_pinned <dir> — same sweeps, parameterized by tree
# so the self-test below can run them against a mutated scratch copy.
count_threaded() {
    grep -rc "agentType: 'rdm-mechanical'" "$1"/*.js "$1"/lib/*.mjs 2>/dev/null |
        grep -v '/spike-agent-type\.js:' | awk -F: '{s+=$2} END {print s+0}'
}
count_pinned() {
    grep -rcE "model: _?mechanicalModel,?$" "$1"/*.js "$1"/lib/*.mjs 2>/dev/null |
        grep -v '/spike-agent-type\.js:' | awk -F: '{s+=$2} END {print s+0}'
}
MECH_BOOTSTRAPS=4 # document/backlog/estimate/plan-review, one `model:mechanical` each
THREADED=$(count_threaded "$WF_DIR")
PINNED=$(count_pinned "$WF_DIR")
[ "$THREADED" -eq "$MECH_EXPECTED" ] ||
    fail "2c: found $THREADED threaded agentType sites but $MECH_EXPECTED are asserted — a call site was added or removed without updating MECHANICAL_SITES"
[ "$THREADED" -eq "$((PINNED + MECH_BOOTSTRAPS))" ] ||
    fail "2c: $THREADED threaded sites but $PINNED mechanical-model pins + $MECH_BOOTSTRAPS bootstraps = $((PINNED + MECH_BOOTSTRAPS)) — a mechanical call site is missing agentType, or a threaded site lost its model pin"
# Self-test: plant a NEW mechanical site (model pin, no agentType) in a scratch
# copy of the tree — the derived check must fire where the static one cannot.
rm -rf "$SCRATCH/2c-tree"
mkdir -p "$SCRATCH/2c-tree/lib"
cp "$WF_DIR"/*.js "$SCRATCH/2c-tree/" 2>/dev/null || true
cp "$WF_DIR"/lib/*.mjs "$SCRATCH/2c-tree/lib/" 2>/dev/null || true
cat >>"$SCRATCH/2c-tree/document.js" <<'PLANTED'
await agent(P, {
  label: 'fetch:newly-added-mechanical-site',
  phase: 'Fetch',
  schema: SOME_SCHEMA,
  model: mechanicalModel,
})
PLANTED
PLANTED_THREADED=$(count_threaded "$SCRATCH/2c-tree")
PLANTED_PINNED=$(count_pinned "$SCRATCH/2c-tree")
if [ "$PLANTED_THREADED" -eq "$MECH_EXPECTED" ] &&
    [ "$PLANTED_THREADED" -ne "$((PLANTED_PINNED + MECH_BOOTSTRAPS))" ]; then
    : # correct: static check still passes, derived check catches it
else
    fail "2c: derived completeness check did NOT catch a planted untrimmed mechanical site (threaded=$PLANTED_THREADED pinned=$PLANTED_PINNED) — it is vacuous"
fi
pass "agentType completeness: $THREADED threaded = $PINNED pinned + $MECH_BOOTSTRAPS bootstraps; derived check catches a planted untrimmed site"

# (iv) REFERENT RESOLUTION — the other half of the reference. (i)-(iii) assert
#      the call sites SPELL the name; none of them assert the name RESOLVES.
#      Delete .claude/agents/rdm-mechanical.md, rename it, or edit one word of
#      its `name:` frontmatter and every threaded workflow raises
#      `agent type '...' not found` on first dispatch — with the whole suite
#      green. This mirrors verify-agent-config-distribution.sh, which resolves
#      every literal `.claude/workflows/<name>.js` mention to a real file.
#      Derived from the sweep rather than hardcoded, so the two sides of the
#      reference cannot drift apart.
AGENTS_DIR="$REPO_ROOT/.claude/agents"
REFERENCED=$(grep -rhoE "agentType: *'[^']*'" "$WF_DIR"/*.js "$WF_DIR"/lib/*.mjs 2>/dev/null |
    grep -v "no-such-agent" | sed "s/.*'\(.*\)'/\1/" | sort -u)
[ -n "$REFERENCED" ] ||
    fail "2c(iv): no agentType literal found to resolve — the referent check would pass vacuously"
REF_COUNT=0
for name in $REFERENCED; do
    REF_COUNT=$((REF_COUNT + 1))
    found=""
    for def in "$AGENTS_DIR"/*.md; do
        [ -f "$def" ] || continue
        # frontmatter `name:` must equal the referenced agent type exactly
        defname=$(sed -n '/^name:[[:space:]]*/{s/^name:[[:space:]]*//p;q;}' "$def")
        [ "$defname" = "$name" ] && found="$def" && break
    done
    [ -n "$found" ] ||
        fail "2c(iv): workflows reference agentType '$name' but no file in .claude/agents/ declares 'name: $name' — an unresolvable agentType RAISES on first dispatch"
done
# Self-test: corrupt the frontmatter name in a scratch copy; resolution must fail.
rm -rf "$SCRATCH/2c-agents"
mkdir -p "$SCRATCH/2c-agents"
cp "$AGENTS_DIR"/*.md "$SCRATCH/2c-agents/" 2>/dev/null || true
sed -i.bak 's/^name: rdm-mechanical$/name: rdm-mechanical-TYPO/' "$SCRATCH/2c-agents/rdm-mechanical.md"
corrupted_hit=""
for def in "$SCRATCH/2c-agents"/*.md; do
    case "$def" in *.bak) continue ;; esac
    defname=$(sed -n '/^name:[[:space:]]*/{s/^name:[[:space:]]*//p;q;}' "$def")
    [ "$defname" = "rdm-mechanical" ] && corrupted_hit="$def"
done
[ -z "$corrupted_hit" ] ||
    fail "2c(iv): referent detector still resolved 'rdm-mechanical' after the frontmatter name was corrupted — it is vacuous"
pass "all $REF_COUNT referenced agentType name(s) resolve to a .claude/agents/ definition; detector catches a corrupted name"

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
  acTableHasGap,
  AC_ENTRY_SCHEMA,
  AC_REVIEW_SCHEMA,
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
const { survivors: out, acTable: outAcTable } = await buildReviewPipeline('code', deps(spy))(CTX);

assert.deepEqual(out.map((f) => f.id), ['real-bug'], 'code: only the real, un-refuted, high-confidence finding survives');
// The `ac` dimension in this fixture returns the bare FINDINGS shape (no `ac`
// array), matching the pre-AC-table-channel fixtures elsewhere in this file —
// so acTable stays null (no structured table was resolved).
assert.equal(outAcTable, null, 'no ac table resolved when the ac finder returns no `ac` array');

const findCalls = spy.calls.filter((c) => c.label.startsWith('find:'));
const refuteCalls = spy.calls.filter((c) => c.label.startsWith('refute:'));
assert.equal(findCalls.length, CODE_DIM_KEYS.length, 'one finder per code dimension');
assert.equal(
  refuteCalls.length,
  2,
  'a fresh refuter per GATING finding — the planted `suggestion` is passed through, not refuted'
);
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
  const { survivors: rOut } = await buildReviewPipeline('code', deps(spyF))(CTX);
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
  const { survivors: rOut } = await buildReviewPipeline('code', deps(spyR))(CTX);
  assert.deepEqual(rOut.map((f) => f.id), ['infra'], 'a refuter crash keeps the finding un-refuted, not silently dropped');
}

// ============================================================================
// PLAN mode — same battery on the plan dimension set (thin variation).
// ============================================================================
assert.deepEqual(
  DIMENSIONS.plan.map((d) => d.key),
  ['coherence', 'architectural-fit', 'unit-of-work', 'restraint'],
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
  restraint: [],
};
const planVerdicts = {
  'vague-step': { refuted: false, confidence: 90 },
  nonissue: { refuted: true, confidence: 85 }, // refuter kills it
  'weak-plan': { refuted: false, confidence: 40 }, // NOT refuted, but below floor
};

const pspy = makeSpyAgent(planFindings, planVerdicts);
const { survivors: pout, acTable: poutAcTable } = await buildReviewPipeline('plan', deps(pspy))(CTX);

// refutable dropped, below-floor dropped, real survives — full parity with code mode.
assert.deepEqual(pout.map((f) => f.id), ['vague-step'], 'plan: refutable dropped, below-floor dropped, real survives');
// Plan mode never sets an AC table — the `ac` dimension does not exist there.
assert.equal(poutAcTable, null, 'plan mode always resolves acTable to null');
const pFind = pspy.calls.filter((c) => c.label.startsWith('find:'));
const pRefute = pspy.calls.filter((c) => c.label.startsWith('refute:'));
assert.equal(pFind.length, 4, 'one finder per plan dimension');
assert.equal(
  pRefute.length,
  2,
  'a fresh refuter per GATING plan finding — the planted `suggestion` is passed through, not refuted'
);
assert.ok(pFind.every((c) => c.label.startsWith('find:plan:')), 'plan finders labelled by dimension');
assert.ok(pFind.every((c) => c.prompt.includes(CTX.target)), 'context.target threaded into plan finder prompts');

// Plan-mode determinism parity with code mode.
const poutA = await buildReviewPipeline('plan', deps(makeSpyAgent(planFindings, planVerdicts)))(CTX);
const poutB = await buildReviewPipeline('plan', deps(makeSpyAgent(planFindings, planVerdicts)))(CTX);
assert.equal(JSON.stringify(poutA), JSON.stringify(poutB), 'plan review output is deterministic across runs');

// Unknown mode is rejected, not silently empty.
assert.throws(() => buildReviewPipeline('bogus', deps(spy)), /unknown review mode/, 'unknown mode throws');

// ============================================================================
// AC2 — code-mode findPrompt output is byte-exact against a pinned baseline.
// The pin serves two purposes at once:
//   1. Leak detection. The baseline was first captured BEFORE the
//      plan-severity-calibration change; any difference (including a single
//      stray byte) means plan-mode work leaked into code-mode prompts.
//   2. Project-agnostic prose. The baseline was re-fixtured when the code
//      dimensions stopped hardcoding this project's own language and crate
//      conventions and started directing the finder agent at the consuming
//      project's principles document. Re-pinning at the new neutral strings
//      keeps a project-specific convention from creeping back in.
// Both purposes depend on the comparison staying BYTE-EXACT — never relax it
// to a substring, regex, or normalized match.
// ============================================================================
const CODE_PROMPT_BASELINE = {
  // `ac` intentionally diverges from the FINDINGS-schema baseline shape below —
  // it is the ONE dimension that returns the structured AC_REVIEW_SCHEMA (see
  // the AC-table-channel change) — so its baseline is the AC_REVIEW prompt, not
  // the shared FINDINGS-schema wording every other dimension shares.
  ac: 'You are a READ-ONLY reviewer. Do not edit any files.\nReview target: phase widget/phase-1-foo.\nInspect the implementation diff (use git log / git diff in the worktree).\nYour single dimension is AC compliance (ac). For each acceptance criterion in the target, rate PASS / FAIL / PARTIAL with evidence (file:line, test name). Flag any criterion that is unmet, ambiguous, or untestable.\nReport only findings you can back with concrete evidence. One strong finding beats five weak ones.\nReturn JSON matching the AC_REVIEW schema: an `ac` array with ONE entry per acceptance criterion — criterion, status (PASS|FAIL|PARTIAL), and evidence (file:line, test name) — plus an OPTIONAL `findings` array (same shape as the FINDINGS schema) for narrative notes that do not reduce to a single criterion\'s status.\nOnly leave `ac` empty if the target states no acceptance criteria at all — report that itself as a `findings` entry.',
  correctness: 'You are a READ-ONLY reviewer. Do not edit any files.\nReview target: phase widget/phase-1-foo.\nInspect the implementation diff (use git log / git diff in the worktree).\nYour single dimension is Correctness & error handling (correctness). Logic bugs, edge cases, race conditions, and error paths. Judge error handling against the conventions the project states in its principles document (docs/principles.md if present, otherwise CLAUDE.md / AGENTS.md in the project root) — which error type each layer must use, and where context may be added. User-facing errors must be actionable: what went wrong and what the reader can do about it.\nReport only findings you can back with concrete evidence. One strong finding beats five weak ones.\nReturn JSON matching the FINDINGS schema: a `findings` array, each with id, concern, location, severity (blocking|concern|suggestion), confidence (0-100), what_fails, why, recommendation.\nReturn an empty `findings` array if the dimension is clean.',
  tests: 'You are a READ-ONLY reviewer. Do not edit any files.\nReview target: phase widget/phase-1-foo.\nInspect the implementation diff (use git log / git diff in the worktree).\nYour single dimension is Tests (tests). Do tests exist and cover the key behaviors and edge cases? Was TDD followed? Are there untested branches or newly added logic with no test?\nReport only findings you can back with concrete evidence. One strong finding beats five weak ones.\nReturn JSON matching the FINDINGS schema: a `findings` array, each with id, concern, location, severity (blocking|concern|suggestion), confidence (0-100), what_fails, why, recommendation.\nReturn an empty `findings` array if the dimension is clean.',
  architecture: 'You are a READ-ONLY reviewer. Do not edit any files.\nReview target: phase widget/phase-1-foo.\nInspect the implementation diff (use git log / git diff in the worktree).\nYour single dimension is Architecture (architecture). Does logic live where the project\'s stated layering contract puts it, with the interaction layers on top staying thin? No duplicated logic across interfaces? Read the project\'s principles document (docs/principles.md if present, otherwise CLAUDE.md / AGENTS.md) for the layering contract and the commit-scope convention, and flag any change that violates one.\nReport only findings you can back with concrete evidence. One strong finding beats five weak ones.\nReturn JSON matching the FINDINGS schema: a `findings` array, each with id, concern, location, severity (blocking|concern|suggestion), confidence (0-100), what_fails, why, recommendation.\nReturn an empty `findings` array if the dimension is clean.',
};
// Scoped to the dimensions that existed when the baseline was captured. The
// dimensions added later (api-docs, changelog, security) have NO byte-exact
// entry here on purpose — widening the fixture would pin prose the baseline was
// never meant to guard. Do NOT read that as "covered elsewhere": the
// coverage-parity assertion further down compares only DIMENSIONS.code's KEY
// SET and `when`-trigger wiring, never prompt text. The project-agnostic
// property of api-docs' and changelog's prose is asserted by AC2b below.
for (const dim of DIMENSIONS.code.filter((d) => CODE_PROMPT_BASELINE[d.key])) {
  assert.equal(
    findPrompt('code', dim, CTX),
    CODE_PROMPT_BASELINE[dim.key],
    'code-mode findPrompt("' + dim.key + '") must stay byte-identical to the pinned baseline'
  );
}
console.log('AC2: code-mode findPrompt output is byte-exact against the pinned baseline');

// ============================================================================
// AC2b — the five code dimensions rewritten for project-agnosticism
// (correctness, architecture, api-docs, changelog, security) must state generic
// intent and route the concrete conventions through the consuming project's
// principles document. `correctness` and `architecture` are additionally
// byte-pinned above; `api-docs`, `changelog` and `security` are NOT (see the
// note on the baseline's scope), so without this block the dimensions whose
// rdm-specific hardcoding was the whole point of the rewrite would have no
// content-level coverage at all. Asserted in BOTH directions so a regression
// fails either way:
//   - negative: no crate name, language name, language-specific doc-section
//     name, or hardcoded changelog filename may appear in the dimension's
//     title or focus;
//   - positive: the focus must still name the principles document and its
//     CLAUDE.md / AGENTS.md fallback, so genericity cannot be "achieved" by
//     deleting the convention pointer outright.
// The scope is deliberately these five keys; `ac` / `tests` were already
// neutral. `security` keeps its threat taxonomy (injection, path traversal,
// secret leakage, authorization) — rebuilding THAT on a language-neutral
// threat-model vocabulary is the sibling phase's unit — but its
// safety-escape-hatch clause no longer names a specific language's construct
// or comment convention, so it belongs here.
// ============================================================================
{
  const REWRITTEN_CODE_DIMS = ['correctness', 'architecture', 'api-docs', 'changelog', 'security'];
  const forbiddenCodeTokens = [
    'rdm-core',
    'rdm-cli',
    'rdm-server',
    'rdm-mcp',
    'anyhow',
    'rustdoc',
    'Rust',
    'cargo',
    'Cargo',
    'crate',
    'missing_docs',
    '# Errors',
    '# Panics',
    '# Safety',
    'CHANGELOG.md',
  ];
  for (const key of REWRITTEN_CODE_DIMS) {
    const dim = DIMENSIONS.code.find((d) => d.key === key);
    assert.ok(dim, 'the ' + key + ' dimension must exist in DIMENSIONS.code');
    const scopedText = [dim.title, dim.focus].join('\n');
    for (const tok of forbiddenCodeTokens) {
      assert.ok(
        scopedText.indexOf(tok) === -1,
        'the ' + key + ' dimension prose must be project-agnostic — found forbidden token: ' + tok
      );
    }
    assert.ok(
      dim.focus.includes('principles document'),
      'the ' + key + " dimension focus must direct the finder at the project's principles document"
    );
    assert.ok(
      dim.focus.includes('docs/principles.md') &&
        dim.focus.includes('CLAUDE.md') &&
        dim.focus.includes('AGENTS.md'),
      'the ' + key + ' dimension focus must name docs/principles.md and its CLAUDE.md / AGENTS.md fallback'
    );
  }

  // CARVE-OUT LEDGER — now EMPTY. This used to permit exactly one carve-out
  // (`security`, for its `unsafe` / `// SAFETY:` wording). That wording is gone,
  // so the permitted set is the empty set: NO code dimension's title or focus
  // may name a language-specific construct or comment convention. Keeping the
  // assertion (rather than deleting it) is what stops the carve-out from
  // silently re-opening — a language-specific idiom reintroduced into ANY code
  // dimension fails here.
  const LANGUAGE_SPECIFIC_IDIOMS = ['`unsafe`', '// SAFETY:'];
  const stillLanguageSpecific = DIMENSIONS.code
    .filter((d) => LANGUAGE_SPECIFIC_IDIOMS.some((tok) => [d.title, d.focus].join('\n').includes(tok)))
    .map((d) => d.key);
  assert.deepEqual(
    stillLanguageSpecific,
    [],
    'no code dimension may carry a language-specific idiom — the carve-out is closed ' +
      'and must not re-open; got: ' +
      JSON.stringify(stillLanguageSpecific)
  );
}
console.log('AC2b: the rewritten code dimensions are project-agnostic and point at the principles document');
console.log('AC2b: no code dimension carries a language-specific idiom — the carve-out is closed');

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
// review-gate-intent phase 6, AC2 — coherence's finder prompt must carry both
// the wrong-thing blocking bar and the delegation rule as distinct literal
// substrings, so the test cannot pass on only one of the two being added.
// ============================================================================
const COHERENCE_STOPPING_RULE_KEYPHRASES = [
  'A plan may delegate implementation decisions to whoever carries it out',
  'blocking only when an implementer following the plan as written would build the wrong thing',
];
{
  const coherenceDim = DIMENSIONS.plan.find((d) => d.key === 'coherence');
  const coherencePrompt = findPrompt('plan', coherenceDim, CTX);
  for (const phrase of COHERENCE_STOPPING_RULE_KEYPHRASES) {
    assert.ok(
      coherencePrompt.includes(phrase),
      'coherence findPrompt is missing the stopping-rule keyphrase: ' + phrase
    );
  }
}
console.log('AC2: coherence findPrompt carries both the wrong-thing bar and the delegation rule');

// ============================================================================
// review-gate-intent phase 6, AC3 — the counterweight `restraint` dimension:
// always-on in plan mode (both the explicit-signals and fail-open paths), and
// a seeded over-specification finding survives the pipeline.
// ============================================================================
assert.ok(
  DIMENSIONS.plan.map((d) => d.key).includes('restraint'),
  'restraint must be a plan-mode dimension'
);
assert.ok(
  selectDimensions('plan', {}).map((d) => d.key).includes('restraint'),
  'restraint is always-on: explicit empty signals still include it'
);
assert.ok(
  selectDimensions('plan', null).map((d) => d.key).includes('restraint'),
  'restraint is always-on: fail-open (null signals) still include it'
);
{
  const overSpecFindings = {
    coherence: [],
    'architectural-fit': [],
    'unit-of-work': [],
    restraint: [
      {
        id: 'over-specified',
        concern: 'restraint',
        severity: 'blocking',
        confidence: 90,
        what_fails: 'the plan prescribes an exact match threshold the implementer should be left to choose',
      },
    ],
  };
  const overSpecVerdicts = { 'over-specified': { refuted: false, confidence: 92 } };
  const rspy = makeSpyAgent(overSpecFindings, overSpecVerdicts);
  const { survivors: rOut } = await buildReviewPipeline('plan', deps(rspy))(CTX);
  assert.ok(
    rOut.some((f) => f.id === 'over-specified'),
    'a seeded blocking restraint finding survives buildReviewPipeline(plan) filtering'
  );
}
console.log('AC3: restraint is always-on in plan mode and a seeded over-specification finding survives the pipeline');

// ============================================================================
// review-gate-intent phase 6, AC5 — the coherence dimension's additions and the
// new restraint dimension's title/focus must name no repo path or crate: a
// static grep over exactly the new/changed text, scoped tightly so it can't
// pass by accident on unrelated content elsewhere in the module.
// ============================================================================
{
  const forbiddenTokens = ['rdm-core', 'rdm-cli', 'rdm-server', 'rdm-mcp', '.claude/', 'scripts/'];
  const coherenceDim = DIMENSIONS.plan.find((d) => d.key === 'coherence');
  const restraintDim = DIMENSIONS.plan.find((d) => d.key === 'restraint');
  assert.ok(restraintDim, 'the restraint dimension must exist in DIMENSIONS.plan');
  const scopedText = [coherenceDim.focus, restraintDim.title, restraintDim.focus].join('\n');
  for (const tok of forbiddenTokens) {
    assert.ok(
      scopedText.indexOf(tok) === -1,
      'coherence/restraint dimension prose must be repo-agnostic — found forbidden token: ' + tok
    );
  }
}
console.log('AC5: coherence and restraint dimension prose is repo-agnostic (no crate names or paths)');

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
const { survivors: cout } = await buildReviewPipeline('plan', deps(cspy))(CTX);
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
const { survivors: degraded } = await buildReviewPipeline('code', deps(nspy2))(CTX);
assert.deepEqual(degraded, [], 'no-model callers keep the pre-existing lenient behavior');
assert.ok(
  nspy2.calls.every((c) => c.model === undefined),
  'no model key is invented when the caller supplies none'
);

// (d) A genuinely clean review (findings: []) must NOT trip the guard.
const cleanSpy = makeSpyAgent({}, {});
const { survivors: cleanOut } = await buildReviewPipeline('code', deps(cleanSpy))({ ...CTX, findModel: 'haiku', verifyModel: 'opus' });
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
  ['coherence', 'architectural-fit', 'restraint'],
  'coherence, architectural-fit, and restraint stay always-on in plan mode'
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
// AC-TABLE CHANNEL (classify-outcome-ac-table-channel) — a surviving FAIL/
// PARTIAL AC-table criterion mechanically forces `rework`, independent of
// finding severity and refutation.
// ============================================================================
assert.equal(acTableHasGap(null), false, 'a null AC table is not a gap');
assert.equal(acTableHasGap(undefined), false, 'an undefined AC table is not a gap');
assert.equal(acTableHasGap([]), false, 'an empty AC table is not a gap');
assert.equal(acTableHasGap([{ criterion: 'x', status: 'PASS', evidence: 'y' }]), false, 'an all-PASS table is not a gap');
assert.equal(acTableHasGap([{ criterion: 'x', status: 'FAIL', evidence: 'y' }]), true, 'a FAIL entry is a gap');
assert.equal(acTableHasGap([{ criterion: 'x', status: 'PARTIAL', evidence: 'y' }]), true, 'a PARTIAL entry is a gap');

// (a) zero findings, a FAIL acTable — classifyOutcome still returns 'rework'.
// Proves the guarantee no longer depends on finding severity/refutation at all.
assert.equal(
  classifyOutcome({ tier: 'medium', codeReviews: [[]], acTable: [{ criterion: 'x', status: 'FAIL', evidence: 'none' }] }),
  'rework',
  'a FAIL AC-table entry forces rework even with zero surviving findings'
);
assert.equal(
  classifyOutcome({ tier: 'medium', codeReviews: [[]], acTable: [{ criterion: 'x', status: 'PARTIAL', evidence: 'partial' }] }),
  'rework',
  'a PARTIAL AC-table entry forces rework even with zero surviving findings'
);
assert.equal(
  classifyOutcome({ tier: 'medium', codeReviews: [[]], acTable: [{ criterion: 'x', status: 'PASS', evidence: 'ok' }] }),
  'reviewed',
  'an all-PASS AC table does not force rework'
);
assert.equal(
  classifyOutcome({ tier: 'medium', codeReviews: [[]], acTable: null }),
  'reviewed',
  'a null AC table (e.g. plan mode, or ac dimension did not run) does not force rework'
);
// The AC-table gate can only ever yield rework, never escalated — a blocking
// plan finding still wins (rule 1 fires first).
assert.equal(
  classifyOutcome({ tier: 'medium', planFindings: BLOCKER, acTable: [{ criterion: 'x', status: 'FAIL', evidence: 'y' }] }),
  'escalated',
  'a blocking plan finding still escalates ahead of the AC-table gate'
);

// (b) Driven-pipeline: a fake `ac` finder returns the AC_REVIEW_SCHEMA shape
// (an `ac` array with a FAIL entry, no findings), and the harness asserts
// runReview resolves { survivors: [], acTable: [FAIL entry] } — the FAIL entry
// intact, not folded into (or lost as) a finding.
{
  const acSpy = { agent: null, calls: [] };
  acSpy.agent = async (prompt, opts) => {
    const label = (opts && opts.label) || '';
    acSpy.calls.push({ label, prompt });
    if (label === 'find:code:ac') {
      return { ac: [{ criterion: 'the CLI must reject empty input', status: 'FAIL', evidence: 'no such check exists' }] };
    }
    if (label.startsWith('find:')) return { findings: [] };
    throw new Error('unexpected agent label: ' + label);
  };
  const acResult = await buildReviewPipeline('code', deps(acSpy))(CTX);
  assert.deepEqual(acResult.survivors, [], 'no findings survive when only the ac dimension reports (via the ac table)');
  assert.deepEqual(
    acResult.acTable,
    [{ criterion: 'the CLI must reject empty input', status: 'FAIL', evidence: 'no such check exists' }],
    'runReview resolves the ac table intact, with the FAIL entry preserved'
  );
  const acFindCall = acSpy.calls.find((c) => c.label === 'find:code:ac');
  assert.ok(acFindCall, 'the ac dimension finder was actually invoked');
  assert.ok(acFindCall.prompt.includes('AC_REVIEW'), 'the ac dimension is prompted for the AC_REVIEW schema shape');
}

// (c) Plan-mode regression: runReview('plan', ...) still resolves acTable:
// null and unchanged bare-survivors-consuming behavior — the ac dimension does
// not exist in plan mode, so nothing can ever populate it.
{
  const planAcResult = await buildReviewPipeline('plan', deps(makeSpyAgent(planFindings, planVerdicts)))(CTX);
  assert.equal(planAcResult.acTable, null, 'plan mode never populates an ac table');
  assert.deepEqual(
    planAcResult.survivors.map((f) => f.id),
    ['vague-step'],
    'plan mode survivors are unchanged by the ac-table-channel addition'
  );
}
console.log('AC-TABLE-CHANNEL: a surviving FAIL/PARTIAL AC-table criterion mechanically forces rework');

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
# pushes it into the prompt, plus its entry in the Node-only export list (which
# would otherwise reference a now-undefined name and turn the strip into a parse
# error rather than a behavioral mutation). All three are whole-statement
# removals, so the mutated file stays syntactically valid — it leaves the
# block-comment prose above the constant in place, which is harmless.
awk '
    /^const PLAN_SEVERITY_CALIBRATION =$/ { skip = 1 }
    skip && /;$/ { skip = 0; next }
    skip { next }
    /lines\.push\(PLAN_SEVERITY_CALIBRATION\);/ { next }
    /^  PLAN_SEVERITY_CALIBRATION,$/ { next }
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

# --- 4a. PROJECT-AGNOSTIC DIMENSION PROSE MUTATION SELF-TESTS -----------------
# Prove AC2b (embedded in section 3's test.mjs) is not vacuous, in ALL THREE of
# the directions it asserts. Three independent hermetic scratch copies of the lib:
#   M1 re-introduces a project-specific token into a rewritten dimension's title
#      (the negative half — a regression back to rdm's own conventions);
#   M2 renames the principles-document pointer away (the positive half — a
#      "genericity" achieved by deleting the convention channel instead of
#      redirecting it);
#   M3 leaks a language-specific idiom into a code dimension (the ledger half —
#      proving the now-EMPTY carve-out cannot silently re-open).
# All three mutations are literal string substitutions inside existing string
# literals, so the mutated file always stays importable JS and a parse error
# cannot masquerade as a passing self-test.
say "4a. Project-agnostic dimension prose mutation self-tests (proves AC2b is not vacuous)"

cat >"$TMP/agnostic-mut-test.mjs" <<'NODE_AGNOSTIC_MUT'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const mutatedLibPath = process.argv[2];
const half = process.argv[3]; // 'negative' | 'positive' | 'ledger'
const mod = await import(pathToFileURL(mutatedLibPath).href); // must still parse/import cleanly

const REWRITTEN_CODE_DIMS = ['correctness', 'architecture', 'api-docs', 'changelog', 'security'];
const forbiddenCodeTokens = [
  'rdm-core',
  'rdm-cli',
  'rdm-server',
  'rdm-mcp',
  'anyhow',
  'rustdoc',
  'Rust',
  'cargo',
  'Cargo',
  'crate',
  'missing_docs',
  '# Errors',
  '# Panics',
  '# Safety',
  'CHANGELOG.md',
];

function assertNoForbiddenToken(m) {
  for (const key of REWRITTEN_CODE_DIMS) {
    const dim = m.DIMENSIONS.code.find((d) => d.key === key);
    assert.ok(dim, 'missing dimension ' + key);
    const scopedText = [dim.title, dim.focus].join('\n');
    for (const tok of forbiddenCodeTokens) {
      assert.ok(scopedText.indexOf(tok) === -1, 'forbidden token in ' + key + ': ' + tok);
    }
  }
}

function assertPrinciplesPointer(m) {
  for (const key of REWRITTEN_CODE_DIMS) {
    const dim = m.DIMENSIONS.code.find((d) => d.key === key);
    assert.ok(dim, 'missing dimension ' + key);
    assert.ok(dim.focus.includes('principles document'), 'no principles pointer in ' + key);
    assert.ok(
      dim.focus.includes('docs/principles.md') &&
        dim.focus.includes('CLAUDE.md') &&
        dim.focus.includes('AGENTS.md'),
      'no principles fallback chain in ' + key
    );
  }
}

function assertCarveOutLedger(m) {
  const LANGUAGE_SPECIFIC_IDIOMS = ['`unsafe`', '// SAFETY:'];
  const stillLanguageSpecific = m.DIMENSIONS.code
    .filter((d) => LANGUAGE_SPECIFIC_IDIOMS.some((tok) => [d.title, d.focus].join('\n').includes(tok)))
    .map((d) => d.key);
  assert.deepEqual(stillLanguageSpecific, [], 'carve-out re-opened: ' + JSON.stringify(stillLanguageSpecific));
}

if (half === 'negative') {
  assert.throws(
    () => assertNoForbiddenToken(mod),
    'the forbidden-token check must FAIL once a project-specific token is planted back into a rewritten dimension'
  );
} else if (half === 'ledger') {
  assert.throws(
    () => assertCarveOutLedger(mod),
    'the carve-out ledger must FAIL once a language-specific idiom leaks into any code dimension'
  );
} else {
  assert.throws(
    () => assertPrinciplesPointer(mod),
    'the principles-pointer check must FAIL once the pointer is renamed away'
  );
}

console.log('agnostic mutation self-test (' + half + ') passed');
NODE_AGNOSTIC_MUT

AGMUT="$TMP/agnostic-mut"
mkdir -p "$AGMUT"

# CONTROL: all three halves must PASS against the real, unmutated file —
# otherwise every mutation below would "fail correctly" for the wrong reason.
cat >"$TMP/agnostic-control.mjs" <<'NODE_AGNOSTIC_CONTROL'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const mod = await import(pathToFileURL(process.argv[2]).href);
const REWRITTEN_CODE_DIMS = ['correctness', 'architecture', 'api-docs', 'changelog', 'security'];
for (const key of REWRITTEN_CODE_DIMS) {
  const dim = mod.DIMENSIONS.code.find((d) => d.key === key);
  assert.ok(dim, 'missing dimension ' + key);
  const scopedText = [dim.title, dim.focus].join('\n');
  for (const tok of ['rdm-core', 'rustdoc', 'missing_docs', 'CHANGELOG.md', 'anyhow']) {
    assert.ok(scopedText.indexOf(tok) === -1, 'control: forbidden token in ' + key + ': ' + tok);
  }
  assert.ok(dim.focus.includes('principles document'), 'control: no principles pointer in ' + key);
}
const LANGUAGE_SPECIFIC_IDIOMS = ['`unsafe`', '// SAFETY:'];
const stillLanguageSpecific = mod.DIMENSIONS.code
  .filter((d) => LANGUAGE_SPECIFIC_IDIOMS.some((tok) => [d.title, d.focus].join('\n').includes(tok)))
  .map((d) => d.key);
assert.deepEqual(
  stillLanguageSpecific,
  [],
  'control: the carve-out ledger does not hold on the real lib: ' + JSON.stringify(stillLanguageSpecific)
);
console.log('agnostic control passed');
NODE_AGNOSTIC_CONTROL

if run_node "$TMP/agnostic-control.mjs" "$LIB" >/dev/null 2>&1; then
    pass "4a-control: the unmutated lib satisfies all three halves of AC2b"
else
    fail "4a-control: the unmutated lib FAILS AC2b — every mutation below is vacuous"
fi

# M1 (negative half): plant a project-specific token back into api-docs' title.
sed "s/title: 'Public API docs',/title: 'Public API docs (rdm-core rustdoc)',/" \
    "$LIB" >"$AGMUT/m1.mjs"
if diff -q "$LIB" "$AGMUT/m1.mjs" >/dev/null 2>&1; then
    fail "4a-M1: the planted-token mutation did not apply — the anchor text moved"
fi
if run_node "$TMP/agnostic-mut-test.mjs" "$AGMUT/m1.mjs" negative >/dev/null 2>&1; then
    pass "4a-M1: the forbidden-token check fires on a planted project-specific token"
else
    fail "4a-M1: AC2b's forbidden-token check did NOT fire on a planted rdm-core/rustdoc token"
fi

# M2 (positive half): rename the principles-document pointer away.
sed 's/principles document/design notes/g' "$LIB" >"$AGMUT/m2.mjs"
if diff -q "$LIB" "$AGMUT/m2.mjs" >/dev/null 2>&1; then
    fail "4a-M2: the pointer-removal mutation did not apply — the anchor text moved"
fi
if run_node "$TMP/agnostic-mut-test.mjs" "$AGMUT/m2.mjs" positive >/dev/null 2>&1; then
    pass "4a-M2: the principles-pointer check fires when the pointer is renamed away"
else
    fail "4a-M2: AC2b's principles-pointer check did NOT fire on a removed pointer"
fi

# M3 (ledger half): leak a language-specific idiom into `correctness`, i.e. into
# a code dimension. The now-empty exact-set assertion must fire.
# shellcheck disable=SC2016  # the backticks are literal prose in the planted idiom
sed 's/User-facing errors must be actionable/Every `unsafe` block must be justified; user-facing errors must be actionable/' \
    "$LIB" >"$AGMUT/m3.mjs"
if diff -q "$LIB" "$AGMUT/m3.mjs" >/dev/null 2>&1; then
    fail "4a-M3: the leaked-idiom mutation did not apply — the anchor text moved"
fi
if run_node "$TMP/agnostic-mut-test.mjs" "$AGMUT/m3.mjs" ledger >/dev/null 2>&1; then
    pass "4a-M3: the carve-out ledger fires when a language-specific idiom leaks into a code dimension"
else
    fail "4a-M3: AC2b's carve-out ledger did NOT fire on an idiom leaked into correctness"
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

# --- 5b-mechanical. Mechanical-tier pin: fetch/gate agents pinned, act:* is not.
#
# JUDGMENT-SITE MODEL BINDING WAS EVALUATED AND DELIBERATELY LEFT AS-IS.
# This section pins the MECHANICAL tier only. The separate questions — whether
# plan-review's finders/refuters should carry an explicit model (today they
# inherit the session model, because lib/plan-review.mjs passes no
# findModel/verifyModel), and whether refuters can move off Opus at all — were
# measured against an adjudicated finding corpus. The decision, its numbers and
# the follow-up task live in docs/refuter-model-tiering.md. No model binding
# changed, so no criterion here needed updating; read that doc before
# re-litigating it.
say "5b-mechanical. Mechanical-tier pin: fetch:roadmap, fetch:<kind>, gate:clear-tag:* resolve to the mechanical model"
# shellcheck disable=SC1091
. "$REPO_ROOT/scripts/lib/mechanical-tier-check.sh"

agent_option_blocks "$PLAN_REVIEW" >"$TMP/mech-blocks"
[ -s "$TMP/mech-blocks" ] || fail "AC-MECHANICAL-TIER: could not extract any agent() option blocks from plan-review.js"

# label_re 'fetch:' deliberately matches BOTH the literal 'fetch:roadmap'
# label and the dynamic 'fetch:' + kind label (task/phase) — the label regex
# is an unanchored substring match, and neither label's own literal text
# contains a regex metacharacter, so a broad prefix safely covers both
# fetch:roadmap and fetch:<kind> in one assertion (avoids embedding the `+`
# concatenation operator from the dynamic label's source text into a regex).
assert_label_model "$TMP/mech-blocks" 'fetch:' '_mechanicalModel' ||
    fail "AC-MECHANICAL-TIER: fetch:roadmap and fetch:<kind> (task/phase) must resolve to model: _mechanicalModel"
assert_label_model "$TMP/mech-blocks" 'gate:clear-tag:' '_mechanicalModel' ||
    fail "AC-MECHANICAL-TIER: every gate:clear-tag:* call must resolve to model: _mechanicalModel"
pass "AC-MECHANICAL-TIER: fetch:roadmap, fetch:<kind>, and gate:clear-tag:* resolve to model: _mechanicalModel"

# Negative: act:* is the orchestrator/judgment step and must NOT be pinned to
# the mechanical tier.
assert_label_not_model "$TMP/mech-blocks" 'act:' '_mechanicalModel' ||
    fail "AC-MECHANICAL-TIER: act:* must NOT be pinned to model: _mechanicalModel (judgment stage)"
pass "AC-MECHANICAL-TIER: act:* is left unpinned (judgment stage)"

# Self-test: plant a repoint away from _mechanicalModel on fetch:roadmap and
# prove the check now fails; restore and prove it passes again.
sed "/label: 'fetch:roadmap'/,/^      })/ s/model: _mechanicalModel,/model: 'claude-opus-4-8',/" "$PLAN_REVIEW" >"$TMP/pr.mech-mutant"
agent_option_blocks "$TMP/pr.mech-mutant" >"$TMP/mech-blocks-mutant"
if assert_label_model "$TMP/mech-blocks-mutant" 'fetch:roadmap' '_mechanicalModel'; then
    fail "AC-MECHANICAL-TIER: detector missed a fetch:roadmap repoint away from _mechanicalModel"
fi
pass "AC-MECHANICAL-TIER: detector fires when fetch:roadmap is repointed away from _mechanicalModel"

# --- 5b-drift. PLAN-REVIEW DRIVER BLOCK: byte-identical (lib vs workflow) ------
# The plan-review DRIVER (parsePlanArgs + the fetch/act/gate orchestration in
# runPlanReviewDriver) is the single source of truth in lib/plan-review.mjs and
# is copied BYTE-IDENTICAL into plan-review.js's `plan-review-driver` block. Like
# dispatch-phase's dispatch-outcome block, this copy is NOT stamped by the
# generator — gate it for byte-equality here so a drifted copy cannot ship.
say "5b-drift. plan-review-driver block is byte-identical between the lib and the workflow"
[ -f "$PLAN_LIB" ] || fail "lib/plan-review.mjs not found: $PLAN_LIB"
extract_driver_block() {
    awk '
        index($0, ">>> plan-review-driver:begin") { infence = 1; next }
        index($0, ">>> plan-review-driver:end")   { infence = 0 }
        infence { print }
    ' "$1"
}
extract_driver_block "$PLAN_LIB" >"$TMP/plan-driver-lib"
extract_driver_block "$PLAN_REVIEW" >"$TMP/plan-driver-wf"
[ -s "$TMP/plan-driver-lib" ] || fail "no plan-review-driver block found between markers in $PLAN_LIB"
[ -s "$TMP/plan-driver-wf" ] || fail "no plan-review-driver block found between markers in $PLAN_REVIEW"
if diff -u "$TMP/plan-driver-lib" "$TMP/plan-driver-wf" >"$TMP/plan-driver-diff" 2>&1; then
    pass "plan-review-driver block matches byte-for-byte between lib and workflow"
else
    cat "$TMP/plan-driver-diff" >&2
    fail "plan-review-driver block DRIFTED — copy the lib block verbatim into $PLAN_REVIEW"
fi
# The driver's load-bearing symbols must be present in BOTH copies (guards against
# a partial mirror the byte-diff above would also catch, but names the gap).
for sym in 'function parsePlanArgs' 'function buildReviewUnits' 'async function runPlanReviewDriver' \
    "buildReviewPipeline('plan')" 'stripNonPhaseUnitOfWork' 'filterPlanReviewTag' 'classifyPlanOutcome'; do
    grep -q "$sym" "$TMP/plan-driver-lib" || fail "plan-review-driver block in the LIB is missing $sym"
    grep -q "$sym" "$TMP/plan-driver-wf" || fail "plan-review-driver block in the WORKFLOW is missing $sym (partial mirror?)"
done
# The runtime entry that calls runPlanReviewDriver lives OUTSIDE the copied block
# (it uses top-level `return` / ambient globals, illegal in a Node module), so it
# must NOT appear in the lib copy.
grep -q 'return await runPlanReviewDriver' "$PLAN_REVIEW" ||
    fail "plan-review.js must invoke the driver via a thin runtime entry (return await runPlanReviewDriver(...))"
if grep -q 'return await runPlanReviewDriver' "$PLAN_LIB"; then
    fail "the top-level runtime entry leaked into the lib copy — it must stay OUTSIDE the block"
fi
pass "plan-review-driver block is byte-in-sync and the runtime entry is workflow-only"

# --- 5b-exec. PLAN-REVIEW DRIVER: executed against a fake agent/parallel -------
# The blocking gap the prior pass shipped: parsePlanArgs and the fetch/act/gate
# orchestration were only STATIC-grepped, never executed. Import the canonical
# lib and DRIVE it with a recording fake agent + a reference parallel, asserting
# the real control flow: four-target precedence across three arg SHAPES, the
# malformed-JSON fallback, the no-target throw, fail-closed on an unread plan, the
# per-unit independent act+gate, the single-target flattening, and the
# implementation-plan no-act/no-gate carve-out. Zero LLM calls.
say "5b-exec. plan-review driver executes: arg parsing + fetch/act/gate orchestration"
cat >"$TMP/plan-driver-test.mjs" <<'NODE_DRIVER_TEST'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const mod = await import(pathToFileURL(process.argv[2]).href);
const { parsePlanArgs, buildReviewUnits, runPlanReviewDriver, formatUnitBudget } = mod;
for (const name of ['parsePlanArgs', 'buildReviewUnits', 'runPlanReviewDriver', 'formatUnitBudget']) {
  assert.equal(typeof mod[name], 'function', name + ' must be exported from lib/plan-review.mjs');
}

// ---- parsePlanArgs: four target kinds across three arg SHAPES ----------------
// (a) flag STRING form.
assert.equal(parsePlanArgs('--task fix-bug').kind, 'task', 'flag string --task');
assert.equal(parsePlanArgs('--task fix-bug').task, 'fix-bug', 'flag string --task slug');
assert.equal(parsePlanArgs('--roadmap big-thing').kind, 'roadmap', 'flag string --roadmap (no phase)');
assert.equal(parsePlanArgs('big-thing phase-2-foo').kind, 'phase', 'positional <slug> <phase> => phase');
assert.equal(parsePlanArgs('big-thing phase-2-foo').phase, 'phase-2-foo', 'positional phase captured');
// Positional <slug> with NO phase behaves exactly like --roadmap <slug>.
assert.equal(parsePlanArgs('big-thing').kind, 'roadmap', 'bare positional <slug> => roadmap');
assert.equal(parsePlanArgs('big-thing').roadmap, 'big-thing', 'bare positional roadmap captured');
assert.equal(parsePlanArgs('--implementation-plan').kind, 'implementation-plan', 'flag string --implementation-plan');
// (b) JSON payload form.
assert.equal(parsePlanArgs('{"task":"j-task"}').kind, 'task', 'JSON payload task');
assert.equal(parsePlanArgs('{"roadmap":"r","phase":"phase-1-x"}').kind, 'phase', 'JSON payload roadmap+phase');
assert.equal(parsePlanArgs('{"roadmap":"r"}').kind, 'roadmap', 'JSON payload roadmap-only');
assert.equal(parsePlanArgs('{"implementationPlan":true,"planText":"P"}').kind, 'implementation-plan', 'JSON payload impl-plan');
assert.equal(parsePlanArgs('{"implementationPlan":true,"planText":"P"}').planText, 'P', 'planText captured from JSON');
// (c) structured OBJECT form.
assert.equal(parsePlanArgs({ task: 'o-task' }).kind, 'task', 'object task');
assert.equal(parsePlanArgs({ roadmap: 'r', phase: 'phase-3-y' }).kind, 'phase', 'object roadmap+phase');
assert.equal(parsePlanArgs({ roadmap: 'r' }).kind, 'roadmap', 'object roadmap-only');
assert.equal(parsePlanArgs({ implementationPlan: true }).kind, 'implementation-plan', 'object impl-plan');

// ---- Precedence is fixed and total when several targets co-occur -------------
// implementation-plan wins over everything; then task; then roadmap+phase; then roadmap.
assert.equal(parsePlanArgs({ implementationPlan: true, task: 't', roadmap: 'r', phase: 'p' }).kind, 'implementation-plan',
  'impl-plan outranks task/roadmap/phase');
assert.equal(parsePlanArgs({ task: 't', roadmap: 'r', phase: 'p' }).kind, 'task', 'task outranks roadmap+phase');
assert.equal(parsePlanArgs({ roadmap: 'r', phase: 'p' }).kind, 'phase', 'roadmap+phase => phase (outranks bare roadmap)');

// ---- Malformed-JSON fallback: a `{`-leading string that does NOT parse -------
// degrades to a positional target, NOT a throw, NOT a silent empty object.
// A single-token `{`-leading string that does not parse falls back to a bare
// positional target => roadmap (raw string preserved as the token).
assert.equal(parsePlanArgs('{bad').kind, 'roadmap', 'malformed JSON falls back to a positional target');
assert.equal(parsePlanArgs('{bad').roadmap, '{bad', 'malformed JSON fallback tokenizes the raw string');
// A multi-token malformed string tokenizes as `<slug> [phase]` (fallback, not a throw).
assert.equal(parsePlanArgs('{not json').kind, 'phase', 'malformed multi-token JSON => positional slug+phase');

// ---- No-target throw ---------------------------------------------------------
assert.throws(() => parsePlanArgs(''), /no target/, 'empty args throws an actionable no-target error');
assert.throws(() => parsePlanArgs({}), /no target/, 'empty object throws an actionable no-target error');

// ---- buildReviewUnits: fail-closed on an empty/unread body -------------------
assert.equal(buildReviewUnits({ kind: 'task', task: 't' }, null).fetchFailed, true, 'null fetch => fetchFailed');
assert.equal(buildReviewUnits({ kind: 'task', task: 't' }, { body: '', tags: [] }).fetchFailed, true, 'empty body => fetchFailed');
assert.equal(buildReviewUnits({ kind: 'phase', roadmap: 'r', phase: 'p' }, { body: '   ', tags: [] }).fetchFailed, true,
  'whitespace-only body => fetchFailed');
{
  const b = buildReviewUnits({ kind: 'roadmap', roadmap: 'r' },
    { body: 'RB', tags: ['needs-plan-review'], phases: [{ stem: 'phase-1-a', body: 'PA', tags: ['needs-plan-review'] }] });
  assert.equal(b.fetchFailed, false, 'roadmap with body does not fail');
  assert.equal(b.units.length, 2, 'roadmap => body unit + one unit per phase');
  assert.equal(b.units[0].targetType, 'roadmap', 'first unit is the roadmap body');
  assert.equal(b.units[1].targetType, 'phase', 'second unit is a phase');
  assert.equal(b.units[1].ident, 'phase-1-a', 'phase unit ident is the stem');
}

// ---- runPlanReviewDriver: a recording fake agent + a reference parallel ------
// findingsByTarget maps a review-unit target substring to the survivors the fake
// pipeline returns for it, so we can seed a rework on ONE phase and reviewed on
// the rest and prove independent gating end-to-end through the real driver.
// `budgetByTarget` (optional) maps the same target substrings to the per-unit
// refutation-budget object the pipeline reports for that unit, so the driver's
// budget threading is EXERCISED rather than assumed. Every review context the
// driver builds is recorded in `reviewCtxs` (which is how the `maxRefutations`
// thread from parsePlanArgs into runPlanReview is checked), and every log line
// in `logs`.
function makeHarness(findingsByTarget, fetchResults, budgetByTarget) {
  const calls = [];
  const reviewCtxs = [];
  const logs = [];
  const budgets = budgetByTarget || {};
  const agent = async (prompt, opts) => {
    calls.push({ label: opts && opts.label, phase: opts && opts.phase, prompt });
    const label = (opts && opts.label) || '';
    if (label.indexOf('fetch:') === 0) return fetchResults[label] !== undefined ? fetchResults[label] : null;
    // act / gate:clear-tag agents just acknowledge.
    return { ok: true };
  };
  const parallel = (thunks) => Promise.all(thunks.map((t) => t()));
  // runPlanReview is a `runReview` from the canonical review source and
  // resolves { survivors, acTable, budget } — acTable is always null in plan
  // mode; `budget` is the per-unit refutation-budget accounting.
  const runPlanReview = async (ctx) => {
    reviewCtxs.push(ctx);
    const target = (ctx && ctx.target) || '';
    const budgetFor = () => {
      for (const key of Object.keys(budgets)) {
        if (target.indexOf(key) !== -1) return budgets[key];
      }
      return null;
    };
    for (const key of Object.keys(findingsByTarget)) {
      if (target.indexOf(key) !== -1) {
        return { survivors: findingsByTarget[key], acTable: null, budget: budgetFor() };
      }
    }
    return { survivors: [], acTable: null, budget: budgetFor() };
  };
  const log = (line) => logs.push(String(line));
  return { deps: { agent, parallel, runPlanReview, log }, calls, reviewCtxs, logs };
}
const blockingCoherence = [{ id: 'c', concern: 'coherence', severity: 'blocking', confidence: 90, what_fails: 'ambiguous' }];

// (1) --roadmap: one phase reworks, the roadmap body + the other phases pass.
//     Independent gating: the reworked phase gets NO gate:clear-tag agent call;
//     each reviewed unit gets exactly one.
{
  const { deps, calls } = makeHarness(
    { 'phase-1-a': blockingCoherence }, // only phase-1-a has a blocking finding
    {
      'fetch:roadmap': {
        body: 'RB', tags: ['needs-plan-review'],
        phases: [
          { stem: 'phase-1-a', body: 'PA', tags: ['needs-plan-review'] },
          { stem: 'phase-2-b', body: 'PB', tags: ['needs-plan-review'] },
        ],
      },
    }
  );
  const res = await runPlanReviewDriver({ roadmap: 'big-thing' }, deps);
  assert.equal(res.kind, 'roadmap', 'roadmap result kind');
  const byIdent = Object.fromEntries(res.units.map((u) => [u.ident, u]));
  assert.equal(byIdent['big-thing'].outcome, 'reviewed', 'roadmap body reviewed');
  assert.equal(byIdent['big-thing'].tagCleared, true, 'roadmap body tag cleared');
  assert.equal(byIdent['phase-1-a'].outcome, 'rework', 'phase-1-a reworks');
  assert.equal(byIdent['phase-1-a'].clearsPlanReviewTag, false, 'reworked phase keeps its tag');
  assert.equal(byIdent['phase-1-a'].tagCleared, false, 'reworked phase tag NOT cleared');
  assert.equal(byIdent['phase-2-b'].outcome, 'reviewed', 'phase-2-b reviewed');
  assert.equal(byIdent['phase-2-b'].tagCleared, true, 'phase-2-b tag cleared');
  assert.strictEqual(byIdent['phase-1-a'].status, null, 'plan gate never persists a status');
  // Exactly TWO gate:clear-tag calls (the two reviewed units), none for the reworked phase.
  const clearCalls = calls.filter((c) => (c.label || '').indexOf('gate:clear-tag:') === 0);
  assert.equal(clearCalls.length, 2, 'only the two reviewed units get a tag-clear agent call');
  assert.ok(!clearCalls.some((c) => c.label.indexOf('phase-1-a') !== -1), 'reworked phase gets NO tag-clear call');
  // The reworked phase DID get a fix-application act call (it has survivors);
  // reviewed-clean units did not. `act:round-note:*` is a separate step (the
  // round-capping audit note) and is asserted on its own below.
  const actCalls = calls.filter((c) => (c.label || '').indexOf('act:') === 0 && (c.label || '').indexOf('act:round-note:') !== 0);
  assert.equal(actCalls.length, 1, 'only the unit with survivors gets a fix-application act call');
  assert.ok(actCalls[0].label.indexOf('phase-1-a') !== -1, 'the act call targets the reworked phase');
  // Round-note write: only the non-reviewed unit (phase-1-a) gets one.
  const roundNoteCalls = calls.filter((c) => (c.label || '').indexOf('act:round-note:') === 0);
  assert.equal(roundNoteCalls.length, 1, 'only the non-reviewed unit gets a round-note write call');
  assert.ok(roundNoteCalls[0].label.indexOf('phase-1-a') !== -1, 'the round-note call targets the reworked phase');
}

// (2) single --task target: flattened onto the top-level result; reviewed clears tag.
{
  const { deps, calls } = makeHarness({}, { 'fetch:task': { body: 'TB', tags: ['needs-plan-review', 'depends-unlanded'] } });
  const res = await runPlanReviewDriver({ task: 'fix-bug' }, deps);
  assert.equal(res.kind, 'task', 'task result kind');
  assert.equal(res.outcome, 'reviewed', 'single target flattens outcome onto the top level');
  assert.ok(Array.isArray(res.findings), 'single target flattens findings onto the top level');
  assert.equal(res.units.length, 1, 'one unit for a task target');
  // The tag write preserves the sibling: filterPlanReviewTag(['needs-plan-review','depends-unlanded']) => ['depends-unlanded'].
  const tagCall = calls.find((c) => (c.label || '').indexOf('gate:clear-tag:') === 0);
  assert.ok(tagCall, 'a reviewed task triggers a tag-clear agent call');
  assert.ok(tagCall.prompt.indexOf('--tags "depends-unlanded"') !== -1, 'the sibling tag is preserved in the write-back');
  assert.ok(tagCall.prompt.indexOf('needs-plan-review') === -1 || tagCall.prompt.indexOf('clear needs-plan-review') !== -1,
    'needs-plan-review is not written back into the tag list');
}

// (3) fail-closed: an unread task body escalates and mutates NOTHING.
{
  const { deps, calls } = makeHarness({}, { 'fetch:task': { body: '', tags: ['needs-plan-review'] } });
  const res = await runPlanReviewDriver({ task: 'ghost' }, deps);
  assert.equal(res.fetchError, true, 'unread plan reports a fetch error');
  assert.equal(res.outcome, 'escalated', 'unread plan is fail-closed to escalated');
  assert.equal(calls.filter((c) => (c.label || '').indexOf('gate:clear-tag:') === 0).length, 0,
    'fail-closed: NO tag-clear agent call on an unread plan');
  assert.equal(calls.filter((c) => (c.label || '').indexOf('act:') === 0).length, 0,
    'fail-closed: NO act agent call on an unread plan');
}

// (4) --implementation-plan: report-only. No fetch, no act, no gate — the driver
//     must never call the agent for anything but... nothing. runPlanReview is the
//     only async touched.
{
  const { deps, calls } = makeHarness({ 'PLAN TEXT': blockingCoherence }, {});
  const res = await runPlanReviewDriver({ implementationPlan: true, planText: 'PLAN TEXT here' }, deps);
  assert.equal(res.kind, 'implementation-plan', 'impl-plan result kind');
  assert.equal(res.outcome, 'rework', 'impl-plan still classifies the outcome');
  assert.ok(!('units' in res), 'impl-plan is report-only (no gated units)');
  assert.equal(calls.length, 0, 'impl-plan makes NO agent calls (no fetch, no act, no gate)');
}

// ---- (5) REFUTATION BUDGET threading through the plan-review driver ----------
// The code-mode analogue (dispatch-phase's gates + the OUTCOME's reviewBudget)
// is covered in verify-workflow-dispatch.sh; this is the plan-mode driver's own
// wiring around the already-tested pipeline core: parsePlanArgs resolves the
// budget, reviewUnit and the implementation-plan branch thread it into every
// runPlanReview context, carry the reported `budget` on their result, and
// append formatUnitBudget's clause to the per-unit log line ONLY on a hit.

// (5a) formatUnitBudget itself: a clause on a hit, byte-nothing otherwise.
const HIT_BUDGET = { max: 5, produced: 13, gating: 13, graded: 5, passedThroughNonGating: 0, passedThroughBudget: 8, refuterErrors: 0, hit: true };
const CLEAN_BUDGET = { max: 5, produced: 3, gating: 3, graded: 3, passedThroughNonGating: 0, passedThroughBudget: 0, refuterErrors: 0, hit: false };
assert.equal(formatUnitBudget(HIT_BUDGET), ' [review budget hit: 13 produced, 5 graded, 8 ungraded]',
  'formatUnitBudget renders produced/graded/ungraded on a hit');
assert.equal(formatUnitBudget(CLEAN_BUDGET), '', 'an under-budget unit logs a byte-unchanged line');
assert.equal(formatUnitBudget(null), '', 'a budget-less unit logs a byte-unchanged line');

// (5b) the resolved budget reaches EVERY runPlanReview context — default,
//      caller override, and the 0-is-legal case (never conflated with unset).
{
  const h = makeHarness({}, { 'fetch:task': { body: 'TB', tags: ['needs-plan-review'] } });
  await runPlanReviewDriver({ task: 'fix-bug' }, h.deps);
  assert.equal(h.reviewCtxs.length, 1, 'one review context for a single task unit');
  assert.equal(h.reviewCtxs[0].maxRefutations, parsePlanArgs({ task: 'fix-bug' }).maxRefutations,
    'the context carries exactly the budget parsePlanArgs resolved');
  assert.equal(h.reviewCtxs[0].maxRefutations, 5, 'and that default is the review core\'s 5');
}
{
  const h = makeHarness({}, { 'fetch:task': { body: 'TB', tags: ['needs-plan-review'] } });
  await runPlanReviewDriver({ task: 'fix-bug', maxRefutations: 2 }, h.deps);
  assert.equal(h.reviewCtxs[0].maxRefutations, 2, 'a caller override is threaded into the review context');
}
{
  const h = makeHarness({}, { 'fetch:task': { body: 'TB', tags: ['needs-plan-review'] } });
  await runPlanReviewDriver({ task: 'fix-bug', maxRefutations: 0 }, h.deps);
  assert.strictEqual(h.reviewCtxs[0].maxRefutations, 0,
    '0 is legal and distinct from unset — a falsy check must NOT fall back to the default');
}

// (5c) a roadmap run: EVERY unit's context carries the budget, the per-unit
//      result carries the reported `budget`, and only the budget-hit unit's log
//      line gains the clause.
{
  const h = makeHarness(
    { 'phase-1-a': blockingCoherence },
    {
      'fetch:roadmap': {
        body: 'RB', tags: ['needs-plan-review'],
        phases: [
          { stem: 'phase-1-a', body: 'PA', tags: ['needs-plan-review'] },
          { stem: 'phase-2-b', body: 'PB', tags: ['needs-plan-review'] },
        ],
      },
    },
    { 'phase-1-a': HIT_BUDGET, 'phase-2-b': CLEAN_BUDGET }
  );
  const res = await runPlanReviewDriver({ roadmap: 'big-thing', maxRefutations: 4 }, h.deps);
  assert.equal(h.reviewCtxs.length, 3, 'one review context per unit (roadmap body + two phases)');
  assert.ok(h.reviewCtxs.every((c) => c.maxRefutations === 4),
    'EVERY unit is reviewed under the same resolved budget');
  const byIdent = Object.fromEntries(res.units.map((u) => [u.ident, u]));
  assert.ok(byIdent['phase-1-a'].budget, 'the budget-hit unit carries a budget field on its result');
  assert.equal(byIdent['phase-1-a'].budget.hit, true, 'and it reports the hit');
  assert.equal(byIdent['phase-1-a'].budget.passedThroughBudget, 8, 'with the ungraded count intact');
  assert.ok(byIdent['phase-2-b'].budget, 'an under-budget unit still carries a budget field');
  assert.equal(byIdent['phase-2-b'].budget.hit, false, 'reporting no hit');
  assert.equal(byIdent['big-thing'].budget, null, 'a unit whose pipeline reported no budget carries null, never undefined');
  const hitLine = h.logs.find((l) => l.indexOf('phase/phase-1-a') !== -1);
  const cleanLine = h.logs.find((l) => l.indexOf('phase/phase-2-b') !== -1);
  assert.ok(hitLine && hitLine.indexOf('[review budget hit: 13 produced, 5 graded, 8 ungraded]') !== -1,
    'the budget-hit unit is VISIBLY distinguishable in its log line');
  assert.ok(cleanLine && cleanLine.indexOf('review budget hit') === -1,
    'an under-budget unit logs no clause — a bounded run can never read as complete coverage, nor the reverse');
}

// (5d) the single-target flatten carries `budget` onto the top level too.
{
  const h = makeHarness({}, { 'fetch:task': { body: 'TB', tags: ['needs-plan-review'] } }, { 'TB': HIT_BUDGET });
  const res = await runPlanReviewDriver({ task: 'fix-bug' }, h.deps);
  assert.ok(res.budget, 'a flattened single-target result carries the unit budget on the top level');
  assert.equal(res.budget.hit, true, 'and it reports the hit');
  assert.ok(res.units[0].budget && res.units[0].budget.hit === true, 'and the unit itself carries it too');
}

// (5e) the --implementation-plan branch threads and reports the budget too,
//      even though it has no persisted item to gate.
{
  const h = makeHarness({ 'PLAN TEXT': blockingCoherence }, {}, { 'PLAN TEXT': HIT_BUDGET });
  const res = await runPlanReviewDriver(
    { implementationPlan: true, planText: 'PLAN TEXT here', maxRefutations: 3 },
    h.deps
  );
  assert.equal(h.reviewCtxs.length, 1, 'the impl-plan branch runs exactly one review');
  assert.equal(h.reviewCtxs[0].maxRefutations, 3, 'the impl-plan branch threads the resolved budget');
  assert.ok(res.budget, 'the impl-plan result carries the budget');
  assert.equal(res.budget.hit, true, 'and it reports the hit');
  const line = h.logs.find((l) => l.indexOf('implementation-plan') !== -1);
  assert.ok(line && line.indexOf('[review budget hit: 13 produced, 5 graded, 8 ungraded]') !== -1,
    'the impl-plan log line carries the clause on a hit');
}
{
  const h = makeHarness({ 'PLAN TEXT': blockingCoherence }, {}, {});
  const res = await runPlanReviewDriver({ implementationPlan: true, planText: 'PLAN TEXT here' }, h.deps);
  assert.equal(res.budget, null, 'a budget-less impl-plan run reports null, never undefined');
  const line = h.logs.find((l) => l.indexOf('implementation-plan') !== -1);
  assert.ok(line && line.indexOf('review budget hit') === -1, 'and logs a byte-unchanged line');
}

console.log('plan-review driver execution assertions passed');
NODE_DRIVER_TEST
if run_node "$TMP/plan-driver-test.mjs" "$PLAN_LIB"; then
    pass "plan-review driver executes correctly: arg precedence, fail-closed, per-unit gate, flatten, impl-plan carve-out"
else
    fail "plan-review driver execution assertions failed"
fi

# --- 5b-mut. PLANTED-MUTATION SELF-TESTS FOR THE PLAN-REVIEW DRIVER -----------
# Section 5b-exec's budget assertions (5a–5e) are only worth having if they
# actually fire. Four independent mutations of the DRIVER — each one a regression
# a maintainer could plausibly introduce while editing the budget threading —
# must flip a 5b-exec assertion, plus a control run against the real file that
# must pass. The mutated copy needs its sibling review.mjs alongside it, since
# lib/plan-review.mjs imports the core from './review.mjs'.
say "5b-mut. plan-review driver budget-threading mutation self-tests (prove 5b-exec is not vacuous)"
PMUT="$TMP/plan-mut/.claude/workflows/lib"
mkdir -p "$PMUT"

reset_pmut() {
    cp "$PLAN_LIB" "$PMUT/plan-review.mjs"
    cp "$LIB" "$PMUT/review.mjs"
}

reset_pmut
if run_node "$TMP/plan-driver-test.mjs" "$PMUT/plan-review.mjs" >/dev/null 2>&1; then
    pass "5b-mut(control): 5b-exec passes against an unmutated copy — the mutations below are discriminating"
else
    fail "5b-mut(control): 5b-exec FAILED against an unmutated copy — the mutation self-tests would be meaningless"
fi

plan_mutate_and_expect_fail() {
    label="$1"
    desc="$2"
    reset_pmut
    shift 2
    "$@" || fail "5b-mut($label): mutation setup failed"
    if run_node "$TMP/plan-driver-test.mjs" "$PMUT/plan-review.mjs" >/dev/null 2>&1; then
        fail "5b-mut($label): $desc did NOT flip a 5b-exec assertion — the check is vacuous"
    fi
    pass "5b-mut($label): $desc flips a 5b-exec assertion"
}

# (i) Stop threading maxRefutations into the per-unit review context: every unit
#     silently falls back to the pipeline default and a caller override is lost.
pmut_thread_unit() {
    perl -0pi -e "s/(target: unit\.target,\n)(\s*)maxRefutations: maxRefutations,\n/\$1/" "$PMUT/plan-review.mjs"
    ! grep -A2 'target: unit.target,' "$PMUT/plan-review.mjs" | grep -q 'maxRefutations'
}
plan_mutate_and_expect_fail i 'dropping the maxRefutations thread into reviewUnit' pmut_thread_unit

# (ii) Same, on the --implementation-plan branch.
pmut_thread_impl() {
    perl -0pi -e "s/(target: planText,\n)(\s*)maxRefutations: maxRefutations,\n/\$1/" "$PMUT/plan-review.mjs"
    ! grep -A2 'target: planText,' "$PMUT/plan-review.mjs" | grep -q 'maxRefutations'
}
plan_mutate_and_expect_fail ii 'dropping the maxRefutations thread into the implementation-plan branch' pmut_thread_impl

# (iii) Drop the `budget` field from the reported per-unit result: a consumer can
#       no longer see the bound was hit on any unit.
pmut_result_field() {
    perl -0pi -e "s/\n(\s*)budget: r\.budget \|\| null,//" "$PMUT/plan-review.mjs"
    ! grep -q 'budget: r.budget || null,' "$PMUT/plan-review.mjs"
}
plan_mutate_and_expect_fail iii 'dropping the budget field from the per-unit result' pmut_result_field

# (iv) Make formatUnitBudget always return '': the per-unit log line reads as
#      complete coverage even when the unit passed findings over ungraded.
pmut_format() {
    perl -0pi -e "s/  if \(!budget \|\| budget\.hit !== true\) return ''/  if (true) return '' \/\/ MUTANT/" \
        "$PMUT/plan-review.mjs"
    grep -q 'MUTANT' "$PMUT/plan-review.mjs"
}
plan_mutate_and_expect_fail iv 'silencing formatUnitBudget so a budget hit never shows in the log' pmut_format

pass "5b-mut: all four driver mutations flip a 5b-exec assertion, and the control passes"

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

# --- 5d. ROUND-CAPPING (review-gate-intent phase 6, AC4) ---------------------
# Drive runPlanReviewDriver three times against a STATEFUL fake agent whose
# fetch:<kind> call returns the SAME plan body plus whatever round-audit-note
# the previous invocation's round-note write appended — simulating the body a
# real plan repo would hand back across three separate driver invocations —
# with a finder that keeps re-reporting the identical blocking finding every
# round. Pins down the coherence-1 fix: round 2's OUTCOME must still be
# non-`reviewed` (not silently pass) even though the finding is a repeat in the
# note. A separate case proves a wont-fixed finding is dropped from BOTH the
# report and the outcome on round 1.
say "5d. round-capping: persistent finding stays non-reviewed through round 2, escalates on round 3, wont-fix suppresses"
cat >"$TMP/plan-round-cap-test.mjs" <<'NODE_ROUND_CAP_TEST'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const mod = await import(pathToFileURL(process.argv[2]).href);
const { runPlanReviewDriver, classifyRoundOutcome } = mod;

// ---- the cap is an anti-loop valve, not a penalty for needing three rounds ----
// Round 3+ escalates only when findings are STILL unresolved. A plan actually
// fixed on the third pass must pass, or the cap would hand a human a plan with
// nothing left to decide. Asserted directly on the capper so the rule is pinned
// independently of the driver's state threading.
{
  const blocking = [{ id: 'b1', severity: 'blocking', confidence: 90 }];
  assert.equal(classifyRoundOutcome(1, []), 'reviewed', 'round 1, no findings: reviewed');
  assert.equal(classifyRoundOutcome(2, []), 'reviewed', 'round 2, no findings: reviewed');
  assert.equal(
    classifyRoundOutcome(3, []),
    'reviewed',
    'round 3 with an EMPTY survivor list must pass — the cap must not escalate a plan that was actually fixed'
  );
  assert.equal(
    classifyRoundOutcome(4, []),
    'reviewed',
    'the same holds past the cap: a clean survivor list is clean on any round'
  );
  assert.equal(
    classifyRoundOutcome(3, blocking),
    'escalated',
    'round 3 with an unresolved blocking finding still escalates — the anti-loop valve is intact'
  );
  assert.notEqual(
    classifyRoundOutcome(2, blocking),
    'escalated',
    'the escalation is the cap firing, not the finding alone'
  );
}

// makeStatefulHarness — a fake agent that actually threads body state across
// calls: fetch:<kind> returns the current stored body; act:round-note:* mutates
// it by appending the round-note block the prompt asked it to write (extracted
// from the prompt text itself, mirroring what a real agent would do with the
// same instructions).
function makeStatefulHarness(initialBody, tags, findings, wontfixTexts) {
  let body = initialBody;
  const calls = [];
  const agent = async (prompt, opts) => {
    const label = (opts && opts.label) || '';
    calls.push({ label, prompt });
    if (label.indexOf('fetch:wontfix') === 0) return { texts: wontfixTexts || [] };
    if (label.indexOf('fetch:') === 0) return { body, tags };
    if (label.indexOf('act:round-note:') === 0) {
      const m = /2\. Append exactly this block[^\n]*\n\n([\s\S]*?)\n\n3\. Write/.exec(prompt);
      assert.ok(m, 'round-note prompt must contain the appendable block between markers');
      body = body + '\n\n' + m[1];
      return { ok: true };
    }
    // act:<kind>:<ident> (small-fix / large-finding step) and gate:clear-tag:*.
    return { ok: true };
  };
  const parallel = (thunks) => Promise.all(thunks.map((t) => t()));
  // runPlanReview resolves { survivors, acTable } — acTable is always null in
  // plan mode.
  const runPlanReview = async () => ({ survivors: findings, acTable: null });
  const log = () => {};
  return { deps: { agent, parallel, runPlanReview, log }, calls, getBody: () => body };
}

const persistent = {
  id: 'f1',
  concern: 'coherence',
  severity: 'blocking',
  confidence: 90,
  what_fails: 'the retry backoff strategy is unspecified and would change behavior significantly',
};

// ---- Rounds 1-3 on an unchanged item with a persistently-reported finding ---
{
  const h = makeStatefulHarness('ORIGINAL BODY', ['needs-plan-review'], [persistent], []);

  // Round 1: no prior round note in the body -> round 1 -> classifyPlanOutcome
  // over the full survivor set -> 'rework' (a non-architectural blocking finding).
  const r1 = await runPlanReviewDriver({ task: 'flaky-thing' }, h.deps);
  assert.equal(r1.outcome, 'rework', 'round 1: outcome reflects the unresolved blocking finding');
  assert.ok(r1.findings.some((f) => f.id === 'f1'), 'round 1: the finding is present in the report');
  assert.equal(r1.units[0].round, 1, 'round 1: round number is 1');
  assert.ok(h.getBody().indexOf('## Plan Review Round 1 — rework') !== -1, 'round 1: audit note appended to the body');
  assert.equal(
    h.calls.filter((c) => c.label.indexOf('gate:clear-tag:') === 0).length,
    0,
    'round 1: non-reviewed outcome must not clear the tag'
  );

  // Round 2: the SAME finding is reported again against the body now carrying
  // round 1's note. This is the coherence-1 regression pin: the outcome must
  // STILL be non-reviewed (matching round 1), even though the finding is a
  // REPEAT and therefore excluded from the round's newly-reported subset.
  const r2 = await runPlanReviewDriver({ task: 'flaky-thing' }, h.deps);
  assert.notEqual(r2.outcome, 'reviewed', 'round 2: an unresolved repeat must NOT silently pass');
  assert.equal(r2.outcome, 'rework', 'round 2: outcome matches round 1 exactly (still classified from the full set)');
  assert.equal(r2.units[0].round, 2, 'round 2: round number advances to 2');
  assert.ok(r2.findings.some((f) => f.id === 'f1'), 'round 2: the finding is STILL present in the full report (not dropped)');
  assert.equal(r2.units[0].newlyReported.length, 0, 'round 2: the repeat is excluded from newly-reported (reporting-only)');
  assert.equal(r2.units[0].repeats.length, 1, 'round 2: the repeat is recorded as a repeat');
  assert.ok(h.getBody().indexOf('## Plan Review Round 2 — rework') !== -1, 'round 2: a second audit note is appended');

  // Round 3: the cap fires because the finding is STILL unresolved, and the
  // still-open finding remains listed (deduped, not dropped).
  const r3 = await runPlanReviewDriver({ task: 'flaky-thing' }, h.deps);
  assert.equal(r3.outcome, 'escalated', 'round 3: an unresolved finding escalates once the cap is reached');
  assert.equal(r3.units[0].round, 3, 'round 3: round number advances to 3');
  assert.ok(r3.findings.some((f) => f.id === 'f1'), 'round 3: the still-open finding is still listed, not silently dropped');
  assert.equal(r3.findings.length, 1, 'round 3: findings are deduped, not accumulated across rounds');
  assert.ok(h.getBody().indexOf('## Plan Review Round 3 — escalated') !== -1, 'round 3: a third audit note is appended');
}

// ---- wont-fix suppression: dropped from BOTH the report and the outcome ----
{
  const wontfixed = {
    id: 'wf1',
    concern: 'coherence',
    severity: 'blocking',
    confidence: 88,
    what_fails: 'the retry backoff timing is unspecified',
  };
  const h = makeStatefulHarness('TB', ['needs-plan-review'], [wontfixed], [
    'Task closed wont-fix: retry backoff timing already covered elsewhere',
  ]);
  const res = await runPlanReviewDriver({ task: 'wf-target' }, h.deps);
  assert.equal(res.outcome, 'reviewed', 'a wont-fixed finding must not hold the outcome open');
  assert.equal(res.findings.length, 0, 'a wont-fixed finding must be dropped from the report');
  assert.equal(res.units[0].tagCleared, true, 'reviewed (post-suppression) clears the needs-plan-review tag');
  assert.equal(
    h.calls.filter((c) => c.label.indexOf('act:round-note:') === 0).length,
    0,
    'a reviewed outcome (post-suppression) never writes a round note'
  );
}

console.log('round-capping assertions passed');
NODE_ROUND_CAP_TEST
if run_node "$TMP/plan-round-cap-test.mjs" "$PLAN_LIB"; then
    pass "round-capping: persistent finding stays non-reviewed through round 2, escalates on round 3, wont-fix suppresses"
else
    fail "round-capping assertions failed"
fi

# --- 6. PLAN HELPER MUTATION SELF-TESTS (non-vacuity) ------------------------
# Prove the AC-1 phase-scoping and AC-2 tag-filter checks are not vacuous: on a
# scratch copy of the lib, break each helper to a pass-through and require the
# corresponding assertion to THROW; then heal by restoring. Mirrors section 4's
# SCRATCH-only isolation — never touches $LIB.
say "6. Plan helper + driver mutation self-tests (prove the AC-1/AC-2 + driver-exec checks catch a regression)"
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

# (c) parsePlanArgs precedence -> corrupted: swapping the task/roadmap+phase
#     precedence order must make the 5b-exec precedence assertion FAIL — proving
#     the driver-execution harness is non-vacuous. lib/plan-review.mjs imports
#     ./review.mjs, so the scratch tree carries a pristine review.mjs beside the
#     mutated driver.
cp "$LIB" "$PLANMUT/.claude/workflows/lib/review.mjs"
# Move the `else if (roadmap && phase)` branch ABOVE the `else if (task)` branch by
# corrupting the task guard to never fire, so `{task,roadmap,phase}` resolves to
# 'phase' instead of 'task'.
sed 's/  else if (task) kind = .task./  else if (task \&\& false) kind = '"'"'task'"'"' \/\/ MUTANT/' \
    "$PLAN_LIB" >"$PLANMUT/.claude/workflows/lib/plan-review.mjs"
grep -q 'MUTANT' "$PLANMUT/.claude/workflows/lib/plan-review.mjs" ||
    fail "parsePlanArgs precedence mutation setup did not inject the corruption"

cat >"$TMP/plan-args-mut-test.mjs" <<'NODE_ARGS_MUT'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';
const mod = await import(pathToFileURL(process.argv[2]).href); // must still import
assert.throws(
  () => assert.equal(mod.parsePlanArgs({ task: 't', roadmap: 'r', phase: 'p' }).kind, 'task'),
  'a corrupted task-precedence must FAIL the "task outranks roadmap+phase" check — else it is vacuous'
);
console.log('parsePlanArgs precedence mutation self-test passed');
NODE_ARGS_MUT
run_node "$TMP/plan-args-mut-test.mjs" "$PLANMUT/.claude/workflows/lib/plan-review.mjs" ||
    fail "parsePlanArgs precedence mutation self-test did not behave as expected"

pass "plan helper + driver checks fire on planted mutations (proven non-vacuous)"

# --- 7. PLAN-REVIEW HOIST (workflow-token-reduction phase 3) ------------------
# `fetch:roadmap` / `fetch:<kind>` are the ONE mechanical hoist in this phase
# that is not a pure cost question: they have twice transcribed junk over real
# plan tags in production (runs wf_e3402021-0af and wf_f4be8027-dbb, recorded on
# task fix-plan-review-gate-tag-clobber), and `agent(..., { schema })` provably
# cannot catch it — BOTH corrupt returns were schema-valid. So this section
# asserts the hoisted path carries the target's REAL FIELD VALUES read from the
# real `./target/debug/rdm` binary, and then replays the recorded corruption
# payload as a NEGATIVE, proving a shape-only/schema check would have passed it.
#
# Driver-side validation of a hoisted payload is deliberately NOT this phase's
# job (it is task fix-plan-review-gate-tag-clobber's) — this section gates the
# hoist itself: that the shim's real values survive intact through
# buildReviewUnits, filterPlanReviewTag and the gate prompt.
say "7. Plan-review hoist: REAL field values from the real binary, plus the recorded-corruption negative"

RDM_BIN="$REPO_ROOT/target/debug/rdm"
if [ ! -x "$RDM_BIN" ]; then
    fail "7: $RDM_BIN not found — run \`cargo build\` first (this section drives the REAL binary)"
fi

# Hermetic seed: a temp git-backed plan repo with a task carrying known tags and
# a roadmap with two differently-tagged phases. Modelled on
# scripts/verify-workflow-estimate.sh's real-binary seed.
SEED_ROOT="$TMP/plan-seed"
SEED_PROJ="plan-hoist-verify"
mkdir -p "$SEED_ROOT"
seed_rdm() { "$RDM_BIN" --root "$SEED_ROOT" "$@"; }

seed_rdm init --default-project "$SEED_PROJ" >/dev/null 2>&1 || fail "7: seed rdm init failed"
seed_rdm task create hoist-target --title "Hoist target" \
    --body "A task body the harness can compare against verbatim." \
    --tags "needs-plan-review,bug,auth" --no-edit --project "$SEED_PROJ" >/dev/null 2>&1 ||
    fail "7: seed task create failed"
seed_rdm roadmap create hoist-rm --title "Hoist roadmap" \
    --body "Roadmap body." --tags "needs-plan-review,infra" --no-edit --project "$SEED_PROJ" >/dev/null 2>&1 ||
    fail "7: seed roadmap create failed"
seed_rdm phase create alpha --title "Alpha" --number 1 --body "Phase alpha body." \
    --tags "needs-plan-review,alpha-tag" --no-edit --roadmap hoist-rm --project "$SEED_PROJ" >/dev/null 2>&1 ||
    fail "7: seed phase 1 create failed"
seed_rdm phase create beta --title "Beta" --number 2 --body "Phase beta body." \
    --tags "needs-plan-review,beta-tag" --no-edit --roadmap hoist-rm --project "$SEED_PROJ" >/dev/null 2>&1 ||
    fail "7: seed phase 2 create failed"
seed_rdm commit -m "chore(plan): seed" >/dev/null 2>&1 || fail "7: seed commit failed"

seed_rdm task show hoist-target --project "$SEED_PROJ" --format json 2>/dev/null >"$TMP/seed-task.json" ||
    fail "7: seed task show --format json failed"
seed_rdm roadmap show hoist-rm --project "$SEED_PROJ" --format json 2>/dev/null >"$TMP/seed-roadmap.json" ||
    fail "7: seed roadmap show --format json failed"
seed_rdm phase show phase-1-alpha --roadmap hoist-rm --project "$SEED_PROJ" --format json 2>/dev/null >"$TMP/seed-phase-1.json" ||
    fail "7: seed phase 1 show --format json failed"
seed_rdm phase show phase-2-beta --roadmap hoist-rm --project "$SEED_PROJ" --format json 2>/dev/null >"$TMP/seed-phase-2.json" ||
    fail "7: seed phase 2 show --format json failed"
pass "7: hermetic plan repo seeded with the REAL binary (task + roadmap + 2 phases, distinct tags)"

cat >"$TMP/plan-hoist.mjs" <<'NODE_PLAN_HOIST'
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const [libPath, seedDir] = process.argv.slice(2);
const { runPlanReviewDriver, parsePlanArgs, hoistedFetchedOk } = await import('file://' + libPath);

const readJson = (f) => JSON.parse(fs.readFileSync(path.join(seedDir, f), 'utf8'));
const taskJson = readJson('seed-task.json');
const roadmapJson = readJson('seed-roadmap.json');
const phase1Json = readJson('seed-phase-1.json');
const phase2Json = readJson('seed-phase-2.json');

// The payload the local rdm-plan-review shim is instructed to assemble: the
// binary's OWN body/tags copied verbatim, never summarized.
const TASK_FETCHED = { body: taskJson.body, tags: taskJson.tags };
const ROADMAP_FETCHED = {
  body: roadmapJson.body,
  tags: roadmapJson.tags,
  phases: [phase1Json, phase2Json].map((p) => ({ stem: p.stem, body: p.body, tags: p.tags })),
};

// Sanity: the seed really does carry the tags this section asserts on, so a
// green run can never be an artefact of the binary emitting nothing.
assert.deepEqual(taskJson.tags, ['needs-plan-review', 'bug', 'auth'], 'seed task tags are as created');
assert.deepEqual(phase1Json.tags, ['needs-plan-review', 'alpha-tag'], 'seed phase-1 tags are as created');
assert.deepEqual(phase2Json.tags, ['needs-plan-review', 'beta-tag'], 'seed phase-2 tags are as created');

function makeDeps(o) {
  o = o || {};
  const calls = [];
  const agent = async (prompt, opts) => {
    calls.push({ label: (opts && opts.label) || '', prompt, opts });
    return { ok: true };
  };
  const parallel = async (thunks) => Promise.all(thunks.map((t) => Promise.resolve().then(t)));
  return {
    calls,
    deps: {
      agent,
      parallel,
      log: () => {},
      // A clean review, so every unit reaches `reviewed` and the gate fires.
      runPlanReview: async () => ({ survivors: o.survivors || [], acTable: null }),
      mechanicalModel: o.mechanicalModel,
    },
  };
}
const labels = (h) => h.calls.map((c) => c.label);
const promptFor = (h, label) => (h.calls.find((c) => c.label === label) || {}).prompt;

// ============================================================================
// 7a. TASK path — REAL values, not merely a schema-valid shape.
// ============================================================================
{
  const h = makeDeps({});
  const out = await runPlanReviewDriver(
    { task: 'hoist-target', fetched: TASK_FETCHED, wontFixedTexts: [], mechanicalModel: 'haiku' },
    h.deps
  );
  assert.ok(!labels(h).some((l) => l.startsWith('fetch:')), '7a: no fetch: agent ran on the hoisted task path');
  assert.equal(out.units.length, 1, '7a: exactly one unit');
  assert.deepEqual(out.units[0].ident, 'hoist-target', '7a: the unit is keyed by the real slug');
  assert.equal(out.outcome, 'reviewed', '7a: a clean review reaches reviewed and gates');

  // The REAL-VALUE assertion: the gate's tag write must carry exactly the
  // sibling-preserved list read off the binary, with needs-plan-review removed
  // and NOTHING invented.
  const gatePrompt = promptFor(h, 'gate:clear-tag:task:hoist-target');
  assert.ok(gatePrompt, '7a: the gate:clear-tag agent ran');
  assert.ok(gatePrompt.includes('--tags "bug,auth"'), '7a: the gate writes exactly the sibling-preserved real tags: --tags "bug,auth"');
  for (const invented of ['plan-target', 'fetch', 'roadmap', 'hoist-target"']) {
    assert.ok(!gatePrompt.includes('--tags "' + invented), '7a: the gate never writes an invented tag list starting with ' + invented);
  }
}
console.log('7a OK: task hoist carries the binary\'s real tags into the gate');

// ============================================================================
// 7b. ROADMAP path — one unit per REAL phase, with each phase's own real tags.
// ============================================================================
{
  const h = makeDeps({});
  const out = await runPlanReviewDriver({ roadmap: 'hoist-rm', fetched: ROADMAP_FETCHED, wontFixedTexts: [] }, h.deps);
  assert.ok(!labels(h).some((l) => l.startsWith('fetch:')), '7b: no fetch: agent ran on the hoisted roadmap path');
  assert.equal(out.units.length, 3, '7b: one unit for the roadmap body plus one per REAL phase (2)');
  assert.deepEqual(
    out.units.map((u) => u.ident),
    ['hoist-rm', phase1Json.stem, phase2Json.stem],
    '7b: the units carry the REAL phase stems from the binary, not the roadmap slug repeated'
  );
  const p1 = promptFor(h, 'gate:clear-tag:phase:' + phase1Json.stem);
  const p2 = promptFor(h, 'gate:clear-tag:phase:' + phase2Json.stem);
  const rm = promptFor(h, 'gate:clear-tag:roadmap:hoist-rm');
  assert.ok(p1.includes('--tags "alpha-tag"'), '7b: phase 1 gate writes its OWN real sibling tag');
  assert.ok(p2.includes('--tags "beta-tag"'), '7b: phase 2 gate writes its OWN real sibling tag');
  assert.ok(rm.includes('--tags "infra"'), '7b: the roadmap gate writes the roadmap\'s OWN real sibling tag');
  assert.ok(!p1.includes('beta-tag') && !p2.includes('alpha-tag'), '7b: no phase inherits a sibling phase\'s tags');
}
console.log('7b OK: roadmap hoist yields one unit per real phase, each with its own real tags');

// ============================================================================
// 7c. NEGATIVE — the recorded wf_e3402021-0af corruption payload. It is
// SCHEMA-VALID (a string body, an array of strings for tags, an array of
// well-shaped phase objects), so a shape-only check passes it. The REAL-VALUE
// assertions above must fail on it.
// ============================================================================
const CORRUPTION = {
  body: 'Fetched roadmap and phase data for workflow-token-reduction',
  tags: ['fetch', 'roadmap', 'workflow-token-reduction'],
  phases: [{ stem: 'workflow-token-reduction', body: roadmapJson.body, tags: roadmapJson.tags }],
};
{
  // Shape-only / schema-shaped check: PASSES. This is the false assurance.
  const shapeOk =
    typeof CORRUPTION.body === 'string' &&
    Array.isArray(CORRUPTION.tags) &&
    CORRUPTION.tags.every((t) => typeof t === 'string') &&
    Array.isArray(CORRUPTION.phases) &&
    CORRUPTION.phases.every((p) => typeof p.stem === 'string' && typeof p.body === 'string' && Array.isArray(p.tags));
  assert.equal(shapeOk, true, '7c: the recorded corruption payload IS schema-valid — a shape-only check passes it');
  assert.equal(hoistedFetchedOk(CORRUPTION, 'roadmap'), true, '7c: even the driver\'s own shape guard accepts it (shape is not the defence)');

  const h = makeDeps({});
  const out = await runPlanReviewDriver({ roadmap: 'hoist-rm', fetched: CORRUPTION, wontFixedTexts: [] }, h.deps);

  // ... and every REAL-VALUE assertion fails on it.
  assert.notEqual(out.units.length, 3, '7c: the corruption payload does NOT produce one unit per real phase (five of six vanished, in the real incident)');
  assert.notDeepEqual(
    out.units.map((u) => u.ident),
    ['hoist-rm', phase1Json.stem, phase2Json.stem],
    '7c: the corruption payload does NOT carry the real phase stems'
  );
  const rm = promptFor(h, 'gate:clear-tag:roadmap:hoist-rm');
  assert.ok(!rm.includes('--tags "infra"'), '7c: the corruption payload does NOT write the roadmap\'s real sibling tags');
  assert.ok(
    rm.includes('fetch') || rm.includes('workflow-token-reduction'),
    '7c: it writes the junk transcribed from the agent\'s own prompt instead — exactly the recorded incident'
  );
}
console.log('7c OK: the recorded corruption payload passes a shape-only check and FAILS every real-value assertion');

// ============================================================================
// 7d. FALLBACK — every hoist is optional. Absent / malformed reaches the agent.
// ============================================================================
{
  const h = makeDeps({});
  await runPlanReviewDriver({ task: 'hoist-target' }, h.deps).catch(() => {});
  assert.equal(labels(h).filter((l) => l === 'fetch:task').length, 1, '7d: fetched absent -> exactly one fetch:task agent call');
  assert.equal(labels(h).filter((l) => l === 'fetch:wontfix').length, 0, '7d: the fetch failed closed before the wont-fix search (fail-closed preserved)');
}
for (const [name, bad] of [
  ['null', null],
  ['string', 'hoist-target'],
  ['empty body', { body: '', tags: [] }],
  ['whitespace body', { body: '   ', tags: [] }],
  ['no body key', { tags: ['bug'] }],
  // `tags` is WRITTEN BACK by the gate (`--tags` replaces the whole list), so a
  // payload that omits or malforms it must reach the schema-enforced agent
  // rather than defaulting to [] and clobbering every real tag the item carries.
  ['no tags key', { body: taskJson.body }],
  ['non-array tags', { body: taskJson.body, tags: 'bug' }],
  ['null tags', { body: taskJson.body, tags: null }],
  ['non-string tag entry', { body: taskJson.body, tags: ['bug', 7] }],
]) {
  const h = makeDeps({});
  await runPlanReviewDriver({ task: 'hoist-target', fetched: bad }, h.deps).catch(() => {});
  assert.equal(labels(h).filter((l) => l === 'fetch:task').length, 1, '7d: malformed fetched (' + name + ') falls back to the fetch agent');
}
{
  // The consequence the tags requirement exists to prevent: a hoisted payload
  // with a REAL body but no tags must never reach a gate write at all — an
  // accepted-then-defaulted [] would be issued as `--tags ""`, replacing the
  // item's whole tag list. Assert on the write, not just on the fallback count.
  const h = makeDeps({});
  await runPlanReviewDriver({ task: 'hoist-target', fetched: { body: taskJson.body } }, h.deps).catch(() => {});
  const gate = promptFor(h, 'gate:clear-tag:task:hoist-target');
  assert.equal(gate, undefined, '7d: a tags-less hoisted payload never reaches the gate — no tag write at all');
  assert.ok(
    !labels(h).some((l) => l.startsWith('gate:clear-tag')),
    '7d: ... so no `--tags ""` clobber of the real list can be issued'
  );
}
for (const [name, bad] of [
  ['no phases array', { body: 'b', tags: [] }],
  ['phase entry missing tags', { body: 'b', tags: [], phases: [{ stem: 'phase-1-alpha', body: 'x' }] }],
  ['phase entry non-array tags', { body: 'b', tags: [], phases: [{ stem: 'phase-1-alpha', body: 'x', tags: 'alpha' }] }],
  ['phase entry missing stem', { body: 'b', tags: [], phases: [{ body: 'x', tags: ['alpha'] }] }],
  ['phase entry blank stem', { body: 'b', tags: [], phases: [{ stem: '  ', body: 'x', tags: ['alpha'] }] }],
  ['phase entry missing body', { body: 'b', tags: [], phases: [{ stem: 'phase-1-alpha', tags: ['alpha'] }] }],
  ['phase entry not an object', { body: 'b', tags: [], phases: ['phase-1-alpha'] }],
  ['roadmap no tags key', { body: 'b', phases: [] }],
]) {
  const h = makeDeps({});
  await runPlanReviewDriver({ roadmap: 'hoist-rm', fetched: bad }, h.deps).catch(() => {});
  assert.equal(
    labels(h).filter((l) => l === 'fetch:roadmap').length,
    1,
    '7d: malformed roadmap fetched (' + name + ') falls back to the fetch agent'
  );
  assert.ok(
    !labels(h).some((l) => l.startsWith('gate:clear-tag')),
    '7d: malformed roadmap fetched (' + name + ') writes no tags'
  );
}
{
  // wontFixedTexts hoist + fallback.
  const h = makeDeps({});
  await runPlanReviewDriver({ task: 'hoist-target', fetched: TASK_FETCHED, wontFixedTexts: ['already decided'] }, h.deps);
  assert.equal(labels(h).filter((l) => l === 'fetch:wontfix').length, 0, '7d: hoisted wontFixedTexts -> zero fetch:wontfix calls');
  const b = makeDeps({});
  await runPlanReviewDriver({ task: 'hoist-target', fetched: TASK_FETCHED }, b.deps);
  assert.equal(labels(b).filter((l) => l === 'fetch:wontfix').length, 1, '7d: wontFixedTexts absent -> exactly one fetch:wontfix call');
  const c = makeDeps({});
  await runPlanReviewDriver({ task: 'hoist-target', fetched: TASK_FETCHED, wontFixedTexts: 'nope' }, c.deps);
  assert.equal(labels(c).filter((l) => l === 'fetch:wontfix').length, 1, '7d: a non-array wontFixedTexts falls back to the agent');
}
{
  // mechanicalModel hoist: parsePlanArgs surfaces it and it pins the mechanical
  // agents, overriding an absent injected dep.
  const parsed = parsePlanArgs({ task: 'hoist-target', mechanicalModel: '  haiku-x  ' });
  assert.equal(parsed.mechanicalModel, 'haiku-x', '7d: parsePlanArgs trims and surfaces mechanicalModel');
  assert.equal(parsePlanArgs({ task: 't', mechanicalModel: '   ' }).mechanicalModel, null, '7d: a blank mechanicalModel is rejected');
  const h = makeDeps({});
  await runPlanReviewDriver({ task: 'hoist-target', fetched: TASK_FETCHED, mechanicalModel: 'haiku-x' }, h.deps);
  const gate = h.calls.find((c) => c.label === 'gate:clear-tag:task:hoist-target');
  assert.equal(gate.opts.model, 'haiku-x', '7d: the hoisted mechanicalModel pins the mechanical gate agent');
}
{
  // `fetched` must come from STRUCTURED object keys only — never parsed out of
  // the $ARGUMENTS flag string, or a prose target could masquerade as a payload.
  const parsed = parsePlanArgs('--task hoist-target');
  assert.equal(parsed.fetched, null, '7d: a raw $ARGUMENTS string never yields a fetched payload');
  assert.equal(parsed.wontFixedTexts, null, '7d: nor a wontFixedTexts payload');
  assert.equal(parsed.mechanicalModel, null, '7d: nor a mechanicalModel');
  assert.equal(parsed.kind, 'task', '7d: ... while the pre-existing flag-string parsing is unchanged');
}
console.log('7d OK: every plan-review hoist is optional and falls back on anything malformed');

console.log('ALL PLAN-REVIEW HOIST CHECKS PASSED');
NODE_PLAN_HOIST

if run_node "$TMP/plan-hoist.mjs" "$PLAN_LIB" "$TMP"; then
    pass "plan-review hoist: real binary values survive into the units and the gate; the recorded corruption is caught"
else
    fail "plan-review hoist checks failed against $PLAN_LIB"
fi

# Planted-mutation self-tests on the plan-review hoists.
say "7e. Plan-review hoist planted-mutation self-tests"
assert_plan_mutant_fails() {
    mutant=$1
    desc=$2
    if cmp -s "$PLAN_LIB" "$mutant"; then
        fail "7e: planted mutation was a no-op — $desc"
    fi
    if run_node "$TMP/plan-hoist.mjs" "$mutant" "$TMP" >/dev/null 2>&1; then
        fail "7e: plan-review hoist checks PASSED against a lib that $desc — they are vacuous"
    fi
    pass "7e: assertions fire when the lib $desc"
}

# (1) Drop the fetch fallback: take the (possibly absent) hoist unconditionally.
sed 's/^  if (hoistedFetchedOk(parsed.fetched, kind)) {$/  if (true) {/' "$PLAN_LIB" >"$TMP/plan-mutant-no-fetch-fallback.mjs"
assert_plan_mutant_fails "$TMP/plan-mutant-no-fetch-fallback.mjs" "drops the fetch:roadmap / fetch:<kind> fallback"

# (2) Drop the wont-fix fallback.
sed 's/^  if (Array.isArray(parsed.wontFixedTexts)) {$/  if (true) {/' "$PLAN_LIB" >"$TMP/plan-mutant-no-wontfix-fallback.mjs"
assert_plan_mutant_fails "$TMP/plan-mutant-no-wontfix-fallback.mjs" "drops the fetch:wontfix fallback"

# (3) Let `fetched` be read out of the $ARGUMENTS flag string as well.
sed "s/^  const fetched = a.fetched \&\& typeof a.fetched === 'object' ? a.fetched : null\$/  const fetched = (a.fetched \&\& typeof a.fetched === 'object' ? a.fetched : null) || { body: rawTarget, tags: [] }/" \
    "$PLAN_LIB" >"$TMP/plan-mutant-fetched-from-string.mjs"
assert_plan_mutant_fails "$TMP/plan-mutant-fetched-from-string.mjs" "lets a raw \$ARGUMENTS string masquerade as a fetched payload"

# (4) Weaken the shape guard so an empty body is accepted (defeating fail-closed).
sed "s/^  if (typeof fetched.body !== 'string' || String(fetched.body).trim() === '') return false\$/  if (typeof fetched.body !== 'string') return false/" \
    "$PLAN_LIB" >"$TMP/plan-mutant-weak-fetched-guard.mjs"
assert_plan_mutant_fails "$TMP/plan-mutant-weak-fetched-guard.mjs" "accepts an empty-body hoisted payload"

# (5) Drop the `tags` requirement from the shape guard — the exact weakening that
# lets a tags-less payload through to buildReviewUnits' `[]` default and on into a
# `--tags ""` gate write that replaces the item's whole tag list.
sed "s/^  if (!stringArrayOk(fetched.tags)) return false\$//" \
    "$PLAN_LIB" >"$TMP/plan-mutant-no-tags-requirement.mjs"
assert_plan_mutant_fails "$TMP/plan-mutant-no-tags-requirement.mjs" "drops the hoisted-payload tags requirement"

# (6) Drop the per-phase entry checks on the roadmap path (stem/body/tags), so a
# phase entry with no tags of its own is accepted and gated with an empty list.
sed "s/^    if (!phasesOk) return false\$//" "$PLAN_LIB" >"$TMP/plan-mutant-no-phase-entry-checks.mjs"
assert_plan_mutant_fails "$TMP/plan-mutant-no-phase-entry-checks.mjs" "drops the per-phase-entry shape checks"

# --- 7f. SHIM: the LOCAL rdm-plan-review shim gathers the payload verbatim -----
# `.claude/skills/rdm-plan-review/SKILL.md` is a LOCAL dogfood shim; its
# distributed template (rdm-core/src/templates/skill-plan-review-{cli,mcp}.md) is
# NOT a Workflow shim yet (tracked by task
# convert-remaining-skill-templates-to-workflow-shims), so this check belongs
# here and NOT in verify-agent-config-distribution.sh.
say "7f. rdm-plan-review SKILL.md gathers the target payload itself and passes it as args.fetched"

assert_plan_shim_gathers() {
    doc=$1
    # It must name each of the three reads it performs...
    grep -qF 'task show <slug> --project rdm --format json' "$doc" || return 1
    grep -qF 'phase show <phase> --roadmap <slug> --project rdm --format json' "$doc" || return 1
    grep -qF 'roadmap show <slug> --project rdm --format json' "$doc" || return 1
    # ... pass the payload under the right key, at least twice (the gathering
    # bullet and the never-summarize instruction), ...
    [ "$(grep -cF 'fetched' "$doc")" -ge 2 ] || return 1
    grep -qF 'wontFixedTexts' "$doc" || return 1
    grep -qF 'mechanicalModel' "$doc" || return 1
    # ... and carry the never-summarize instruction naming the recorded failures.
    grep -qiF 'verbatim' "$doc" || return 1
    grep -qF 'wf_e3402021-0af' "$doc" || return 1
    grep -qF 'wf_f4be8027-dbb' "$doc" || return 1
    return 0
}
assert_plan_shim_gathers "$SKILL_MD" ||
    fail "7f: $SKILL_MD must gather task/phase/roadmap 'show --format json' itself, pass fetched/wontFixedTexts/mechanicalModel, and carry the verbatim instruction naming both recorded corruption runs"
pass "7f: the local shim gathers the payload and passes it verbatim, citing both recorded corruption runs"

sed 's/--format json/--format jsn/g' "$SKILL_MD" >"$TMP/plan-shim-typo.md"
if assert_plan_shim_gathers "$TMP/plan-shim-typo.md"; then
    fail "7f: detector missed a typo'd gathering command in the shim"
fi
sed 's/fetched/fetchd/g' "$SKILL_MD" >"$TMP/plan-shim-key-typo.md"
if assert_plan_shim_gathers "$TMP/plan-shim-key-typo.md"; then
    fail "7f: detector missed a typo'd arg key in the shim"
fi
pass "7f: detector fires on a typo'd gathering command AND a typo'd arg key"

# --- 8. NON-GATING REFUTATION SKIP (workflow-token-reduction phase 6) ---------
# A refuter is dispatched only where its verdict could change the outcome. The
# blocker set behind `hasBlocking` is ['blocking'] (['blocking','concern'] at the
# `large` tier), and the AC table is a separate structured channel that never
# reads a finding's severity — so `suggestion` is the ONE severity that can never
# gate, at any tier. This section gates that skip end to end: the pipeline
# dispatches no refuter for it, the pass-through is MARKED so a downstream act
# step can tell reported-only from verified, the confidence floor still applies
# to it, a refuter CRASH is still not marked as a deliberate skip, and both act
# prompts stop asserting "these survived refutation" once the payload is mixed.
say '8. Non-gating refutation skip: no refuter for a suggestion, marked pass-through, honest act prompts'

DISPATCH_LIB="$REPO_ROOT/.claude/workflows/lib/dispatch-phase.mjs"
[ -f "$DISPATCH_LIB" ] || fail "8: dispatch lib not found: $DISPATCH_LIB"

cat >"$TMP/nongating-test.mjs" <<'NODE_NONGATING_TEST'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const [libPath, dispatchPath, planPath] = process.argv.slice(2);
const mod = await import(pathToFileURL(libPath).href);
const dispatchMod = await import(pathToFileURL(dispatchPath).href);
const planMod = await import(pathToFileURL(planPath).href);

const { buildReviewPipeline, NON_GATING_SEVERITIES, needsRefutation, UNREFUTED_DISPOSITION } = mod;
const { buildCodeActPrompt, CODE_ACT_SCHEMA } = dispatchMod;
const { buildActPrompt } = planMod;

// Same reference runtime as section 3: order-preserving, with the documented
// error semantics (a thrown thunk -> null; a thrown stage -> null item).
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
function makeSpyAgent(plantFindings, plantVerdicts) {
  const calls = [];
  async function agent(prompt, opts) {
    const label = (opts && opts.label) || '';
    calls.push({ label, prompt });
    const parts = label.split(':');
    if (parts[0] === 'find') return { findings: plantFindings[parts[2]] || [] };
    if (parts[0] === 'refute') {
      const id = parts.slice(2).join(':');
      return plantVerdicts[id] || { refuted: false, confidence: 90 };
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
// The rule itself: derived from the widest blocker set, and FAIL-SAFE.
// ============================================================================
assert.deepEqual(NON_GATING_SEVERITIES, ['suggestion'], 'only `suggestion` is non-gating at every tier');
assert.equal(needsRefutation({ severity: 'blocking' }), true, 'blocking is refuted');
assert.equal(needsRefutation({ severity: 'concern' }), true, 'concern is refuted (it gates at the large tier)');
assert.equal(needsRefutation({ severity: 'suggestion' }), false, 'suggestion is not refuted');
// Fail-safe: anything not EXPLICITLY listed is still refuted.
assert.equal(needsRefutation({}), true, 'a missing severity is still refuted');
assert.equal(needsRefutation({ severity: 'Suggestion' }), true, 'an off-case severity is still refuted');
assert.equal(needsRefutation({ severity: 'nit' }), true, 'an unknown severity is still refuted');
assert.equal(needsRefutation(null), true, 'a null finding is still refuted');

// ============================================================================
// Pipeline, both modes: gating findings get a distinctly-labelled refuter each,
// the suggestion gets NONE, and the pass-through is marked + floor-filtered.
// ============================================================================
const GATING_AND_NOT = [
  { id: 'b1', severity: 'blocking', confidence: 90, what_fails: 'real bug' },
  { id: 'c1', severity: 'concern', confidence: 90, what_fails: 'real concern' },
  { id: 's1', severity: 'suggestion', confidence: 90, what_fails: 'readability nit' },
  { id: 's2', severity: 'suggestion', confidence: 60, what_fails: 'below-floor nit' },
];

for (const [mode, dimKey] of [['code', 'correctness'], ['plan', 'coherence']]) {
  const spy = makeSpyAgent({ [dimKey]: GATING_AND_NOT }, {});
  const { survivors } = await buildReviewPipeline(mode, deps(spy))(CTX);
  const refuteCalls = spy.calls.filter((c) => c.label.startsWith('refute:'));

  assert.equal(refuteCalls.length, 2, mode + ': exactly one refuter per GATING finding');
  assert.deepEqual(
    refuteCalls.map((c) => c.label).sort(),
    ['refute:' + mode + ':b1', 'refute:' + mode + ':c1'],
    mode + ': the two refuters are distinctly labelled, one per gating finding'
  );
  assert.ok(
    !refuteCalls.some((c) => c.label.includes('s1') || c.label.includes('s2')),
    mode + ': NO refuter is dispatched for a `suggestion`'
  );

  // s2 is dropped by the confidence floor even though no refuter graded it —
  // the floor is not bypassed by the pass-through path.
  assert.deepEqual(survivors.map((f) => f.id), ['b1', 'c1', 's1'], mode + ': below-floor suggestion still dropped');

  const byId = Object.fromEntries(survivors.map((f) => [f.id, f]));
  assert.equal(byId.s1.unrefuted, true, mode + ': the passed-through finding is marked `unrefuted: true`');
  assert.equal(byId.b1.unrefuted, undefined, mode + ': a refuter-graded blocking finding is NOT marked unrefuted');
  assert.equal(byId.c1.unrefuted, undefined, mode + ': a refuter-graded concern finding is NOT marked unrefuted');
}

// A refuter CRASH on a gating finding still keeps the finding (a crash is not
// proof of refutation) but must NOT masquerade as a deliberate skip.
{
  const spy = makeSpyAgent({ correctness: [GATING_AND_NOT[0]] }, {});
  const base = spy.agent;
  spy.agent = async (prompt, opts) => {
    if (opts && opts.label.startsWith('refute:')) throw new Error('boom refuter');
    return base(prompt, opts);
  };
  const { survivors } = await buildReviewPipeline('code', deps(spy))(CTX);
  assert.deepEqual(survivors.map((f) => f.id), ['b1'], 'a crashed refuter keeps its gating finding');
  assert.equal(survivors[0].unrefuted, undefined, 'a crashed refuter does NOT mark the finding `unrefuted`');
}

// ============================================================================
// The disposition rule, single-sourced, and both act prompts consuming it.
// ============================================================================
assert.ok(
  UNREFUTED_DISPOSITION.includes('reported, not verified'),
  'the disposition rule states the findings were reported, not verified'
);
assert.ok(UNREFUTED_DISPOSITION.includes('not major'), 'the disposition rule bounds what may be incorporated');
// A finding that is real but too big to take in flight must have somewhere to
// GO. Without the filing branch the only outlet is a `skipped` reason string
// that is never persisted, which silently loses e.g. a low-severity security
// note the pre-change size branch would have filed as a task.
assert.ok(/\bFILE\b/.test(UNREFUTED_DISPOSITION), 'the disposition rule offers a filing branch, not only skip');
assert.ok(
  UNREFUTED_DISPOSITION.includes('evaporate'),
  'the disposition rule forbids letting a real observation evaporate into a skip reason'
);

const VERIFIED_ONLY = [{ id: 'f1', severity: 'concern', confidence: 90, what_fails: 'x' }];
const MIXED = VERIFIED_ONLY.concat([{ id: 'f2', severity: 'suggestion', confidence: 90, what_fails: 'y', unrefuted: true }]);

// af-2: the LEADING claim is conditional. With a mixed payload neither prompt may
// keep asserting that everything in it survived refutation.
const codeMixed = buildCodeActPrompt('phase', 'rm', 'phase-1-x', 'wt/rm', MIXED);
assert.ok(codeMixed.includes(UNREFUTED_DISPOSITION), 'code act prompt carries the disposition rule verbatim');
assert.ok(
  !codeMixed.includes('These findings survived refutation'),
  'code act prompt drops the unconditional "survived refutation" lead on a mixed payload'
);
assert.ok(!codeMixed.includes('ALREADY-VERIFIED'), 'code act prompt drops the ALREADY-VERIFIED lead on a mixed payload');
assert.ok(codeMixed.includes('skipped'), 'code act prompt tells the agent how to record a skip');

const planMixed = buildActPrompt('phase', 'rm', 'phase-1-x', MIXED);
assert.ok(planMixed.includes(UNREFUTED_DISPOSITION), 'plan act prompt carries the disposition rule verbatim');
assert.ok(
  !planMixed.includes('do not re-review'),
  'plan act prompt drops the flat "do not re-review" directive on a mixed payload'
);
assert.ok(
  !planMixed.includes('already-verified findings'),
  'plan act prompt drops the already-verified lead on a mixed payload'
);

// ... and with NO un-refuted survivor both prompts are byte-identical to the
// pre-change ones, so this change cannot silently perturb the existing lane.
const CODE_ACT_BASELINE = 'You are acting on ALREADY-VERIFIED code-review findings for rm/phase-1-x (worktree: wt/rm).\nThese findings survived refutation and are non-gating (the reviewed outcome is already decided).\n[\n  {\n    "id": "f1",\n    "severity": "concern",\n    "confidence": 90,\n    "what_fails": "x"\n  }\n]\nFor EACH finding, decide SMALL vs LARGE:\n- SMALL — localized, low-risk, no new acceptance criterion (a typo, a missing doc comment, a tightened error message, an extra test). Fix it directly in the worktree at wt/rm and re-run the relevant tests. Do not create a separate landing commit — the fix folds into the eventual land-time commit.\n- LARGE — new modules, cross-cutting changes, or anything that would warrant its own acceptance criterion. Do NOT edit code for these: file it with `./target/debug/rdm task create <slug> --title "Code review finding: <desc>" --body "<details>" --tags code-review --no-edit --project rdm`.\nReturn JSON matching the CODE_ACT schema: a `handled` array with ONE entry per finding you were given — id, action (fixed-inline|filed-as-task), and taskSlug when you filed a task.';
const PLAN_ACT_BASELINE = 'You are the plan-review orchestrator applying already-verified findings. The findings below already\nsurvived independent refutation — do not re-review; act on them.\nFindings (ranked, most-severe first):\n[\n  {\n    "id": "f1",\n    "severity": "concern",\n    "confidence": 90,\n    "what_fails": "x"\n  }\n]\nFor each finding, decide small vs large:\n- SMALL (a localized wording/typo/missing-detail fix to the plan document itself): apply it by reading the\n  current body and writing the ENTIRE modified body back — `--body` is whole-document-authoritative, there\n  is no patch mechanism. Use the matching command:\n    ./target/debug/rdm phase update phase-1-x --roadmap rm --body "<full updated body>" --no-edit --project rdm\n- LARGE (a structural concern: a missing prerequisite, scope too big for one phase, a conflicting design\n  decision): do NOT edit the plan document — file it as a task, with `--no-plan-review` so this finding\n  does not itself get re-stamped `needs-plan-review`:\n    ./target/debug/rdm task create <slug> --title "Plan review finding: <desc>" --body "<details>" --tags plan-review --no-plan-review --no-edit --project rdm\nAfter applying any changes, run: ./target/debug/rdm commit -m "chore(plan): address plan review findings on rm/phase-1-x"\nIf there is nothing small to fix and nothing large to file, make no changes.\nReturn a STAMP_ACK object: { ok: true } if you completed without error (including the no-op case), else { ok: false }.';
assert.equal(
  buildCodeActPrompt('phase', 'rm', 'phase-1-x', 'wt/rm', VERIFIED_ONLY),
  CODE_ACT_BASELINE,
  'an all-verified code act prompt is byte-identical to the pre-change baseline'
);
assert.equal(
  buildActPrompt('phase', 'rm', 'phase-1-x', VERIFIED_ONLY),
  PLAN_ACT_BASELINE,
  'an all-verified plan act prompt is byte-identical to the pre-change baseline'
);

// The schema must be able to RECORD the disposition rule's "skip the rest and
// say why" branch, or the act step has to misreport a skip as something else.
const action = CODE_ACT_SCHEMA.properties.handled.items.properties.action;
assert.ok(action.enum.includes('skipped'), 'CODE_ACT action enum accepts `skipped`');
assert.ok(action.enum.includes('fixed-inline') && action.enum.includes('filed-as-task'), 'the existing actions survive');
assert.equal(
  CODE_ACT_SCHEMA.properties.handled.items.properties.reason.type,
  'string',
  'CODE_ACT carries an optional `reason` for a skip'
);
assert.ok(
  !CODE_ACT_SCHEMA.properties.handled.items.required.includes('reason'),
  '`reason` is optional — a fixed-inline entry must not be forced to carry one'
);
// The rendered skills state the act step's reporting vocabulary in prose. Pin
// that prose to the SCHEMA's enum rather than to a literal, so widening the enum
// without sweeping the prose (exactly what happened when `skipped` was added)
// fails here instead of shipping a skill that contradicts the schema.
assert.equal(
  action.enum.join(' / '),
  'fixed-inline / filed-as-task / skipped',
  'the CODE_ACT action enum must match the vocabulary section 8b greps for in every rendered skill'
);

console.log('8: non-gating refutation skip assertions passed');
NODE_NONGATING_TEST

if run_node "$TMP/nongating-test.mjs" "$LIB" "$DISPATCH_LIB" "$PLAN_LIB"; then
    pass "8: suggestion is passed through un-refuted and marked; gating severities keep their refuter; act prompts stay honest"
else
    fail "8: non-gating refutation skip assertions failed"
fi

# --- 8b. RENDERED SKILL PROSE (whole file, not just the generated region) -----
# The refuter invariant is stated TWICE in every review skill: once inside the
# generated `## Review specification` region, and once in hand-authored prose the
# generator does not own. Grepping only the region would let the hand-authored
# copy keep contradicting the code, so these greps are deliberately whole-file.
say "8b. Every rendered review skill states the pass-through rule, in hand-authored prose too"

REVIEW_DOCS="$TEMPLATES/skill-review-cli.md $TEMPLATES/skill-review-mcp.md \
$TEMPLATES/skill-plan-review-cli.md $TEMPLATES/skill-plan-review-mcp.md \
$REPO_ROOT/.claude/skills/rdm-review/SKILL.md $REPO_ROOT/.claude/skills/rdm-plan-review/SKILL.md"

for doc in $REVIEW_DOCS; do
    [ -f "$doc" ] || fail "8b: expected review skill doc not found: $doc"
    grep -qF 'unrefuted: true' "$doc" ||
        fail "8b: $doc never mentions the \`unrefuted: true\` marker"
    grep -qF 'reported, not verified' "$doc" ||
        fail "8b: $doc does not carry the un-refuted disposition rule"
    # The retired absolutes. Each one is now FALSE for a non-gating finding, so
    # none of them may survive anywhere in the file.
    ! grep -qF 'Findings are never surfaced, fixed, or acted on until a *separate* agent' "$doc" ||
        fail "8b: $doc still claims EVERY finding is refuted before being surfaced"
    ! grep -qF 'no finding is surfaced, fixed, or acted on until a *separate* refuter agent' "$doc" ||
        fail "8b: $doc still claims EVERY finding is refuted before being acted on"
    ! grep -qF 'Never fix or file an unverified finding.' "$doc" ||
        fail "8b: $doc still forbids acting on any un-refuted finding"
    ! grep -qF 'Suggestions may skip refutation (low stakes)' "$doc" ||
        fail "8b: $doc still describes the suggestion skip as an optional low-stakes shortcut"
    # The § Refute lead is a THIRD statement of the same invariant, and a
    # partially-updated doc (§ Act rewritten, § Refute not) contradicts itself
    # rather than merely lagging. Catch that shape too.
    ! grep -qF 'For every finding, dispatch a **separate** read-only refuter' "$doc" ||
        fail "8b: $doc's Refute section still claims EVERY finding gets a refuter, contradicting its own Act section"
    # The act step's reporting vocabulary must name every action the code lane's
    # schema accepts, or a skill reader is told to skip a finding and then given
    # no way to report that it skipped one.
    # Two literals, because the rendered prose wraps between them.
    grep -qF 'state how it was handled (fixed-inline / filed-as-task /' "$doc" ||
        fail "8b: $doc's act step no longer states the fixed-inline/filed-as-task vocabulary"
    grep -qF 'skipped, with a reason' "$doc" ||
        fail "8b: $doc's act step still reports a two-action vocabulary that cannot express a skip"
done
pass "8b: all six rendered review docs state the marker + disposition rule and drop every retired absolute"

# --- 8c. PLANTED-MUTATION SELF-TESTS (non-vacuity, both directions) -----------
# Four independent mutations, each of which MUST flip one of the section-8
# assertions. Without these, a refactor that quietly re-broadened the skip (or
# dropped the marker, the disposition rule, or the conditional act-prompt lead)
# would sail through a green harness.
say "8c. Non-gating skip mutation self-tests (prove section 8 is not vacuous)"
NGMUT="$TMP/ng-mut/.claude/workflows/lib"
mkdir -p "$NGMUT"

reset_ngmut() {
    cp "$LIB" "$NGMUT/review.mjs"
    cp "$DISPATCH_LIB" "$NGMUT/dispatch-phase.mjs"
    cp "$PLAN_LIB" "$NGMUT/plan-review.mjs"
}

# (a) Widen the skip to `concern`: the "one refuter per gating finding" count
#     must stop holding.
reset_ngmut
sed "s/^const NON_GATING_SEVERITIES = \['suggestion'\];/const NON_GATING_SEVERITIES = ['suggestion', 'concern']; \/\/ MUTANT/" \
    "$LIB" >"$NGMUT/review.mjs"
grep -q 'MUTANT' "$NGMUT/review.mjs" || fail "8c(a): mutation setup did not widen NON_GATING_SEVERITIES"

cat >"$TMP/ng-mut-widen.mjs" <<'NODE_NG_WIDEN'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';
const mod = await import(pathToFileURL(process.argv[2]).href);
async function refParallel(thunks) {
  return Promise.all(thunks.map((t) => Promise.resolve().then(t).catch(() => null)));
}
async function refPipeline(items, ...stages) {
  return Promise.all(items.map(async (item, i) => {
    let acc = item;
    for (const stage of stages) {
      try { acc = await stage(acc, item, i); } catch { return null; }
    }
    return acc;
  }));
}
const calls = [];
const agent = async (prompt, opts) => {
  const label = (opts && opts.label) || '';
  calls.push(label);
  if (label.startsWith('find:')) {
    return label.endsWith(':correctness')
      ? { findings: [
          { id: 'b1', severity: 'blocking', confidence: 90 },
          { id: 'c1', severity: 'concern', confidence: 90 },
          { id: 's1', severity: 'suggestion', confidence: 90 },
        ] }
      : { findings: [] };
  }
  return { refuted: false, confidence: 90 };
};
await mod.buildReviewPipeline('code', { agent, pipeline: refPipeline, parallel: refParallel, log: () => {} })({ target: 't' });
const refuters = calls.filter((l) => l.startsWith('refute:'));
assert.throws(
  () => assert.equal(refuters.length, 2),
  'widening NON_GATING_SEVERITIES to `concern` must FAIL the gating-refuter count — else the check is vacuous'
);
console.log('8c(a) widen mutation self-test passed');
NODE_NG_WIDEN
run_node "$TMP/ng-mut-widen.mjs" "$NGMUT/review.mjs" ||
    fail "8c(a): widening the skip did not flip the gating-refuter count assertion"

# (b) Drop the `unrefuted` marker from the pass-through: an act step could no
#     longer tell reported-only from verified.
reset_ngmut
sed "s/unrefuted: true, unrefutedReason: 'non-gating'/unrefutedMUTANT: true, unrefutedReason: 'non-gating'/" \
    "$LIB" >"$NGMUT/review.mjs"
grep -q 'unrefutedMUTANT' "$NGMUT/review.mjs" || fail "8c(b): mutation setup did not rename the marker"

cat >"$TMP/ng-mut-marker.mjs" <<'NODE_NG_MARKER'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';
const mod = await import(pathToFileURL(process.argv[2]).href);
async function refParallel(thunks) {
  return Promise.all(thunks.map((t) => Promise.resolve().then(t).catch(() => null)));
}
async function refPipeline(items, ...stages) {
  return Promise.all(items.map(async (item, i) => {
    let acc = item;
    for (const stage of stages) {
      try { acc = await stage(acc, item, i); } catch { return null; }
    }
    return acc;
  }));
}
const agent = async (prompt, opts) => {
  const label = (opts && opts.label) || '';
  if (label.startsWith('find:')) {
    return label.endsWith(':correctness')
      ? { findings: [{ id: 's1', severity: 'suggestion', confidence: 90 }] }
      : { findings: [] };
  }
  return { refuted: false, confidence: 90 };
};
const { survivors } = await mod.buildReviewPipeline('code', { agent, pipeline: refPipeline, parallel: refParallel, log: () => {} })({ target: 't' });
assert.throws(
  () => assert.equal(survivors[0].unrefuted, true),
  'a renamed marker must FAIL the pass-through marker check — else the check is vacuous'
);
console.log('8c(b) marker mutation self-test passed');
NODE_NG_MARKER
run_node "$TMP/ng-mut-marker.mjs" "$NGMUT/review.mjs" ||
    fail "8c(b): dropping the marker did not flip the marker assertion"

# (c) Strip the disposition sentence: both act prompts lose the rule that makes
#     an un-refuted finding safe to hand to an acting agent.
reset_ngmut
sed 's/were \*\*reported, not verified\*\*/were MUTANT/' "$LIB" >"$NGMUT/review.mjs"
grep -q 'were MUTANT' "$NGMUT/review.mjs" || fail "8c(c): mutation setup did not strip the disposition wording"

cat >"$TMP/ng-mut-disposition.mjs" <<'NODE_NG_DISP'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';
const dispatchMod = await import(pathToFileURL(process.argv[2]).href);
const planMod = await import(pathToFileURL(process.argv[3]).href);
const MIXED = [{ id: 'f2', severity: 'suggestion', confidence: 90, unrefuted: true }];
const codePrompt = dispatchMod.buildCodeActPrompt('phase', 'rm', 'p1', 'wt', MIXED);
const planPrompt = planMod.buildActPrompt('phase', 'rm', 'p1', MIXED);
assert.throws(
  () => assert.ok(codePrompt.includes('reported, not verified')),
  'stripping the disposition wording must FAIL the code act-prompt check — else the check is vacuous'
);
assert.throws(
  () => assert.ok(planPrompt.includes('reported, not verified')),
  'stripping the disposition wording must FAIL the plan act-prompt check — else the check is vacuous'
);
console.log('8c(c) disposition mutation self-test passed');
NODE_NG_DISP
run_node "$TMP/ng-mut-disposition.mjs" "$NGMUT/dispatch-phase.mjs" "$NGMUT/plan-review.mjs" ||
    fail "8c(c): stripping the disposition rule did not flip the act-prompt assertions"

# (d) Restore the UNCONDITIONAL "survived refutation" lead on the code act
#     prompt: the mixed payload would again be described as fully verified.
reset_ngmut
sed 's/const hasUnrefuted = list.some((f) => f \&\& f.unrefuted);/const hasUnrefuted = false; \/\/ MUTANT/' \
    "$DISPATCH_LIB" >"$NGMUT/dispatch-phase.mjs"
grep -q 'MUTANT' "$NGMUT/dispatch-phase.mjs" || fail "8c(d): mutation setup did not force the unconditional lead"

cat >"$TMP/ng-mut-lead.mjs" <<'NODE_NG_LEAD'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';
const dispatchMod = await import(pathToFileURL(process.argv[2]).href);
const MIXED = [{ id: 'f2', severity: 'suggestion', confidence: 90, unrefuted: true }];
const prompt = dispatchMod.buildCodeActPrompt('phase', 'rm', 'p1', 'wt', MIXED);
assert.throws(
  () => assert.ok(!prompt.includes('These findings survived refutation')),
  'an unconditional "survived refutation" lead must FAIL the mixed-payload check — else the check is vacuous'
);
console.log('8c(d) act-prompt lead mutation self-test passed');
NODE_NG_LEAD
run_node "$TMP/ng-mut-lead.mjs" "$NGMUT/dispatch-phase.mjs" ||
    fail "8c(d): forcing the unconditional lead did not flip the mixed-payload assertion"

pass "8c: all four mutations flip their assertion — section 8 is non-vacuous"

# --- 9. REFUTATION BUDGET (bound-review-fan-out phase 4) ---------------------
# The pipeline grades at most DEFAULT_MAX_REFUTATIONS gating findings per review
# unit; everything past the cut takes the EXISTING un-refuted pass-through with a
# `budget` reason. This section gates the whole bound end to end: the chosen N is
# pinned to the evidence that produced it, the ranking is total and stable, the
# under/at/over-budget boundaries behave, all FOUR provenance states are tellable
# apart by markers alone, the output is deterministic even under shuffled refuter
# resolution, and — the blocking correctness question — an over-budget finding can
# never turn a `rework` outcome into `reviewed`.
say '9. Refutation budget: under/at/over budget, four-state distinguishability, determinism, monotonicity'

# The chosen N must never be changeable without the evidence that produced it.
# These greps pin the derivation comment to the concrete phase-2 figures.
grep -q 'const DEFAULT_MAX_REFUTATIONS = 5;' "$LIB" ||
    fail "9: DEFAULT_MAX_REFUTATIONS is not declared as 5 in $LIB"
for lit in 'determiningFindingRank' '98.2' '94.5' 'docs/token-baseline.json'; do
    grep -qF "$lit" "$LIB" ||
        fail "9: the DEFAULT_MAX_REFUTATIONS derivation no longer cites '$lit' — N must never change without its evidence"
done
# The cut must stay free of the two globals this runtime forbids (section 2
# greps the workflow scripts; re-assert scoped to the canonical source).
for forbidden in 'Date.now(' 'Math.random('; do
    ! grep -qF "$forbidden" "$LIB" || fail "9: $LIB must not use $forbidden"
done

cat >"$TMP/budget-test.mjs" <<'NODE_BUDGET_TEST'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const [libPath, dispatchPath] = process.argv.slice(2);
const mod = await import(pathToFileURL(libPath).href);
const dispatchMod = await import(pathToFileURL(dispatchPath).href);

const {
  buildReviewPipeline,
  DEFAULT_MAX_REFUTATIONS,
  resolveRefutationBudget,
  rankBudgetCandidates,
  survives,
  classifyOutcome,
  CONFIDENCE_FLOOR,
} = mod;
const { runCodeGate } = dispatchMod;

// The SAME reference runtime sections 3 and 8 use.
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
// makeSpyAgent(plantFindings, plantVerdicts, opts) — records every call.
//   opts.throwOnRefute — a Set of finding ids whose refuter throws.
//   opts.resolveDelay  — id -> number of microtask ticks before resolving, so a
//                        FIXED permutation of refuter completion order can be
//                        planted without Math.random.
function makeSpyAgent(plantFindings, plantVerdicts, opts) {
  const o = opts || {};
  const calls = [];
  const logs = [];
  async function agent(prompt, options) {
    const label = (options && options.label) || '';
    calls.push({ label, prompt });
    const parts = label.split(':');
    if (parts[0] === 'find') return { findings: plantFindings[parts[2]] || [] };
    if (parts[0] === 'refute') {
      const id = parts.slice(2).join(':');
      if (o.throwOnRefute && o.throwOnRefute.indexOf(id) !== -1) throw new Error('boom refuter ' + id);
      const ticks = (o.resolveDelay && o.resolveDelay[id]) || 0;
      for (let i = 0; i < ticks; i++) await Promise.resolve();
      return plantVerdicts[id] || { refuted: false, confidence: 90 };
    }
    throw new Error('unexpected agent label: ' + label);
  }
  return { agent, calls, logs };
}
function deps(spy) {
  return { agent: spy.agent, pipeline: refPipeline, parallel: refParallel, log: (m) => spy.logs.push(m) };
}
const CTX = { target: 'phase widget/phase-1-foo' };
const refuteIds = (spy) => spy.calls.filter((c) => c.label.startsWith('refute:')).map((c) => c.label.split(':')[2]);

// Deterministic gating-finding factory: b01..bNN, all `blocking`, descending
// confidence so the rank order is unambiguous and independent of id sorting.
function gatingFindings(n, base) {
  const out = [];
  for (let i = 0; i < n; i++) {
    const id = 'b' + String(i + 1).padStart(2, '0');
    out.push({ id: id, severity: 'blocking', confidence: (base || 99) - i, what_fails: 'defect ' + id });
  }
  return out;
}

// ============================================================================
// 9a. The chosen N, and the configuration surface around it.
// ============================================================================
assert.equal(DEFAULT_MAX_REFUTATIONS, 5, 'the default refutation budget is 5 (phase 2 withinTop5: 100 % / 98.2 %)');

assert.equal(resolveRefutationBudget(undefined), 5, 'unset falls back to the default');
assert.equal(resolveRefutationBudget(null), 5, 'null falls back to the default');
assert.equal(resolveRefutationBudget(''), 5, "'' falls back to the default");
assert.equal(resolveRefutationBudget(5), 5, 'a number is accepted');
assert.equal(resolveRefutationBudget('5'), 5, 'an integer-only string is accepted');
assert.equal(resolveRefutationBudget(0), 0, '0 is LEGAL and distinct from unset (grade nothing)');
assert.notEqual(resolveRefutationBudget(0), resolveRefutationBudget(undefined), '0 is never conflated with unset');
for (const bad of ['5abc', -1, -0, 1.5, {}, [], true, 'five']) {
  assert.throws(
    () => resolveRefutationBudget(bad),
    /maxRefutations must be a non-negative integer/,
    'rejects ' + JSON.stringify(bad) + ' with an actionable message'
  );
}
// The error must state what 0 means and that there is no uncapped sentinel.
let budgetErr = null;
try {
  resolveRefutationBudget('5abc');
} catch (e) {
  budgetErr = e.message;
}
assert.ok(/0 means grade nothing/.test(budgetErr), 'the error documents the 0 semantics');
assert.ok(/no "uncapped" sentinel/.test(budgetErr), 'the error documents that there is no uncapped sentinel');

// ============================================================================
// 9b. rankBudgetCandidates: severity → confidence desc → id → source order.
// ============================================================================
const RANK_SET = [
  { order: 0, finding: { id: 'z', severity: 'suggestion', confidence: 100 } },
  { order: 1, finding: { id: 'y', severity: 'concern', confidence: 80 } },
  { order: 2, finding: { id: 'x', severity: 'blocking', confidence: 70 } },
  { order: 3, finding: { id: 'w', severity: 'blocking', confidence: 90 } },
  { order: 4, finding: { id: 'nit', severity: 'unknown-severity', confidence: 100 } },
  { order: 5, finding: { id: 'a', severity: 'concern', confidence: 80 } },
  { order: 6, finding: { id: 'dup', severity: 'blocking', confidence: 70 } },
  { order: 7, finding: { id: 'dup', severity: 'blocking', confidence: 70 } },
  { order: 8, finding: { id: 'noconf', severity: 'concern' } },
];
assert.deepEqual(
  rankBudgetCandidates(RANK_SET).map((c) => c.order),
  [3, 6, 7, 2, 5, 1, 8, 0, 4],
  'ranking is severity, then confidence DESC, then id, then source order (an unknown severity sorts last)'
);
// Totality: the two `dup` entries are separated ONLY by `order`.
const dupOnly = rankBudgetCandidates([RANK_SET[7], RANK_SET[6]]).map((c) => c.order);
assert.deepEqual(dupOnly, [6, 7], 'two candidates sharing an id are ordered by source order, whatever the input order');
// Purity: the input array is not mutated.
const before = RANK_SET.map((c) => c.order);
rankBudgetCandidates(RANK_SET);
assert.deepEqual(RANK_SET.map((c) => c.order), before, 'rankBudgetCandidates does not mutate its input');
assert.deepEqual(rankBudgetCandidates(null), [], 'a non-array input yields an empty ranking');

// ============================================================================
// 9c-run. UNDER budget: 3 gating candidates, N = 5.
// ============================================================================
{
  const spy = makeSpyAgent({ correctness: gatingFindings(3) }, {});
  const { survivors, budget } = await buildReviewPipeline('code', deps(spy))(CTX);
  assert.equal(refuteIds(spy).length, 3, 'under budget: every gating finding is graded');
  assert.equal(budget.hit, false, 'under budget: hit is false');
  assert.equal(budget.passedThroughBudget, 0, 'under budget: nothing is passed through for budget');
  assert.equal(budget.max, 5, 'under budget: the default cap is reported');
  assert.equal(budget.produced, 3, 'under budget: produced counts the candidates that existed');
  assert.equal(budget.graded, 3, 'under budget: graded counts the refuters dispatched');
  assert.ok(!survivors.some((f) => f.unrefutedReason), 'under budget: no survivor carries an unrefutedReason');
  // REGRESSION GUARD: the find/refute restructure must not perturb the existing
  // lane. Pin the resolved survivors to an explicit baseline.
  assert.deepEqual(
    survivors,
    [
      { id: 'b01', severity: 'blocking', confidence: 99, what_fails: 'defect b01', concern: 'correctness' },
      { id: 'b02', severity: 'blocking', confidence: 98, what_fails: 'defect b02', concern: 'correctness' },
      { id: 'b03', severity: 'blocking', confidence: 97, what_fails: 'defect b03', concern: 'correctness' },
    ],
    'under budget: survivors are byte-identical to the pre-change baseline (no marker, no extra field)'
  );
  assert.ok(
    !spy.logs.join('\n').includes('BUDGET HIT'),
    'under budget: the log line is unchanged — no budget clause'
  );
}

// ============================================================================
// 9d. EXACTLY at budget: 5 gating candidates, N = 5. The boundary must NOT
//     read as a hit — an off-by-one here silently reports a bounded run as
//     complete coverage, or vice versa.
// ============================================================================
{
  const spy = makeSpyAgent({ correctness: gatingFindings(5) }, {});
  const { budget } = await buildReviewPipeline('code', deps(spy))(CTX);
  assert.equal(refuteIds(spy).length, 5, 'at budget: exactly 5 refuters');
  assert.equal(budget.hit, false, 'at budget: hit is FALSE at the boundary');
  assert.equal(budget.graded, 5, 'at budget: graded === 5');
  assert.equal(budget.passedThroughBudget, 0, 'at budget: nothing overflowed');
}

// ============================================================================
// 9e. OVER budget: 13 gating candidates, N = 5.
// ============================================================================
{
  const planted = gatingFindings(13);
  const spy = makeSpyAgent({ correctness: planted }, {});
  const { survivors, budget } = await buildReviewPipeline('code', deps(spy))(CTX);
  const graded = refuteIds(spy);
  assert.equal(graded.length, 5, 'over budget: exactly 5 refuters, not 13');
  // The graded 5 are precisely the top 5 under the module's OWN ranking.
  const expectedTop = rankBudgetCandidates(
    planted.map((f, i) => ({ order: i, finding: { ...f, concern: 'correctness' } }))
  )
    .slice(0, 5)
    .map((c) => c.finding.id);
  assert.deepEqual(graded.slice().sort(), expectedTop.slice().sort(), 'over budget: the graded 5 are the top 5');
  assert.deepEqual(
    budget,
    {
      max: 5,
      produced: 13,
      gating: 13,
      graded: 5,
      passedThroughNonGating: 0,
      passedThroughBudget: 8,
      refuterErrors: 0,
      hit: true,
    },
    'over budget: the budget accounting is exact'
  );
  const overflowSurvivors = survivors.filter((f) => graded.indexOf(f.id) === -1);
  assert.equal(overflowSurvivors.length, 8, 'over budget: all 8 overflow findings survive the floor at confidence >= 70');
  assert.ok(
    overflowSurvivors.every((f) => f.unrefuted === true && f.unrefutedReason === 'budget'),
    'over budget: every overflow finding takes the EXISTING un-refuted pass-through, marked reason `budget`'
  );
  const logLine = spy.logs.join('\n');
  assert.ok(logLine.includes('BUDGET HIT'), 'over budget: the log announces the bound');
  assert.ok(logLine.includes('13 finding(s) produced'), 'over budget: the log states how many were produced');
  assert.ok(logLine.includes('5 graded'), 'over budget: the log states how many were graded');
  assert.ok(logLine.includes('8 passed through for budget'), 'over budget: the log states how many were passed through');
  assert.ok(logLine.includes('cap 5'), 'over budget: the log states the cap');
}

// ============================================================================
// 9f. FOUR STATES in ONE driven run: graded-and-survived, skipped-as-non-gating,
//     passed-over-for-budget, and grading-crashed. A single classifier over each
//     survivor must yield exactly one label — no ambiguous, no unlabelled.
// ============================================================================
{
  const planted = gatingFindings(8).concat([
    { id: 's1', severity: 'suggestion', confidence: 90, what_fails: 'readability nit' },
  ]);
  // b01 is the highest-confidence gating candidate, so it is inside the top 5;
  // its refuter throws.
  const spy = makeSpyAgent({ correctness: planted }, {}, { throwOnRefute: ['b01'] });
  const { survivors } = await buildReviewPipeline('code', deps(spy))(CTX);

  function classify(f) {
    const labels = [];
    if (!f.unrefuted && !f.refuterError) labels.push('graded-and-survived');
    if (f.unrefuted === true && f.unrefutedReason === 'non-gating') labels.push('skipped-as-non-gating');
    if (f.unrefuted === true && f.unrefutedReason === 'budget') labels.push('passed-over-for-budget');
    if (f.refuterError === true && !f.unrefuted) labels.push('grading-crashed');
    return labels;
  }
  const seen = {};
  for (const f of survivors) {
    const labels = classify(f);
    assert.equal(labels.length, 1, 'finding ' + f.id + ' must map to EXACTLY one of the four states, got ' + labels.join('+'));
    seen[labels[0]] = (seen[labels[0]] || 0) + 1;
  }
  assert.equal(Object.keys(seen).sort().join(','), 'graded-and-survived,grading-crashed,passed-over-for-budget,skipped-as-non-gating',
    'all four states occur in one run and are distinguishable by markers alone');

  const byId = Object.fromEntries(survivors.map((f) => [f.id, f]));
  assert.equal(byId.b01.refuterError, true, 'the crashed refuter marks its finding refuterError');
  assert.equal(byId.b01.unrefuted, undefined, 'a crashed refuter NEVER marks the finding unrefuted');
  assert.equal(byId.b01.unrefutedReason, undefined, 'a crashed refuter carries no unrefutedReason');
  assert.equal(byId.s1.unrefutedReason, 'non-gating', 'the suggestion is marked non-gating');
  assert.equal(byId.s1.refuterError, undefined, 'a deliberate skip is not a crash');
  assert.equal(byId.b08.unrefutedReason, 'budget', 'the lowest-ranked gating finding was cut for budget');
  assert.equal(byId.b08.refuterError, undefined, 'a budget skip is not a crash');
  assert.equal(byId.b02.unrefuted, undefined, 'a graded survivor carries no marker at all');
  assert.equal(byId.b02.refuterError, undefined, 'a graded survivor carries no crash marker');
}

// ============================================================================
// 9g. DETERMINISM — repeated runs, and a fixed-permutation shuffled refuter
//     resolution order (no Math.random anywhere).
// ============================================================================
{
  const planted = gatingFindings(13).concat([
    { id: 'dup', severity: 'blocking', confidence: 70, what_fails: 'shared id A' },
  ]);
  // The SAME id emitted by a second dimension: without the source-`order`
  // tiebreak, the cut between these two is nondeterministic.
  const plantedTests = [{ id: 'dup', severity: 'blocking', confidence: 70, what_fails: 'shared id B' }];
  const shuffle = { b01: 7, b02: 3, b03: 11, b04: 1, b05: 5, dup: 9 };
  const snapshots = [];
  for (let run = 0; run < 5; run++) {
    const spy = makeSpyAgent({ correctness: planted, tests: plantedTests }, {}, { resolveDelay: run === 4 ? shuffle : null });
    const { survivors, budget } = await buildReviewPipeline('code', deps(spy))(CTX);
    snapshots.push({
      out: JSON.stringify({ survivors, budget }),
      refuters: refuteIds(spy).slice().sort().join(','),
    });
  }
  for (let i = 1; i < snapshots.length; i++) {
    assert.equal(snapshots[i].out, snapshots[0].out, 'run ' + i + ': { survivors, budget } is byte-identical across runs');
    assert.equal(snapshots[i].refuters, snapshots[0].refuters, 'run ' + i + ': the refuter label set is identical');
  }
  assert.ok(
    snapshots[0].refuters.length > 0,
    'the determinism check is not vacuous — refuters were actually dispatched'
  );
}

// ============================================================================
// 9h. The budget is not a no-op, and only GATING findings consume it.
// ============================================================================
{
  // N = 0: grade nothing; every gating candidate passes through for budget.
  const spy = makeSpyAgent({ correctness: gatingFindings(4) }, {});
  const { survivors, budget } = await buildReviewPipeline('code', deps(spy))({ ...CTX, maxRefutations: 0 });
  assert.equal(refuteIds(spy).length, 0, 'N=0 dispatches NO refuter');
  assert.equal(budget.max, 0, 'N=0 is honored, not treated as unset');
  assert.equal(budget.graded, 0, 'N=0 grades nothing');
  assert.equal(budget.passedThroughBudget, 4, 'N=0 passes every gating candidate through for budget');
  assert.ok(
    survivors.every((f) => f.unrefutedReason === 'budget'),
    'N=0: every survivor is marked as cut for budget'
  );
}
{
  // Suggestions never consume budget: 5 gating + 3 suggestions with N = 5 still
  // grades all 5 gating findings and passes the 3 suggestions through as
  // `non-gating`, not as `budget`.
  const suggestions = [
    { id: 's1', severity: 'suggestion', confidence: 95 },
    { id: 's2', severity: 'suggestion', confidence: 94 },
    { id: 's3', severity: 'suggestion', confidence: 93 },
  ];
  const spy = makeSpyAgent({ correctness: gatingFindings(5).concat(suggestions) }, {});
  const { survivors, budget } = await buildReviewPipeline('code', deps(spy))(CTX);
  assert.equal(refuteIds(spy).length, 5, 'suggestions do not displace a gating finding from the budget');
  assert.equal(budget.hit, false, 'suggestions do not push the unit over budget');
  assert.equal(budget.produced, 8, 'produced counts every candidate, gating or not');
  assert.equal(budget.gating, 5, 'gating counts only the budget-consuming half');
  assert.equal(budget.passedThroughNonGating, 3, 'the 3 suggestions are non-gating pass-throughs');
  assert.equal(budget.passedThroughBudget, 0, 'no suggestion is ever reported as cut for budget');
  assert.ok(
    survivors.filter((f) => f.id.startsWith('s')).every((f) => f.unrefutedReason === 'non-gating'),
    'suggestions carry the non-gating reason'
  );
}
{
  // A finder dimension that CRASHED contributes no candidates, and its absence
  // must not be conflated with a clean dimension in `budget.produced`.
  const spy = makeSpyAgent({ correctness: gatingFindings(2) }, {});
  const base = spy.agent;
  spy.agent = async (prompt, options) => {
    if (options && options.label === 'find:code:tests') throw new Error('boom finder');
    return base(prompt, options);
  };
  const { budget } = await buildReviewPipeline('code', deps(spy))(CTX);
  assert.equal(budget.produced, 2, 'a crashed finder contributes no candidates to `produced`');
  assert.equal(budget.hit, false, 'a crashed finder does not manufacture a budget hit');
}
{
  // A per-run override reaches the pipeline.
  const spy = makeSpyAgent({ correctness: gatingFindings(6) }, {});
  const { budget } = await buildReviewPipeline('code', deps(spy))({ ...CTX, maxRefutations: 2 });
  assert.equal(refuteIds(spy).length, 2, 'context.maxRefutations overrides the default');
  assert.equal(budget.max, 2, 'the override is reported in the accounting');
  assert.equal(budget.passedThroughBudget, 4, 'the rest overflow');
}
{
  // An invalid override throws BEFORE any agent is dispatched.
  const spy = makeSpyAgent({ correctness: gatingFindings(2) }, {});
  await assert.rejects(
    () => buildReviewPipeline('code', deps(spy))({ ...CTX, maxRefutations: '5abc' }),
    /maxRefutations must be a non-negative integer/,
    'an invalid budget throws'
  );
  assert.equal(spy.calls.length, 0, 'an invalid budget throws before a single agent() call burns tokens');
}

// ============================================================================
// 9i. THE FLOOR IS NOT BYPASSED. An over-budget finding below the floor is still
//     dropped; `survives` itself gained no budget-aware branch.
// ============================================================================
{
  const planted = gatingFindings(5).concat([
    { id: 'z-high', severity: 'blocking', confidence: 90, what_fails: 'over budget, above floor' },
    { id: 'z-low', severity: 'blocking', confidence: 69, what_fails: 'over budget, below floor' },
  ]);
  const spy = makeSpyAgent({ correctness: planted }, {});
  const { survivors, budget } = await buildReviewPipeline('code', deps(spy))(CTX);
  assert.equal(budget.passedThroughBudget, 2, 'both extra findings are over budget');
  const ids = survivors.map((f) => f.id);
  assert.ok(ids.includes('z-high'), 'an over-budget finding at 90 confidence survives');
  assert.ok(!ids.includes('z-low'), 'an over-budget finding at 69 confidence is DROPPED by the floor');
}
assert.equal(CONFIDENCE_FLOOR, 70, 'the floor is still 70');
assert.equal(survives({ confidence: 69 }, null), false, 'survives applies the floor to an ungraded finding');
assert.equal(survives({ confidence: 70 }, null), true, 'survives keeps an ungraded finding at the floor');

// ============================================================================
// 9j. AC6 — the monotonicity PROOF, executed rather than only asserted in prose.
//
// For a planted 8-candidate set, over EVERY subset treated as "the grader would
// have refuted this", for both tiers and for N in {0,1,3,5,99}: the budgeted
// survivor set is always a SUPERSET of the unbudgeted one, and
// `classifyOutcome(unbudgeted) === 'rework'` implies
// `classifyOutcome(budgeted) === 'rework'`.
// ============================================================================
{
  const PROOF_SET = [
    { id: 'p1', severity: 'blocking', confidence: 95 },
    { id: 'p2', severity: 'blocking', confidence: 90 },
    { id: 'p3', severity: 'concern', confidence: 88 },
    { id: 'p4', severity: 'concern', confidence: 80 },
    { id: 'p5', severity: 'blocking', confidence: 75 },
    { id: 'p6', severity: 'concern', confidence: 72 },
    { id: 'p7', severity: 'blocking', confidence: 90 },
    { id: 'p8', severity: 'concern', confidence: 95 },
  ];
  const records = PROOF_SET.map((f, i) => ({ order: i, finding: f }));
  const ranked = rankBudgetCandidates(records);
  let checked = 0;
  let sawRework = 0;
  for (let mask = 0; mask < 1 << PROOF_SET.length; mask++) {
    const verdictFor = (id) => {
      const i = PROOF_SET.findIndex((f) => f.id === id);
      return mask & (1 << i) ? { refuted: true, confidence: 10 } : { refuted: false, confidence: 90 };
    };
    const unbudgeted = ranked
      .filter((c) => survives(c.finding, verdictFor(c.finding.id)))
      .map((c) => c.finding);
    for (const n of [0, 1, 3, 5, 99]) {
      const budgeted = ranked
        .filter((c, i) => survives(c.finding, i < n ? verdictFor(c.finding.id) : null))
        .map((c) => c.finding);
      // (3) superset
      for (const f of unbudgeted) {
        assert.ok(budgeted.indexOf(f) !== -1, 'budgeted survivors are a SUPERSET of the unbudgeted ones');
      }
      for (const tier of [undefined, 'large']) {
        const u = classifyOutcome({ planFindings: [], codeReviews: [unbudgeted], tier: tier });
        const b = classifyOutcome({ planFindings: [], codeReviews: [budgeted], tier: tier });
        if (u === 'rework') {
          sawRework++;
          assert.equal(b, 'rework', 'a budget hit can NEVER turn a rework outcome into reviewed');
        }
        checked++;
      }
    }
  }
  assert.ok(checked === (1 << 8) * 5 * 2, 'the property test covered every subset x N x tier combination');
  assert.ok(sawRework > 0, 'the property test is not vacuous — rework outcomes actually occurred');
}
{
  // Severity-first ranking is what protects the determining finding in the
  // common case: 8 gating candidates whose ONLY blocking one is emitted LAST by
  // the finder still sorts to rank 1, so it is graded even at N = 5 while six
  // higher-confidence `concern` candidates are not.
  const planted = [
    { id: 'c1', severity: 'concern', confidence: 99 },
    { id: 'c2', severity: 'concern', confidence: 98 },
    { id: 'c3', severity: 'concern', confidence: 97 },
    { id: 'c4', severity: 'concern', confidence: 96 },
    { id: 'c5', severity: 'concern', confidence: 95 },
    { id: 'c6', severity: 'concern', confidence: 94 },
    { id: 'zz-blocker', severity: 'blocking', confidence: 90, what_fails: 'the real defect' },
    { id: 'c7', severity: 'concern', confidence: 93 },
  ];
  const spy = makeSpyAgent({ correctness: planted }, {});
  const { survivors, budget } = await buildReviewPipeline('code', deps(spy))({ ...CTX, maxRefutations: 5 });
  assert.equal(budget.max, 5, 'the scenario runs at the chosen N');
  assert.ok(refuteIds(spy).includes('zz-blocker'), 'severity-first ranking keeps the sole blocker inside the budget');
  const blocker = survivors.find((f) => f.id === 'zz-blocker');
  assert.ok(blocker, 'the blocking finding survives');
  assert.equal(blocker.unrefutedReason, undefined, 'it was graded, not cut for budget');
  assert.equal(
    classifyOutcome({ planFindings: [], codeReviews: [survivors] }),
    'rework',
    'the unit classifies rework'
  );
}
{
  // Same shape, but the blocker really IS over budget: six higher-confidence
  // blocking candidates rank above it, so with N = 5 it is ungraded.
  const planted = gatingFindings(6, 99).concat([
    { id: 'zz-late', severity: 'blocking', confidence: 90, what_fails: 'the rank-7 defect' },
  ]);
  const spy = makeSpyAgent({ correctness: planted }, {});
  const { survivors, budget } = await buildReviewPipeline('code', deps(spy))({ ...CTX, maxRefutations: 5 });
  assert.equal(budget.hit, true, 'rank-7 scenario: the bound was hit');
  const late = survivors.find((f) => f.id === 'zz-late');
  assert.ok(late, 'the rank-7 blocking finding survives');
  assert.equal(late.unrefuted, true, 'it was never graded');
  assert.equal(late.unrefutedReason, 'budget', 'it was cut for budget, not skipped as non-gating');
  assert.equal(
    classifyOutcome({ planFindings: [], codeReviews: [survivors] }),
    'rework',
    'an ungraded over-budget blocker STILL forces rework'
  );
}
{
  // The inverse guard: the SAME over-budget blocker at 69 confidence is dropped
  // by the floor and the unit classifies `reviewed`. That is the floor doing its
  // documented job — it is the ONLY thing that can drop an overflow finding.
  const planted = gatingFindings(6, 99).concat([
    { id: 'zz-late', severity: 'blocking', confidence: 69, what_fails: 'below the floor' },
  ]);
  const spy = makeSpyAgent({ correctness: planted }, { b01: { refuted: true, confidence: 10 } });
  const { survivors } = await buildReviewPipeline('code', deps(spy))({ ...CTX, maxRefutations: 5 });
  assert.ok(!survivors.some((f) => f.id === 'zz-late'), 'a below-floor overflow finding is dropped');
}
{
  // The AC table is NEVER budgeted: classifyOutcome step 2 is bit-identical
  // under every N, including 0.
  const AC_GAP = [{ criterion: 'AC1', status: 'FAIL', evidence: 'none' }];
  for (const n of [0, 1, 5, 99]) {
    assert.equal(
      classifyOutcome({ planFindings: [], codeReviews: [[]], acTable: AC_GAP }),
      'rework',
      'the AC-table gate is unaffected by the budget (N=' + n + ')'
    );
  }
}
{
  // The act step must never mistake a gating budget-skipped survivor for a mere
  // observation: runCodeGate invokes `d.act` only on a CLEAN final round.
  let actCalls = 0;
  const blockingBudgetSkipped = [
    { id: 'zz', severity: 'blocking', confidence: 90, unrefuted: true, unrefutedReason: 'budget' },
  ];
  const gate = await runCodeGate(
    { maxRework: 0, tier: 'medium' },
    {
      implement: async () => null,
      review: async () => ({ survivors: blockingBudgetSkipped, acTable: null, budget: { max: 5, produced: 6, gating: 6, graded: 5, passedThroughNonGating: 0, passedThroughBudget: 1, refuterErrors: 0, hit: true } }),
      act: async () => {
        actCalls++;
        return { handled: [] };
      },
    }
  );
  assert.equal(actCalls, 0, 'd.act is NOT invoked when the only survivor is a blocking budget-skipped finding');
  assert.equal(gate.budgetRounds.length, 1, 'runCodeGate records one budget per review round');
  assert.equal(gate.budgetRounds[0].hit, true, 'the round-level budget is carried out of runCodeGate');
}

console.log('9: refutation budget assertions passed');
NODE_BUDGET_TEST

if run_node "$TMP/budget-test.mjs" "$LIB" "$DISPATCH_LIB"; then
    pass "9: the bound is chosen from evidence, ranked totally, boundaried correctly, four-state legible, deterministic, and monotone"
else
    fail "9: refutation budget assertions failed"
fi

# --- 9b-skills. THE FOUR-STATE VOCABULARY IN EVERY RENDERED SKILL -------------
# Every rendered review skill must name all four provenance states and the budget
# rule, or a skill reader is handed a finding it cannot classify. Whole-file
# greps, like 8b's, so hand-authored prose cannot contradict the generated span.
say "9b-skills. Every rendered review skill states the budget rule and the four-state marker table"
for doc in $REVIEW_DOCS; do
    [ -f "$doc" ] || fail "9b-skills: expected review skill doc not found: $doc"
    grep -qF "unrefutedReason: 'budget'" "$doc" ||
        fail "9b-skills: $doc never names the \`budget\` pass-through reason"
    grep -qF "unrefutedReason: 'non-gating'" "$doc" ||
        fail "9b-skills: $doc never names the \`non-gating\` pass-through reason"
    grep -qF 'refuterError: true' "$doc" ||
        fail "9b-skills: $doc never names the \`refuterError\` crash marker"
    grep -qF 'Refutation budget' "$doc" ||
        fail "9b-skills: $doc does not state the refutation budget rule"
    grep -qF 'determiningFindingRank' "$doc" ||
        fail "9b-skills: $doc does not point at the evidence behind the default"
    grep -qF 'maxRefutations' "$doc" ||
        fail "9b-skills: $doc does not name the per-run override"
done
pass "9b-skills: all six rendered review docs state the budget rule, its evidence, and all four state markers"

# --- 9c. PLANTED-MUTATION SELF-TESTS (non-vacuity) ----------------------------
# Seven independent mutations, each of which MUST flip one of section 9's
# assertions, plus a control run against the REAL file that must PASS. Without
# these, a refactor that quietly broke the ranking, the markers, the floor, the
# cut, or the default would sail through a green harness.
say "9c. Refutation-budget mutation self-tests (prove section 9 is not vacuous)"
BMUT="$TMP/budget-mut/.claude/workflows/lib"
mkdir -p "$BMUT"

reset_bmut() {
    cp "$LIB" "$BMUT/review.mjs"
    cp "$DISPATCH_LIB" "$BMUT/dispatch-phase.mjs"
}

# The CONTROL: section 9 must PASS against the real, unmutated file. Without this
# the seven negatives below could all "pass" simply because the section is broken.
reset_bmut
if run_node "$TMP/budget-test.mjs" "$BMUT/review.mjs" "$BMUT/dispatch-phase.mjs" >/dev/null 2>&1; then
    pass "9c(control): section 9 passes against an unmutated copy — the self-tests below are discriminating"
else
    fail "9c(control): section 9 FAILED against an unmutated copy — the mutation self-tests would be meaningless"
fi

mutate_and_expect_fail() {
    label="$1"
    desc="$2"
    reset_bmut
    shift 2
    "$@" || fail "9c($label): mutation setup failed"
    if run_node "$TMP/budget-test.mjs" "$BMUT/review.mjs" "$BMUT/dispatch-phase.mjs" >/dev/null 2>&1; then
        fail "9c($label): $desc did NOT flip a section-9 assertion — the check is vacuous"
    fi
    pass "9c($label): $desc flips a section-9 assertion"
}

# (i) Drop the source-`order` tiebreak: the duplicate-id determinism check breaks.
mut_order() {
    sed 's/^    return oa - ob;$/    return 0; \/\/ MUTANT/' "$LIB" >"$BMUT/review.mjs"
    grep -q 'MUTANT' "$BMUT/review.mjs"
}
mutate_and_expect_fail i 'dropping the source-order tiebreak' mut_order

# (ii) Sort ASCENDING by confidence: the top-N selection is wrong.
mut_conf() {
    sed 's/^    if (ca !== cb) return cb - ca;$/    if (ca !== cb) return ca - cb; \/\/ MUTANT/' "$LIB" >"$BMUT/review.mjs"
    grep -q 'MUTANT' "$BMUT/review.mjs"
}
mutate_and_expect_fail ii 'sorting confidence ascending instead of descending' mut_conf

# (iii) Remove `unrefutedReason` from the overflow object: the four-state check breaks.
mut_reason() {
    sed "s/{ ...c.finding, unrefuted: true, unrefutedReason: 'budget' }/{ ...c.finding, unrefuted: true }/" \
        "$LIB" >"$BMUT/review.mjs"
    ! grep -qF "unrefuted: true, unrefutedReason: 'budget' }" "$BMUT/review.mjs"
}
mutate_and_expect_fail iii 'dropping the budget unrefutedReason discriminator' mut_reason

# (iv) Bypass the floor on the overflow path: a below-floor overflow finding is retained.
mut_floor() {
    sed 's/^    const survivors = graded.filter((g) => survives(g.finding, g.verdict)).map((g) => g.finding);$/    const survivors = graded.filter((g) => g.skipped || survives(g.finding, g.verdict)).map((g) => g.finding); \/\/ MUTANT/' \
        "$LIB" >"$BMUT/review.mjs"
    grep -q 'MUTANT' "$BMUT/review.mjs"
}
mutate_and_expect_fail iv 'letting a pass-through bypass the confidence floor' mut_floor

# (v) Drop `refuterError` from the .catch: a crash is indistinguishable from a graded survivor.
mut_crash() {
    sed 's/\.catch(() => ({ finding: { ...c.finding, refuterError: true }, verdict: null }))/.catch(() => ({ finding: c.finding, verdict: null }))/' \
        "$LIB" >"$BMUT/review.mjs"
    ! grep -q 'refuterError: true }, verdict: null' "$BMUT/review.mjs"
}
mutate_and_expect_fail v 'dropping the refuterError crash marker' mut_crash

# (vi) Off-by-one on the cut: slice(0, N + 1).
mut_slice() {
    sed 's/^    const toGrade = ranked.slice(0, maxRefutations);$/    const toGrade = ranked.slice(0, maxRefutations + 1); \/\/ MUTANT/' \
        "$LIB" >"$BMUT/review.mjs"
    grep -q 'MUTANT' "$BMUT/review.mjs"
}
mutate_and_expect_fail vi 'an off-by-one on the budget cut' mut_slice

# (vii) Change the default to 3 (the value phase 2's own pre-registered rule rejects).
mut_default() {
    sed 's/^const DEFAULT_MAX_REFUTATIONS = 5;$/const DEFAULT_MAX_REFUTATIONS = 3; \/\/ MUTANT/' "$LIB" >"$BMUT/review.mjs"
    grep -q 'MUTANT' "$BMUT/review.mjs"
}
mutate_and_expect_fail vii 'changing the default budget to 3' mut_default

pass "9c: all seven mutations flip a section-9 assertion, and the control passes — section 9 is non-vacuous"

# --- 10c. UNIT-OF-WORK SEPARATION AND PHASE SCOPING ---------------------------
# --- 10d. CODE MODE'S ac/correctness ARE UNCHANGED, AND acTable IS SEVERITY-FREE
# --- 10e. THE NON-EMPTY ALWAYS-ON INVARIANT -----------------------------------
# (bound-review-fan-out phase 5.) The collapsed-plan-finder A/B measured whether
# plan mode's three always-on lenses may run in ONE agent; its DECISION and the
# decision/pipeline XOR live in scripts/verify-finder-collapse.sh. These three
# sections gate the invariants that must hold EITHER WAY, so they are not
# conditional on that decision: `unit-of-work` stays a separate, phase-scoped
# TRIGGERED dimension; code mode's `ac` keeps its own schema and its structured
# AC table keeps bypassing severity entirely; and the always-on set is never
# empty in either mode.
say "10c/10d/10e. unit-of-work separation, the unchanged ac/acTable path, and the non-empty always-on invariant"

cat >"$TMP/dimensions-test.mjs" <<'NODE_DIM_TEST'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const [libPath] = process.argv.slice(2);
const mod = await import(pathToFileURL(libPath).href);
const {
  DIMENSIONS,
  selectDimensions,
  buildReviewPipeline,
  stripNonPhaseUnitOfWork,
  classifyOutcome,
  acTableHasGap,
  AC_REVIEW_SCHEMA,
  FINDINGS_SCHEMA,
} = mod;

async function refParallel(thunks) {
  return Promise.all(thunks.map((t) => Promise.resolve().then(t).catch(() => null)));
}
async function refPipeline(items, ...stages) {
  return Promise.all(
    items.map(async (item) => {
      let acc = item;
      for (const stage of stages) acc = await stage(acc);
      return acc;
    })
  );
}

// --- 10c: unit-of-work is a SEPARATE, TRIGGERED dimension --------------------
const uow = DIMENSIONS.plan.filter((d) => d.key === 'unit-of-work');
assert.equal(uow.length, 1, 'unit-of-work must remain exactly one distinct DIMENSIONS.plan entry');
assert.equal(typeof uow[0].when, 'function', 'unit-of-work must keep its `when` predicate (it is TRIGGERED)');
assert.equal(uow[0].when({ targetType: 'phase' }), true);
assert.equal(uow[0].when({ targetType: 'task' }), false);
assert.equal(uow[0].when({}), false);

// selectDimensions' three-way contract, restated for the plan mode specifically.
const keysFor = (signals) => selectDimensions('plan', signals).map((d) => d.key);
assert.ok(!keysFor({}).includes('unit-of-work'), 'explicit empty signals must NOT select unit-of-work');
assert.ok(keysFor({ targetType: 'phase' }).includes('unit-of-work'), 'a phase target must select unit-of-work');
assert.deepEqual(keysFor(null), DIMENSIONS.plan.map((d) => d.key), 'omitted signals must fail OPEN to every dimension');
// plan-review.js threads NO signals, so the fail-open path is the one it takes.
assert.ok(keysFor(null).includes('unit-of-work'), 'the fail-open path plan-review.js takes must include unit-of-work');

// stripNonPhaseUnitOfWork is the CONSUMER-SIDE scoping that fail-open cannot do.
const survivors = [
  { id: 'a', concern: 'coherence', severity: 'blocking' },
  { id: 'b', concern: 'architectural-fit', severity: 'concern' },
  { id: 'c', concern: 'restraint', severity: 'suggestion' },
  { id: 'd', concern: 'unit-of-work', severity: 'blocking' },
];
assert.deepEqual(stripNonPhaseUnitOfWork(survivors, 'task').map((f) => f.id), ['a', 'b', 'c'],
  'on a non-phase unit ONLY the unit-of-work finding is removed');
assert.deepEqual(stripNonPhaseUnitOfWork(survivors, 'roadmap').map((f) => f.id), ['a', 'b', 'c']);
assert.deepEqual(stripNonPhaseUnitOfWork(survivors, 'phase').map((f) => f.id), ['a', 'b', 'c', 'd'],
  'on a phase unit nothing is removed');
// Idempotent, and never touches an always-on lens finding.
assert.deepEqual(
  stripNonPhaseUnitOfWork(stripNonPhaseUnitOfWork(survivors, 'task'), 'task').map((f) => f.id),
  ['a', 'b', 'c']
);

// --- 10d: code mode is UNCHANGED, and the AC table bypasses severity ---------
assert.deepEqual(
  DIMENSIONS.code.map((d) => d.key),
  ['ac', 'correctness', 'tests', 'architecture', 'api-docs', 'changelog', 'security'],
  'DIMENSIONS.code must be exactly these seven, in this order — ac and correctness are NOT merged'
);
assert.deepEqual(
  DIMENSIONS.code.filter((d) => !d.when).map((d) => d.key),
  ['ac', 'correctness'],
  'exactly ac and correctness are always-on in code mode'
);
assert.ok(!DIMENSIONS.code.some((d) => Array.isArray(d.lenses)), 'no code dimension may carry merged lenses');

// Drive the real pipeline: the `ac` finder alone resolves AC_REVIEW_SCHEMA, its
// table is captured, and attribution never touches it.
const schemaByLabel = new Map();
function schemaSpyAgent(findings, acTable) {
  return async (prompt, opts) => {
    schemaByLabel.set(opts.label, opts.schema);
    if (opts.label === 'find:code:ac') return { ac: acTable, findings: findings.ac || [] };
    if (opts.label.startsWith('find:')) return { findings: findings[opts.label.split(':')[2]] || [] };
    return { refuted: false, confidence: 95 };
  };
}
const acTable = [
  { criterion: 'AC1', status: 'PASS', evidence: 'x' },
  { criterion: 'AC2', status: 'FAIL', evidence: 'y' },
];
const codeRun = buildReviewPipeline('code', {
  agent: schemaSpyAgent({}, acTable),
  pipeline: refPipeline,
  parallel: refParallel,
  log: () => {},
});
const codeResult = await codeRun({ target: 'phase r/p', signals: null });
assert.equal(schemaByLabel.get('find:code:ac'), AC_REVIEW_SCHEMA, 'the ac finder must resolve AC_REVIEW_SCHEMA');
for (const [label, schema] of schemaByLabel) {
  if (label === 'find:code:ac' || !label.startsWith('find:')) continue;
  assert.equal(schema, FINDINGS_SCHEMA, label + ' must resolve FINDINGS_SCHEMA');
}
assert.deepEqual(codeResult.acTable, acTable, 'the ac table must be captured and returned');
assert.ok(!codeResult.acTable.some((e) => 'concern' in e), 'the AC table must never acquire a `concern` field');

// plan mode never sets one.
const planRun = buildReviewPipeline('plan', {
  agent: async (p, o) => (o.label.startsWith('find:') ? { findings: [] } : { refuted: false, confidence: 95 }),
  pipeline: refPipeline,
  parallel: refParallel,
  log: () => {},
});
assert.equal((await planRun({ target: 'phase r/p', signals: null })).acTable, null, 'plan mode returns acTable: null');

// THE STRUCTURAL POINT: the AC channel never reads a finding's severity. Zero
// findings, one FAIL criterion, and the outcome is still rework.
assert.equal(classifyOutcome({ acTable, codeReviews: [[]] }), 'rework',
  'an AC-table FAIL must force rework with zero findings — the channel bypasses severity');
assert.equal(classifyOutcome({ acTable: [{ criterion: 'AC1', status: 'PASS' }], codeReviews: [[]] }), 'reviewed');
assert.equal(acTableHasGap(null), false, 'an absent table is not a gap');
assert.equal(acTableHasGap([]), false, 'an empty table is not a gap');

// --- 10e: the always-on set is never empty ----------------------------------
for (const mode of ['code', 'plan']) {
  assert.ok(
    DIMENSIONS[mode].some((d) => !d.when),
    'mode "' + mode + '" must keep at least one `when`-less (always-on) dimension — a structural check, so a ' +
      'future refactor that gave every dimension a `when` fails HERE rather than at runtime'
  );
  assert.ok(selectDimensions(mode, {}).length > 0, 'explicit empty signals must still select the always-on set');
}
// The throw is REACHABLE: patch every plan dimension to be triggered-off and the
// guard must fire rather than returning an empty selection.
const savedPlan = DIMENSIONS.plan;
try {
  DIMENSIONS.plan = savedPlan.map((d) => ({ ...d, when: () => false }));
  assert.throws(() => selectDimensions('plan', {}), /always-on set must never be empty/,
    'selectDimensions must throw when nothing is selected');
} finally {
  DIMENSIONS.plan = savedPlan;
}

console.log('10c/10d/10e: unit-of-work separate + phase-scoped, ac/acTable unchanged, always-on never empty');
NODE_DIM_TEST

if run_node "$TMP/dimensions-test.mjs" "$LIB" >"$TMP/dim.out" 2>&1; then
    pass "10c/10d/10e: $(tail -1 "$TMP/dim.out")"
else
    cat "$TMP/dim.out" >&2
    fail "10c/10d/10e: the dimension invariants do not hold"
fi

# --- 10g. THE PROSE THE PHASE OWES ------------------------------------------
# `restraint` is always-on and always has been; CLAUDE.md and the schemas doc
# said otherwise for a while. And the reason code mode's ac+correctness are NOT
# merged must render into the SHIPPED code-review skills (and only those).
say "10g. restraint is listed as always-on, and the ac/correctness non-merge rationale renders code-side only"

if grep -rnF 'coherence/architectural-fit always-on' "$REPO_ROOT" \
    --include='*.md' --include='*.mjs' --include='*.js' >&2; then
    fail "10g: the stale always-on plan dimension list survives somewhere in the repo"
fi
grep -qF 'coherence/architectural-fit/restraint always-on' "$REPO_ROOT/CLAUDE.md" ||
    fail "10g: CLAUDE.md's review-pipeline bullet does not list restraint as always-on"
grep -qF 'coherence, architectural fit, restraint, and (for phases) unit-of-work' "$REPO_ROOT/CLAUDE.md" ||
    fail "10g: CLAUDE.md's Plan review section does not list restraint"
# shellcheck disable=SC2016  # a literal markdown table cell, backticks included
grep -qF '| `plan` | `coherence`, `architectural-fit`, `restraint` |' "$REPO_ROOT/docs/workflow-schemas.md" ||
    fail "10g: the workflow-schemas dimension table's plan row does not list restraint"
grep -qF 'scripts/verify-finder-collapse.sh' "$REPO_ROOT/CLAUDE.md" ||
    fail "10g: CLAUDE.md does not list the finder-collapse harness"
pass "10g: restraint is listed as always-on everywhere, and the stale string is gone"

CODE_RENDERS="$TEMPLATES/skill-review-cli.md $TEMPLATES/skill-review-mcp.md $REPO_ROOT/.claude/skills/rdm-review/SKILL.md"
PLAN_RENDERS="$TEMPLATES/skill-plan-review-cli.md $TEMPLATES/skill-plan-review-mcp.md $REPO_ROOT/.claude/skills/rdm-plan-review/SKILL.md"
for doc in $CODE_RENDERS; do
    grep -qF 'NOT merged into one always-on finder' "$doc" ||
        fail "10g: $doc is missing the ac/correctness non-merge rationale"
    grep -qF 'AC-review schema' "$doc" ||
        fail "10g: $doc's non-merge rationale does not name the AC-review schema"
done
for doc in $PLAN_RENDERS; do
    if grep -qF 'NOT merged into one always-on finder' "$doc"; then
        fail "10g: $doc (a PLAN render) carries the code-only non-merge rationale — mode isolation is broken"
    fi
    grep -qF 'concern: <coherence|architectural-fit|restraint|unit-of-work>' "$doc" ||
        fail "10g: $doc's finding template does not name all four plan concern keys"
    for key in coherence architectural-fit restraint unit-of-work; do
        grep -qF "$key" "$doc" || fail "10g: $doc never names the plan dimension key '$key'"
    done
done
pass "10g: the non-merge rationale renders into the three CODE skills and into no PLAN skill"

# --- 10h. PROJECT-AGNOSTIC PROSE ON THE RENDERED SURFACES --------------------
# AC2b (section 3) guards the runtime projection — the `focus` strings a finder
# agent actually receives. This guards the DOCUMENTATION projection: the `//|`
# spec prose gen-skill-review.sh renders into the four SHIPPED skill templates
# and the two dogfood copies. The two projections are independent (a `//|` line
# is inert at runtime; a `focus` string never reaches a template), so a
# regression could land in either one alone. `unsafe` is absent from the
# WHOLE-FILE token list only because each code skill's own hand-written
# step-2/step-3 prose (outside the `rdm:review-spec` markers) still names it as
# a diff trigger signal; that prose is not dimension prose and is owned
# elsewhere. The dimension prose itself is checked for it separately below.
say "10h: rendered review skills carry no project-specific convention prose"
AGNOSTIC_TOKENS='rdm-core|rdm-cli|rdm-server|anyhow|rustdoc|missing_docs|# Panics|# Safety|# Errors'
for doc in $CODE_RENDERS $PLAN_RENDERS; do
    if grep -nE "$AGNOSTIC_TOKENS" "$doc" >&2; then
        fail "10h: $doc carries project-specific convention prose (see the hits above)"
    fi
done

# Region-scoped half: inside the `rdm:review-spec` markers — the rendered
# dimension prose and nothing else — the language-specific idioms the security
# dimension used to carry must be gone too. This is the documentation-side
# mirror of AC2b's now-empty carve-out ledger.
# shellcheck disable=SC2016  # the backticks are literal prose in the searched idiom
REGION_TOKENS='`unsafe`|// SAFETY:'
for doc in $CODE_RENDERS $PLAN_RENDERS; do
    if awk '/rdm:review-spec:begin/{f=1} f; /rdm:review-spec:end/{f=0}' "$doc" |
        grep -nE "$REGION_TOKENS" >&2; then
        fail "10h: $doc's review-spec region carries a language-specific idiom (see the hits above)"
    fi
done
# Non-vacuity for the region-scoped half: plant the retired idiom back inside
# the region and prove the detector fires.
mkdir -p "$TMP/agnostic-region"
# shellcheck disable=SC2016  # the backticks are literal prose in the planted regression
sed 's/Injection, path traversal/Every `unsafe` block needs a `\/\/ SAFETY:` comment. Injection, path traversal/' \
    "$TEMPLATES/skill-review-cli.md" >"$TMP/agnostic-region/planted.md"
if diff -q "$TEMPLATES/skill-review-cli.md" "$TMP/agnostic-region/planted.md" >/dev/null 2>&1; then
    fail "10h: the planted region-idiom mutation did not apply — the anchor text moved"
fi
if awk '/rdm:review-spec:begin/{f=1} f; /rdm:review-spec:end/{f=0}' "$TMP/agnostic-region/planted.md" |
    grep -qE "$REGION_TOKENS"; then
    pass "10h: rendered dimension prose carries no language-specific idiom; the detector fires on a planted one"
else
    fail "10h: the region-scoped detector did NOT fire on a planted \`unsafe\`/SAFETY regression — the check is vacuous"
fi
# Non-vacuity: the same grep MUST fire on a planted copy.
AGDOC="$TMP/agnostic-doc"
mkdir -p "$AGDOC"
# shellcheck disable=SC2016  # the backticks are literal prose in the planted regression
sed 's/documentation the project/rustdoc `# Panics` documentation the project/' \
    "$TEMPLATES/skill-review-cli.md" >"$AGDOC/planted.md"
if diff -q "$TEMPLATES/skill-review-cli.md" "$AGDOC/planted.md" >/dev/null 2>&1; then
    fail "10h: the planted-prose mutation did not apply — the anchor text moved"
fi
if grep -qE "$AGNOSTIC_TOKENS" "$AGDOC/planted.md"; then
    pass "10h: rendered skills are project-agnostic; the detector fires on a planted regression"
else
    fail "10h: the detector did NOT fire on a planted rustdoc/# Panics regression — the check is vacuous"
fi

# --- 10f. PLANTED-MUTATION SELF-TESTS (non-vacuity for 10c/10d/10e) ----------
say "10f. Dimension-invariant mutation self-tests (prove 10c/10d/10e are not vacuous)"
DMUT="$TMP/dim-mut/.claude/workflows/lib"
mkdir -p "$DMUT"

# CONTROL: 10c/10d/10e must PASS against the real, unmutated file.
cp "$LIB" "$DMUT/review.mjs"
if run_node "$TMP/dimensions-test.mjs" "$DMUT/review.mjs" >/dev/null 2>&1; then
    pass "10f-control: the unmutated copy passes 10c/10d/10e"
else
    fail "10f-control: the unmutated copy FAILS 10c/10d/10e — every mutation below is vacuous"
fi

dim_mutate_and_expect_fail() {
    dtag="$1"
    ddesc="$2"
    dfn="$3"
    cp "$LIB" "$DMUT/review.mjs"
    "$dfn" || fail "10f-$dtag: could not plant the mutation ($ddesc)"
    if run_node "$TMP/dimensions-test.mjs" "$DMUT/review.mjs" >/dev/null 2>&1; then
        fail "10f-$dtag: 10c/10d/10e still passed after $ddesc — that assertion group is vacuous"
    fi
    pass "10f-$dtag: $ddesc flips a 10c/10d/10e assertion"
    cp "$LIB" "$DMUT/review.mjs"
}

# (c) unit-of-work loses its `when` predicate: it would then run on every unit
#     as an always-on dimension, and its findings would reach non-phase units.
dmut_uow_when() {
    sed "s|^      when: (s) => s.targetType === 'phase',\$|      // MUTANT: predicate removed|" \
        "$LIB" >"$DMUT/review.mjs"
    grep -q 'MUTANT' "$DMUT/review.mjs"
}
# shellcheck disable=SC2016  # `when` is prose, not a command substitution
dim_mutate_and_expect_fail c 'giving unit-of-work no `when` predicate' dmut_uow_when

# (c2) stripNonPhaseUnitOfWork stops filtering: a unit-of-work finding survives
#      on a task/roadmap unit, which is the silent-scoping-loss failure mode.
dmut_strip() {
    sed "s|^  return list.filter((f) => !(f \&\& f.concern === 'unit-of-work'));\$|  return list.slice(); // MUTANT|" \
        "$LIB" >"$DMUT/review.mjs"
    grep -q 'MUTANT' "$DMUT/review.mjs"
}
dim_mutate_and_expect_fail c2 'disabling the consumer-side unit-of-work scoping' dmut_strip

# (d) the ac dimension resolves FINDINGS_SCHEMA: the structured AC table would
#     never be produced, and the acceptance-criteria channel would collapse into
#     ordinary findings.
dmut_ac_schema() {
    sed 's|^          schema: isAcDimension ? AC_REVIEW_SCHEMA : FINDINGS_SCHEMA,$|          schema: FINDINGS_SCHEMA, // MUTANT|' \
        "$LIB" >"$DMUT/review.mjs"
    grep -q 'MUTANT' "$DMUT/review.mjs"
}
dim_mutate_and_expect_fail d 'making the ac dimension resolve FINDINGS_SCHEMA' dmut_ac_schema

# (e) the acTable capture is dropped: classifyOutcome's step-2 channel goes dark
#     and an unmet acceptance criterion stops forcing rework.
dmut_ac_capture() {
    sed 's|^            acTable = found.ac;$|            void found; // MUTANT|' "$LIB" >"$DMUT/review.mjs"
    grep -q 'MUTANT' "$DMUT/review.mjs"
}
dim_mutate_and_expect_fail e 'dropping the acTable capture' dmut_ac_capture

# (f) the empty-selection guard is removed: a mode whose dimensions all become
#     triggered would silently review NOTHING and report a clean result.
dmut_empty_guard() {
    sed 's|^  if (sel.length === 0) {$|  if (false) { // MUTANT|' "$LIB" >"$DMUT/review.mjs"
    grep -q 'MUTANT' "$DMUT/review.mjs"
}
dim_mutate_and_expect_fail f 'removing the non-empty always-on guard from selectDimensions' dmut_empty_guard

pass "10f: all five mutations flip a 10c/10d/10e assertion, and the control passes"

say "verify-workflow-review.sh: ALL GREEN"
