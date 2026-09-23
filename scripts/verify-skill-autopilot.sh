#!/bin/sh
# Hermetic regression for the prose `rdm-autopilot` skill
# (.claude/skills/rdm-autopilot/SKILL.md), which replaced the JS
# `.claude/workflows/autopilot.js` / `.claude/workflows/lib/autopilot.mjs` loop
# (workflow-orchestration roadmap, phase 3). It gates a prose skill against a
# real binary rather than a Node-injected fake.
#
# *** Coverage-decision classification (written BEFORE the JS loop was deleted,
# per the phase's own "coverage decision before the deletion" instruction) ***
#
# The retired scripts/verify-workflow-autopilot.sh drove `lib/autopilot.mjs`'s
# pure functions under injected fakes in Node. A prose skill has no such
# module to import, so none of that machinery ports mechanically. Every
# section of the old harness is classified below as PORTABLE (a) — the
# promised behavior still exists and can be pinned as static text in SKILL.md,
# or as an exact rdm command shape driven against the real binary — E2E-ONLY
# (b) — same, but only checkable by actually running the documented commands,
# not by grepping text — or MOOT (c) — the assertion was about JS-only
# mechanics with no prose equivalent.
#
#   1. BEHAVIOR (old, Node/fakes) — MOSTLY MOOT, PARTLY PORTABLE.
#      - MOOT: buildMechanicalModelPrompt and other prompt-string builders,
#        describeRaw's truncation/cyclic-payload handling, and the triple-unwrap
#        defense in interpretNext for a JSON-string-re-encoded `result` field —
#        none of these functions exist once there's no schema-constrained
#        subagent to hand a prompt string to or receive a re-encoded payload
#        from. SKILL.md's own "Removed / changed" section states this
#        explicitly for the triple-unwrap defense and the `next` hoist.
#      - PORTABLE (static text): the malformed-vs-well-formed distinction
#        ("never treat a malformed payload as nothing" -> `unparseable`), the
#        known-good stop-reason allowlist (`nothing`, `blocked-on-dependencies`,
#        `budget`, `plan-only-exhausted`, `mechanical-model-unresolved`) vs. the
#        abnormal-termination banner for anything else, the summary template
#        shape, the `[fetch]`-tagged summary-only note, and the four guardrails
#        (single roadmap, no `main` mutation, no `Done:` trailer construction,
#        `--permission-mode auto`). All pinned in section 1 below.
#      - PORTABLE (real command shape): the advance/park write+read-back
#        contract's command shapes are exact `rdm phase update`/`rdm phase
#        show` invocations, driven for real in section 2 below (this is where
#        an old Node fake becomes a real binary call instead).
#
#   1b. DRIVEN LOOP (old, buildAutopilot + fakes) — MOSTLY MOOT, PARTLY
#      PORTABLE/E2E-ONLY.
#      - MOOT: mechanical-model threading into five dependency-injected
#        functions (fetchNext/estimateList/estimateWriteback/advance/park) and
#        the advance-null-as-failure / park-null-still-summarizing tests — all
#        of these existed because a mechanical subagent could return null/a
#        garbage ack; a direct Bash command instead returns a real process exit
#        code, so there is no "null ack" class of failure left to model.
#        Likewise the double/garbage-wrapped `fetch:next` payload tests (no
#        intermediate agent re-encodes JSON as a string anymore).
#      - PORTABLE (static text): drive-to-reviewed / rework-retry-once-then-park
#        / escalated / budget-stop / estimate-pre-pass-always-runs /
#        `--plan-only` dedup-via-in-context-set are all still real POLICY this
#        skill promises in prose — pinned as literal text assertions in
#        section 1.
#      - E2E-ONLY: the actual advance/park write, and its read-back retry
#        contract, is real and driven against the binary in section 2 — this
#        is the one piece of 1b that survives as an executable check rather
#        than a text check.
#
#   1c. HOIST self-tests — MOOT. These proved the OLD assertions weren't
#      vacuous; the assertions themselves are gone (see 1/1b), so their
#      self-tests have nothing left to guard.
#
#   2. BLOCK DRIFT (lib vs. workflow byte-identity) — MOOT. There is no
#      `lib/autopilot.mjs` and no stamped copy anymore; the skill is the one
#      and only source of the loop's prose.
#
#   3. STATIC INVARIANTS (JS greps: one nested `workflow()` dispatch call, no
#      import/require, both markers, no land/merge/main-mutation string,
#      no *_SCHEMA with a top-level `type:'array'`, meta.phases parity) —
#      MOOT, JS-runtime/JS-schema-specific. The two invariants with a real
#      prose equivalent — "the loop never touches `main`" and "the loop never
#      hand-builds a `Done:` trailer" — are re-pinned as literal-text
#      assertions in section 1 (they are exactly guardrails 2 and 3 in
#      SKILL.md's own "four guardrails" block).
#
#   3b. AC-MODEL (every agent() call in the five mechanical deps carries an
#      explicit model:) — MOOT. There are no agent() calls left in the loop
#      itself; the only remaining Workflow call (`rdm-wf-estimate`) is an
#      unchanged caller already covered by its own harness.
#
#   4. MODULE PARSE (autopilot.js loads under module semantics) — MOOT. There
#      is no JS file to parse.
#
#   5. SIBLING GATE — PORTABLE, unchanged in spirit: the prose loop nests
#      exactly ONE Workflow, `rdm-wf-estimate` (the per-phase unit is the prose
#      `rdm-dispatch-phase` orchestrator, entered with `Skill`), so that
#      harness staying green is the right regression signal. Re-run in section 3
#      below.
#
#   6. LAND-TIME COMPLETION TRAILER — PORTABLE, workflow-agnostic; this section
#      never touched autopilot.js/mjs at all (rdm-land / `rdm hook done-line`
#      own it). Copied verbatim into section 4 below so this coverage is not
#      lost in the migration.
#
# *** Known, permanent coverage gap (see also SKILL.md's own note) ***
# The loop's POLICY decisions — the shared budget counter counting a rework
# re-dispatch of the SAME phase against `--max-phases`, rework-retry-exactly-
# once, `--plan-only` exhaustion dedup via an in-context set, and the estimate
# pre-pass actually firing on every run — are now made by an LLM reasoning in
# prose, not by callable JS. No deterministic harness (this one included) can
# drive that reasoning with injected fakes and assert on its branches the way
# scripts/verify-workflow-autopilot.sh's section 1b used to. This is an
# accepted, permanent trade-off of the prose migration (recorded in the
# roadmap phase body and in `docs/workflow-vs-prose-boundary.md`), not an
# oversight this harness fails to close.
#
# This harness covers what's left:
#
#   1. STATIC INVARIANTS   — NARROWED (agent-orchestrated-dispatch phase 6).
#                             The per-phase unit is no longer a Workflow this
#                             skill hands a payload to: it is the prose
#                             `rdm-dispatch-phase` orchestrator, entered with
#                             `Skill` into this same session. Every grep that
#                             asserted the old engine's arg shape, phaseMeta
#                             hoist, rdm-wf- rename halves or shipped-template
#                             wording therefore lost its subject and was
#                             DELETED rather than rewritten — per the operator's
#                             verification principle, a check that only asks
#                             whether strings are present in static prose is not
#                             evidence. What survives here is the one contract
#                             nothing else covers: the unit is entered with
#                             `Skill`, never with `Agent` (an Agent subagent has
#                             no `Workflow` tool, so both review-engine calls
#                             would be unreachable), with two planted-mutation
#                             self-tests. The wider sweep of this harness class
#                             is owned by task/retire-static-grep-harnesses.
#   2. DYNAMIC OUTCOME CONTRACT — against the real binary in a hermetic temp
#                             plan repo: the exact `rdm phase update`/`rdm
#                             phase show --format json` command shapes SKILL.md
#                             documents for the advance and park steps land the
#                             status (+ blocked_reason) it promises, confirmed
#                             by a read-back.
#   3. SIBLING GATE         — verify-workflow-estimate.sh (the one Workflow this
#                             skill still nests) stays green. The retired
#                             dispatch engine's harness is gone with the engine
#                             (agent-orchestrated-dispatch phase 7); the review
#                             pipeline the prose orchestrator calls is gated by
#                             scripts/verify-workflow-review.sh, which CI runs
#                             directly out of the scripts/verify-*.sh glob
#                             rather than nesting redundantly here.
#   4. LAND-TIME TRAILER    — copied verbatim from the old harness's section 6:
#                             a trailer-less autopilot-shaped branch commit
#                             gains its completion trailer from `rdm hook
#                             done-line` + `git commit --amend`, with no
#                             rebase, and `rdm hook post-commit` then completes
#                             the item.
#
# Requires: a cargo-built rdm at target/debug/rdm (from this repo).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
RDM_BIN="$REPO_ROOT/target/debug/rdm"
SKILL="$REPO_ROOT/.claude/skills/rdm-autopilot/SKILL.md"

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

