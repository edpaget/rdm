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
# Three regression cases, each its own OK/FAIL section:
#   A. Pi agent_end host path      — replicates the extension's decision contract
#      (run `rdm review pending --format json`, inject iff an item carries an
#      `identifier`). We assert the contract in `sh` rather than booting Pi: a
#      JS/Pi runtime is not available in a Rust CI job, exactly as the sibling
#      web-loop harness asserts `rdm review pending` rather than Claude's runtime.
#   B. Isolation assertion         — the consolidated guarantee: each roadmap
#      worktree's `review pending` lists its own roadmap and not the other, with
#      each item's stamped branch matching its roadmap.
#   C. Trigger-from-main robustness — from the source repo's `main` checkout a
#      trigger never misfires for an in-flight roadmap review (branch-identity
#      scopes the `roadmap/*` items out of `main`); the branch-gone variant
#      proves the same query still exits cleanly after the worktree + branch are
#      removed.
#   D. Amend-after-finalize restamp — amending the implementation commit while
#      an item is still needs-review orphans the stamped review_sha; `rdm review
#      restamp` re-points the stamp at the new HEAD so the item stays in scope
#      for `rdm review pending`.
#
# The needs-review Stop hook / Pi agent_end auto-review extension this harness
# used to also drive (hook-review-on-finalize.sh / extension-review-on-finalize.ts)
# was retired once phase 6 of the unify-code-review roadmap made review active on
# every finalize path — see docs/autonomous-loop.md. This harness now covers only
# the surviving `rdm review pending`/`restamp` scoping guarantees those hosts (and
# any future host) depend on.
#
# Run after touching the worktree/review-trigger model (rdm worktree, the
# branch-scoped `rdm review pending` filter, or the needs-review stamping in
# phase/task update).
#
# Requires: cargo-built rdm at target/debug/rdm (from this repo).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
RDM_BIN="$REPO_ROOT/target/debug/rdm"

if [ ! -x "$RDM_BIN" ]; then
    echo "error: $RDM_BIN not found or not executable — run 'cargo build' first." >&2
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

# Convenience: `rdm review pending --format json` from a given cwd, plan/project
# resolved via env exactly as the host triggers resolve them.
pending_json() (
    cd "$1"
    RDM_ROOT="$PLAN" RDM_PROJECT="verify" "$RDM_BIN" review pending --format json
)

