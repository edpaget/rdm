#!/usr/bin/env bash
#
# verify-workflow-do-auto-task.sh — the TASK-flow twin of
# verify-workflow-do-auto.sh.
#
# `rdm-do --auto --task <slug>` routes into the `dispatch-phase` Workflow in task
# mode (`{ task: <slug> }`) instead of the prose plan/implement/review loop. This
# harness gates that wiring end to end:
#
#   1. STATIC INVARIANTS — dispatch-phase.js really accepts `{ task }`, fetches
#      via `rdm task show` (not `phase show --roadmap`), pins tasks to the fixed
#      `medium` tier, uses the per-task `task/<slug>` worktree, and emits a
#      task-keyed OUTCOME — each with a planted-mutation self-test where the
#      detector could otherwise go false-green.
#   2. PURE OUTCOME SHAPE — `buildTaskOutcome` is driven in Node with fabricated
#      findings, asserting the task-keyed OUTCOME contract and that it carries
#      no roadmap/phase keys. (The exhaustive branch coverage lives in
#      verify-workflow-dispatch.sh's BEHAVIOR section; this is the shape gate.)
#   3. DYNAMIC OUTCOME CONTRACT — against the real binary in a hermetic temp plan
#      repo: the exact `task update` command shapes SKILL.md documents for each
#      OUTCOME (`reviewed` / `rework` / `escalated`) land the status + reason the
#      skill promises, read back via `task show --format json`, including that
#      the reason survives a later status change.
#
# No Rust/Cargo changes are made or required; it exercises the already-built
# target/debug/rdm binary and greps the skill/workflow source files.
#
# Requires: a cargo-built rdm at target/debug/rdm (from this repo).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
RDM_BIN="$REPO_ROOT/target/debug/rdm"
SKILL="$REPO_ROOT/.claude/skills/rdm-do/SKILL.md"
DISPATCH_WF="$REPO_ROOT/.claude/workflows/dispatch-phase.js"
LIB="$REPO_ROOT/.claude/workflows/lib/dispatch-phase.mjs"

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
[ -f "$LIB" ] || fail "dispatch-phase lib not found: $LIB"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

run_node() {
    node "$@"
}

# --- 1. STATIC INVARIANTS -----------------------------------------------------
say "1. Static invariants: dispatch-phase.js really implements task mode"

grep -qF 'dispatchArgs.task' "$DISPATCH_WF" ||
    fail "dispatch-phase.js must read a task slug from args (dispatchArgs.task)"
pass "task mode is selected from args.task"

grep -qF 'rdm task show ' "$DISPATCH_WF" ||
    fail "task mode must fetch via 'rdm task show <slug>' (no --roadmap)"
pass "task metadata is fetched via 'rdm task show'"

grep -q "label: 'fetch:task-meta'" "$DISPATCH_WF" ||
    fail "task mode must use a distinct 'fetch:task-meta' agent label"
pass "task fetch uses its own agent label"

# A whole-file grep for the literal is satisfied by the `worktreeRef` assignment
# alone; it does NOT prove the ref is threaded into the implement call sites or
# into reviewTarget. Assert the actual usages.
assert_worktree_threading() {
    grep -qF "const worktreeRef = isTask ? 'task/' + taskSlug : roadmapSlug" "$1" || return 1
    # Both implement CALL SITES (initial + rework) must take the ref, not the slug.
    # Anchored on the `agent(` wrapper so the function DEFINITION line, which also
    # contains `buildImplementPrompt(worktreeRef,`, is not miscounted as a call.
    [ "$(grep -cF 'agent(buildImplementPrompt(worktreeRef,' "$1")" -eq 2 ] || return 1
    grep -qF 'agent(buildImplementPrompt(roadmapSlug,' "$1" && return 1
    # The code-review target must be task-aware too.
    grep -qF "const reviewTarget = isTask ? 'task/' + taskSlug" "$1" || return 1
    return 0
}
assert_worktree_threading "$DISPATCH_WF" ||
    fail "worktreeRef must be threaded into BOTH buildImplementPrompt call sites, and reviewTarget must be task-aware"
pass "worktree ref and review target are threaded through task mode"

# Self-test: revert the call sites to the roadmap slug and prove the detector fires.
sed 's/buildImplementPrompt(worktreeRef,/buildImplementPrompt(roadmapSlug,/g' "$DISPATCH_WF" >"$TMP/wt-mutant"
if assert_worktree_threading "$TMP/wt-mutant"; then
    fail "detector missed implement call sites reverted to roadmapSlug"
fi
pass "worktree-threading detector fires on a planted roadmapSlug revert"

