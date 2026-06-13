#!/bin/sh
# Stop hook: reprompt the agent to run rdm-review while an item is in `needs-review`.
#
# The `needs-review` status itself is the sentinel — there is no marker file. Once
# rdm-review moves the item to `reviewed` (or any other status), the next stop finds
# nothing pending and is allowed. `stop_hook_active` short-circuits the reprompt loop.
#
# Dependencies: POSIX `sh`, `grep`, and the `rdm` binary on PATH only (no `jq`).
#
# Project resolution follows the standard chain: `RDM_PROJECT` env var, then
# `default_project` in `rdm.toml`. Set one of those so the `rdm search` calls below
# resolve to the right project.
#
# Manual reproduction (from a directory with a configured plan repo):
#   1. Put an item in needs-review, then BLOCK:
#        rdm task update <slug> --status needs-review --no-edit
#        echo '{"stop_hook_active": false}' | CLAUDE_PROJECT_DIR="$PWD" .claude/hooks/rdm-review-on-finalize.sh
#        # expect: {"decision":"block","reason":"..."} on stdout
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

# Query phases and tasks separately. `search --status needs-review` with no `--type`
# resolves the ambiguous status to a *phase* status (phases are tried first), so it never
# matches tasks — we must ask for each kind explicitly to catch any pending item.
# An empty search returns the literal `[]`; a pending item carries an "identifier" field.
phases=$(rdm search "" --status needs-review --type phase --format json 2>/dev/null)
tasks=$(rdm search "" --status needs-review --type task --format json 2>/dev/null)

if printf '%s%s' "$phases" "$tasks" | grep -q '"identifier"'; then
  cat <<'EOF'
{"decision":"block","reason":"There are rdm item(s) in `needs-review`. Before stopping, invoke the rdm-review skill on the needs-review item(s): categorize the findings — fix small issues inline, and file large ones as rdm tasks. If review passes, set the item's status to `reviewed` and write the `Done:` line in the commit message."}
EOF
  exit 0
fi

exit 0
