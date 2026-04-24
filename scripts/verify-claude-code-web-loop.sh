#!/bin/sh
# End-to-end verification for the Claude Code web sandbox loop.
#
# Stands up temp dirs simulating the GitHub remotes + a sandbox, runs the
# SessionStart template, and drives a complete
#   bootstrap → phase show → source-repo commit with Done: → phase status flips
# round-trip using bare clones in place of GitHub. No network; hermetic.
#
# Intended to catch regressions across:
#   - templates/claude-code-web/.claude/hooks/SessionStart.sh
#   - rdm bootstrap
#   - rdm hook post-commit (source-repo side)
#
# Requires: cargo-built rdm at target/debug/rdm (from this repo).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
RDM_BIN="$REPO_ROOT/target/debug/rdm"
TEMPLATE_DIR="$REPO_ROOT/templates/claude-code-web"

if [ ! -x "$RDM_BIN" ]; then
    echo "error: $RDM_BIN not found or not executable — run 'cargo build' first." >&2
    exit 1
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

# Clear rdm-related env vars inherited from the caller's shell so the
# simulated sandbox doesn't pick up the developer's real plan repo.
unset RDM_ROOT RDM_PROJECT RDM_STAGE RDM_FORMAT RDM_PLAN_REPO RDM_PLAN_REPO_TOKEN RDM_PLAN_REPO_PATH

# Git identity so `git commit` never falls back to a missing global config.
export GIT_AUTHOR_NAME="verify-bot"
export GIT_AUTHOR_EMAIL="verify@example.invalid"
export GIT_COMMITTER_NAME="verify-bot"
export GIT_COMMITTER_EMAIL="verify@example.invalid"

say() { printf '\n\033[1;34m==>\033[0m %s\n' "$*"; }
fail() { printf '\n\033[1;31m[FAIL]\033[0m %s\n' "$*" >&2; exit 1; }
ok() { printf '\033[1;32m[ OK ]\033[0m %s\n' "$*"; }

# ----------------------------------------------------------------------------
# Step 1: seed a plan repo with one roadmap + phase and push to a bare remote.
# ----------------------------------------------------------------------------
say "Seeding plan repo with verify-demo/phase-1-ping"

PLAN_SEED="$TMP/plan-seed"
PLAN_ORIGIN="$TMP/plan-origin.git"
# rdm init with a fresh XDG_CONFIG_HOME so we don't pollute the real one.
SEED_XDG="$TMP/seed-xdg"
mkdir -p "$SEED_XDG"

XDG_CONFIG_HOME="$SEED_XDG" "$RDM_BIN" --root "$PLAN_SEED" init --default-project verify >/dev/null
XDG_CONFIG_HOME="$SEED_XDG" "$RDM_BIN" --root "$PLAN_SEED" roadmap create verify-demo \
    --title "Verify Demo" --body "End-to-end verification roadmap." --no-edit --project verify >/dev/null
XDG_CONFIG_HOME="$SEED_XDG" "$RDM_BIN" --root "$PLAN_SEED" phase create ping \
    --title "Ping" --number 1 --no-edit --roadmap verify-demo --project verify <<'EOF' >/dev/null
## Purpose

End-to-end verification ping phase.
EOF
# The phase file's stem is `phase-1-ping` (phase-<n>-<slug>).

git clone --quiet --bare "$PLAN_SEED" "$PLAN_ORIGIN"
ok "plan repo seeded and pushed to bare remote"

# ----------------------------------------------------------------------------
# Step 2: simulate a fresh sandbox with a shim rdm on PATH and run
# SessionStart.sh.
# ----------------------------------------------------------------------------
say "Running SessionStart template in a simulated sandbox"

SANDBOX_HOME="$TMP/sandbox-home"
mkdir -p "$SANDBOX_HOME/.local/bin"
ln -s "$RDM_BIN" "$SANDBOX_HOME/.local/bin/rdm"

SANDBOX_XDG_CONFIG="$SANDBOX_HOME/.config"
SANDBOX_XDG_DATA="$SANDBOX_HOME/.local/share"

RUN_SANDBOX_LOG="$TMP/sessionstart.log"
set +e
HOME="$SANDBOX_HOME" \
    XDG_CONFIG_HOME="$SANDBOX_XDG_CONFIG" \
    XDG_DATA_HOME="$SANDBOX_XDG_DATA" \
    PATH="$SANDBOX_HOME/.local/bin:$PATH" \
    RDM_PLAN_REPO="file://$PLAN_ORIGIN" \
    RDM_DEFAULT_PROJECT="verify" \
    bash "$TEMPLATE_DIR/.claude/hooks/SessionStart.sh" >"$RUN_SANDBOX_LOG" 2>&1
