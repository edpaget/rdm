#!/bin/sh
# Generate the local Codex lane from the same templates shipped downstream.
set -eu
SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH='' cd -- "$SCRIPT_DIR/.." && pwd)
node "$SCRIPT_DIR/gen-codex-runtime.mjs"
exec cargo run --quiet --manifest-path "$REPO_ROOT/Cargo.toml" --bin rdm -- \
    agent-config codex --skills --project rdm --principles-file docs/principles.md --out "$REPO_ROOT"
