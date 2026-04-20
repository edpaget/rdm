#!/bin/sh
# Claude Code web session-start hook for rdm.
#
# Installs rdm (if missing) and clones the configured plan repo into a cache
# path, then points rdm's global config at it so subsequent rdm calls in the
# session find the plan repo without further setup.
#
# Required env:
#   RDM_PLAN_REPO         Git URL of the plan repo to bootstrap.
#
# Optional env:
#   RDM_DEFAULT_PROJECT   Name of the default project inside the plan repo.
#   RDM_PLAN_REPO_PATH    Override the local bootstrap path.
#
# Idempotent: safe to run on every session start.

set -eu

if [ -z "${RDM_PLAN_REPO:-}" ]; then
    echo "[rdm hook] RDM_PLAN_REPO is not set; skipping plan-repo bootstrap."
    echo "[rdm hook] Set it in Claude Code sandbox env to enable this hook."
    exit 0
fi

INSTALL_DIR="${RDM_INSTALL_DIR:-$HOME/.local/bin}"

if ! command -v rdm >/dev/null 2>&1; then
    echo "[rdm hook] rdm not found on PATH; installing to $INSTALL_DIR"
    mkdir -p "$INSTALL_DIR"
    curl --proto '=https' --tlsv1.2 -fsSL \
        https://github.com/edpaget/rdm/releases/latest/download/install.sh \
        | sh -s -- --dir "$INSTALL_DIR"
    PATH="$INSTALL_DIR:$PATH"
    export PATH
fi

BOOTSTRAP_ARGS="--plan-repo $RDM_PLAN_REPO"
if [ -n "${RDM_PLAN_REPO_PATH:-}" ]; then
    BOOTSTRAP_ARGS="$BOOTSTRAP_ARGS --path $RDM_PLAN_REPO_PATH"
fi

# shellcheck disable=SC2086  # intentional word-splitting on BOOTSTRAP_ARGS
BOOTSTRAP_OUTPUT=$(rdm bootstrap $BOOTSTRAP_ARGS)
echo "$BOOTSTRAP_OUTPUT"

RDM_ROOT_RESOLVED=$(
    echo "$BOOTSTRAP_OUTPUT" \
        | sed -n 's/^Plan repo ready at //p' \
        | head -n1
)
if [ -z "$RDM_ROOT_RESOLVED" ]; then
    echo "[rdm hook] could not parse RDM_ROOT from bootstrap output; continuing." >&2
    exit 0
fi

rdm config set root "$RDM_ROOT_RESOLVED" --global >/dev/null

if [ -n "${RDM_DEFAULT_PROJECT:-}" ]; then
    rdm config set default_project "$RDM_DEFAULT_PROJECT" --global >/dev/null
fi

echo "[rdm hook] Plan repo ready: $RDM_ROOT_RESOLVED"
if [ -n "${RDM_DEFAULT_PROJECT:-}" ]; then
    echo "[rdm hook] Default project: $RDM_DEFAULT_PROJECT"
fi
echo "[rdm hook] Try: rdm roadmap list"
