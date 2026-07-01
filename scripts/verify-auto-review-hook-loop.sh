#!/bin/sh
# End-to-end regression for the auto-review Stop hook loop.
#
# The `rdm review pending` scoping logic is unit-covered by
# rdm-cli/tests/cli_review.rs, but the actual Stop hook SCRIPTS plus the Claude
# Code Stop payload contract are only covered by manual-reproduction comments.
# This harness drives the REAL hook scripts against a hermetic temp plan +
# source repo so a refactor of the hook, the JSON shape, or --project/root
# resolution can't silently stop auto-review from firing without a test failing.
#
# Two hook scripts exist:
#   - .claude/hooks/rdm-review-on-finalize.sh (the concrete in-repo instance;
#     resolves $CLAUDE_PROJECT_DIR/target/debug/rdm and hardcodes --project rdm)
#   - rdm-core/src/templates/hook-review-on-finalize.sh (the shipped template;
#     uses bare `rdm` on PATH + RDM_PROJECT, no hardcoded --project)
#
# Coverage split (deliberate, to avoid duplicating sibling harnesses):
#   - The CONCRETE script gets all four contract states here (FIRE / OUT OF
#     SCOPE / LOOP GUARD / CLEARED); no sibling harness exercises it at all.
#   - The TEMPLATE's FIRE and OUT-OF-SCOPE states are already covered by
#     scripts/verify-worktree-review-loop.sh and
#     scripts/verify-claude-code-web-loop.sh. Those siblings NEVER send
#     stop_hook_active:true to the template and NEVER test a reviewed-cleared
#     item against it, so this harness adds exactly those two missing template
#     cases (LOOP GUARD + CLEARED), plus a local template FIRE anchor so those
#     two silences can't pass vacuously (they must be measured against a
#     template invocation proven to block under this harness's own wiring).
#
# Seven assertions total: 4 concrete-script + 3 template (FIRE anchor + LOOP
# GUARD + CLEARED).
#
# Requires: cargo-built rdm at target/debug/rdm (from this repo).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
RDM_BIN="$REPO_ROOT/target/debug/rdm"
HOOK_CONCRETE="$REPO_ROOT/.claude/hooks/rdm-review-on-finalize.sh"
HOOK_TEMPLATE="$REPO_ROOT/rdm-core/src/templates/hook-review-on-finalize.sh"

if [ ! -x "$RDM_BIN" ]; then
    echo "error: $RDM_BIN not found or not executable — run 'cargo build' first." >&2
    exit 1
fi

if [ ! -f "$HOOK_CONCRETE" ]; then
    echo "error: concrete hook not found at $HOOK_CONCRETE — did the path move?" >&2
    exit 1
fi

if [ ! -f "$HOOK_TEMPLATE" ]; then
    echo "error: hook template not found at $HOOK_TEMPLATE — did the path move?" >&2
    exit 1
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

# Clear rdm-related env vars inherited from the caller's shell so the simulated
# hosts don't pick up the developer's real plan repo.
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

# rdm on PATH for the template invocations (it shells out to bare `rdm`, no
# --root, no jq — exactly like the sibling worktree-review harness).
BIN="$TMP/bin"
mkdir -p "$BIN"
ln -s "$RDM_BIN" "$BIN/rdm"
HOOK_PATH="$BIN:$PATH"

# The concrete in-repo hook: resolves $CLAUDE_PROJECT_DIR/target/debug/rdm and
# hardcodes --project rdm, reads RDM_ROOT/config for the plan repo. Args: cwd,
# stdin JSON.
run_concrete() (
    cd "$1"
    printf '%s' "$2" |
        CLAUDE_PROJECT_DIR="$REPO_ROOT" RDM_ROOT="$PLAN" sh "$HOOK_CONCRETE"
)

# The shipped template: bare `rdm` on PATH, project via RDM_PROJECT. Args: cwd,
# stdin JSON.
run_template() (
    cd "$1"
    printf '%s' "$2" |
        RDM_ROOT="$PLAN" RDM_PROJECT="rdm" PATH="$HOOK_PATH" sh "$HOOK_TEMPLATE"
)

# ----------------------------------------------------------------------------
# Setup: a plan repo whose project is named literally `rdm` (the concrete hook
# hardcodes --project rdm) with one needs-reviewable task, and a source repo
# with a feature branch that the task is finalized on.
# ----------------------------------------------------------------------------
say "Seeding plan repo (project rdm) with task demo-item"

"$RDM_BIN" --root "$PLAN" init --default-project rdm >/dev/null
"$RDM_BIN" --root "$PLAN" task create demo-item \
    --title "Demo Item" --body "Auto-review hook loop regression item." \
    --no-edit --project rdm >/dev/null
ok "plan repo seeded with task demo-item"

say "Creating source repo with main + feature/x"

git init --quiet -b main "$SRC"
(cd "$SRC" && git commit --quiet --allow-empty -m "chore: initial")
(cd "$SRC" && git checkout --quiet -b feature/x && git commit --quiet --allow-empty -m "feat: x work")
ok "source repo on feature/x with a divergent commit"

# Finalize demo-item to needs-review FROM feature/x so it stamps
# review_branch=feature/x + review_sha at that tip.
say "Finalizing demo-item to needs-review on feature/x"
(cd "$SRC" && RDM_ROOT="$PLAN" "$RDM_BIN" task update demo-item \
    --status needs-review --no-edit --project rdm >/dev/null)
ok "demo-item finalized on feature/x"

# ============================================================================
# Concrete scenario 1: FIRE. On branch feature/x (in scope) with a fresh stop
# payload, the hook must block.
# ============================================================================
say "Concrete 1/FIRE: block on the finalized branch"

