#!/bin/sh
# Stamp the estimate-core block into every workflow-script consumer, then sync
# each consumer's embedded rdm-core/src/templates/workflows/ copy to match.
#
# The Claude Code Workflow runtime cannot import/require a helper module (proven
# by the P1 import spike — see docs/workflow-schemas.md § "Import spike"), so the
# shared estimate orchestration is kept single-source in
# `.claude/workflows/lib/estimate.mjs` (the canonical estimate source) and copied
# VERBATIM into each consumer between matching marker comments. Edit the lib,
# then run this script.
#
# The embedded copy under `rdm-core/src/templates/workflows/` — what
# `include_str!` ships into `rdm agent-config claude --skills`/`--plugin` — is
# kept in sync with the `.claude/workflows/rdm-wf-estimate.js` consumer this
# script writes, as a whole-file copy, in the same run. There is no separate
# regeneration step for it.
#
# Usage:
#   scripts/gen-workflow-estimate.sh           # rewrite consumers in place
#   scripts/gen-workflow-estimate.sh --check   # exit non-zero if anything drifted
#
# The `--check` mode is what scripts/verify-workflow-estimate.sh and CI use to
# prove no consumer or embedded copy was hand-edited out of sync with the
# source of truth.

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

# The list of consumers that embed the block. Add new consumers here — they are
# kept in sync automatically. autopilot (now the prose `rdm-autopilot` skill,
# not a workflow script) invokes the real `rdm-wf-estimate` Workflow directly
# via the Workflow tool for its estimate pre-pass, rather than reusing a stamped
# copy of this block — so it is not a consumer here.
stamp_block \
    "$WORKFLOWS_DIR/lib/estimate.mjs" \
    'estimate-core:begin' \
    'estimate-core:end' \
    "$CHECK" \
    "$WORKFLOWS_DIR/rdm-wf-estimate.js" || status=1

sync_full_copy \
    "$WORKFLOWS_DIR/rdm-wf-estimate.js" \
    "$EMBEDDED_DIR/rdm-wf-estimate.js" \
    "$CHECK" || status=1

exit "$status"
