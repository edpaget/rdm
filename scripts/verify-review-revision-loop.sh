#!/bin/sh
# End-to-end regression for the LLM revision workflow (rdm-revise loop).
#
# Drives the full agent loop against a temp git-backed plan repo with the
# real rdm binary: a submitted request-changes review is worked comment by
# comment — edit through `rdm ... update`, capture the mutation commit,
# record it as applied_commit — and driven to `addressed`. Everything runs
# in temp dirs against target/debug/rdm; no network, hermetic.
#
# Five regression cases, each its own OK/FAIL section:
#   A. Resolved anchor        — a text-quote comment resolves against the
#      review's created_commit body; the edit + explicit --applied-commit
#      round-trips into the comment's resolution record.
#   B. Whole-document comment — a comment with no anchor reports
#      `unresolved` and is worked wholistically.
#   C. Drifted anchor         — the quoted span is reworded *after* submit
#      (context preserved), so the anchor reports `drifted`; the agent
#      leaves a clarification reply with NO status, and closing the review
#      is refused while the comment stays open.
#   D. Wont-fix escape hatch  — a comment is closed with reasoning and no
#      applied commit.
#   E. Completion             — once the clarified comment is resolved, the
#      review closes as `addressed` and drops out of `review requests`.
#
# Run after touching the review resolution model (ops/reviews.rs update
# paths, anchor drift resolution, `rdm review requests/update`, the MCP
# review tools, or the rdm-revise skill templates).
#
# Requires: cargo-built rdm at target/debug/rdm (from this repo).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
RDM_BIN="$REPO_ROOT/target/debug/rdm"

if [ ! -x "$RDM_BIN" ]; then
    echo "error: $RDM_BIN not found or not executable — run 'cargo build' first." >&2
    exit 1
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

# Clear rdm-related env vars inherited from the caller's shell so the run
# never touches the developer's real plan repo.
unset RDM_ROOT RDM_PROJECT RDM_STAGE RDM_FORMAT RDM_PLAN_REPO RDM_PLAN_REPO_TOKEN RDM_PLAN_REPO_PATH

# Git identity so `git commit` never falls back to a missing global config.
export GIT_AUTHOR_NAME="verify-bot"
export GIT_AUTHOR_EMAIL="verify@example.invalid"
export GIT_COMMITTER_NAME="verify-bot"
export GIT_COMMITTER_EMAIL="verify@example.invalid"

say() { printf '\n\033[1;34m==>\033[0m %s\n' "$*"; }
fail() {
    printf '\n\033[1;31m[FAIL]\033[0m %s\n' "$*" >&2
    exit 1
}
ok() { printf '\033[1;32m[ OK ]\033[0m %s\n' "$*"; }

PLAN="$TMP/plan"
PROJ="revise-proj"
rdm() { "$RDM_BIN" --root "$PLAN" "$@"; }
plan_head() { git -C "$PLAN" rev-parse HEAD; }

# ---------------------------------------------------------------------------
say "Setup: plan repo, project, and a task with three quotable spans"
# ---------------------------------------------------------------------------

mkdir -p "$PLAN"
rdm init >/dev/null
rdm project create "$PROJ" --title "Revise Project" >/dev/null

rdm task create fix-login --title "Fix login" --no-edit --project "$PROJ" >/dev/null <<'EOF'
The login flow rejects valid tokens when the clock skews.

Repro: post a token minted five seconds in the future.

Cleanup: the retry helper duplicates backoff logic.
EOF

# ---------------------------------------------------------------------------
say "Setup: submitted request-changes review with four comments"
# ---------------------------------------------------------------------------

REVIEW_ID=$(rdm review start --on task/fix-login --author reviewer --no-edit \
    --project "$PROJ" --format json | sed -n 's/.*"id": "\([^"]*\)".*/\1/p' | head -1)
[ -n "$REVIEW_ID" ] || fail "could not extract the review id from review start"

# Comment 1: text-quote anchor that will resolve cleanly (scenario A).
rdm review comment "$REVIEW_ID" --quote "rejects valid tokens" \
    --body "Name the error code the client sees." --no-edit --project "$PROJ" >/dev/null
# Comment 2: whole-document (scenario B).
rdm review comment "$REVIEW_ID" \
    --body "Overall: the repro section needs the exact curl invocation." \
    --no-edit --project "$PROJ" >/dev/null
# Comment 3: text-quote anchor that will DRIFT after submit (scenario C).
rdm review comment "$REVIEW_ID" --quote "minted five seconds in the future" \
    --body "Five seconds is too tight — justify the window." --no-edit --project "$PROJ" >/dev/null