# --- 1. STATIC INVARIANTS (narrowed) ------------------------------------------
# Deliberately small. The per-phase unit is no longer a Workflow this skill
# hands args to, so the arg-shape / hoist-payload greps this section used to
# carry have no subject at all (see the header note above). What is left is the
# ONE contract a mangled edit could silently break without any other harness
# noticing: the per-phase unit MUST be entered with `Skill` into this same
# session, never with `Agent` — an Agent-spawned subagent has no `Workflow`
# tool, so the orchestrator's two review-engine calls would be unreachable.
say "1. Static invariants on .claude/skills/rdm-autopilot/SKILL.md"

# The Skill-entry contract, asserted as a shape rather than a sentence: the
# file must contain a Skill({ ... skill: 'rdm-dispatch-phase' ... }) entry and
# must NOT contain an Agent(...) dispatch of that same skill.
assert_skill_entry() {
    _f=$1
    grep -qE "Skill\(\{[^}]*skill: *'rdm-dispatch-phase'" "$_f" || return 1
    ! grep -qE "Agent\(\{[^}]*rdm-dispatch-phase" "$_f" || return 1
    return 0
}
assert_skill_entry "$SKILL" ||
    fail "SKILL.md must enter the per-phase unit with Skill({ skill: 'rdm-dispatch-phase', ... }) and must never dispatch it with Agent — an Agent subagent has no Workflow tool, so both review-engine calls would be unreachable"
