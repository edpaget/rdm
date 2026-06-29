#!/bin/sh
# Cross-host end-to-end regression for the one-worktree-per-roadmap review loop.
#
# Drives the real model — do (implement in place) → finalize (stamp
# review_branch/review_sha) → trigger → review — across two roadmaps in two
# sibling roadmap worktrees, and asserts the ISOLATION property explicitly:
# roadmap A's review trigger fires A's review and stays silent about roadmap B,
# and vice-versa. Everything runs in temp dirs against target/debug/rdm; no
# network, hermetic.
#
# Four regression cases, each its own OK/FAIL section:
#   A. Claude Stop hook host path  — the real hook-review-on-finalize.sh template
#      blocks from a roadmap worktree, scoped (via its data source) to that
#      roadmap only.
#   B. Pi agent_end host path      — replicates the extension's decision contract
#      (run `rdm review pending --format json`, inject iff an item carries an
#      `identifier`). We assert the contract in `sh` rather than booting Pi: a
#      JS/Pi runtime is not available in a Rust CI job, exactly as the sibling
#      web-loop harness asserts `rdm review pending` rather than Claude's runtime.
#   C. Isolation assertion         — the consolidated guarantee: each roadmap
#      worktree's `review pending` lists its own roadmap and not the other, with
#      each item's stamped branch matching its roadmap.
#   D. Trigger-from-main robustness — from the source repo's `main` checkout a
#      trigger never misfires for an in-flight roadmap review (branch-identity
#      scopes the `roadmap/*` items out of `main`); the branch-gone variant
#      proves the same query still exits cleanly after the worktree + branch are
#      removed.
#
# Run after touching the worktree/review-trigger model (rdm worktree, the
# branch-scoped `rdm review pending` filter, the needs-review stamping in
# phase/task update, or the hook/extension templates).
#
# Requires: cargo-built rdm at target/debug/rdm (from this repo).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
RDM_BIN="$REPO_ROOT/target/debug/rdm"
HOOK_TEMPLATE="$REPO_ROOT/rdm-core/src/templates/hook-review-on-finalize.sh"

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
# touch the developer/CI user's real dirs (matches the sibling web-loop harness).
export HOME="$TMP/home"
export XDG_CONFIG_HOME="$TMP/xdg-config"
export XDG_DATA_HOME="$TMP/xdg-data"
export XDG_STATE_HOME="$TMP/xdg-state"
mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_STATE_HOME"

PLAN="$TMP/plan"
SRC="$TMP/src"

# rdm on PATH for the hook template (it shells out to bare `rdm`, no --root, no jq).
BIN="$TMP/bin"
mkdir -p "$BIN"
ln -s "$RDM_BIN" "$BIN/rdm"
HOOK_PATH="$BIN:$PATH"

# Convenience: `rdm review pending --format json` from a given cwd, plan/project
# resolved via env exactly as the host triggers resolve them.
pending_json() (
    cd "$1"
    RDM_ROOT="$PLAN" RDM_PROJECT="verify" "$RDM_BIN" review pending --format json
)

# Convenience: the real Stop hook template from a given cwd, fed a fresh-stop
# payload, with rdm on PATH and plan/project resolved via env.
run_stop_hook() (
    cd "$1"
    printf '%s' '{"stop_hook_active": false}' |
        RDM_ROOT="$PLAN" RDM_PROJECT="verify" PATH="$HOOK_PATH" sh "$HOOK_TEMPLATE"
)

# ----------------------------------------------------------------------------
# Step 1: seed a plan repo with two roadmaps (alpha, beta), each one phase.
# ----------------------------------------------------------------------------
say "Seeding plan repo with roadmaps alpha and beta (one phase each)"

"$RDM_BIN" --root "$PLAN" init --default-project verify >/dev/null
for rm in alpha beta; do
    "$RDM_BIN" --root "$PLAN" roadmap create "$rm" \
        --title "Roadmap ${rm}" --body "Isolation regression roadmap ${rm}." \
        --no-edit --project verify >/dev/null
    "$RDM_BIN" --root "$PLAN" phase create work \
        --title "Work" --number 1 --no-edit --roadmap "$rm" --project verify <<EOF >/dev/null