# Comment 4: wont-fix target (scenario D).
rdm review comment "$REVIEW_ID" \
    --body "Also fold the retry-helper cleanup into this task." \
    --no-edit --project "$PROJ" >/dev/null

rdm review submit "$REVIEW_ID" --verdict request-changes \
    --body "Tighten the task description before implementation." \
    --no-edit --project "$PROJ" >/dev/null

rdm review requests --format json --project "$PROJ" >"$TMP/requests.json"
grep -q "\"id\": \"$REVIEW_ID\"" "$TMP/requests.json" ||
    fail "review requests must list the submitted request-changes review"
ok "review '$REVIEW_ID' is in the change-request queue"

# ---------------------------------------------------------------------------
say "Drift construction: reword comment 3's span after submit (context kept)"
# ---------------------------------------------------------------------------

# The review's created_commit points at the pre-edit body; rewording only
# the quoted span (its surrounding context survives) makes the anchor
# resolve as drifted rather than unresolved.
rdm task update fix-login --no-edit --project "$PROJ" >/dev/null <<'EOF'
The login flow rejects valid tokens when the clock skews.

Repro: post a token minted with generous clock skew.

Cleanup: the retry helper duplicates backoff logic.
EOF

rdm review show "$REVIEW_ID" --format json --project "$PROJ" >"$TMP/show0.json"
[ "$(grep -c '"state": "resolved"' "$TMP/show0.json")" = "1" ] ||
    fail "expected exactly one resolved anchor (comment 1)"
[ "$(grep -c '"state": "drifted"' "$TMP/show0.json")" = "1" ] ||
    fail "expected exactly one drifted anchor (comment 3)"
[ "$(grep -c '"state": "unresolved"' "$TMP/show0.json")" = "2" ] ||
    fail "expected two unresolved comments (whole-document 2 and 4)"
grep -q '"quote": "minted five seconds in the future"' "$TMP/show0.json" ||
    fail "drifted resolution must carry the quote the reviewer saw"
ok "resolutions: 1 resolved, 1 drifted, 2 whole-document"

# ---------------------------------------------------------------------------
say "A. Resolved anchor: edit, capture the commit, mark addressed"
# ---------------------------------------------------------------------------

rdm task update fix-login --no-edit --project "$PROJ" >/dev/null <<'EOF'
The login flow rejects valid tokens with error AUTH-401 when the clock skews.

Repro: post a token minted with generous clock skew.

Cleanup: the retry helper duplicates backoff logic.
EOF
SHA_A=$(plan_head)

rdm review update "$REVIEW_ID" --comment 1 --status addressed \
    --applied-commit "$SHA_A" \
    --reply "Named the AUTH-401 error code; anchor resolved cleanly." \
    --project "$PROJ" >/dev/null

rdm review show "$REVIEW_ID" --format json --project "$PROJ" >"$TMP/show-a.json"
grep -q "\"applied_commit\": \"$SHA_A\"" "$TMP/show-a.json" ||
    fail "comment 1 must record the applied edit commit $SHA_A"
[ "$(grep -c '"status": "addressed"' "$TMP/show-a.json")" = "1" ] ||
    fail "exactly one comment should be addressed after scenario A"
ok "comment 1 addressed with applied_commit $SHA_A"

# ---------------------------------------------------------------------------
say "B. Whole-document comment: work it wholistically, note the missing anchor"
# ---------------------------------------------------------------------------

rdm task update fix-login --no-edit --project "$PROJ" >/dev/null <<'EOF'
The login flow rejects valid tokens with error AUTH-401 when the clock skews.

Repro: post a token minted with generous clock skew.

    curl -X POST /login -H "Authorization: Bearer $SKEWED_TOKEN"

Cleanup: the retry helper duplicates backoff logic.
EOF
SHA_B=$(plan_head)

rdm review update "$REVIEW_ID" --comment 2 --status addressed \
    --applied-commit "$SHA_B" \
    --reply "Added the exact curl invocation. No anchor was resolved (whole-document comment); applied against the current body." \
    --project "$PROJ" >/dev/null

rdm review show "$REVIEW_ID" --format json --project "$PROJ" >"$TMP/show-b.json"
grep -q "\"applied_commit\": \"$SHA_B\"" "$TMP/show-b.json" ||
    fail "comment 2 must record the applied edit commit $SHA_B"
[ "$(grep -c '"status": "addressed"' "$TMP/show-b.json")" = "2" ] ||
    fail "two comments should be addressed after scenario B"
ok "whole-document comment 2 addressed with applied_commit $SHA_B"