pass "the per-phase unit is entered with Skill, never dispatched with Agent"

# Self-test A: an Agent dispatch of the orchestrator must turn it red.
cp "$SKILL" "$TMP/skill-agent-mutant.md"
printf "\nAgent({ agentType: 'general-purpose', prompt: 'run rdm-dispatch-phase' })\n" \
    >>"$TMP/skill-agent-mutant.md"
if assert_skill_entry "$TMP/skill-agent-mutant.md"; then
    fail "self-test A: a planted Agent dispatch of rdm-dispatch-phase was NOT caught — the Skill-entry check is vacuous"
fi
pass "self-test A: a planted Agent dispatch of the orchestrator turns the check red"

# Self-test B: losing the Skill entry entirely must turn it red.
sed "s/skill: 'rdm-dispatch-phase'/skill: 'rdm-dispatch-phse'/g" "$SKILL" >"$TMP/skill-typo-mutant.md"
if assert_skill_entry "$TMP/skill-typo-mutant.md"; then
    fail "self-test B: a typo'd Skill entry was NOT caught — the Skill-entry check is vacuous"
fi
pass "self-test B: a typo'd Skill entry turns the check red"

# --- 2. DYNAMIC OUTCOME CONTRACT ----------------------------------------------
say "2. Dynamic advance/park write+read-back contract against the real binary"

# Hermetic HOME + XDG + git identity so neither rdm nor git touches the real
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

say "2a. Seeding a hermetic plan repo (project 'verify') with roadmap 'rm' and 2 phases"
rdm_plan init --default-project verify >/dev/null
RM_INTENT_GOAL='a whole roadmap advances without a human dispatching each phase'
rdm_plan roadmap create rm --title "RM" --body "Autopilot advance/park contract regression roadmap.

## Intent

**Goal.** $RM_INTENT_GOAL
**Non-goals.** Replacing the estimator.
**Done looks like.** An operator starts one run and reads one summary." \
    --no-edit --project verify >/dev/null
