#!/bin/sh
# Hermetic end-to-end regression for the argv/JSON contract an editor plugin
# integration depends on when it shells out to rdm directly (no MCP layer).
# Matches the scripts/verify-*.sh glob CI already runs, so no separate CI
# wiring is needed once this file exists.
#
#   1. Round-trip: create (body via stdin, --no-edit) -> list shows it ->
#      update --status -> show reflects the new status AND the
#      stdin-supplied body (a free corroboration of the create-from-stdin
#      write path riding on this same flow) -> search finds it by a fuzzy
#      typo of its title -> info --format json resolves the seeded project.
#   2. Body-resolution contract, asserted directly against the same rdm
#      subprocess boundary a plugin calls through: create with both --body
#      and stdin stores the --body value and drops the piped one; update
#      with stdin and no --body (a tags-only update) never reads stdin into
#      the body; update --body "" against a non-empty body is rejected and
#      points at --clear-body, leaving the body unchanged; --clear-body
#      empties a previously non-empty body. The identical behaviors are
#      also asserted deterministically (and more exhaustively) by
#      rdm-cli/tests/cli_task.rs under `cargo nextest run`
#      (body_flag_beats_stdin, task_update_body_flag_beats_stdin,
#      task_update_tags_ignores_stdin, task_update_status_ignores_stdin,
#      task_update_empty_body_refuses_clobber, task_update_clear_body_succeeds)
#      -- see CLAUDE.md's Dogfooding section for the citation.
#   3. stdin-EOF contract: a portable (no GNU `timeout`) FIFO watchdog proves
#      `create` (default, no --body) blocks reading stdin to EOF and
#      unblocks promptly once EOF arrives, and that a caller who closes
#      stdin immediately (</dev/null) gets an immediate return. This is the
#      one behavior the existing Rust suite does not cover --
#      task_create_body_flag_no_hang_with_open_stdin_pipe proves only that
#      --body mode ignores an open stdin pipe, never that the default path
#      genuinely blocks on it.
#   4. Error surfacing: a bad command exits non-zero with an actionable,
#      non-panic stderr message and empty stdout. Separately, a needs-review
#      update run from a branched *code* repo (not the plan repo --
#      needs_review_warning reads std::env::current_dir() at rdm invocation
#      time, an unrelated source-repo CWD) writes its warning only to
#      stderr, never stdout, and a following --format json show still parses
#      as clean, uncontaminated JSON.
#
# Run after touching resolve_body/map_body_clobber or needs_review_warning
# in rdm-cli/src/commands/, or any of the exercised task/search/info command
# surfaces.
#
# Requires: cargo-built rdm at target/debug/rdm (from this repo).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
RDM_BIN="$REPO_ROOT/target/debug/rdm"
export RDM_BIN

if [ ! -x "$RDM_BIN" ]; then
    echo "error: $RDM_BIN not found or not executable — run 'cargo build' first." >&2
    exit 1
fi

say() { printf '\n\033[1;34m==>\033[0m %s\n' "$*"; }
fail() {
    printf '\n\033[1;31m[FAIL]\033[0m %s\n' "$*" >&2
    exit 1
}
ok() { printf '\033[1;32m[ OK ]\033[0m %s\n' "$*"; }

TMP=$(mktemp -d)
PROJECT="plugin-loop"

# shellcheck disable=SC1091
. "$SCRIPT_DIR/lib/rdm-plan-fixture.sh"

# Track background PIDs from the stdin-EOF section so a failed assertion can
# still forcibly reap them instead of leaking an orphaned rdm/watcher process
# on the CI runner.
BLOCK_PID=""
NOBLOCK_PID=""

_reap_background() {
    for pid in "$BLOCK_PID" "$NOBLOCK_PID"; do
        if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
            kill -9 "$pid" 2>/dev/null || true
            wait "$pid" 2>/dev/null || true
        fi
    done
}

trap '_reap_background; rm -rf "$TMP"; fixture_teardown' EXIT INT HUP TERM

fixture_setup "$PROJECT" || fail "fixture_setup failed"
fixture_code_repo || fail "fixture_code_repo failed"

# ---------------------------------------------------------------------------
say "1. Round-trip: create -> list -> update -> show -> search -> info"
# ---------------------------------------------------------------------------