rc=$?
set -e
if [ "$rc" -ne 0 ]; then
    cat "$RUN_SANDBOX_LOG" >&2
    fail "SessionStart.sh exited $rc"
fi

grep -q "Plan repo ready" "$RUN_SANDBOX_LOG" \
    || { cat "$RUN_SANDBOX_LOG" >&2; fail "SessionStart.sh did not print 'Plan repo ready'"; }
ok "SessionStart.sh ran and reported success"

SANDBOX_PLAN="$SANDBOX_XDG_DATA/rdm/plan-repo"
[ -f "$SANDBOX_PLAN/rdm.toml" ] || fail "no rdm.toml in sandbox plan repo clone"
ok "plan repo cloned into $SANDBOX_PLAN"

# ----------------------------------------------------------------------------
# Step 3: verify rdm commands work against the bootstrapped clone.
# ----------------------------------------------------------------------------
say "Reading seeded roadmap and phase from inside the sandbox"

if ! env HOME="$SANDBOX_HOME" XDG_CONFIG_HOME="$SANDBOX_XDG_CONFIG" XDG_DATA_HOME="$SANDBOX_XDG_DATA" \
        "$RDM_BIN" roadmap list --project verify | grep -q verify-demo; then
    fail "roadmap 'verify-demo' not visible in sandbox"
fi
ok "rdm roadmap list sees verify-demo"

if ! env HOME="$SANDBOX_HOME" XDG_CONFIG_HOME="$SANDBOX_XDG_CONFIG" XDG_DATA_HOME="$SANDBOX_XDG_DATA" \
        "$RDM_BIN" phase show phase-1-ping --roadmap verify-demo --project verify --no-body --format json \
        | grep -q '"status": "not-started"'; then
    fail "seeded phase not found or not in 'not-started' state"
fi
ok "rdm phase show reports not-started"

# ----------------------------------------------------------------------------
# Step 4: create a source repo, make a commit with a Done: directive, and
# simulate the source-repo post-commit hook that updates the plan repo.
# ----------------------------------------------------------------------------
say "Creating source repo and committing with a Done: directive"

SOURCE="$TMP/source"
git init --quiet "$SOURCE"
(
    cd "$SOURCE"
    git commit --quiet --allow-empty -m "chore: initial"
    echo "hello" > feature.txt
    git add feature.txt
    git commit --quiet -m "feat: implement ping

Done: verify-demo/phase-1-ping"
)
SOURCE_SHA=$(cd "$SOURCE" && git rev-parse HEAD)
ok "source repo commit: $SOURCE_SHA"

# ----------------------------------------------------------------------------
# Step 5: run `rdm hook post-commit` with cwd=source to apply the Done:
# directive to the sandbox plan repo (what the user's source-repo hook does).
# ----------------------------------------------------------------------------
say "Running rdm hook post-commit against the sandbox plan repo"

# Ensure default branch matches plan repo's default branch (main).
(cd "$SOURCE" && git branch -M main >/dev/null 2>&1 || true)

(
    cd "$SOURCE"
    env HOME="$SANDBOX_HOME" XDG_CONFIG_HOME="$SANDBOX_XDG_CONFIG" XDG_DATA_HOME="$SANDBOX_XDG_DATA" \
        "$RDM_BIN" --root "$SANDBOX_PLAN" hook post-commit
)

# ----------------------------------------------------------------------------
# Step 6: verify the plan repo phase is now done with the source commit SHA.
# ----------------------------------------------------------------------------
say "Confirming phase flipped to done with source commit SHA"

PHASE_JSON=$(env HOME="$SANDBOX_HOME" XDG_CONFIG_HOME="$SANDBOX_XDG_CONFIG" XDG_DATA_HOME="$SANDBOX_XDG_DATA" \
    "$RDM_BIN" phase show phase-1-ping --roadmap verify-demo --project verify --no-body --format json)

echo "$PHASE_JSON" | grep -q '"status": "done"' \
    || { echo "$PHASE_JSON" >&2; fail "phase did not flip to done"; }
echo "$PHASE_JSON" | grep -q "$SOURCE_SHA" \
    || { echo "$PHASE_JSON" >&2; fail "phase did not record source commit SHA $SOURCE_SHA"; }
ok "phase 'done' with commit $SOURCE_SHA"

# ----------------------------------------------------------------------------
# Step 7: push the plan-repo update back to the bare remote to close the
# loop (what a user would do before pulling on another machine).
# ----------------------------------------------------------------------------
say "Pushing plan repo update to origin bare"

(cd "$SANDBOX_PLAN" && git push --quiet origin HEAD)
ok "plan repo update visible in $PLAN_ORIGIN"

# ----------------------------------------------------------------------------
# Done.
# ----------------------------------------------------------------------------
printf '\n\033[1;32mAll checks passed.\033[0m\n'
