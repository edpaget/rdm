#!/bin/sh
# Sourced helper library shared by scripts/capture-golden.sh (the re-bless
# entry point) and scripts/verify-golden-json.sh (the CI-gated drift check),
# so the two scripts can never drift on which commands are captured, how
# they are seeded, or how their output is redacted.
#
# This file is SOURCED, never executed directly:
#
#   RDM_BIN="$REPO_ROOT/target/debug/rdm"
#   . "$REPO_ROOT/scripts/lib/rdm-plan-fixture.sh"
#   . "$REPO_ROOT/scripts/lib/golden-capture.sh"
#   golden_capture_all "$out_dir"      # leaves FIXTURE_* set on success
#   golden_redact "$out_dir"           # uses the still-live FIXTURE_ROOT
#   fixture_teardown                   # caller's job, same as the fixture
#                                       # library's own setup/teardown pairing
#
# Public functions:
#   golden_capture_all <out_dir>
#       Calls fixture_setup/fixture_code_repo (from rdm-plan-fixture.sh,
#       which MUST already be sourced), seeds one submitted request-changes
#       review on task/fixture-task-open and one worktree on the same task,
#       then runs the 20-command JSON-contract inventory (see GOLDEN_NAMES
#       below) and writes each command's raw stdout verbatim to
#       <out_dir>/<name>.json. On success, FIXTURE_ROOT/FIXTURE_PLAN/
#       FIXTURE_PROJECT are left set (exactly like fixture_setup leaves
#       them) so a following golden_redact call can still resolve the
#       temp-path strings to redact; the caller is responsible for calling
#       fixture_teardown afterward. On failure, fixture_teardown is called
#       internally before returning non-zero, since a mid-seed failure
#       leaves nothing worth redacting.
#
#   golden_redact <dir>
#       Applies the five-rule redaction below to every *.json file directly
#       under <dir>, in place. Requires FIXTURE_ROOT to still be set (i.e.
#       called after golden_capture_all and before fixture_teardown) so it
#       can compute the raw and OS-canonicalized forms of the fixture's temp
#       directory.
#
# The 20-command inventory (GOLDEN_NAMES, in capture order) and the reason
# each of these three commands is dropped instead of captured, per the
# phase's own step-1 escape hatch:
#   - `status`         (rdm-cli/src/commands/status.rs `run(root, fetch)`
#                        takes no `format` parameter at all)
#   - `hook done-line`  (rdm-cli/src/commands/hook.rs's DoneLine arm always
#                        `println!("{line}")`, ignoring `--format`)
#   - `model resolve`   (rdm-cli/src/commands/model.rs `run_resolve` always
#                        `println!("{}", policy.resolve(...))`, ignoring
#                        `--format`)
# All three accept `--format json` without a parse error but silently emit
# plain human text regardless. Each has a filed rdm follow-up task instead
# of a golden file (see tests/golden/README.md for the slugs).
#
# Redaction rules (also documented in tests/golden/README.md — keep both in
# sync; this header is authoritative if the two ever disagree):
#   (a) Absolute temp paths: the fixture's FIXTURE_ROOT, in BOTH its raw
#       mktemp form and its OS-canonicalized form (macOS resolves
#       /var/folders/... to /private/var/folders/... in rdm's own printed
#       output — `rdm info`'s `root` field prints the raw form while
#       `rdm worktree add/list/current` print the canonicalized form of the
#       SAME directory), literal-substring-replaced with `<TMPDIR>`,
#       longest-first so one never partially shadows the other.
#   (b) `created`/`completed` NaiveDate fields (roadmap/phase/task
#       frontmatter, `YYYY-MM-DD`) -> `<DATE>`.
#   (c) Commit-SHA-shaped fields (`commit`, `applied_commit`,
#       `created_commit`, `review_sha`) -> `<SHA>`.
#   (d) Review `created`/`submitted` RFC3339 datetimes (distinct from (b)'s
#       NaiveDate fields — these carry a full timestamp, stamped via
#       `Utc::now()`) -> `<DATETIME>`.
#   (e) The review `id` field itself (`YYYY-MM-DD-HHMM-hex`, volatile by day
#       AND by run) -> `<REVIEW-ID>`. Scoped to the review-id shape so the
#       small stable per-comment integer `id` field is never touched.