"$RDM_BIN" --root "$FIXTURE_PLAN" task create widget-loop \
    --title "Widget Loop Task" --no-edit --project "$PROJECT" \
    >"$TMP/create.out" <<'EOF' || fail "task create (stdin body) failed"
Body content supplied entirely over stdin for the plugin round-trip test.
EOF
ok "task create (body via stdin, --no-edit) exited 0"

LIST_OUT=$("$RDM_BIN" --root "$FIXTURE_PLAN" task list --project "$PROJECT") ||
    fail "task list failed"
case "$LIST_OUT" in
    *widget-loop*'Widget Loop Task'*) ;;
    *) fail "task list output does not contain the created task's slug and title:
$LIST_OUT" ;;
esac
ok "task list shows the newly created task"

"$RDM_BIN" --root "$FIXTURE_PLAN" task update widget-loop \
    --status in-progress --no-edit --project "$PROJECT" >/dev/null ||
    fail "task update --status failed"
ok "task update --status in-progress exited 0"

SHOW_OUT=$("$RDM_BIN" --root "$FIXTURE_PLAN" task show widget-loop --project "$PROJECT") ||
    fail "task show failed"
case "$SHOW_OUT" in
    *'Status: in-progress'*) ;;
    *) fail "task show does not reflect the updated status:
$SHOW_OUT" ;;
esac
case "$SHOW_OUT" in
    *'Body content supplied entirely over stdin for the plugin round-trip test.'*) ;;
    *) fail "task show does not reflect the stdin-supplied body:
$SHOW_OUT" ;;
esac
ok "task show reflects the updated status and the stdin-supplied body"

SEARCH_OUT=$("$RDM_BIN" --root "$FIXTURE_PLAN" search "Widget Lop Task" --project "$PROJECT") ||
    fail "search failed"
case "$SEARCH_OUT" in
    *widget-loop*) ;;
    *) fail "fuzzy-typo search did not find the task:
$SEARCH_OUT" ;;
esac
ok "search finds the task via a fuzzy typo of its title (typo tolerance proven)"

INFO_OUT=$("$RDM_BIN" --root "$FIXTURE_PLAN" info --format json --project "$PROJECT") ||
    fail "info --format json failed"
case "$INFO_OUT" in
    *'"project": "'"$PROJECT"'"'*) ;;
    *) fail "info --format json project field does not equal '$PROJECT':
$INFO_OUT" ;;
esac
case "$INFO_OUT" in
    '{'*) ;;
    *) fail "info --format json output does not look like JSON:
$INFO_OUT" ;;
esac
ok "info --format json resolves the seeded project"

# ---------------------------------------------------------------------------
say "2. Body-resolution contract: --body-authoritative, update-ignores-stdin, --body \"\" rejection, --clear-body"
# ---------------------------------------------------------------------------

"$RDM_BIN" --root "$FIXTURE_PLAN" task create body-flag-wins \
    --title "Body Flag Wins" --body "Inline body wins." --no-edit \
    --project "$PROJECT" >/dev/null <<'EOF' || fail "task create (--body + stdin) failed"
Piped body loses.
EOF
BODY_FLAG_SHOW=$("$RDM_BIN" --root "$FIXTURE_PLAN" task show body-flag-wins --project "$PROJECT") ||
    fail "task show (body-flag-wins) failed"
case "$BODY_FLAG_SHOW" in
    *'Inline body wins.'*) ;;
    *) fail "task create with both --body and stdin did not store the --body value:
$BODY_FLAG_SHOW" ;;
esac
case "$BODY_FLAG_SHOW" in
    *'Piped body loses.'*) fail "task create with both --body and stdin leaked the piped stdin value:
$BODY_FLAG_SHOW" ;;
    *) ;;
esac
ok "task create: --body is authoritative over a simultaneously piped stdin body"

"$RDM_BIN" --root "$FIXTURE_PLAN" task create update-ignores-stdin \
    --title "Update Ignores Stdin" --body "Original body for stdin-ignore test." \
    --no-edit --project "$PROJECT" >/dev/null ||
    fail "task create (update-ignores-stdin) failed"
"$RDM_BIN" --root "$FIXTURE_PLAN" task update update-ignores-stdin \
    --tags plugin-loop-corroboration --no-edit --project "$PROJECT" \
    >/dev/null <<'EOF' || fail "task update (tags-only, with stdin held) failed"
