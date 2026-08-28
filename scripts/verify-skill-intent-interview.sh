#!/bin/sh
# Hermetic, prose-only regression for the intent-interview loop added to the
# shipped `rdm-roadmap` and `rdm-plan-review` skill templates
# (`review-gate-intent` roadmap, phase 2).
#
# Two capture steps are asserted:
#
#   AUTHORING — a bounded, human-in-the-loop interview in `rdm-roadmap`,
#   positioned BEFORE phase design, in both `skill-roadmap-{cli,mcp}.md`.
#
#   PREDATES  — the same capture offered from interactive `rdm-plan-review`
#   for a roadmap/task that predates the artifact, in both
#   `skill-plan-review-{cli,mcp}.md`, positioned strictly OUTSIDE the
#   generated `<!-- rdm:review-spec:begin -->` / `...:end -->` marker region
#   (that region is stamped by `gen-skill-review.sh` from
#   `.claude/workflows/lib/review.mjs`; hand-editing it fails that
#   generator's `--check` mode).
#
# This harness gates four things:
#
#   1. STATIC INVARIANTS on the four `rdm-core/src/templates/` sources —
#      marker presence, the predates-marker landing before the review-spec
#      begin marker, all five interview "bounds" present within each step's
#      own text block, and no forbidden rdm path literal / crate name
#      leaking into that prose (it ships to other repos).
#   2. PLANTED-REMOVAL SELF-TESTS proving the two marker detectors are not
#      vacuous — deleting each step's heading from a scratch copy makes its
#      own grep fail.
#   3. DYNAMIC RE-CHECK against the real `target/debug/rdm agent-config
#      claude --skills` emission (both cli and --mcp), re-running the same
#      marker/position/bound checks on the emitted tree.
#   4. SIBLING GATE — `verify-workflow-review.sh` (both --mode code and
#      --mode plan) stays green, since the plan-review template it renders
#      into is edited by this same phase, outside the generated region.
#
# Requires: a cargo-built rdm at target/debug/rdm (from this repo).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
RDM_BIN="$REPO_ROOT/target/debug/rdm"

TPL_ROADMAP_CLI="$REPO_ROOT/rdm-core/src/templates/skill-roadmap-cli.md"
TPL_ROADMAP_MCP="$REPO_ROOT/rdm-core/src/templates/skill-roadmap-mcp.md"
TPL_PLAN_REVIEW_CLI="$REPO_ROOT/rdm-core/src/templates/skill-plan-review-cli.md"
TPL_PLAN_REVIEW_MCP="$REPO_ROOT/rdm-core/src/templates/skill-plan-review-mcp.md"

AUTHORING_MARKER='Interview the operator'
DESIGN_MARKER='Design phases'
PREDATES_MARKER='Capture intent, if the target predates it'
REVIEW_SPEC_BEGIN='<!-- rdm:review-spec:begin'

# The five bounds every interview-prose step must state explicitly (AC4):
# a question cap, one question per turn, closed-form with a recommended
# default, early termination on an operator signal, and the `(not captured)`
# fallback rather than a fabricated intent.
BOUND_CAP='at most 3-5 questions'
BOUND_ONE_AT_A_TIME='one at a time'
BOUND_CLOSED_FORM='recommended default'
BOUND_TERMINATE='Terminate early'
BOUND_NOT_CAPTURED='(not captured)'

# Forbidden literals: this prose ships to other repos and must name no rdm
# path or crate.
FORBIDDEN_LITERALS='rdm-core|rdm-cli|rdm-server|rdm-core/src|\.claude/workflows|crate'

say() { printf '\n\033[1;34m==>\033[0m %s\n' "$*"; }
fail() {
    printf '\n\033[1;31m[FAIL]\033[0m %s\n' "$*" >&2
    exit 1
}
pass() { printf '\033[1;32m[ok]\033[0m %s\n' "$*"; }

[ -x "$RDM_BIN" ] || fail "$RDM_BIN not found or not executable — run 'cargo build' first."
for f in "$TPL_ROADMAP_CLI" "$TPL_ROADMAP_MCP" "$TPL_PLAN_REVIEW_CLI" "$TPL_PLAN_REVIEW_MCP"; do
    [ -f "$f" ] || fail "template not found: $f"
