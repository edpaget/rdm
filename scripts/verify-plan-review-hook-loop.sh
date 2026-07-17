#!/bin/sh
# End-to-end regression for the plan-review Stop hook loop.
#
# `rdm-core/src/templates/hook-plan-review-on-create.sh` is the shipped template
# that reprompts the agent to run rdm-plan-review while any rdm item carries the
# `needs-plan-review` sentinel tag (stamped by `roadmap create` / `phase create` /
# `task create` when the `plan_review` config flag is enabled). Unlike the
# needs-review Stop hook, there is no concrete in-repo instance of this hook yet
# (dogfooding is a later roadmap phase) — this harness drives only the template,
# against a hermetic temp plan + source repo, so a refactor of the hook, the JSON
# shape, or project resolution can't silently stop plan-review from firing without
# a test failing.
#
# Four scenarios (mirrors the FIRE / LOOP-GUARD / CLEARED states from the sibling
# scripts/verify-auto-review-hook-loop.sh harness, plus FAIL-OPEN):
#   1. FIRE: a task is created with `plan_review = true` (stamping the tag) — a
#      fresh stop payload must block.
#   2. LOOP-GUARD: the item is still tagged, but `stop_hook_active: true` — the
#      hook must be silent (loop prevention takes precedence over firing).
#   3. CLEARED: the tag is cleared (`--tags ""`) — a fresh stop payload must be
#      silent (nothing pending).
#   4. FAIL-OPEN: the item is re-tagged (so a working query WOULD block), but
#      `rdm` is absent from PATH — the hook must exit 0 and stay silent. A
#      broken query must never wedge the agent's stop.
#
# Requires: cargo-built rdm at target/debug/rdm (from this repo).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
RDM_BIN="$REPO_ROOT/target/debug/rdm"
HOOK_TEMPLATE="$REPO_ROOT/rdm-core/src/templates/hook-plan-review-on-create.sh"

if [ ! -x "$RDM_BIN" ]; then
    echo "error: $RDM_BIN not found or not executable — run 'cargo build' first." >&2
    exit 1
fi

if [ ! -f "$HOOK_TEMPLATE" ]; then
    echo "error: hook template not found at $HOOK_TEMPLATE — did the path move?" >&2
    exit 1
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

# Clear rdm-related env vars inherited from the caller's shell so the simulated
# host doesn't pick up the developer's real plan repo.
unset RDM_ROOT RDM_PROJECT RDM_STAGE RDM_FORMAT RDM_PLAN_REPO RDM_PLAN_REPO_TOKEN RDM_PLAN_REPO_PATH

# Git identity so `git commit` never falls back to a missing global config.
export GIT_AUTHOR_NAME="verify-bot"
export GIT_AUTHOR_EMAIL="verify@example.invalid"
export GIT_COMMITTER_NAME="verify-bot"
export GIT_COMMITTER_EMAIL="verify@example.invalid"

say() { printf '\n\033[1;34m==>\033[0m %s\n' "$*"; }
fail() {
    printf '\n\033[1;31m[FAIL]\033[0m %s\n' "$*" >&2
    exit 1
}
ok() { printf '\033[1;32m[ OK ]\033[0m %s\n' "$*"; }

# Hermetic HOME + XDG so neither `rdm` writes nor git's global-config lookup
# touch the developer/CI user's real dirs (matches the sibling harnesses).
export HOME="$TMP/home"
export XDG_CONFIG_HOME="$TMP/xdg-config"
export XDG_DATA_HOME="$TMP/xdg-data"
export XDG_STATE_HOME="$TMP/xdg-state"
mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_STATE_HOME"

PLAN="$TMP/plan"
SRC="$TMP/src"

# rdm on PATH for the template invocation (it shells out to bare `rdm`, no --root,
# no jq — exactly like the sibling worktree-review / auto-review harnesses).
BIN="$TMP/bin"
mkdir -p "$BIN"
ln -s "$RDM_BIN" "$BIN/rdm"
HOOK_PATH="$BIN:$PATH"

# The shipped template: bare `rdm` on PATH, project via default_project in
# rdm.toml (set by `init --default-project`) — the template never passes
# --project. Args: cwd, stdin JSON.
run_template() (
    cd "$1"
    printf '%s' "$2" |
        RDM_ROOT="$PLAN" PATH="$HOOK_PATH" sh "$HOOK_TEMPLATE"
)