SNEAKY STDIN BODY
EOF
UPDATE_IGNORES_SHOW=$("$RDM_BIN" --root "$FIXTURE_PLAN" task show update-ignores-stdin --project "$PROJECT") ||
    fail "task show (update-ignores-stdin) failed"
case "$UPDATE_IGNORES_SHOW" in
    *'Original body for stdin-ignore test.'*) ;;
    *) fail "a tags-only update did not preserve the existing body:
$UPDATE_IGNORES_SHOW" ;;
esac
case "$UPDATE_IGNORES_SHOW" in
    *'SNEAKY STDIN BODY'*) fail "a tags-only update read stdin into the body:
$UPDATE_IGNORES_SHOW" ;;
    *) ;;
esac
ok "task update: a tags-only update never reads stdin into the body"

"$RDM_BIN" --root "$FIXTURE_PLAN" task create empty-body-rejected \
    --title "Empty Body Rejected" --body "Existing content for empty-body test." \
    --no-edit --project "$PROJECT" >/dev/null ||
    fail "task create (empty-body-rejected) failed"
set +e
"$RDM_BIN" --root "$FIXTURE_PLAN" task update empty-body-rejected \
    --body "" --no-edit --project "$PROJECT" \
    >"$TMP/empty-body.out" 2>"$TMP/empty-body.err"
empty_body_rc=$?
set -e
[ "$empty_body_rc" -ne 0 ] ||
    fail "task update --body \"\" against a non-empty body exited 0 — expected a refusal"
grep -q -- '--clear-body' "$TMP/empty-body.err" ||
    fail "task update --body \"\" rejection did not point at --clear-body:
$(cat "$TMP/empty-body.err")"
EMPTY_BODY_SHOW=$("$RDM_BIN" --root "$FIXTURE_PLAN" task show empty-body-rejected --project "$PROJECT") ||
    fail "task show (empty-body-rejected) failed"
case "$EMPTY_BODY_SHOW" in
    *'Existing content for empty-body test.'*) ;;
    *) fail "task update --body \"\" clobbered the body despite being rejected:
$EMPTY_BODY_SHOW" ;;
esac
ok "task update --body \"\" against a non-empty body is rejected, points at --clear-body, and leaves the body unchanged"

"$RDM_BIN" --root "$FIXTURE_PLAN" task create clear-body-empties \
    --title "Clear Body Empties" --body "Content that will be cleared." \
    --no-edit --project "$PROJECT" >/dev/null ||
    fail "task create (clear-body-empties) failed"
"$RDM_BIN" --root "$FIXTURE_PLAN" task update clear-body-empties \
    --clear-body --no-edit --project "$PROJECT" >/dev/null ||
    fail "task update --clear-body failed"
CLEAR_BODY_SHOW=$("$RDM_BIN" --root "$FIXTURE_PLAN" task show clear-body-empties --project "$PROJECT") ||
    fail "task show (clear-body-empties) failed"
case "$CLEAR_BODY_SHOW" in
    *'Content that will be cleared.'*) fail "--clear-body did not empty the body:
$CLEAR_BODY_SHOW" ;;
    *) ;;
esac
ok "task update --clear-body empties a previously non-empty body"

ok "the identical behaviors are also asserted deterministically by cargo nextest run (body_flag_beats_stdin, task_update_body_flag_beats_stdin, task_update_tags_ignores_stdin, task_update_status_ignores_stdin, task_update_empty_body_refuses_clobber, task_update_clear_body_succeeds)"

# ---------------------------------------------------------------------------
say "3. stdin-EOF contract: a portable FIFO watchdog, no GNU timeout"
# ---------------------------------------------------------------------------

FIFO="$TMP/stdin.fifo"
mkfifo "$FIFO"

"$RDM_BIN" --root "$FIXTURE_PLAN" task create stdin-blocks \
    --title "Stdin Blocks" --no-edit --project "$PROJECT" <"$FIFO" &
BLOCK_PID=$!

# Open the fifo for writing without writing anything, so the reader (rdm's
# stdin) stays open past its own open() call but delivers no bytes yet.
exec 8>"$FIFO"
sleep 1
if kill -0 "$BLOCK_PID" 2>/dev/null; then
    ok "create with no --body blocks on an open, empty stdin pipe (has not returned yet)"
else
    exec 8>&-
    fail "create with no --body returned before stdin reached EOF — expected it to block"
fi
exec 8>&- # close the write end: delivers EOF to the reader
if wait "$BLOCK_PID"; then
    ok "create unblocks and exits 0 promptly once stdin reaches EOF"
