#!/bin/sh
# Stop hook: reprompt the agent to run rdm-review while an item is in `needs-review`.
#
# The `needs-review` status itself is the sentinel — there is no marker file. Once
# rdm-review moves the item to `reviewed` (or any other status), the next stop finds
# nothing pending and is allowed. `stop_hook_active` short-circuits the reprompt loop.
#
# Scope: `rdm review pending` only reports items whose stamped source-repo SHA is
# reachable from the current HEAD (or that are unstamped — those fail open). This
# keeps a session finishing branch A from being reprompted to review an item that
# was finalized on branch B, whose diff isn't even checked out here. It is the
# single shared source of truth for "what is in scope to review" — the rdm-review
# skill consults the same command.
#
# One-worktree-per-roadmap model: this hook fires from the roadmap worktree's cwd
# while it sits on the `roadmap/<slug>` branch. Every phase of the roadmap is
# finalized in place on that same branch, so the branch-scoped filter resolves
# exactly the roadmap's stamped, in-scope items — no per-phase worktree and no
# nested-move assumption.
#
# Dependencies: POSIX `sh`, `grep`, and the `rdm` binary on PATH only (no `jq`).
#
# Project resolution follows the standard chain: `RDM_PROJECT` env var, then
# `default_project` in `rdm.toml`. Set one of those so the `rdm review pending` call
# below resolves to the right project.
#
# Manual reproduction (from a directory with a configured plan repo):
#   1. Put an item in needs-review on THIS branch, then BLOCK:
#        rdm task update <slug> --status needs-review --no-edit
#        echo '{"stop_hook_active": false}' | CLAUDE_PROJECT_DIR="$PWD" .claude/hooks/rdm-review-on-finalize.sh
#        # expect: {"decision":"block","reason":"..."} on stdout
#      Cross-branch check: finalize an item on another branch, switch back here, then
#      run the same BLOCK invocation — expect exit 0, no output (out of scope).
#   2. ALLOW on stop_hook_active (loop prevention):
#        echo '{"stop_hook_active": true}' | CLAUDE_PROJECT_DIR="$PWD" .claude/hooks/rdm-review-on-finalize.sh
#        # expect: exit 0, no output
#   3. Move out of needs-review, then ALLOW (nothing pending):
#        rdm task update <slug> --status reviewed --no-edit
#        echo '{"stop_hook_active": false}' | CLAUDE_PROJECT_DIR="$PWD" .claude/hooks/rdm-review-on-finalize.sh
#        # expect: exit 0, no output

input=$(cat)

# Loop prevention: if we're already inside a stop-hook-triggered continuation, allow.
# This is a deliberate grep heuristic (no jq by design); the Stop payload is emitted by
# Claude Code, so a substring match on the field is sufficient and safe here.
if printf '%s' "$input" | grep -Eq '"stop_hook_active"[[:space:]]*:[[:space:]]*true'; then
    exit 0
fi

# `rdm review pending` returns the needs-review phases AND tasks that are in scope for
# the current source-repo branch (stamped-and-reachable, or unstamped/fail-open). It is
# the single shared source of truth for the hook and the rdm-review skill.
# An empty result is the literal `[]`; a pending item carries an "identifier" field.
pending=$(rdm review pending --format json 2>/dev/null)

if printf '%s' "$pending" | grep -q '"identifier"'; then
    cat <<'EOF'
{"decision":"block","reason":"There are rdm item(s) in `needs-review`. Before stopping, invoke the rdm-review skill on the needs-review item(s): categorize the findings — fix small issues inline, and file large ones as rdm tasks. If review passes, set the item's status to `reviewed` and write the `Done:` line in the commit message."}
EOF
    exit 0
fi

exit 0