rdm_plan phase create a --title "Phase A" --number 1 --body "Phase A." \
    --no-edit --roadmap rm --project verify >/dev/null
rdm_plan phase create b --title "Phase B" --number 2 --body "Phase B." \
    --no-edit --roadmap rm --project verify >/dev/null
rdm_plan phase update phase-1-a --status in-progress --no-edit --roadmap rm --project verify >/dev/null
rdm_plan phase update phase-2-b --status in-progress --no-edit --roadmap rm --project verify >/dev/null
rdm_plan commit -m "chore(plan): seed rm roadmap with 2 in-progress phases" >/dev/null
pass "seeded roadmap rm with phase-1-a/phase-2-b, both in-progress"

say "2b. advance -> rdm phase update --status reviewed, confirmed by a read-back"
# Stays green under the core-enforced `reviewed` gate: this hermetic plan repo
# is seeded with no `gates.reviewed` key, and the gate ships default-OFF, so
# `check_reviewed_gate` returns NotApplicable and the write is unconditional.
# See docs/core-enforced-gates.md.
rdm_plan phase update phase-1-a --status reviewed --no-edit --roadmap rm --project verify >/dev/null
OUT_A=$(rdm_plan phase show phase-1-a --roadmap rm --project verify --format json --no-body)
printf '%s' "$OUT_A" | grep -qF '"status": "reviewed"' || fail "phase-1-a expected status reviewed, got: $OUT_A"
pass "advance's write+read-back contract holds: --status reviewed lands and reads back"

say "2c. park -> rdm phase update --status blocked --reason '[code] rework budget exhausted', confirmed by a read-back"
rdm_plan phase update phase-2-b --status blocked --reason "[code] rework budget exhausted" \
    --no-edit --roadmap rm --project verify >/dev/null
OUT_B=$(rdm_plan phase show phase-2-b --roadmap rm --project verify --format json --no-body)
printf '%s' "$OUT_B" | grep -qF '"status": "blocked"' || fail "phase-2-b expected status blocked, got: $OUT_B"
printf '%s' "$OUT_B" | grep -qF '"blocked_reason": "[code] rework budget exhausted"' ||
    fail "phase-2-b blocked_reason must be the documented rework-exhausted park reason, got: $OUT_B"
pass "park's write+read-back contract holds: --status blocked with a [code]-tagged reason lands and reads back"

rdm_plan commit -m "chore(plan): land advance/park OUTCOME contract regression" >/dev/null

# --- 3. SIBLING GATE -----------------------------------------------------------
say "3. Sibling gate: the one Workflow this skill still nests stays green"

if bash "$SCRIPT_DIR/verify-workflow-estimate.sh" >/dev/null 2>&1; then
    pass "verify-workflow-estimate.sh still green"
else
    bash "$SCRIPT_DIR/verify-workflow-estimate.sh" >&2 || true
    fail "verify-workflow-estimate.sh regressed"
fi

# --- 4. LAND-TIME COMPLETION TRAILER -----------------------------------------
# Copied verbatim (in spirit) from the retired verify-workflow-autopilot.sh's
# section 6. Autopilot leaves a reviewed phase's branch commit WITHOUT a
# completion trailer; `rdm-land` is the land-time writer. This drives the exact
# documented rdm-land sequence against the REAL binary in a hermetic temp plan
# + source repo, and asserts the trailer arrives with NO rebase and no
# interactive step, and that the merge hook then completes the item. This
# section is workflow-agnostic and never touched autopilot.js/lib/autopilot.mjs
# even before their retirement, so it is unaffected by this migration.
say "4. Land-time completion trailer: a trailer-less autopilot branch gains it with no rebase"

SRC="$TMP/src"

rdm_plan roadmap create rm2 --title "RM2" --body "Land-time trailer regression roadmap." \
    --no-edit --project verify >/dev/null
rdm_plan phase create x --title "Phase X" --number 1 --body "Phase X." \
    --no-edit --roadmap rm2 --project verify >/dev/null