done

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

# Extracts the text block for one step, from its own marker line up to (but
# not including) the step's boundary. Used to scope the bound-phrase and
# forbidden-literal checks to the step that actually needs them, rather than
# the whole file. $3 selects the boundary, since the two skills structure
# their steps differently:
#
#   numbered — the roadmap templates' steps are flat top-level numbered items
#              (`N. **Title**`) under one `## Steps` list, with no nested
#              numbered sub-lists inside a step's own body, so the next
#              sibling `N. **` item (or a `##` heading) safely bounds it.
#   heading  — the plan-review templates' steps are `### N. Title`
#              sub-headings, each containing its OWN nested numbered
#              sub-list (`1. **...**`, `2. **...**`, ...); bounding on the
#              next `N. **` would stop after the step's first sub-item, so
#              only a `##`/`###` heading closes the block.
extract_step_block() {
    file="$1"
    marker="$2"
    boundary="$3"
    if [ "$boundary" = "heading" ]; then
        awk -v marker="$marker" '
            index($0, marker) { p=1; print; next }
            p && /^#{2,3} / { exit }
            p { print }
        ' "$file"
    else
        awk -v marker="$marker" '
            index($0, marker) { p=1; print; next }
            p && /^[0-9]+\. \*\*/ { exit }
            p && /^#{2,3} / { exit }
            p { print }
        ' "$file"
    fi
}

assert_bounds_and_forbidden() {
    # Runs the five bound-phrase checks and the forbidden-literal check
    # against a step's extracted text block. $1 = label, $2 = block file.
    label="$1"
    block="$2"
    [ -s "$block" ] || fail "$label: could not extract step block"
    for bound in "$BOUND_CAP" "$BOUND_ONE_AT_A_TIME" "$BOUND_CLOSED_FORM" "$BOUND_TERMINATE" "$BOUND_NOT_CAPTURED"; do
        grep -qF "$bound" "$block" || fail "$label: missing bound phrase: $bound"
    done
    if grep -Eq "$FORBIDDEN_LITERALS" "$block"; then
        fail "$label: forbidden rdm path literal or crate name found in interview prose"
    fi
}

# --- 1. STATIC INVARIANTS ON THE SHIPPED TEMPLATES ----------------------------
say "1. Static invariants on rdm-core/src/templates/*"

for f in "$TPL_ROADMAP_CLI" "$TPL_ROADMAP_MCP"; do
    grep -qF "$AUTHORING_MARKER" "$f" || fail "$f: missing authoring interview step marker '$AUTHORING_MARKER'"
done
pass "authoring interview step marker present in skill-roadmap-{cli,mcp}.md"

for f in "$TPL_PLAN_REVIEW_CLI" "$TPL_PLAN_REVIEW_MCP"; do
    grep -qF "$PREDATES_MARKER" "$f" || fail "$f: missing predates-the-artifact capture step marker '$PREDATES_MARKER'"
done
pass "predates-the-artifact capture step marker present in skill-plan-review-{cli,mcp}.md"

say "1a. Step position: interview runs BEFORE phase design"
for f in "$TPL_ROADMAP_CLI" "$TPL_ROADMAP_MCP"; do
    interview_line=$(grep -n -F "$AUTHORING_MARKER" "$f" | head -1 | cut -d: -f1)
    design_line=$(grep -n -F "$DESIGN_MARKER" "$f" | head -1 | cut -d: -f1)
    [ -n "$interview_line" ] || fail "$f: interview marker line not found"
    [ -n "$design_line" ] || fail "$f: '$DESIGN_MARKER' line not found"
    [ "$interview_line" -lt "$design_line" ] ||
        fail "$f: interview step (line $interview_line) must precede '$DESIGN_MARKER' (line $design_line)"
done
pass "interview step precedes phase design in both skill-roadmap-{cli,mcp}.md"

