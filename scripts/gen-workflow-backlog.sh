#!/bin/sh
# Stamp the backlog-groom block into its workflow-script consumer, then sync
# the consumer's embedded rdm-core/src/templates/workflows/ copy to match.
#
# The Claude Code Workflow runtime cannot import/require a helper module (proven
# by the P1 import spike — see docs/workflow-schemas.md § "Import spike"), so the
# pure control logic for the batched backlog-grooming pass is kept single-source
# in `.claude/workflows/lib/backlog.mjs` (the canonical source) and copied
# VERBATIM into `rdm-wf-backlog.js` between matching marker comments. Edit the
# lib, then run this script.
#
# The embedded copy under `rdm-core/src/templates/workflows/` — what
# `include_str!` ships into `rdm agent-config claude --skills`/`--plugin` — is
# kept in sync with the `.claude/workflows/rdm-wf-backlog.js` consumer this
# script writes, as a whole-file copy, in the same run. There is no separate
# regeneration step for it.
#
# Usage:
#   scripts/gen-workflow-backlog.sh           # rewrite the consumer in place
#   scripts/gen-workflow-backlog.sh --check   # exit non-zero if anything drifted
#
# The `--check` mode is what scripts/verify-workflow-backlog.sh and CI use to
# prove the consumer and embedded copy were not hand-edited out of sync with
# the source of truth.

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)

# shellcheck disable=SC1091
. "$REPO_ROOT/scripts/lib/gen-workflow-block.sh"

WORKFLOWS_DIR="$REPO_ROOT/.claude/workflows"
EMBEDDED_DIR="$REPO_ROOT/rdm-core/src/templates/workflows"

# Detect --check from the ORIGINAL args.
CHECK=0
if [ "${1:-}" = "--check" ]; then
    CHECK=1
fi

status=0

stamp_block \
    "$WORKFLOWS_DIR/lib/backlog.mjs" \
    'backlog-groom:begin' \
    'backlog-groom:end' \
    "$CHECK" \
    "$WORKFLOWS_DIR/rdm-wf-backlog.js" || status=1

sync_full_copy \
    "$WORKFLOWS_DIR/rdm-wf-backlog.js" \
    "$EMBEDDED_DIR/rdm-wf-backlog.js" \
    "$CHECK" || status=1

exit "$status"