## Purpose

Implement-in-place phase for roadmap ${rm}.
EOF
done
# Each phase file's stem is `phase-1-work` (phase-<n>-<slug>); identifiers are
# `alpha/phase-1-work` and `beta/phase-1-work`.
ok "plan repo seeded with alpha/phase-1-work and beta/phase-1-work"

# ----------------------------------------------------------------------------
# Step 2: a source/code repo. Worktrees need a base commit on `main`.
# ----------------------------------------------------------------------------
say "Creating source repo with an initial commit on main"

git init --quiet -b main "$SRC"
(cd "$SRC" && git commit --quiet --allow-empty -m "chore: initial")
ok "source repo on main with a base commit"

# ----------------------------------------------------------------------------
# Step 3: a roadmap worktree per roadmap — the real phase-3 command. Each lands
# on branch `roadmap/<slug>` at a sibling path; capture the printed path.
# ----------------------------------------------------------------------------
say "Creating one roadmap worktree per roadmap (rdm worktree add)"

WT_ALPHA=$(cd "$SRC" && "$RDM_BIN" --root "$PLAN" worktree add alpha --project verify)
WT_BETA=$(cd "$SRC" && "$RDM_BIN" --root "$PLAN" worktree add beta --project verify)
[ -d "$WT_ALPHA" ] || fail "alpha worktree path not created: $WT_ALPHA"
[ -d "$WT_BETA" ] || fail "beta worktree path not created: $WT_BETA"
[ "$(cd "$WT_ALPHA" && git rev-parse --abbrev-ref HEAD)" = "roadmap/alpha" ] ||
    fail "alpha worktree is not on branch roadmap/alpha"
[ "$(cd "$WT_BETA" && git rev-parse --abbrev-ref HEAD)" = "roadmap/beta" ] ||
    fail "beta worktree is not on branch roadmap/beta"
ok "roadmap/alpha → $WT_ALPHA"
ok "roadmap/beta  → $WT_BETA"

# ----------------------------------------------------------------------------
# Step 4: implement-in-place + finalize. In each roadmap worktree make a commit
# (simulated phase work), then finalize phase-1-work to needs-review FROM THAT
# WORKTREE'S CWD so it stamps review_branch=roadmap/<slug> + review_sha.
# ----------------------------------------------------------------------------
say "Implementing in place and finalizing each phase to needs-review"

(cd "$WT_ALPHA" && git commit --quiet --allow-empty -m "feat: alpha phase 1 work")
(cd "$WT_ALPHA" && "$RDM_BIN" --root "$PLAN" phase update phase-1-work \
    --status needs-review --no-edit --roadmap alpha --project verify >/dev/null)
(cd "$WT_BETA" && git commit --quiet --allow-empty -m "feat: beta phase 1 work")
(cd "$WT_BETA" && "$RDM_BIN" --root "$PLAN" phase update phase-1-work \
    --status needs-review --no-edit --roadmap beta --project verify >/dev/null)
ok "alpha finalized on roadmap/alpha; beta finalized on roadmap/beta"

# ============================================================================
# Case A: Claude Stop hook host path.
#
# The real hook template emits {"decision":"block"} whenever `rdm review
# pending` (its single source of truth) reports an in-scope item. The block
# *reason* is an intentionally generic, static message — it names no roadmap —
# so we assert the SCOPING via the hook's data source (the same `review pending`
# the hook greps), and assert the hook's coarse block/allow signal directly.
# ============================================================================
say "Case A: Claude Stop hook blocks from each roadmap worktree, scoped to that roadmap"

HOOK_A=$(run_stop_hook "$WT_ALPHA")
printf '%s' "$HOOK_A" | grep -q '"decision":"block"' ||
    {
        printf '%s\n' "$HOOK_A" >&2
        fail "A: Stop hook should block from the alpha worktree"
    }