say "1b. Marker region: predates-capture lands OUTSIDE (before) rdm:review-spec"
for f in "$TPL_PLAN_REVIEW_CLI" "$TPL_PLAN_REVIEW_MCP"; do
    predates_line=$(grep -n -F "$PREDATES_MARKER" "$f" | head -1 | cut -d: -f1)
    begin_line=$(grep -n -F "$REVIEW_SPEC_BEGIN" "$f" | head -1 | cut -d: -f1)
    [ -n "$predates_line" ] || fail "$f: predates-capture marker line not found"
    [ -n "$begin_line" ] || fail "$f: '$REVIEW_SPEC_BEGIN' marker line not found"
    [ "$predates_line" -lt "$begin_line" ] ||
        fail "$f: predates-capture step (line $predates_line) must land before the generated review-spec block (line $begin_line)"
done
pass "predates-capture step lands strictly outside the rdm:review-spec marker region in both files"

say "1c. All five interview bounds stated, and no forbidden rdm path literal / crate name"
extract_step_block "$TPL_ROADMAP_CLI" "$AUTHORING_MARKER" numbered >"$TMP/roadmap-cli.block"
extract_step_block "$TPL_ROADMAP_MCP" "$AUTHORING_MARKER" numbered >"$TMP/roadmap-mcp.block"
extract_step_block "$TPL_PLAN_REVIEW_CLI" "$PREDATES_MARKER" heading >"$TMP/plan-review-cli.block"
extract_step_block "$TPL_PLAN_REVIEW_MCP" "$PREDATES_MARKER" heading >"$TMP/plan-review-mcp.block"

assert_bounds_and_forbidden "skill-roadmap-cli.md" "$TMP/roadmap-cli.block"
assert_bounds_and_forbidden "skill-roadmap-mcp.md" "$TMP/roadmap-mcp.block"
assert_bounds_and_forbidden "skill-plan-review-cli.md" "$TMP/plan-review-cli.block"
assert_bounds_and_forbidden "skill-plan-review-mcp.md" "$TMP/plan-review-mcp.block"
pass "all five bounds present, no forbidden literals, in all four extracted step blocks"

say "1d. AskUserQuestion granted in rdm-roadmap's allowed-tools (both variants)"
for f in "$TPL_ROADMAP_CLI" "$TPL_ROADMAP_MCP"; do
    awk '/^---$/{n++; next} n==1' "$f" | grep -qF 'AskUserQuestion' ||
        fail "$f: allowed-tools frontmatter must grant AskUserQuestion"
done
pass "AskUserQuestion present in rdm-roadmap allowed-tools (cli + mcp)"

# --- 2. PLANTED-REMOVAL SELF-TESTS --------------------------------------------
say "2. Planted-removal self-tests (proving the marker detectors are not vacuous)"

sed "/${AUTHORING_MARKER}/d" "$TPL_ROADMAP_CLI" >"$TMP/roadmap-cli.mutant"
if grep -qF "$AUTHORING_MARKER" "$TMP/roadmap-cli.mutant"; then
    fail "authoring-marker detector broken — planted removal did not strip the marker from the scratch copy"
fi
pass "authoring-marker detector fires on a scratch copy with the step heading removed"

sed "/${PREDATES_MARKER}/d" "$TPL_PLAN_REVIEW_CLI" >"$TMP/plan-review-cli.mutant"
if grep -qF "$PREDATES_MARKER" "$TMP/plan-review-cli.mutant"; then
    fail "predates-marker detector broken — planted removal did not strip the marker from the scratch copy"
fi
pass "predates-marker detector fires on a scratch copy with the step heading removed"

# --- 3. DYNAMIC RE-CHECK AGAINST THE REAL EMITTED TREE ------------------------
say "3. Dynamic re-check against 'rdm agent-config claude --skills' (cli and --mcp)"

"$RDM_BIN" agent-config claude --skills --project intent-interview-check --out "$TMP/cli-emit" >/dev/null
"$RDM_BIN" agent-config claude --skills --mcp --project intent-interview-check --out "$TMP/mcp-emit" >/dev/null

