#!/bin/sh
# Hermetic regression for the rdm-do `--auto` phase-flow -> dispatch-phase wiring.
#
# workflow-orchestration phase 4 wires the phase-flow branch of `--auto` in
# `.claude/skills/rdm-do/SKILL.md` into the `dispatch-phase` Workflow instead of
# re-implementing plan -> plan-review -> implement -> code-review in prose
# (interactive `rdm-do` is untouched; `--auto --task` routes in the same way). This is a
# dogfood-only, local edit to the skill file — it is NOT propagated to the
# distributed `rdm-core/src/templates/skill-do-cli.md` / `skill-do-mcp.md`
# templates, which stay prose-only. This harness gates three things:
#
#   1. STATIC INVARIANTS — SKILL.md's frontmatter lists the `Workflow` tool; the
#      `## Auto phase dispatch` section references `dispatch-phase`; the
#      referenced workflow file exists and is named `dispatch-phase`; the
#      OUTCOME -> status mapping (`--status reviewed`, `--status blocked`,
#      `[code]`, `[plan]`) is present; the interactive plan-mode path
#      (`EnterPlanMode`/`ExitPlanMode`) is preserved; the `--auto --task`
#      flow is wired into the Workflow (`{ task: <slug> }` + its OUTCOME ->
#      status map) with no stale prose-path claims; the dogfood note is present; and the
#      distributed templates stay prose-only (do NOT mention `dispatch-phase`),
#      with a planted-mutation self-test proving that last detector fires.
#   2. DYNAMIC OUTCOME CONTRACT — against the real binary in a hermetic temp
#      plan+source repo: the exact `phase update` command shapes SKILL.md
#      documents for each OUTCOME (`reviewed` / `rework` / `escalated`) land the
#      status + blocked_reason the skill promises, read back via
#      `phase show --format json`.
#   3. SIBLING GATE — `verify-workflow-dispatch.sh` (the workflow this phase
#      routes `--auto` into) stays green.
#
# No Rust/Cargo changes are made or required by this harness; it only exercises
# the already-built `target/debug/rdm` binary and greps the skill/workflow
# source files.
#
# Requires: a cargo-built rdm at target/debug/rdm (from this repo).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
RDM_BIN="$REPO_ROOT/target/debug/rdm"
SKILL="$REPO_ROOT/.claude/skills/rdm-do/SKILL.md"
DISPATCH_WF="$REPO_ROOT/.claude/workflows/dispatch-phase.js"
TEMPLATE_CLI="$REPO_ROOT/rdm-core/src/templates/skill-do-cli.md"
TEMPLATE_MCP="$REPO_ROOT/rdm-core/src/templates/skill-do-mcp.md"

# Clear rdm-related env vars inherited from the caller's shell for hermeticity.
unset RDM_ROOT RDM_PROJECT RDM_STAGE RDM_FORMAT RDM_PLAN_REPO RDM_PLAN_REPO_TOKEN RDM_PLAN_REPO_PATH 2>/dev/null || true

say() { printf '\n\033[1;34m==>\033[0m %s\n' "$*"; }
fail() {
    printf '\n\033[1;31m[FAIL]\033[0m %s\n' "$*" >&2
    exit 1
}
pass() { printf '\033[1;32m[ok]\033[0m %s\n' "$*"; }

[ -x "$RDM_BIN" ] || fail "$RDM_BIN not found or not executable — run 'cargo build' first."
[ -f "$SKILL" ] || fail "skill file not found: $SKILL"
[ -f "$DISPATCH_WF" ] || fail "dispatch-phase workflow not found: $DISPATCH_WF"
[ -f "$TEMPLATE_CLI" ] || fail "distributed template not found: $TEMPLATE_CLI"
[ -f "$TEMPLATE_MCP" ] || fail "distributed template not found: $TEMPLATE_MCP"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

# --- 1. STATIC INVARIANTS -----------------------------------------------------
say "1. Static invariants on .claude/skills/rdm-do/SKILL.md"

# Frontmatter (between the two `---` fences) lists the Workflow tool.
awk '/^---$/{n++; next} n==1' "$SKILL" >"$TMP/frontmatter"
grep -q -- '- Workflow' "$TMP/frontmatter" || fail "SKILL.md frontmatter must list '- Workflow' in allowed-tools"
pass "frontmatter lists the Workflow tool"

grep -q '^## Auto phase dispatch' "$SKILL" || fail "missing '## Auto phase dispatch' section header"
pass "'## Auto phase dispatch' section header present"

# Text after that header must reference dispatch-phase.
awk '/^## Auto phase dispatch/{p=1} p' "$SKILL" >"$TMP/auto-section"
grep -q 'dispatch-phase' "$TMP/auto-section" || fail "'## Auto phase dispatch' section must reference 'dispatch-phase'"
pass "'## Auto phase dispatch' section references dispatch-phase"