# Exactly the state autopilot leaves behind: the phase advanced to `reviewed`,
# the work committed on the roadmap branch, nothing landed.
# Same as 2b: `gates.reviewed` is unset in this hermetic seed, so the
# `reviewed` gate is NotApplicable here.
rdm_plan phase update phase-1-x --status reviewed --no-edit --roadmap rm2 --project verify >/dev/null
rdm_plan commit -m "chore(plan): seed rm2/phase-1-x as reviewed" >/dev/null
pass "seeded hermetic plan repo: rm2/phase-1-x is reviewed"

# Source repo: a roadmap branch whose tip is the un-pushed reviewed commit with a
# message carrying NO trailer (exactly what an autopilot run leaves).
mkdir -p "$SRC"
git -C "$SRC" init -q -b main
printf 'seed\n' >"$SRC/README.md"
git -C "$SRC" add README.md
git -C "$SRC" commit -qm "chore: seed"
git -C "$SRC" checkout -q -b roadmap/rm2
printf 'work\n' >"$SRC/feature.txt"
git -C "$SRC" add feature.txt
git -C "$SRC" commit -qm "feat: implement phase X"
git -C "$SRC" log -1 --pretty=%B | grep -qF 'Done:' &&
    fail "setup is wrong: the autopilot-shaped commit must start WITHOUT a completion trailer"
pass "roadmap/rm2 tip is a reviewed, trailer-less, un-pushed commit"

# The documented rdm-land precondition-2 synthesis: ask rdm for the trailer (the
# format string has exactly one home) and amend it onto the branch tip. No
# rebase, no interactive editor.
DONE_LINE=$(rdm_plan hook done-line --roadmap rm2 --phase phase-1-x) ||
    fail "rdm hook done-line failed — the land path must abort rather than amend an empty trailer"
[ -n "$DONE_LINE" ] || fail "rdm hook done-line printed nothing"
ORIG_MSG=$(git -C "$SRC" log -1 --pretty=%B)
PRE_AMEND_BASE=$(git -C "$SRC" rev-parse HEAD~1)
printf '%s\n\n%s\n' "$ORIG_MSG" "$DONE_LINE" >"$TMP/amend-msg"
GIT_EDITOR=true git -C "$SRC" commit -q --amend -F "$TMP/amend-msg"

git -C "$SRC" log -1 --pretty=%B | grep -qF 'Done: rm2/phase-1-x' ||
    fail "the amended commit must carry 'Done: rm2/phase-1-x'; got: $(git -C "$SRC" log -1 --pretty=%B)"
pass "the branch tip now carries the completion trailer, synthesized by rdm hook done-line"

# No rebase was needed: the amend preserved the parent commit, and the branch
# still has exactly the same two-commit shape.
[ "$(git -C "$SRC" rev-parse HEAD~1)" = "$PRE_AMEND_BASE" ] ||
    fail "the amend must not have rewritten history below the tip — no rebase is permitted"
[ "$(git -C "$SRC" rev-list --count main..HEAD)" -eq 1 ] ||
    fail "roadmap/rm2 must still be exactly one commit ahead of main (no rebase, no extra commits)"
[ -z "$(git -C "$SRC" rev-parse -q --verify REBASE_HEAD 2>/dev/null || true)" ] ||
    fail "a rebase was started — the land-time trailer must need none"
[ ! -d "$SRC/.git/rebase-merge" ] && [ ! -d "$SRC/.git/rebase-apply" ] ||
    fail "a rebase directory exists — the land-time trailer must need no rebase"
pass "no rebase and no interactive step were required"

# Land it: fast-forward main, then run the merge-to-main hook. The trailer the
# lander synthesized is what flips the phase reviewed -> done.
git -C "$SRC" checkout -q main
git -C "$SRC" merge -q --ff-only roadmap/rm2
LANDED_SHA=$(git -C "$SRC" rev-parse HEAD)
(cd "$SRC" && "$RDM_BIN" --root "$PLAN" hook post-commit) ||
    fail "rdm hook post-commit failed on the landed commit"
PHASE_JSON=$(rdm_plan phase show phase-1-x --roadmap rm2 --project verify --format json --no-body)
printf '%s' "$PHASE_JSON" | grep -qF '"status": "done"' ||
    fail "the landed trailer must flip rm2/phase-1-x to done; got: $PHASE_JSON"
