#!/bin/sh
# Stop hook: reprompt the agent to run rdm-plan-review while an item carries the
# `needs-plan-review` sentinel tag.
#
# The `needs-plan-review` tag itself is the sentinel — there is no marker file and no
# status transition involved (the now-retired needs-review Stop hook watched a status
# value instead). `rdm roadmap create` / `phase create` / `task create` stamp the tag
# onto new items when the `plan_review` config flag is enabled (see `rdm-core::tags`).
# Once the rdm-plan-review skill reviews the item and clears the tag (on PASS / PASS
# WITH CONCERNS), the next stop finds nothing pending and is allowed.
# `stop_hook_active` short-circuits the reprompt loop the same way that retired hook did.
#
# This hook does NOT call `rdm review restamp` (the needs-review lane's mechanism,
# before it was retired in favor of active review on every finalize). Restamping exists
# to keep a branch/commit-scoped stamp (review_sha/review_branch) from going stale
# across an amend or rebase. The `needs-plan-review` tag carries no branch/commit scope
# at all — it is a plain tag on the item — so there is nothing to go stale and nothing
# to restamp.
#
# Dependencies: POSIX `sh`, `grep`, and the `rdm` binary on PATH only (no `jq`).
#
# Project resolution follows the standard chain: `RDM_PROJECT` env var, then
# `default_project` in `rdm.toml`. Set one of those so the `rdm search` call below
# resolves to the right project.
#
# Manual reproduction (from a directory with a configured plan repo that has
# `plan_review = true`):
#   1. Create an item (stamps the tag), then BLOCK:
#        rdm task create demo-item --title "Demo" --no-edit
#        echo '{"stop_hook_active": false}' | CLAUDE_PROJECT_DIR="$PWD" .claude/hooks/rdm-plan-review-on-create.sh
#        # expect: {"decision":"block","reason":"..."} on stdout
#   2. ALLOW on stop_hook_active (loop prevention):
#        echo '{"stop_hook_active": true}' | CLAUDE_PROJECT_DIR="$PWD" .claude/hooks/rdm-plan-review-on-create.sh
#        # expect: exit 0, no output
#   3. Clear the tag, then ALLOW (nothing pending):
#        rdm task update demo-item --tags "" --no-edit
#        echo '{"stop_hook_active": false}' | CLAUDE_PROJECT_DIR="$PWD" .claude/hooks/rdm-plan-review-on-create.sh
#        # expect: exit 0, no output

input=$(cat)

# Loop prevention: if we're already inside a stop-hook-triggered continuation, allow.
# This is a deliberate grep heuristic (no jq by design); the Stop payload is emitted by
# Claude Code, so a substring match on the field is sufficient and safe here.
if printf '%s' "$input" | grep -Eq '"stop_hook_active"[[:space:]]*:[[:space:]]*true'; then
    exit 0
fi

# `rdm search` with no --type spans roadmaps, phases, and tasks in one call. An empty
# result is the literal `[]`; a matching item carries an "identifier" field. Any failure
# here (missing binary, unset project, plan-repo error) falls through to "nothing
# pending" via the empty command substitution and grep's no-match exit — fail open.
pending=$(rdm search "" --tag needs-plan-review --format json 2>/dev/null)

if printf '%s' "$pending" | grep -q '"identifier"'; then
    cat <<'EOF'
{"decision":"block","reason":"There are rdm item(s) tagged `needs-plan-review`. Before stopping, invoke the rdm-plan-review skill on the pending item(s) to review the plan before implementation begins. On PASS or PASS WITH CONCERNS it clears the tag; on REWORK it reports what must change."}
EOF
    exit 0
fi

exit 0