DATA_A=$(pending_json "$WT_ALPHA")
printf '%s' "$DATA_A" | grep -q 'alpha/phase-1-work' ||
    {
        printf '%s\n' "$DATA_A" >&2
        fail "A: alpha worktree's hook data must concern alpha"
    }
printf '%s' "$DATA_A" | grep -q 'beta/phase-1-work' &&
    {
        printf '%s\n' "$DATA_A" >&2
        fail "A: alpha worktree's hook data must NOT mention beta"
    }
ok "A: alpha worktree → block, scoped to alpha (silent about beta)"

HOOK_B=$(run_stop_hook "$WT_BETA")
printf '%s' "$HOOK_B" | grep -q '"decision":"block"' ||
    {
        printf '%s\n' "$HOOK_B" >&2
        fail "A: Stop hook should block from the beta worktree"
    }
DATA_B=$(pending_json "$WT_BETA")
printf '%s' "$DATA_B" | grep -q 'beta/phase-1-work' ||
    {
        printf '%s\n' "$DATA_B" >&2
        fail "A: beta worktree's hook data must concern beta"
    }
printf '%s' "$DATA_B" | grep -q 'alpha/phase-1-work' &&
    {
        printf '%s\n' "$DATA_B" >&2
        fail "A: beta worktree's hook data must NOT mention alpha"
    }
ok "A: beta worktree → block, scoped to beta (silent about alpha)"

# ============================================================================
# Case B: Pi agent_end host path.
#
# The Pi extension's contract (extension-review-on-finalize.ts): run `rdm review
# pending --format json`, collect items carrying an `identifier`, and inject the
# review prompt iff the collected list is non-empty. We replicate that exact
# decision in `sh` — NO cwd move — rather than booting Pi, because no Pi/JS
# runtime is available in a Rust CI job (mirroring how the sibling web-loop
# harness asserts `rdm review pending` rather than Claude's runtime).
# ============================================================================
say "Case B: Pi agent_end contract injects for the current roadmap only"

PI_A=$(pending_json "$WT_ALPHA")
printf '%s' "$PI_A" | grep -q '"identifier": "alpha/phase-1-work"' ||
    {
        printf '%s\n' "$PI_A" >&2
        fail "B: alpha pending JSON must carry alpha's identifier"
    }
printf '%s' "$PI_A" | grep -q '"branch": "roadmap/alpha"' ||
    {
        printf '%s\n' "$PI_A" >&2
        fail "B: alpha pending item must stamp branch roadmap/alpha"
    }
printf '%s' "$PI_A" | grep -q 'beta/phase-1-work' &&
    {
        printf '%s\n' "$PI_A" >&2
        fail "B: alpha pending JSON must carry no beta identifier"
    }
# Contract: a non-empty identifier list ⇒ the extension injects (for alpha only).
printf '%s' "$PI_A" | grep -q '"identifier"' ||
    {
        printf '%s\n' "$PI_A" >&2
        fail "B: extension would not inject — no identifier present"
    }
ok "B: alpha agent_end → inject for alpha only"

PI_B=$(pending_json "$WT_BETA")
printf '%s' "$PI_B" | grep -q '"identifier": "beta/phase-1-work"' ||
    {
        printf '%s\n' "$PI_B" >&2
        fail "B: beta pending JSON must carry beta's identifier"
    }
printf '%s' "$PI_B" | grep -q '"branch": "roadmap/beta"' ||
    {
        printf '%s\n' "$PI_B" >&2
        fail "B: beta pending item must stamp branch roadmap/beta"
    }
printf '%s' "$PI_B" | grep -q 'alpha/phase-1-work' &&
    {
        printf '%s\n' "$PI_B" >&2
        fail "B: beta pending JSON must carry no alpha identifier"
    }
ok "B: beta agent_end → inject for beta only"