# NOTE: a bare whole-file `grep buildTaskOutcome` is NOT sufficient here — the
# function DEFINITION lives inside the byte-copied dispatch-outcome block, so it
# is present regardless of whether the driver ever calls it. Extract the driver's
# itemOutcome() body and assert the task branch actually routes to it.
extract_item_outcome() {
    awk '/^function itemOutcome\(/{p=1} p{print} p&&/^}$/{exit}' "$1"
}
extract_item_outcome "$DISPATCH_WF" >"$TMP/itemoutcome"
[ -s "$TMP/itemoutcome" ] || fail "could not extract itemOutcome() from dispatch-phase.js"
# Assert the EXACT branch form. A looser `grep isTask` would still pass against a
# short-circuit mutation like `if (false && isTask)`, which silently routes task
# mode to buildOutcome; pinning the literal closes that.
assert_item_outcome_wiring() {
    grep -qF 'if (isTask) {' "$1" || return 1
    grep -qF 'return buildTaskOutcome(' "$1" || return 1
    grep -qF 'return buildOutcome(' "$1" || return 1
    return 0
}
assert_item_outcome_wiring "$TMP/itemoutcome" ||
    fail "itemOutcome() must branch on 'if (isTask) {' and return buildTaskOutcome / buildOutcome respectively"
pass "itemOutcome() routes task mode to buildTaskOutcome and phase mode to buildOutcome"

# Self-tests: replay BOTH real mutation shapes and prove the detector fires.
#   (a) short-circuit the branch so task mode falls through to buildOutcome
sed 's/if (isTask) {/if (false \&\& isTask) {/' "$TMP/itemoutcome" >"$TMP/mutant-a"
if assert_item_outcome_wiring "$TMP/mutant-a"; then
    fail "detector missed a short-circuited 'if (false && isTask)' branch"
fi
#   (b) delete the task call entirely, leaving buildTaskOutcome a dead definition
sed 's/return buildTaskOutcome(/return buildOutcome(/' "$TMP/itemoutcome" >"$TMP/mutant-b"
if assert_item_outcome_wiring "$TMP/mutant-b"; then
    fail "detector missed a removed buildTaskOutcome call site"
fi
pass "routing detector fires on both short-circuited and removed task-branch mutations"

# Tasks carry no estimate, so the tier must be pinned — otherwise the `large`
# gate-tightening could be reached with an attacker-controlled/absent field.
grep -qF "isTask ? 'medium'" "$DISPATCH_WF" ||
    fail "task mode must pin the tier to the fixed 'medium'"
pass "tasks are pinned to the fixed medium tier"

# The --plan-only early return is hand-built (NOT routed through buildOutcome),
# so it needs its own task-shaped identifier or it silently emits roadmap/phase.
awk '/--plan-only: the plan gate passed/{p=1} p&&/^}/{print; exit} p' "$DISPATCH_WF" >"$TMP/planonly"
grep -qF 'task: taskSlug' "$TMP/planonly" ||
    fail "the --plan-only early return must emit a task-keyed OUTCOME in task mode"
pass "--plan-only returns a task-keyed OUTCOME in task mode"

say "1b. Static detectors fire on planted mutations (self-test)"
PLANTED="$TMP/planted.js"
sed "s/isTask ? 'medium'/isTask ? 'large'/" "$DISPATCH_WF" >"$PLANTED"
if grep -qF "isTask ? 'medium'" "$PLANTED"; then
    fail "self-test setup failed: planted tier mutation did not apply"
fi
pass "tier detector would fire on a planted 'large' tier"

sed "s/'task\/' + taskSlug/roadmapSlug/g" "$DISPATCH_WF" >"$PLANTED"
if grep -qF "'task/' + taskSlug" "$PLANTED"; then
    fail "self-test setup failed: planted worktree mutation did not apply"
fi
pass "worktree detector would fire on a planted per-roadmap worktree ref"

# --- 2. PURE OUTCOME SHAPE ----------------------------------------------------
say "2. buildTaskOutcome emits the task-keyed OUTCOME contract"

cat >"$TMP/shape.mjs" <<'NODE_TEST'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const mod = await import(pathToFileURL(process.argv[2]).href);
const { buildTaskOutcome } = mod;
assert.equal(typeof buildTaskOutcome, 'function', 'buildTaskOutcome is exported');

const SHAPE = ['findings', 'outcome', 'summary', 'task'];
const B = (id) => ({ id, concern: 'x', severity: 'blocking', confidence: 90, what_fails: id });

const clean = buildTaskOutcome({ task: 't', planFindings: [], codeFindings: [], tier: 'medium' });
assert.equal(clean.task, 't', 'OUTCOME is keyed by the task slug');
assert.deepEqual(Object.keys(clean).sort(), SHAPE, 'task OUTCOME shape');
assert.ok(!('roadmap' in clean), 'task OUTCOME carries no roadmap key');
assert.ok(!('phase' in clean), 'task OUTCOME carries no phase key');