grep -q "name: 'dispatch-phase'" "$DISPATCH_WF" || fail "the referenced workflow is not named 'dispatch-phase'"
pass "referenced workflow exists and is named 'dispatch-phase'"

# OUTCOME-mapping commands present, and — critically — each outcome name is
# bound to its correct tag/status ON THE SAME BULLET LINE. Independent
# whole-file existence greps would let a future edit transpose the tags
# (rework <-> escalated) and still pass, since both tags stay present somewhere;
# anchoring the outcome name to its tag closes that false-green gap.
# The backticks below are literal grep-pattern characters (SKILL.md quotes the
# outcome names in backticks), not shell command substitution — SC2016 N/A.
# shellcheck disable=SC2016
grep -qE '`reviewed`.*--status reviewed' "$SKILL" ||
    fail "the 'reviewed' OUTCOME must map to '--status reviewed' on its bullet in SKILL.md"
# shellcheck disable=SC2016
grep -qE '`rework`.*--status blocked.*\[code\]' "$SKILL" ||
    fail "the 'rework' OUTCOME must map to '--status blocked' with a [code] reason on its bullet in SKILL.md"
# shellcheck disable=SC2016
grep -qE '`escalated`.*--status blocked.*\[plan\]' "$SKILL" ||
    fail "the 'escalated' OUTCOME must map to '--status blocked' with a [plan] reason on its bullet in SKILL.md"
pass "OUTCOME -> status mapping present and correctly paired (reviewed->reviewed, rework->blocked [code], escalated->blocked [plan])"

# Interactive path preserved.
grep -q 'EnterPlanMode' "$SKILL" || fail "SKILL.md must still reference EnterPlanMode (interactive path preserved)"
grep -q 'ExitPlanMode' "$SKILL" || fail "SKILL.md must still reference ExitPlanMode (interactive path preserved)"
pass "interactive plan-mode path (EnterPlanMode/ExitPlanMode) preserved"

# Task-flow wiring: --auto --task routes into the Workflow, not the prose loop.
grep -q '## Auto task dispatch' "$SKILL" ||
    fail "SKILL.md must document an '## Auto task dispatch' section"
grep -qF '{ task: <slug> }' "$SKILL" ||
    fail "SKILL.md must state the task flow invokes the Workflow with '{ task: <slug> }'"
grep -q 'rdm task update <slug> --status reviewed' "$SKILL" ||
    fail "SKILL.md must map the task 'reviewed' OUTCOME to 'task update --status reviewed'"
grep -qF 'task update <slug> --status blocked --reason "[code]' "$SKILL" ||
    fail "SKILL.md must map the task 'rework' OUTCOME to blocked with a [code] reason"
grep -qF 'task update <slug> --status blocked --reason "[plan]' "$SKILL" ||
    fail "SKILL.md must map the task 'escalated' OUTCOME to blocked with a [plan] reason"
pass "task-flow wiring (Workflow '{ task: <slug> }' + OUTCOME -> status map) stated"

# The task flow must NOT still be described as the deferred prose path.
if grep -qF 'still runs the existing prose steps 6-11' "$SKILL"; then
    fail "SKILL.md still describes '--auto --task' as running the prose loop"
fi
if grep -qF 'interactive/task-flow prose path' "$SKILL"; then
    fail "SKILL.md step 6 still calls steps 6-11 the task-flow prose path"
fi
pass "no stale '--auto --task is prose' assertions remain"

# Dogfood note present.
grep -qi 'dogfood' "$SKILL" || fail "SKILL.md must record the dogfood-only nature of this edit"
grep -q 'skill-do-cli.md' "$SKILL" || fail "SKILL.md must name the un-propagated distributed template skill-do-cli.md"
pass "dogfood-only note present, naming the distributed template"

# AC2 positive proof: distributed templates stay prose-only (no dispatch-phase).
if grep -q 'dispatch-phase' "$TEMPLATE_CLI"; then
    fail "AC2: $TEMPLATE_CLI must stay prose-only — it must not mention dispatch-phase"
fi
if grep -q 'dispatch-phase' "$TEMPLATE_MCP"; then
    fail "AC2: $TEMPLATE_MCP must stay prose-only — it must not mention dispatch-phase"
fi
pass "distributed templates (skill-do-cli.md, skill-do-mcp.md) stay prose-only"

# Self-test: prove the prose-only detector is not a no-op — inject dispatch-phase
# into a scratch copy of the template and assert the detector fires there.
cp "$TEMPLATE_CLI" "$TMP/template-cli.scratch"
printf '\nSee dispatch-phase for details.\n' >>"$TMP/template-cli.scratch"
if ! grep -q 'dispatch-phase' "$TMP/template-cli.scratch"; then
    fail "prose-only detector broken — planted 'dispatch-phase' mention was not found on the scratch copy"