# ============================================================================
# Case C: the isolation guarantee, stated explicitly.
#
# A/B above already prove A-trigger⇒A-only and B-trigger⇒B-only; this is the
# consolidated check: each roadmap worktree's `review pending` lists exactly its
# own roadmap, with the listed item's stamped branch matching its roadmap.
# ============================================================================
say "Case C: ISOLATION — each roadmap worktree sees only its own roadmap"

ISO_A=$(pending_json "$WT_ALPHA")
printf '%s' "$ISO_A" | grep -q 'alpha/phase-1-work' ||
    {
        printf '%s\n' "$ISO_A" >&2
        fail "C: alpha worktree must list alpha"
    }
printf '%s' "$ISO_A" | grep -q '"branch": "roadmap/alpha"' ||
    {
        printf '%s\n' "$ISO_A" >&2
        fail "C: alpha's item must carry branch roadmap/alpha"
    }
printf '%s' "$ISO_A" | grep -q 'beta/phase-1-work' &&
    {
        printf '%s\n' "$ISO_A" >&2
        fail "C: alpha worktree must NOT list beta"
    }

ISO_B=$(pending_json "$WT_BETA")
printf '%s' "$ISO_B" | grep -q 'beta/phase-1-work' ||
    {
        printf '%s\n' "$ISO_B" >&2
        fail "C: beta worktree must list beta"
    }
printf '%s' "$ISO_B" | grep -q '"branch": "roadmap/beta"' ||
    {
        printf '%s\n' "$ISO_B" >&2
        fail "C: beta's item must carry branch roadmap/beta"
    }
printf '%s' "$ISO_B" | grep -q 'alpha/phase-1-work' &&
    {
        printf '%s\n' "$ISO_B" >&2
        fail "C: beta worktree must NOT list alpha"
    }
ok "C: alpha⇒alpha-only and beta⇒beta-only, branches match"

# ============================================================================
# Case D: trigger-from-main robustness.
#
# From the source repo's `main` checkout a trigger must never misfire for a
# roadmap's in-flight review: branch-identity scopes the `roadmap/*`-stamped
# items out of `main`. Then the branch-gone variant: remove the alpha worktree
# and delete `roadmap/alpha`, and assert the same query from `main` still exits
# cleanly (no crash, still silent) — it cleanly reports the branch is gone.
# ============================================================================
say "Case D: trigger from main never misfires; branch-gone stays clean"

HOOK_MAIN=$(run_stop_hook "$SRC")
[ -z "$HOOK_MAIN" ] ||
    {
        printf '%s\n' "$HOOK_MAIN" >&2
        fail "D: Stop hook must NOT block from main"
    }
PENDING_MAIN=$(pending_json "$SRC")
printf '%s' "$PENDING_MAIN" | grep -q '"identifier"' &&
    {
        printf '%s\n' "$PENDING_MAIN" >&2
        fail "D: main checkout must see no pending items"
    }
ok "D: main checkout → no block, empty pending (roadmap items scoped out)"

# Branch-gone: tear down the alpha worktree + delete its branch.
(cd "$SRC" && git worktree remove "$WT_ALPHA" && git branch -D roadmap/alpha >/dev/null)
[ ! -d "$WT_ALPHA" ] || fail "D: alpha worktree should be gone after removal"

set +e
PENDING_GONE=$(pending_json "$SRC")
rc=$?
set -e
[ "$rc" -eq 0 ] ||
    {
        printf '%s\n' "$PENDING_GONE" >&2
        fail "D: review pending crashed after branch removal (rc=$rc)"
    }
printf '%s' "$PENDING_GONE" | grep -q '"identifier"' &&
    {
        printf '%s\n' "$PENDING_GONE" >&2
        fail "D: main must stay silent after branch removal"
    }
ok "D: after worktree+branch removal, review pending from main exits cleanly and silent"

# ----------------------------------------------------------------------------
# Done.
# ----------------------------------------------------------------------------
printf '\n\033[1;32mAll checks passed.\033[0m\n'