else
    fail "create did not exit 0 after stdin reached EOF"
fi
BLOCK_PID=""

"$RDM_BIN" --root "$FIXTURE_PLAN" task create stdin-eof-immediately \
    --title "Stdin EOF Immediately" --no-edit --project "$PROJECT" </dev/null &
NOBLOCK_PID=$!
sleep 1
if kill -0 "$NOBLOCK_PID" 2>/dev/null; then
    kill -9 "$NOBLOCK_PID" 2>/dev/null || true
    wait "$NOBLOCK_PID" 2>/dev/null || true
    NOBLOCK_PID=""
    fail "create with stdin already at EOF (</dev/null) was still running after 1s — expected an immediate return"
fi
if wait "$NOBLOCK_PID"; then
    ok "create with stdin already at EOF (</dev/null) returns immediately and exits 0"
else
    fail "create with stdin already at EOF exited non-zero"
fi
NOBLOCK_PID=""

# ---------------------------------------------------------------------------
say "4. Error surfacing and stderr/stdout separability"
# ---------------------------------------------------------------------------

set +e
"$RDM_BIN" --root "$FIXTURE_PLAN" task show definitely-not-a-real-slug \
    --project "$PROJECT" >"$TMP/bad.out" 2>"$TMP/bad.err"
bad_rc=$?
set -e
[ "$bad_rc" -ne 0 ] || fail "task show on a nonexistent slug exited 0 — expected non-zero"
[ -s "$TMP/bad.err" ] || fail "task show on a nonexistent slug wrote nothing to stderr"
if grep -qi 'panicked' "$TMP/bad.err"; then
    fail "task show on a nonexistent slug leaked a Rust panic instead of an actionable error:
$(cat "$TMP/bad.err")"
fi
[ ! -s "$TMP/bad.out" ] || fail "task show on a nonexistent slug wrote to stdout:
$(cat "$TMP/bad.out")"
ok "a bad command exits non-zero with an actionable, non-panic stderr message and empty stdout"

# needs_review_warning is computed from std::env::current_dir() at rdm
# invocation time (rdm-cli/src/commands/task.rs) -- an unrelated SOURCE repo
# CWD, not the --root plan path. Branch the fixture's CODE repo (not the
# plan repo) with no new commit and invoke rdm from a subshell cd'd into it,
# matching scripts/verify-worktree-review-loop.sh's own use of this pattern.
git -C "$FIXTURE_CODE_REPO" checkout -b other-branch --quiet ||
    fail "failed to branch the fixture code repo"

"$RDM_BIN" --root "$FIXTURE_PLAN" task create warning-target \
    --title "Warning Target" --no-edit --project "$PROJECT" >/dev/null ||
    fail "task create (warning-target) failed"

(cd "$FIXTURE_CODE_REPO" && "$RDM_BIN" --root "$FIXTURE_PLAN" task update warning-target \
    --status needs-review --no-edit --project "$PROJECT" \
    >"$TMP/warn.out" 2>"$TMP/warn.err") ||
    fail "task update --status needs-review (from the branched code repo) failed"

if ! grep -q 'warning:' "$TMP/warn.err"; then
    fail "needs-review update from a branched code repo with no new commits did not print a 'warning:' line to stderr:
$(cat "$TMP/warn.err")"
fi
if grep -q 'warning:' "$TMP/warn.out"; then
    fail "the needs-review warning leaked onto stdout:
$(cat "$TMP/warn.out")"
fi
ok "needs-review warning is written to stderr only, never stdout"

JSON_OUT=$("$RDM_BIN" --root "$FIXTURE_PLAN" task show warning-target \
    --format json --project "$PROJECT" 2>"$TMP/json.err") ||
    fail "task show --format json (warning-target) failed"
case "$JSON_OUT" in
    '{'*'}') ;;
    *) fail "task show --format json output is not well-formed JSON:
$JSON_OUT" ;;
esac
case "$JSON_OUT" in
    *warning-target*) ;;
    *) fail "task show --format json output does not contain the expected slug:
$JSON_OUT" ;;
esac
case "$JSON_OUT" in
    *'warning:'*) fail "JSON stdout was contaminated by the stderr warning text" ;;
    *) ;;
esac
ok "--format json stdout stays clean even for an item that just triggered a stderr warning"

say "All plugin-loop contract checks passed."
