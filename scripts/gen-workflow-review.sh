#!/bin/sh
# Stamp the review-refute-fix and plan-review-driver blocks into every
# workflow-script consumer, then sync each consumer's embedded
# rdm-core/src/templates/workflows/ copy to match.
#
# The Claude Code Workflow runtime cannot import/require a helper module (proven
# by the P1 import spike — see docs/workflow-schemas.md § "Import spike"), so the
# shared review pipeline is kept single-source in
# `.claude/workflows/lib/review.mjs` (the canonical review source) and copied
# VERBATIM into each consumer between matching marker comments. The standalone
# plan-review workflow's driver (argument parsing + dependency-injected
# orchestration) is likewise kept single-source in
# `.claude/workflows/lib/plan-review.mjs` and copied verbatim into its
# `plan-review-driver` block. Edit a lib, then run this script. Its sibling
# `scripts/gen-skill-review.sh` renders the review pipeline's `//|` spec prose
# from the SAME review.mjs source into the shipped review skill templates.
#
# The embedded copies under `rdm-core/src/templates/workflows/` — what
# `include_str!` ships into `rdm agent-config claude --skills`/`--plugin` — are
# kept in sync with the `.claude/workflows/` consumers this script writes, as a
# whole-file copy, in the same run. There is no separate regeneration step for
# them.
#
# Usage:
#   scripts/gen-workflow-review.sh           # rewrite consumers in place
#   scripts/gen-workflow-review.sh --check   # exit non-zero if anything drifted
#
# The `--check` mode is what scripts/verify-workflow-review.sh and CI use to
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

# The list of workflow-script consumers that embed the review-refute-fix
# block. Add new consumers here — they are kept in sync automatically. Engine
# scripts carry the `rdm-wf-` prefix that distinguishes them from the
# `rdm-*` skill front doors.
stamp_block \
    "$WORKFLOWS_DIR/lib/review.mjs" \
    'review-refute-fix:begin' \
    'review-refute-fix:end' \
    "$CHECK" \
    "$WORKFLOWS_DIR/rdm-wf-review-refute-fix.js" \
    "$WORKFLOWS_DIR/rdm-wf-plan-review.js" || status=1

# rdm-wf-plan-review.js additionally embeds the plan-review driver block.
stamp_block \
    "$WORKFLOWS_DIR/lib/plan-review.mjs" \
    'plan-review-driver:begin' \
    'plan-review-driver:end' \
    "$CHECK" \
    "$WORKFLOWS_DIR/rdm-wf-plan-review.js" || status=1

# Sync every consumer this script writes into its embedded
# rdm-core/src/templates/workflows/ copy.
for basename in rdm-wf-review-refute-fix.js rdm-wf-plan-review.js; do
    sync_full_copy \
        "$WORKFLOWS_DIR/$basename" \
        "$EMBEDDED_DIR/$basename" \
        "$CHECK" || status=1
done

exit "$status"
