#!/bin/sh
# Standalone review contract: real Git/CLI source resolution and persistence,
# injected independent reviewers, completeness failures, and legacy report mode.
# Shared finder/refuter semantics remain covered by verify-workflow-review.sh.
set -eu
SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH='' cd -- "$SCRIPT_DIR/.." && pwd)
cd "$REPO_ROOT"
cargo build --quiet -p rdm-cli
if command -v node >/dev/null 2>&1; then
    node "$SCRIPT_DIR/verify-review-source.mjs"
else
    mise exec node -- node "$SCRIPT_DIR/verify-review-source.mjs"
fi
sh "$SCRIPT_DIR/gen-workflow-review.sh" --check
sh "$SCRIPT_DIR/gen-skill-review.sh" --check --mode code
sh "$SCRIPT_DIR/gen-skill-review.sh" --check --mode plan
sh "$SCRIPT_DIR/gen-skill-review.sh" --check --mode code --target local
sh "$SCRIPT_DIR/gen-skill-review.sh" --check --mode plan --target local