# Convenience: `rdm review restamp --format json` from a given cwd, plan/project
# resolved via env exactly as the host triggers resolve them.
restamp_json() (
    cd "$1"
    RDM_ROOT="$PLAN" RDM_PROJECT="verify" "$RDM_BIN" review restamp --format json
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
# Case A: Pi agent_end host path.
#
# The Pi extension's contract (extension-plan-review-on-create.ts's sibling, in
# spirit, before the auto-review analog was retired): run `rdm review pending
# --format json`, collect items carrying an `identifier`, and inject the review
# prompt iff the collected list is non-empty. We replicate that exact decision
# in `sh` — NO cwd move — rather than booting Pi, because no Pi/JS runtime is
# available in a Rust CI job (mirroring how the sibling web-loop harness asserts
# `rdm review pending` rather than Claude's runtime).
# ============================================================================
say "Case A: Pi agent_end contract injects for the current roadmap only"

PI_A=$(pending_json "$WT_ALPHA")
printf '%s' "$PI_A" | grep -q '"identifier": "alpha/phase-1-work"' ||
    {
        printf '%s\n' "$PI_A" >&2
        fail "A: alpha pending JSON must carry alpha's identifier"
    }
printf '%s' "$PI_A" | grep -q '"branch": "roadmap/alpha"' ||
    {
        printf '%s\n' "$PI_A" >&2
        fail "A: alpha pending item must stamp branch roadmap/alpha"
    }
printf '%s' "$PI_A" | grep -q 'beta/phase-1-work' &&
    {
        printf '%s\n' "$PI_A" >&2
        fail "A: alpha pending JSON must carry no beta identifier"
    }
# Contract: a non-empty identifier list ⇒ the extension injects (for alpha only).
printf '%s' "$PI_A" | grep -q '"identifier"' ||
    {
        printf '%s\n' "$PI_A" >&2
        fail "A: extension would not inject — no identifier present"
    }
ok "A: alpha agent_end → inject for alpha only"

PI_B=$(pending_json "$WT_BETA")
printf '%s' "$PI_B" | grep -q '"identifier": "beta/phase-1-work"' ||
    {
        printf '%s\n' "$PI_B" >&2
        fail "A: beta pending JSON must carry beta's identifier"
    }
printf '%s' "$PI_B" | grep -q '"branch": "roadmap/beta"' ||
    {
        printf '%s\n' "$PI_B" >&2
        fail "A: beta pending item must stamp branch roadmap/beta"
    }
printf '%s' "$PI_B" | grep -q 'alpha/phase-1-work' &&
    {
        printf '%s\n' "$PI_B" >&2
        fail "A: beta pending JSON must carry no alpha identifier"
    }
ok "A: beta agent_end → inject for beta only"

# ============================================================================
# Case B: the isolation guarantee, stated explicitly.
#
# Case A above already proves alpha-trigger⇒alpha-only and beta-trigger⇒beta-only;
# this is the consolidated check: each roadmap worktree's `review pending` lists
# exactly its own roadmap, with the listed item's stamped branch matching its
# roadmap.
# ============================================================================
say "Case B: ISOLATION — each roadmap worktree sees only its own roadmap"

ISO_A=$(pending_json "$WT_ALPHA")
printf '%s' "$ISO_A" | grep -q 'alpha/phase-1-work' ||
    {
        printf '%s\n' "$ISO_A" >&2
        fail "B: alpha worktree must list alpha"
    }
printf '%s' "$ISO_A" | grep -q '"branch": "roadmap/alpha"' ||
    {
        printf '%s\n' "$ISO_A" >&2
        fail "B: alpha's item must carry branch roadmap/alpha"
    }
printf '%s' "$ISO_A" | grep -q 'beta/phase-1-work' &&
    {
        printf '%s\n' "$ISO_A" >&2
        fail "B: alpha worktree must NOT list beta"
    }

ISO_B=$(pending_json "$WT_BETA")
printf '%s' "$ISO_B" | grep -q 'beta/phase-1-work' ||
    {
        printf '%s\n' "$ISO_B" >&2
        fail "B: beta worktree must list beta"
    }
printf '%s' "$ISO_B" | grep -q '"branch": "roadmap/beta"' ||
    {
        printf '%s\n' "$ISO_B" >&2
        fail "B: beta's item must carry branch roadmap/beta"
    }
printf '%s' "$ISO_B" | grep -q 'alpha/phase-1-work' &&
    {
        printf '%s\n' "$ISO_B" >&2
        fail "B: beta worktree must NOT list alpha"
    }
ok "B: alpha⇒alpha-only and beta⇒beta-only, branches match"

# ============================================================================
# Case C: trigger-from-main robustness.
#
# From the source repo's `main` checkout a trigger must never misfire for a
# roadmap's in-flight review: branch-identity scopes the `roadmap/*`-stamped
# items out of `main`. Then the branch-gone variant: remove the alpha worktree
# and delete `roadmap/alpha`, and assert the same query from `main` still exits
# cleanly (no crash, still silent) — it cleanly reports the branch is gone.
# ============================================================================
say "Case C: trigger from main never misfires; branch-gone stays clean"

PENDING_MAIN=$(pending_json "$SRC")
printf '%s' "$PENDING_MAIN" | grep -q '"identifier"' &&
    {
        printf '%s\n' "$PENDING_MAIN" >&2
        fail "C: main checkout must see no pending items"
    }
ok "C: main checkout → empty pending (roadmap items scoped out)"

# Branch-gone: tear down the alpha worktree + delete its branch.
(cd "$SRC" && git worktree remove "$WT_ALPHA" && git branch -D roadmap/alpha >/dev/null)
[ ! -d "$WT_ALPHA" ] || fail "C: alpha worktree should be gone after removal"

set +e
PENDING_GONE=$(pending_json "$SRC")
rc=$?
set -e
[ "$rc" -eq 0 ] ||
    {
        printf '%s\n' "$PENDING_GONE" >&2
        fail "C: review pending crashed after branch removal (rc=$rc)"
    }
printf '%s' "$PENDING_GONE" | grep -q '"identifier"' &&
    {
        printf '%s\n' "$PENDING_GONE" >&2
        fail "C: main must stay silent after branch removal"
    }
ok "C: after worktree+branch removal, review pending from main exits cleanly and silent"

# ============================================================================
# Case D: amend-after-finalize restamp.
#
# Amending (or rebasing) the implementation commit while an item is still
# needs-review orphans the stamped review_sha. `rdm review restamp` re-points
# the stamp at the new HEAD so the item stays in scope for `rdm review pending`.
# We use the still-live beta worktree.
# ============================================================================
say "Case D: amend after finalize → restamp refreshes the stamp, still in scope"

SHA_BEFORE=$(cd "$WT_BETA" && git rev-parse HEAD)
# Amend the implementation commit while beta/phase-1-work is still needs-review.
(cd "$WT_BETA" && git commit --quiet --amend --allow-empty -m "feat: beta phase 1 work (amended)")
SHA_AFTER=$(cd "$WT_BETA" && git rev-parse HEAD)
[ "$SHA_BEFORE" != "$SHA_AFTER" ] || fail "D: amend must move HEAD"

# Explicit restamp refreshes beta's stamp to the new HEAD (JSON exposes "sha").
RESTAMP_D=$(restamp_json "$WT_BETA")
printf '%s' "$RESTAMP_D" | grep -q '"identifier": "beta/phase-1-work"' ||
    {
        printf '%s\n' "$RESTAMP_D" >&2
        fail "D: restamp must report beta/phase-1-work as refreshed"
    }
printf '%s' "$RESTAMP_D" | grep -q "\"sha\": \"$SHA_AFTER\"" ||
    {
        printf '%s\n' "$RESTAMP_D" >&2
        fail "D: restamp must re-point the stamp at the amended HEAD ($SHA_AFTER)"
    }

# Idempotent: a second restamp with no intervening commit is a no-op.
RESTAMP_D2=$(restamp_json "$WT_BETA")
printf '%s' "$RESTAMP_D2" | grep -q '"identifier"' &&
    {
        printf '%s\n' "$RESTAMP_D2" >&2
        fail "D: second restamp must be a no-op (idempotent)"
    }

# End-to-end: the item is still reported pending post-amend, in scope for beta.
PENDING_D=$(pending_json "$WT_BETA")
printf '%s' "$PENDING_D" | grep -q '"identifier": "beta/phase-1-work"' ||
    {
        printf '%s\n' "$PENDING_D" >&2
        fail "D: beta/phase-1-work must still be pending from the beta worktree after amend"
    }
ok "D: amend → restamp refreshes to new HEAD, idempotent, still pending in scope"

# ----------------------------------------------------------------------------
# Done.
# ----------------------------------------------------------------------------
printf '\n\033[1;32mAll checks passed.\033[0m\n'