printf '%s' "$PHASE_JSON" | grep -qF "$LANDED_SHA" ||
    fail "the completed phase must record the landed commit SHA $LANDED_SHA; got: $PHASE_JSON"
pass "rdm hook post-commit flipped rm2/phase-1-x to done and recorded the landed SHA"

# Negative: `rdm hook done-line` rejects a malformed request, so the land path
# aborts instead of amending an empty trailer.
if rdm_plan hook done-line --roadmap rm2 >/dev/null 2>&1; then
    fail "rdm hook done-line must reject a request with neither --phase nor --task"
fi
if rdm_plan hook done-line --roadmap rm2 --phase phase-1-x --task t >/dev/null 2>&1; then
    fail "rdm hook done-line must reject both --phase and --task together"
fi
pass "rdm hook done-line rejects malformed requests, so the lander aborts rather than amending an empty trailer"

# --- 5. --override-gate IS FOR HUMANS ONLY ------------------------------------
#
# `--override-gate` bypasses the core `reviewed` transition gate's record
# preconditions. It exists for an operator making a judgment call, and its whole
# value is that the bypass is rare and audited. An autonomous loop that reached
# for it would turn the gate into decoration — so assert, mechanically, that
# neither the local autopilot skill nor the shipped template can ever emit it.
#
# SCOPE: `OVERRIDE_SURFACES` below covers exactly two files — the local
# `rdm-autopilot` SKILL.md and its shipped template `skill-autopilot-cli.md`.
# The `rdm-dispatch-phase` orchestrator's own surfaces (its SKILL.md, its
# template, the plugin copy) are deliberately NOT grep-swept: the discovery-based
# sweep that used to do it went with the retired dispatch engine's harness
# (agent-orchestrated-dispatch phase 7) and was not ported, because a
# string-presence assertion over prose is not a verification this roadmap keeps.
# The gate's refusal semantics are proven behaviorally instead, against the real
# binary, in `rdm-core/tests/gate.rs` and `rdm-cli/tests/cli_gate.rs`.
say "5. --override-gate: neither the autopilot skill nor its shipped template emits it"

OVERRIDE_SURFACES="$SKILL
$REPO_ROOT/rdm-core/src/templates/skill-autopilot-cli.md"

check_no_override() {
    # $1: root under which the (relative-or-absolute) files live. Prints the
    # offending file for any surface that mentions the flag.
    for f in $OVERRIDE_SURFACES; do
        rel=${f#"$REPO_ROOT"/}
        target="$1/$rel"
        [ -f "$target" ] || continue
        if grep -qF -e '--override-gate' "$target"; then
            printf '%s\n' "$rel"
        fi
    done
}

OFFENDERS=$(check_no_override "$REPO_ROOT")
[ -z "$OFFENDERS" ] || fail "autopilot surface(s) emit --override-gate, which is operator-only:
$OFFENDERS

An autonomous loop must never bypass the reviewed gate. Park the item instead
and let a human decide."
pass "no autopilot surface emits --override-gate"

# Self-test: plant the flag into a scratch copy and prove the check goes red.
OG_SCRATCH="$TMP/override-scratch"
mkdir -p "$OG_SCRATCH/.claude/skills/rdm-autopilot"
cp "$SKILL" "$OG_SCRATCH/.claude/skills/rdm-autopilot/SKILL.md"
printf '\nrdm phase update <stem> --status reviewed --override-gate "autopilot said so"\n' \
    >>"$OG_SCRATCH/.claude/skills/rdm-autopilot/SKILL.md"
PLANTED=$(check_no_override "$OG_SCRATCH")
printf '%s' "$PLANTED" | grep -q 'rdm-autopilot/SKILL.md' ||
    fail "self-test failed: a planted --override-gate is NOT caught, so section 5 proves nothing"
pass "self-test: a planted --override-gate IS caught"

# Heal: the real tree still passes (restated so both arms are explicit).
[ -z "$(check_no_override "$REPO_ROOT")" ] ||
    fail "self-test failed: the real, unmutated tree does not pass section 5"
pass "self-test: the real, unmutated tree passes"

say "verify-skill-autopilot.sh: ALL GREEN"