fi
pass "prose-only detector fires on a planted 'dispatch-phase' mention (self-test)"

# --- 2. DYNAMIC OUTCOME CONTRACT ----------------------------------------------
say "2. Dynamic OUTCOME -> status contract against the real binary"

# Hermetic HOME + XDG + git identity so neither rdm nor git touch the real
# developer/CI environment (matches the sibling worktree-review-loop harness).
export HOME="$TMP/home"
export XDG_CONFIG_HOME="$TMP/xdg-config"
export XDG_DATA_HOME="$TMP/xdg-data"
export XDG_STATE_HOME="$TMP/xdg-state"
mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_STATE_HOME"

export GIT_AUTHOR_NAME="verify-bot"
export GIT_AUTHOR_EMAIL="verify@example.invalid"
export GIT_COMMITTER_NAME="verify-bot"
export GIT_COMMITTER_EMAIL="verify@example.invalid"

PLAN="$TMP/plan"

rdm_plan() (
    RDM_ROOT="$PLAN" "$RDM_BIN" "$@"
)

say "2a. Seeding a hermetic plan repo (project 'verify') with roadmap 'rm' and 3 phases"
rdm_plan init --default-project verify >/dev/null
rdm_plan roadmap create rm --title "RM" --body "Dynamic OUTCOME contract regression roadmap." \
    --no-edit --project verify >/dev/null
rdm_plan phase create a --title "Phase A" --number 1 --body "Phase A." \
    --no-edit --roadmap rm --project verify >/dev/null
rdm_plan phase create b --title "Phase B" --number 2 --body "Phase B." \
    --no-edit --roadmap rm --project verify >/dev/null
rdm_plan phase create c --title "Phase C" --number 3 --body "Phase C." \
    --no-edit --roadmap rm --project verify >/dev/null
for stem in phase-1-a phase-2-b phase-3-c; do
    rdm_plan phase update "$stem" --status in-progress --no-edit --roadmap rm --project verify >/dev/null
done
rdm_plan commit -m "chore(plan): seed rm roadmap with 3 in-progress phases" >/dev/null
pass "seeded roadmap rm with phase-1-a/phase-2-b/phase-3-c, all in-progress"

say "2b. reviewed -> phase update --status reviewed"
rdm_plan phase update phase-1-a --status reviewed --no-edit --roadmap rm --project verify >/dev/null
OUT_A=$(rdm_plan phase show phase-1-a --roadmap rm --project verify --format json --no-body)
printf '%s' "$OUT_A" | grep -qF '"status": "reviewed"' || fail "phase-1-a expected status reviewed, got: $OUT_A"
pass "reviewed OUTCOME lands --status reviewed"

say "2c. rework -> phase update --status blocked --reason '[code] ...'"
rdm_plan phase update phase-2-b --status blocked --reason "[code] rework budget exhausted: test" \
    --no-edit --roadmap rm --project verify >/dev/null
OUT_B=$(rdm_plan phase show phase-2-b --roadmap rm --project verify --format json --no-body)
printf '%s' "$OUT_B" | grep -qF '"status": "blocked"' || fail "phase-2-b expected status blocked, got: $OUT_B"
printf '%s' "$OUT_B" | grep -qF '"blocked_reason": "[code]' || fail "phase-2-b blocked_reason must start with [code], got: $OUT_B"
pass "rework OUTCOME lands --status blocked with a [code]-tagged reason"

say "2d. escalated -> phase update --status blocked --reason '[plan] ...'"
rdm_plan phase update phase-3-c --status blocked --reason "[plan] ambiguous AC: test" \
    --no-edit --roadmap rm --project verify >/dev/null
OUT_C=$(rdm_plan phase show phase-3-c --roadmap rm --project verify --format json --no-body)
printf '%s' "$OUT_C" | grep -qF '"status": "blocked"' || fail "phase-3-c expected status blocked, got: $OUT_C"
printf '%s' "$OUT_C" | grep -qF '"blocked_reason": "[plan]' || fail "phase-3-c blocked_reason must start with [plan], got: $OUT_C"
pass "escalated OUTCOME lands --status blocked with a [plan]-tagged reason"

rdm_plan commit -m "chore(plan): land reviewed/rework/escalated OUTCOME contract" >/dev/null

# --- 3. SIBLING GATE -----------------------------------------------------------
say "3. Sibling gate: verify-workflow-dispatch.sh (the workflow --auto routes into)"
if bash "$SCRIPT_DIR/verify-workflow-dispatch.sh" >/dev/null 2>&1; then
    pass "verify-workflow-dispatch.sh still green"
else
    bash "$SCRIPT_DIR/verify-workflow-dispatch.sh" >&2 || true
    fail "verify-workflow-dispatch.sh regressed"
fi

say "verify-workflow-do-auto.sh: ALL GREEN"