EMIT_ROADMAP_CLI="$TMP/cli-emit/.claude/skills/rdm-roadmap/SKILL.md"
EMIT_ROADMAP_MCP="$TMP/mcp-emit/.claude/skills/rdm-roadmap/SKILL.md"
EMIT_PLAN_REVIEW_CLI="$TMP/cli-emit/.claude/skills/rdm-plan-review/SKILL.md"
EMIT_PLAN_REVIEW_MCP="$TMP/mcp-emit/.claude/skills/rdm-plan-review/SKILL.md"
for f in "$EMIT_ROADMAP_CLI" "$EMIT_ROADMAP_MCP" "$EMIT_PLAN_REVIEW_CLI" "$EMIT_PLAN_REVIEW_MCP"; do
    [ -f "$f" ] || fail "expected emitted skill file not found: $f"
done

for f in "$EMIT_ROADMAP_CLI" "$EMIT_ROADMAP_MCP"; do
    interview_line=$(grep -n -F "$AUTHORING_MARKER" "$f" | head -1 | cut -d: -f1)
    design_line=$(grep -n -F "$DESIGN_MARKER" "$f" | head -1 | cut -d: -f1)
    [ -n "$interview_line" ] || fail "emitted $f: interview marker missing"
    [ -n "$design_line" ] || fail "emitted $f: '$DESIGN_MARKER' missing"
    [ "$interview_line" -lt "$design_line" ] ||
        fail "emitted $f: interview step must precede phase design"
done
pass "emitted skill-roadmap SKILL.md (cli + mcp) carries the interview step before phase design"

for f in "$EMIT_PLAN_REVIEW_CLI" "$EMIT_PLAN_REVIEW_MCP"; do
    predates_line=$(grep -n -F "$PREDATES_MARKER" "$f" | head -1 | cut -d: -f1)
    begin_line=$(grep -n -F "$REVIEW_SPEC_BEGIN" "$f" | head -1 | cut -d: -f1)
    [ -n "$predates_line" ] || fail "emitted $f: predates-capture marker missing"
    [ -n "$begin_line" ] || fail "emitted $f: '$REVIEW_SPEC_BEGIN' marker missing"
    [ "$predates_line" -lt "$begin_line" ] ||
        fail "emitted $f: predates-capture step must land outside the generated review-spec block"
done
pass "emitted skill-plan-review SKILL.md (cli + mcp) carries the predates-capture step outside rdm:review-spec"

extract_step_block "$EMIT_ROADMAP_CLI" "$AUTHORING_MARKER" numbered >"$TMP/emit-roadmap-cli.block"
extract_step_block "$EMIT_ROADMAP_MCP" "$AUTHORING_MARKER" numbered >"$TMP/emit-roadmap-mcp.block"
extract_step_block "$EMIT_PLAN_REVIEW_CLI" "$PREDATES_MARKER" heading >"$TMP/emit-plan-review-cli.block"
extract_step_block "$EMIT_PLAN_REVIEW_MCP" "$PREDATES_MARKER" heading >"$TMP/emit-plan-review-mcp.block"
assert_bounds_and_forbidden "emitted skill-roadmap-cli SKILL.md" "$TMP/emit-roadmap-cli.block"
assert_bounds_and_forbidden "emitted skill-roadmap-mcp SKILL.md" "$TMP/emit-roadmap-mcp.block"
assert_bounds_and_forbidden "emitted skill-plan-review-cli SKILL.md" "$TMP/emit-plan-review-cli.block"
assert_bounds_and_forbidden "emitted skill-plan-review-mcp SKILL.md" "$TMP/emit-plan-review-mcp.block"
pass "emitted tree carries all five bounds, no forbidden literals, in all four SKILL.md files"

# --- 4. SIBLING GATE -----------------------------------------------------------
say "4. Sibling gate: verify-workflow-review.sh (both --mode code and --mode plan)"
if bash "$SCRIPT_DIR/verify-workflow-review.sh" >/dev/null 2>&1; then
    pass "verify-workflow-review.sh still green"
else
    bash "$SCRIPT_DIR/verify-workflow-review.sh" >&2 || true
    fail "verify-workflow-review.sh regressed"
fi

say "verify-skill-intent-interview.sh: ALL GREEN"