# ----------------------------------------------------------------------------
# Setup: a plan repo with `plan_review = true` and one task (created after the
# flag is enabled, so it is stamped with `needs-plan-review`), plus a minimal
# source repo the hook runs from.
# ----------------------------------------------------------------------------
say "Seeding plan repo (project demo) with plan_review = true"

"$RDM_BIN" --root "$PLAN" init --default-project demo >/dev/null
"$RDM_BIN" --root "$PLAN" config set plan_review true >/dev/null
ok "plan repo initialized with plan_review = true"

say "Creating task demo-item (stamps needs-plan-review)"

"$RDM_BIN" --root "$PLAN" task create demo-item \
    --title "Demo Item" --body "Plan-review hook loop regression item." \
    --no-edit --project demo >/dev/null
ok "task demo-item created and tagged needs-plan-review"

git init --quiet -b main "$SRC"
(cd "$SRC" && git commit --quiet --allow-empty -m "chore: initial")
ok "source repo initialized"

# ============================================================================
# Scenario 1: FIRE. The item is tagged needs-plan-review; a fresh stop payload
# must block.
# ============================================================================
say "1/FIRE: block while an item is tagged needs-plan-review"

OUT=$(run_template "$SRC" '{"stop_hook_active": false}')
printf '%s' "$OUT" | grep -q '"decision":"block"' ||
    {
        printf '%s\n' "$OUT" >&2
        fail "FIRE: hook should block while demo-item is tagged needs-plan-review"
    }
ok "FIRE: {\"decision\":\"block\"} with demo-item pending"

# ============================================================================
# Scenario 2: LOOP GUARD. The item is still tagged, but stop_hook_active:true —
# the loop guard must short-circuit to exit 0 / silence even though the item
# would otherwise fire.
# ============================================================================
say "2/LOOP-GUARD: silent under stop_hook_active:true"

set +e
OUT=$(run_template "$SRC" '{"stop_hook_active": true}')
rc=$?
set -e
[ "$rc" -eq 0 ] || fail "LOOP-GUARD: expected exit 0, got $rc"
[ -z "$OUT" ] ||
    {
        printf '%s\n' "$OUT" >&2
        fail "LOOP-GUARD: hook must be silent under stop_hook_active:true"
    }
ok "LOOP-GUARD: empty stdout, exit 0 despite pending item"

# ============================================================================
# Scenario 3: CLEARED. Clear the tag; nothing is pending, so a fresh stop
# payload must exit 0 / silent.
# ============================================================================
say "3/CLEARED: silent once the tag is cleared"

"$RDM_BIN" --root "$PLAN" task update demo-item --tags "" --no-edit --project demo >/dev/null

set +e
OUT=$(run_template "$SRC" '{"stop_hook_active": false}')
rc=$?
set -e
[ "$rc" -eq 0 ] || fail "CLEARED: expected exit 0, got $rc"
[ -z "$OUT" ] ||
    {
        printf '%s\n' "$OUT" >&2
        fail "CLEARED: hook must be silent after the tag is cleared"
    }
ok "CLEARED: empty stdout, exit 0 after clearing the tag"

# ============================================================================
# Scenario 4: FAIL-OPEN. Re-tag the item so a working query WOULD block, then
# strip `rdm` from PATH — the query fails, and the hook must exit 0 / silent
# rather than block or error (a broken query must never wedge the agent stop).
# ============================================================================
say "4/FAIL-OPEN: silent when rdm is missing from PATH despite a pending item"

"$RDM_BIN" --root "$PLAN" task update demo-item --tags needs-plan-review --no-edit --project demo >/dev/null

set +e
OUT=$(
    cd "$SRC"
    printf '%s' '{"stop_hook_active": false}' |
        RDM_ROOT="$PLAN" PATH="/usr/bin:/bin" sh "$HOOK_TEMPLATE"
)
rc=$?
set -e
[ "$rc" -eq 0 ] || fail "FAIL-OPEN: expected exit 0 with rdm off PATH, got $rc"
[ -z "$OUT" ] ||
    {
        printf '%s\n' "$OUT" >&2
        fail "FAIL-OPEN: hook must be silent when the rdm query fails"
    }
ok "FAIL-OPEN: empty stdout, exit 0 with rdm missing from PATH"

# ----------------------------------------------------------------------------
# Done.
# ----------------------------------------------------------------------------
printf '\n\033[1;32mAll checks passed.\033[0m\n'
