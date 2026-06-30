#!/bin/sh
# Stop hook: reprompt the agent to run rdm-review while an item is in `needs-review`.
#
# The `needs-review` status itself is the sentinel — there is no marker file. Once
# rdm-review moves the item to `reviewed` (or any other status), the next stop finds
# nothing pending and is allowed. `stop_hook_active` short-circuits the reprompt loop.
#
# Scope: `rdm review pending` reports the needs-review items in scope for this
# checkout — by branch identity (the item's stamped `review_branch` equals the
# current branch), falling back to SHA reachability for legacy/unstamped items or
# an unresolvable branch (fail open). This keeps a session finishing branch A from
# being reprompted to review an item finalized on branch B, whose diff isn't even
# checked out here. It is the single shared source of truth for "what is in scope
# to review" — the rdm-review skill consults the same command.
#
# Stale-stamp refresh: `rdm review restamp` runs first (fail-open) so that a commit
# amended or rebased while an item is still needs-review re-points its stamp at the
# current HEAD/branch instead of going stale and silently dropping out of scope. It
# is idempotent (no-op when nothing moved), so calling it every turn is cheap.
#
# One-worktree-per-roadmap model: this hook fires from the roadmap worktree's cwd
# while it sits on the `roadmap/<slug>` branch. Every phase of the roadmap is
# finalized in place on that same branch, so the branch-scoped filter resolves
# exactly the roadmap's stamped, in-scope items — no per-phase worktree and no
# nested-move assumption.
#
# Dependencies: POSIX `sh`, `grep`, and the rdm binary only (no `jq`).
#
# Manual reproduction (from repo root):
#   1. cargo build
#   2. Put an item in needs-review on THIS branch, then BLOCK:
#        ./target/debug/rdm task update <slug> --status needs-review --no-edit --project rdm
#        echo '{"stop_hook_active": false}' | CLAUDE_PROJECT_DIR="$PWD" .claude/hooks/rdm-review-on-finalize.sh
#        # expect: {"decision":"block","reason":"..."} on stdout
#      Cross-branch check: finalize an item on another branch, switch back here, then
#      run the same BLOCK invocation — expect exit 0, no output (out of scope).
#   3. ALLOW on stop_hook_active (loop prevention):
#        echo '{"stop_hook_active": true}' | CLAUDE_PROJECT_DIR="$PWD" .claude/hooks/rdm-review-on-finalize.sh
#        # expect: exit 0, no output
#   4. Move out of needs-review, then ALLOW (nothing pending):
#        ./target/debug/rdm task update <slug> --status reviewed --no-edit --project rdm
#        echo '{"stop_hook_active": false}' | CLAUDE_PROJECT_DIR="$PWD" .claude/hooks/rdm-review-on-finalize.sh
#        # expect: exit 0, no output

input=$(cat)

# Loop prevention: if we're already inside a stop-hook-triggered continuation, allow.
# This is a deliberate grep heuristic (no jq by design); the Stop payload is emitted by
# Claude Code, so a substring match on the field is sufficient and safe here.
if printf '%s' "$input" | grep -Eq '"stop_hook_active"[[:space:]]*:[[:space:]]*true'; then
    exit 0
fi

PROJECT_DIR="${CLAUDE_PROJECT_DIR:-$PWD}"
RDM="$PROJECT_DIR/target/debug/rdm"

# Build the dev binary if it is missing. Fail open on build failure.
if [ ! -x "$RDM" ]; then
    if ! (cd "$PROJECT_DIR" && cargo build) >/dev/null 2>&1; then
        exit 0
    fi
fi

# Refresh any stale stamp first (fail-open): re-point review_sha/review_branch at the
# current HEAD/branch so an amended/rebased commit doesn't drop the item out of scope.
"$RDM" review restamp --project rdm >/dev/null 2>&1 || true

# `rdm review pending` returns the needs-review phases AND tasks that are in scope for
# the current source-repo branch (stamped-and-reachable, or unstamped/fail-open). It is
# the single shared source of truth for the hook and the rdm-review skill.
# An empty result is the literal `[]`; a pending item carries an "identifier" field.
pending=$("$RDM" review pending --project rdm --format json 2>/dev/null)

if printf '%s' "$pending" | grep -q '"identifier"'; then
    cat <<'EOF'
{"decision":"block","reason":"There are rdm item(s) in `needs-review` for project rdm. Before stopping, invoke the rdm-review skill on the needs-review item(s): categorize the findings — fix small issues inline, and file large ones as rdm tasks. If review passes, set the item's status to `reviewed` and write the `Done:` line in the commit message."}
EOF
    exit 0
fi

exit 0