# ---------------------------------------------------------------------------
say "C. Drifted anchor: clarification reply, comment stays open, close refused"
# ---------------------------------------------------------------------------

rdm review update "$REVIEW_ID" --comment 3 \
    --reply "The quoted span changed since the review — should the window be justified in the repro, or relaxed in the fix itself?" \
    --project "$PROJ" >/dev/null

rdm review show "$REVIEW_ID" --format json --project "$PROJ" >"$TMP/show-c.json"
[ "$(grep -c '"status": "open"' "$TMP/show-c.json")" = "2" ] ||
    fail "comments 3 and 4 must still be open after a reply-only update"
grep -q '"reply": "The quoted span changed' "$TMP/show-c.json" ||
    fail "the clarification reply must be recorded on comment 3"

HEAD_BEFORE_REFUSAL=$(plan_head)
if rdm review update "$REVIEW_ID" --state addressed --project "$PROJ" \
    >"$TMP/complete-refused.txt" 2>&1; then
    fail "closing the review must be refused while comments are open"
fi
grep -q "open comment" "$TMP/complete-refused.txt" ||
    fail "the refusal must explain that comments are still open: $(cat "$TMP/complete-refused.txt")"
# Re-fetch AFTER the refused attempt: the refusal must have changed nothing —
# the review is still submitted, the open comments are intact, and no
# plan-repo commit was produced.
rdm review show "$REVIEW_ID" --format json --project "$PROJ" >"$TMP/show-c-after.json"
grep -q '"state": "submitted"' "$TMP/show-c-after.json" ||
    fail "the review must remain submitted after the refused close"
[ "$(grep -c '"status": "open"' "$TMP/show-c-after.json")" = "2" ] ||
    fail "the refused close must leave the open comments untouched"
[ "$(plan_head)" = "$HEAD_BEFORE_REFUSAL" ] ||
    fail "the refused close must not create a plan-repo commit"
ok "clarification left comment 3 open and blocked --state addressed"

# ---------------------------------------------------------------------------
say "D. Wont-fix escape hatch: close comment 4 with reasoning, no commit"
# ---------------------------------------------------------------------------

rdm review update "$REVIEW_ID" --comment 4 --status wont-fix \
    --reply "The retry-helper cleanup is unrelated; tracked as its own task." \
    --project "$PROJ" >/dev/null

rdm review show "$REVIEW_ID" --format json --project "$PROJ" >"$TMP/show-d.json"
[ "$(grep -c '"status": "wont-fix"' "$TMP/show-d.json")" = "1" ] ||
    fail "comment 4 must be wont-fix"
# The wont-fix comment carries reasoning but no applied commit: only the two
# addressed comments record one.
[ "$(grep -c '"applied_commit"' "$TMP/show-d.json")" = "2" ] ||
    fail "wont-fix must not record an applied_commit"
ok "comment 4 wont-fix with reasoning and no applied commit"

# ---------------------------------------------------------------------------
say "E. Completion: resolve the clarified comment, close, queue drains"
# ---------------------------------------------------------------------------

rdm task update fix-login --no-edit --project "$PROJ" >/dev/null <<'EOF'
The login flow rejects valid tokens with error AUTH-401 when the clock skews.

Repro: post a token minted with generous clock skew (the five-second window
in the original report was an observation, not a requirement).

    curl -X POST /login -H "Authorization: Bearer $SKEWED_TOKEN"

Cleanup: the retry helper duplicates backoff logic.
EOF
SHA_E=$(plan_head)

rdm review update "$REVIEW_ID" --comment 3 --status addressed \
    --applied-commit "$SHA_E" \
    --reply "Reviewer confirmed: justified the window in the repro. Anchor had drifted; edit applied against the current body." \
    --project "$PROJ" >/dev/null

rdm review update "$REVIEW_ID" --state addressed --project "$PROJ" >/dev/null

rdm review show "$REVIEW_ID" --format json --project "$PROJ" >"$TMP/show-e.json"
grep -q '"state": "addressed"' "$TMP/show-e.json" ||
    fail "the review must be addressed once every comment is terminal"
if grep -q '"status": "open"' "$TMP/show-e.json"; then
    fail "no comment may remain open after completion"
fi

rdm review requests --format json --project "$PROJ" >"$TMP/requests-after.json"
if grep -q "\"id\": \"$REVIEW_ID\"" "$TMP/requests-after.json"; then
    fail "an addressed review must drop out of the change-request queue"
fi
ok "review closed as addressed and left the change-request queue"

printf '\n\033[1;32mAll review-revision-loop checks passed.\033[0m\n'