# GOLDEN_NAMES — the 20 captured golden filenames (without the .json
# extension), in capture order. Single source of truth for both
# capture-golden.sh and verify-golden-json.sh; only verify-golden-json.sh
# reads it (golden_capture_all's own filenames above are literal), a
# usage shellcheck cannot see across a source boundary.
# shellcheck disable=SC2034
GOLDEN_NAMES="info roadmap-list roadmap-show phase-list phase-show task-list task-show list search next tree describe tag-list backlog-report model-show review-list review-show review-requests worktree-list worktree-current"

# _golden_rdm <args...> — invoke the fixture-rooted rdm binary. Internal.
_golden_rdm() {
    "$RDM_BIN" --root "$FIXTURE_PLAN" "$@"
}

# _golden_capture_one <name> <args...> — run `rdm <args...>` and write its
# raw stdout to <out_dir>/<name>.json, failing loud (naming the command) on
# a non-zero exit instead of leaving a truncated or missing golden file.
# Internal; expects $_golden_out_dir to be set by the caller.
_golden_capture_one() {
    name="$1"
    shift
    if ! _golden_rdm "$@" >"$_golden_out_dir/$name.json" 2>/dev/null; then
        echo "golden_capture_all: 'rdm $*' failed" >&2
        return 1
    fi
}