// The three OUTCOME values the SKILL.md status map switches on must all be
// reachable from the task path.
const seen = new Set([
  clean.outcome,
  buildTaskOutcome({ task: 't', planFindings: [B('p')], tier: 'medium' }).outcome,
  buildTaskOutcome({
    task: 't',
    planFindings: [],
    codeFindings: [B('c')],
    codeFindingsAfterRework: [B('c')],
    tier: 'medium',
  }).outcome,
]);
for (const o of ['reviewed', 'escalated', 'rework']) {
  assert.ok(seen.has(o), `task path can produce the '${o}' OUTCOME`);
}

assert.ok(!JSON.stringify(clean).includes('Done:'), 'task OUTCOME never carries a Done: directive');
console.log('task OUTCOME shape assertions passed');
NODE_TEST

if run_node "$TMP/shape.mjs" "$LIB"; then
    pass "task-keyed OUTCOME shape verified; all three outcome values reachable"
else
    fail "buildTaskOutcome shape assertions failed"
fi

# --- 3. DYNAMIC OUTCOME CONTRACT ----------------------------------------------
say "3. Dynamic task OUTCOME -> status contract against the real binary"

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

say "3a. Seeding a hermetic plan repo (project 'verify') with 3 in-progress tasks"
rdm_plan init --default-project verify >/dev/null
for slug in t-reviewed t-rework t-escalated; do
    rdm_plan task create "$slug" --title "Task $slug" --body "Task $slug body." \
        --no-edit --project verify >/dev/null
    rdm_plan task update "$slug" --status in-progress --no-edit --project verify >/dev/null
done
rdm_plan commit -m "chore(plan): seed 3 in-progress tasks" >/dev/null
pass "seeded t-reviewed/t-rework/t-escalated, all in-progress"

say "3b. reviewed -> task update --status reviewed"
rdm_plan task update t-reviewed --status reviewed --no-edit --project verify >/dev/null
OUT_A=$(rdm_plan task show t-reviewed --project verify --format json --no-body)
printf '%s' "$OUT_A" | grep -qF '"status": "reviewed"' ||
    fail "t-reviewed expected status reviewed, got: $OUT_A"
pass "reviewed OUTCOME lands --status reviewed"

say "3c. rework -> task update --status blocked --reason '[code] ...'"
rdm_plan task update t-rework --status blocked --reason "[code] rework budget exhausted: test" \
    --no-edit --project verify >/dev/null
OUT_B=$(rdm_plan task show t-rework --project verify --format json --no-body)
printf '%s' "$OUT_B" | grep -qF '"status": "blocked"' ||
    fail "t-rework expected status blocked, got: $OUT_B"
printf '%s' "$OUT_B" | grep -qF '[code]' ||
    fail "t-rework reason must carry the [code] tag, got: $OUT_B"
pass "rework OUTCOME lands --status blocked with a [code]-tagged reason"

say "3d. escalated -> task update --status blocked --reason '[plan] ...'"
rdm_plan task update t-escalated --status blocked --reason "[plan] ambiguous body: test" \
    --no-edit --project verify >/dev/null
OUT_C=$(rdm_plan task show t-escalated --project verify --format json --no-body)
printf '%s' "$OUT_C" | grep -qF '"status": "blocked"' ||
    fail "t-escalated expected status blocked, got: $OUT_C"
printf '%s' "$OUT_C" | grep -qF '[plan]' ||
    fail "t-escalated reason must carry the [plan] tag, got: $OUT_C"
pass "escalated OUTCOME lands --status blocked with a [plan]-tagged reason"

say "3e. The park reason survives a later status change"
rdm_plan task update t-rework --status in-progress --no-edit --project verify >/dev/null
OUT_D=$(rdm_plan task show t-rework --project verify --format json --no-body)
printf '%s' "$OUT_D" | grep -qF '"status": "in-progress"' ||
    fail "t-rework expected status in-progress after unpark, got: $OUT_D"
printf '%s' "$OUT_D" | grep -qF '[code]' ||
    fail "the [code] park reason must be preserved across a status change, got: $OUT_D"
pass "park reason is preserved across a later status change"

# NOTE: this is a round-trip sanity check on the seeded data, NOT a regression
# guard on tag selection — the [code]/[plan] mapping lives in SKILL.md prose and
# is applied by an agent, so there is no code path here that could transpose it.
say "3f. The seeded [plan] reason round-trips without picking up a [code] tag"
printf '%s' "$OUT_C" | grep -qF '[code]' &&
    fail "escalated task must NOT carry a [code] reason (tags transposed?)"
pass "escalated carries [plan], not [code]"

printf '\n\033[1;34m==>\033[0m verify-workflow-do-auto-task.sh: ALL GREEN\n'
