#!/bin/sh
# Development-only entrypoint: rebuild this checkout before every invocation.
set -eu
SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH='' cd -- "$SCRIPT_DIR/.." && pwd)
: "${RDM_ROOT:?Set RDM_ROOT to the absolute path of the separate plan repository}"
: "${RDM_SESSION:?Set one stable RDM_SESSION for this conversation and reuse it on every call}"
case "$RDM_ROOT" in
    /*) ;;
    *)
        printf '%s\n' 'RDM_ROOT must be an absolute path to the separate plan repository.' >&2
        exit 1
        ;;
esac
if [ ! -d "$RDM_ROOT" ] || [ ! -r "$RDM_ROOT" ]; then
    printf 'Plan repository is missing or unreadable: %s\n' "$RDM_ROOT" >&2
    exit 1
fi
export RDM_PROJECT="${RDM_PROJECT:-rdm}"
exec cargo run --quiet --manifest-path "$REPO_ROOT/Cargo.toml" --bin rdm -- "$@"