# golden_capture_all <out_dir>
golden_capture_all() {
    _golden_out_dir="$1"
    if [ -z "$_golden_out_dir" ]; then
        echo "golden_capture_all: out_dir argument is required" >&2
        return 1
    fi
    if [ -z "${RDM_BIN:-}" ] || [ ! -x "$RDM_BIN" ]; then
        echo "golden_capture_all: RDM_BIN is not set to an executable rdm binary — run 'cargo build' first and set RDM_BIN=<repo>/target/debug/rdm" >&2
        return 1
    fi
    mkdir -p "$_golden_out_dir" || {
        echo "golden_capture_all: could not create out_dir '$_golden_out_dir'" >&2
        return 1
    }

    fixture_setup || return 1
    fixture_code_repo || {
        fixture_teardown
        return 1
    }

    proj="$FIXTURE_PROJECT"

    # --- Seed one submitted request-changes review on task/fixture-task-open.
    review_json=$(_golden_rdm review start --on task/fixture-task-open --no-edit --project "$proj" --format json 2>/dev/null) || {
        echo "golden_capture_all: 'rdm review start' failed" >&2
        fixture_teardown
        return 1
    }
    _golden_review_id=$(printf '%s\n' "$review_json" | sed -n 's/^[[:space:]]*"id": "\([^"]*\)".*/\1/p' | head -n 1)
    if [ -z "$_golden_review_id" ]; then
        echo "golden_capture_all: could not parse a review id out of 'rdm review start' output" >&2
        fixture_teardown
        return 1
    fi
    if ! _golden_rdm review comment "$_golden_review_id" --body "General feedback." --no-edit --project "$proj" >/dev/null 2>&1; then
        echo "golden_capture_all: 'rdm review comment' failed" >&2
        fixture_teardown
        return 1
    fi
    if ! _golden_rdm review submit "$_golden_review_id" --verdict request-changes --no-edit --project "$proj" >/dev/null 2>&1; then
        echo "golden_capture_all: 'rdm review submit' failed" >&2
        fixture_teardown
        return 1
    fi

    # --- Seed one worktree on the same task, run from inside the fixture
    # code repo (worktree add/list/current take no --project — they resolve
    # purely off cwd + --root).
    worktree_json=$(cd "$FIXTURE_CODE_REPO" && _golden_rdm worktree add task/fixture-task-open --project "$proj" --format json 2>/dev/null) || {
        echo "golden_capture_all: 'rdm worktree add' failed" >&2
        fixture_teardown
        return 1
    }
    _golden_worktree_path=$(printf '%s\n' "$worktree_json" | sed -n 's/^[[:space:]]*"path": "\([^"]*\)".*/\1/p' | head -n 1)
    if [ -z "$_golden_worktree_path" ] || [ ! -d "$_golden_worktree_path" ]; then
        echo "golden_capture_all: could not resolve a worktree path out of 'rdm worktree add' output" >&2
        fixture_teardown
        return 1
    fi

    # --- Capture the 18 project-scoped / project-independent commands.
    _golden_capture_one info info --format json --project "$proj" || {
        fixture_teardown
        return 1
    }
    _golden_capture_one roadmap-list roadmap list --format json --project "$proj" || {
        fixture_teardown
        return 1
    }
    _golden_capture_one roadmap-show roadmap show sample-roadmap --format json --project "$proj" || {
        fixture_teardown
        return 1
    }
    _golden_capture_one phase-list phase list --roadmap sample-roadmap --format json --project "$proj" || {
        fixture_teardown
        return 1
    }
    _golden_capture_one phase-show phase show 1 --roadmap sample-roadmap --format json --project "$proj" || {
        fixture_teardown
        return 1
    }
    _golden_capture_one task-list task list --format json --project "$proj" || {
        fixture_teardown
        return 1
    }
    _golden_capture_one task-show task show fixture-task-open --format json --project "$proj" || {
        fixture_teardown
        return 1
    }
    _golden_capture_one list list --format json --project "$proj" || {
        fixture_teardown
        return 1
    }
    _golden_capture_one search search seed --format json --project "$proj" || {
        fixture_teardown
        return 1
    }
    _golden_capture_one next next --roadmap sample-roadmap --format json --project "$proj" || {
        fixture_teardown
        return 1
    }
    _golden_capture_one tree tree --format json --project "$proj" || {
        fixture_teardown
        return 1
    }
    _golden_capture_one describe describe --format json || {
        fixture_teardown
        return 1
    }
    _golden_capture_one tag-list tag list --format json --project "$proj" || {
        fixture_teardown
        return 1
    }
    _golden_capture_one backlog-report backlog report --format json --project "$proj" || {
        fixture_teardown
        return 1
    }
    _golden_capture_one model-show model show --format json || {
        fixture_teardown
        return 1
    }
    _golden_capture_one review-list review list --format json --project "$proj" || {
        fixture_teardown
        return 1
    }
    _golden_capture_one review-show review show "$_golden_review_id" --format json --project "$proj" || {
        fixture_teardown
        return 1
    }
    _golden_capture_one review-requests review requests --format json --project "$proj" || {
        fixture_teardown
        return 1
    }

    # --- worktree list/current are cwd-derived (no --project), so run them
    # with cwd inside the created worktree — `worktree current` returns
    # `null` from the bare code-repo root instead of a meaningful object.
    if ! (cd "$_golden_worktree_path" && "$RDM_BIN" --root "$FIXTURE_PLAN" worktree list --format json) >"$_golden_out_dir/worktree-list.json" 2>/dev/null; then
        echo "golden_capture_all: 'rdm worktree list' failed" >&2
        fixture_teardown
        return 1
    fi
    if ! (cd "$_golden_worktree_path" && "$RDM_BIN" --root "$FIXTURE_PLAN" worktree current --format json) >"$_golden_out_dir/worktree-current.json" 2>/dev/null; then
        echo "golden_capture_all: 'rdm worktree current' failed" >&2
        fixture_teardown
        return 1
    fi

    unset _golden_out_dir _golden_review_id _golden_worktree_path
    return 0
}

