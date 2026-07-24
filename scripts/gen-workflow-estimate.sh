#!/bin/sh
# Stamp the estimate-core block into every workflow-script consumer.
#
# The Claude Code Workflow runtime cannot import/require a helper module (proven
# by the P1 import spike — see docs/workflow-schemas.md § "Import spike"), so the
# shared estimate orchestration is kept single-source in
# `.claude/workflows/lib/estimate.mjs` (the canonical estimate source) and copied
# VERBATIM into each consumer between matching marker comments. Edit the lib,
# then run this script.
#
# Usage:
#   scripts/gen-workflow-estimate.sh           # rewrite consumers in place
#   scripts/gen-workflow-estimate.sh --check   # exit non-zero if any consumer drifted
#
# The `--check` mode is what scripts/verify-workflow-estimate.sh and CI use to
# prove no consumer was hand-edited out of sync with the source of truth.

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)

SOURCE="$REPO_ROOT/.claude/workflows/lib/estimate.mjs"
BEGIN='estimate-core:begin'
END='estimate-core:end'

# Detect --check from the ORIGINAL args before the positional list is replaced
# with the consumer paths below.
CHECK=0
if [ "${1:-}" = "--check" ]; then
    CHECK=1
fi

# The list of consumers that embed the block. Add new consumers here — they are
# kept in sync automatically. autopilot reuses the block for its estimate
# pre-pass (selectUnestimated / buildEstimateWritebackPrompt).
set -- "$REPO_ROOT/.claude/workflows/estimate.js" \
    "$REPO_ROOT/.claude/workflows/autopilot.js" \
    "$REPO_ROOT/.claude/workflows/lib/autopilot.mjs"

if [ ! -f "$SOURCE" ]; then
    echo "error: source of truth not found: $SOURCE" >&2
    exit 1
fi

# Extract the block strictly between the marker lines of the source. Match the
# markers only where they follow the "// >>> " comment prefix, so an incidental
# mention of the marker token inside the block can't be mistaken for a real
# marker line and silently truncate extraction.
blockfile=$(mktemp)
trap 'rm -f "$blockfile"' EXIT INT HUP TERM

awk -v b=">>> $BEGIN" -v e=">>> $END" '
    index($0, b) { infence = 1; next }
    index($0, e) { infence = 0 }
    infence { print }
' "$SOURCE" >"$blockfile"

if [ ! -s "$blockfile" ]; then
    echo "error: no block found between '$BEGIN' / '$END' markers in $SOURCE" >&2
    exit 1
fi

status=0
for consumer in "$@"; do
    if [ ! -f "$consumer" ]; then
        echo "error: consumer not found: $consumer" >&2
        exit 1
    fi
    if ! grep -q ">>> $BEGIN" "$consumer" || ! grep -q ">>> $END" "$consumer"; then
        echo "error: consumer $consumer is missing the '>>> $BEGIN'/'>>> $END' markers" >&2
        exit 1
    fi

    # Assemble: consumer head (through its begin marker line) + block + consumer
    # tail (from its end marker line onward). The consumer keeps its OWN marker
    # lines; only the region between them is replaced. Markers are matched only
    # after the "// >>> " comment prefix (index()), so an incidental in-block
    # mention of the token can't be mistaken for a marker.
    out=$(mktemp)
    awk -v b=">>> $BEGIN" -v e=">>> $END" -v bf="$blockfile" '
        BEGIN { state = 0 }
        state == 0 {
            print
            if (index($0, b)) {
                while ((getline line < bf) > 0) print line
                close(bf)
                state = 1
            }
            next
        }
        state == 1 {
            if (index($0, e)) { print; state = 2 }
            next
        }
        state == 2 { print }
    ' "$consumer" >"$out"

    if [ "$CHECK" -eq 1 ]; then
        if ! diff -u "$consumer" "$out" >/dev/null 2>&1; then
            echo "DRIFT: $consumer is out of sync with $SOURCE — run scripts/gen-workflow-estimate.sh" >&2
            diff -u "$consumer" "$out" >&2 || true
            status=1
        fi
    else
        if diff -q "$consumer" "$out" >/dev/null 2>&1; then
            echo "unchanged: $consumer"
        else
            cp "$out" "$consumer"
            echo "regenerated: $consumer"
        fi
    fi
    rm -f "$out"
done

exit "$status"