OUT=$(run_concrete "$SRC" '{"stop_hook_active": false}')
printf '%s' "$OUT" | grep -q '"decision":"block"' ||
    {
        printf '%s\n' "$OUT" >&2
        fail "concrete FIRE: hook should block on feature/x"
    }
ok "concrete FIRE: {\"decision\":\"block\"} on feature/x"

# ============================================================================
# Concrete scenario 2: OUT OF SCOPE. On an unrelated branch (main), the item is
# not in scope; the hook must exit 0 with no output.
# ============================================================================
say "Concrete 2/OUT-OF-SCOPE: silent on an unrelated branch"

(cd "$SRC" && git checkout --quiet main)
set +e
OUT=$(run_concrete "$SRC" '{"stop_hook_active": false}')
rc=$?
set -e
[ "$rc" -eq 0 ] || fail "concrete OUT-OF-SCOPE: expected exit 0, got $rc"
[ -z "$OUT" ] ||
    {
        printf '%s\n' "$OUT" >&2
        fail "concrete OUT-OF-SCOPE: hook must be silent on main"
    }
ok "concrete OUT-OF-SCOPE: empty stdout, exit 0 on main"

# ============================================================================
# Concrete scenario 3: LOOP GUARD. Back on feature/x (item IS in scope) but with
# stop_hook_active:true — the loop guard must short-circuit to exit 0 / silence
# even though the item would otherwise fire.
# ============================================================================
say "Concrete 3/LOOP-GUARD: silent under stop_hook_active:true"

(cd "$SRC" && git checkout --quiet feature/x)
set +e
OUT=$(run_concrete "$SRC" '{"stop_hook_active": true}')
rc=$?
set -e
[ "$rc" -eq 0 ] || fail "concrete LOOP-GUARD: expected exit 0, got $rc"
[ -z "$OUT" ] ||
    {
        printf '%s\n' "$OUT" >&2
        fail "concrete LOOP-GUARD: hook must be silent under stop_hook_active:true"
    }
ok "concrete LOOP-GUARD: empty stdout, exit 0 despite in-scope item"

# ============================================================================
# Template scenario (FIRE anchor): prove the shipped template actually blocks
# with THIS harness's wiring (bare rdm on PATH + RDM_PROJECT=rdm + RDM_ROOT)
# before asserting the two template silences below. Without this anchor, a
# misconfigured run_template (unhonored RDM_PROJECT, broken PATH symlink,
# unresolved item) would make the LOOP-GUARD/CLEARED silences pass vacuously —
# silent for the wrong reason. The item is still needs-review on feature/x here.
# ============================================================================
say "Template/FIRE anchor: shipped template blocks on the finalized branch"

OUT=$(run_template "$SRC" '{"stop_hook_active": false}')
printf '%s' "$OUT" | grep -q '"decision":"block"' ||
    {
        printf '%s\n' "$OUT" >&2
        fail "template FIRE anchor: template should block on feature/x (its silences below would be vacuous otherwise)"
    }
ok "template FIRE anchor: {\"decision\":\"block\"} on feature/x"

# ============================================================================
# Template scenario (LOOP GUARD): the shipped template with stop_hook_active:true
# on feature/x (item still in scope) — the sibling harnesses never send this
# payload to the template. Must exit 0 / silent.
# ============================================================================
say "Template/LOOP-GUARD: shipped template silent under stop_hook_active:true"

set +e
OUT=$(run_template "$SRC" '{"stop_hook_active": true}')
rc=$?
set -e
[ "$rc" -eq 0 ] || fail "template LOOP-GUARD: expected exit 0, got $rc"
[ -z "$OUT" ] ||
    {
        printf '%s\n' "$OUT" >&2
        fail "template LOOP-GUARD: template must be silent under stop_hook_active:true"
    }
ok "template LOOP-GUARD: empty stdout, exit 0"

# ============================================================================
# Concrete scenario 4: CLEARED. Move the item to reviewed; nothing is pending, so
# a fresh stop payload must exit 0 / silent.
# ============================================================================
say "Concrete 4/CLEARED: silent once the item is reviewed"

(cd "$SRC" && RDM_ROOT="$PLAN" "$RDM_BIN" task update demo-item \
    --status reviewed --no-edit --project rdm >/dev/null)
set +e
OUT=$(run_concrete "$SRC" '{"stop_hook_active": false}')
rc=$?
set -e
[ "$rc" -eq 0 ] || fail "concrete CLEARED: expected exit 0, got $rc"
[ -z "$OUT" ] ||
    {
        printf '%s\n' "$OUT" >&2
        fail "concrete CLEARED: hook must be silent after review"
    }
ok "concrete CLEARED: empty stdout, exit 0 after reviewed"

# ============================================================================
# Template scenario (CLEARED): the shipped template against the now-reviewed item
# with a fresh stop payload — the sibling harnesses never test a reviewed-cleared
# item against the template. Must exit 0 / silent.
# ============================================================================
say "Template/CLEARED: shipped template silent once the item is reviewed"

set +e
OUT=$(run_template "$SRC" '{"stop_hook_active": false}')
rc=$?
set -e
[ "$rc" -eq 0 ] || fail "template CLEARED: expected exit 0, got $rc"
[ -z "$OUT" ] ||
    {
        printf '%s\n' "$OUT" >&2
        fail "template CLEARED: template must be silent after review"
    }
ok "template CLEARED: empty stdout, exit 0 after reviewed"

# ----------------------------------------------------------------------------
# Done.
# ----------------------------------------------------------------------------
printf '\n\033[1;32mAll checks passed.\033[0m\n'