# _golden_sed_escape <string> — escape sed/ERE metacharacters in <string> so
# it can be used as a LITERAL pattern in a `sed -E` substitution delimited by
# `|` (mktemp-derived paths never contain `|`). Handles the characters that
# can plausibly appear in a temp-dir path plus every ERE metacharacter, for
# safety against a pathological TMPDIR.
_golden_sed_escape() {
    s=$1
    s=$(printf '%s' "$s" | sed 's/\\/\\\\/g')
    s=$(printf '%s' "$s" | sed 's/\./\\./g')
    s=$(printf '%s' "$s" | sed 's/\*/\\*/g')
    s=$(printf '%s' "$s" | sed 's/\^/\\^/g')
    s=$(printf '%s' "$s" | sed 's/\$/\\$/g')
    s=$(printf '%s' "$s" | sed 's/\[/\\[/g')
    s=$(printf '%s' "$s" | sed 's/\]/\\]/g')
    s=$(printf '%s' "$s" | sed 's/(/\\(/g')
    s=$(printf '%s' "$s" | sed 's/)/\\)/g')
    s=$(printf '%s' "$s" | sed 's/+/\\+/g')
    s=$(printf '%s' "$s" | sed 's/?/\\?/g')
    printf '%s' "$s"
}

# golden_redact <dir>
golden_redact() {
    dir="$1"
    if [ -z "$dir" ] || [ ! -d "$dir" ]; then
        echo "golden_redact: dir argument must be an existing directory" >&2
        return 1
    fi
    if [ -z "${FIXTURE_ROOT:-}" ] || [ ! -d "$FIXTURE_ROOT" ]; then
        echo "golden_redact: FIXTURE_ROOT is not set — call golden_redact only between golden_capture_all and fixture_teardown" >&2
        return 1
    fi

    raw_tmpdir="$FIXTURE_ROOT"
    canon_tmpdir=$(cd "$FIXTURE_ROOT" && pwd -P)
    raw_esc=$(_golden_sed_escape "$raw_tmpdir")
    canon_esc=$(_golden_sed_escape "$canon_tmpdir")

    for f in "$dir"/*.json; do
        [ -f "$f" ] || continue

        # (a) absolute temp paths — longest literal string first so one form
        # can never partially shadow the other.
        if [ "$raw_tmpdir" = "$canon_tmpdir" ]; then
            sed -E -i.bak "s|$raw_esc|<TMPDIR>|g" "$f"
        elif [ ${#canon_tmpdir} -gt ${#raw_tmpdir} ]; then
            sed -E -i.bak "s|$canon_esc|<TMPDIR>|g; s|$raw_esc|<TMPDIR>|g" "$f"
        else
            sed -E -i.bak "s|$raw_esc|<TMPDIR>|g; s|$canon_esc|<TMPDIR>|g" "$f"
        fi

        # (d) review created/submitted RFC3339 datetimes — matched (and thus
        # redacted) before (b)'s NaiveDate rule, though the two patterns are
        # mutually exclusive by construction (the NaiveDate pattern requires
        # the closing quote immediately after the 10-digit date).
        sed -E -i.bak 's/"(created|submitted)": "[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:.]+Z"/"\1": "<DATETIME>"/g' "$f"

        # (b) created/completed NaiveDate fields.
        sed -E -i.bak 's/"(created|completed)": "[0-9]{4}-[0-9]{2}-[0-9]{2}"/"\1": "<DATE>"/g' "$f"

        # (c) commit-SHA-shaped fields.
        sed -E -i.bak 's/"(commit|applied_commit|created_commit|review_sha)": "[0-9a-f]{7,40}"/"\1": "<SHA>"/g' "$f"

        # (e) the review id field (YYYY-MM-DD-HHMM-hex), scoped so the
        # numeric per-comment "id" field is never touched.
        sed -E -i.bak 's/"id": "[0-9]{4}-[0-9]{2}-[0-9]{2}-[0-9]{4}-[0-9a-f]+"/"id": "<REVIEW-ID>"/g' "$f"

        rm -f "$f.bak"
    done

    unset raw_tmpdir canon_tmpdir raw_esc canon_esc f
}
