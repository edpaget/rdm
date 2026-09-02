#!/bin/sh
# Re-bless entry point for tests/golden/*.json — rdm's frozen machine-facing
# `--format json` contract snapshots (see tests/golden/README.md).
#
# Run this whenever a DELIBERATE JSON shape change needs to be blessed:
#
#   bash scripts/capture-golden.sh
#   git diff tests/golden/       # review the diff before committing
#
# scripts/verify-golden-json.sh is the drift check that fails CI when the
# committed goldens no longer match what rdm actually emits.

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
RDM_BIN="$REPO_ROOT/target/debug/rdm"
export RDM_BIN

if [ ! -x "$RDM_BIN" ]; then
    echo "error: $RDM_BIN not found or not executable — run 'cargo build' first." >&2
    exit 1
fi

# shellcheck disable=SC1091
. "$SCRIPT_DIR/lib/rdm-plan-fixture.sh"
# shellcheck disable=SC1091
. "$SCRIPT_DIR/lib/golden-capture.sh"

# Per the fixture library's documented convention: register cleanup
# immediately after sourcing, so an abort mid-capture (this script runs
# under `set -eu`) never leaks a fixture. golden_capture_all itself tears
# down on any failure it detects; this trap covers everything else (a
# signal, or the final success path once golden_redact has run).
trap 'fixture_teardown' EXIT INT HUP TERM

OUT_DIR="$REPO_ROOT/tests/golden"
mkdir -p "$OUT_DIR"

golden_capture_all "$OUT_DIR"
golden_redact "$OUT_DIR"

echo "Golden JSON files re-captured and redacted under $OUT_DIR"
echo "Review with: git diff tests/golden/"
