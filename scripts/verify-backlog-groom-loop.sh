#!/bin/sh
# End-to-end regression for the backlog grooming loop (the rdm-backlog
# skill's command surface).
#
# Seeds a hermetic temp plan repo with fixtures that trigger every signal
# `rdm backlog report` surfaces — stale tasks, a duplicate pair, a tag
# cluster, and a fully-terminal (archivable) roadmap — then drives the real
# ./target/debug/rdm through the whole grooming loop: report, consolidate a
# task into an existing roadmap, merge a duplicate pair, retire a stale task
# with a reason, and archive the terminal roadmap. Everything runs in temp
# dirs against target/debug/rdm; no network, hermetic.
#
# Run after touching rdm-core/src/ops/backlog.rs,
# consolidate_task_into_roadmap/merge_tasks in rdm-core/src/ops/task.rs,
# `rdm roadmap archive`, or the rdm-backlog skill template.
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

# section_has <top-level-key> <needle> <json-file>: true only if <needle>
# appears WITHIN that top-level section of the pretty-printed report JSON.
# Top-level keys sit at a two-space indent, so we print lines from the key
# line until the next two-space-indented key, then grep only that slice. This
# keeps each report assertion scoped to its own signal — a slug that also
# appears under stale_tasks cannot false-green a duplicate_clusters check.
section_has() {
    awk -v key="\"$1\":" '
        $0 ~ "^  " key { insec = 1; next }
        insec && /^  "[a-z_]+":/ { insec = 0 }
        insec { print }
    ' "$3" | grep -q "$2"
}

PLAN="$TMP/plan"
PROJ="groom-proj"
rdm() { "$RDM_BIN" --root "$PLAN" "$@"; }

# ---------------------------------------------------------------------------
say "Setup: plan repo and project"
# ---------------------------------------------------------------------------

mkdir -p "$PLAN"
rdm init --default-project "$PROJ" >/dev/null

# ---------------------------------------------------------------------------
say "Seed fixtures: duplicate pair, tag cluster, stale task, consolidate target, terminal roadmap"
# ---------------------------------------------------------------------------

# Duplicate pair with distinct tags so merge's tag-union is actually exercised.
rdm task create dup-a --title "Fix login bug on mobile" --tags bug \
    --no-edit --project "$PROJ" >/dev/null
rdm task create dup-b --title "Fix login bug on mobile devices" --tags mobile \
    --no-edit --project "$PROJ" >/dev/null

# Tag cluster: distinct, non-duplicate-fuzzy titles sharing a tag.
rdm task create tag-a --title "Refactor the settings loader" --tags cluster-tag \
    --no-edit --project "$PROJ" >/dev/null
rdm task create tag-b --title "Document the export pipeline" --tags cluster-tag \
    --no-edit --project "$PROJ" >/dev/null

# Retire candidate.
rdm task create stale-one --title "Stale One" --body "Retire me." \
    --no-edit --project "$PROJ" >/dev/null

# Existing non-terminal roadmap to consolidate into.
rdm roadmap create existing-rm --title "Existing Roadmap" --body "An existing roadmap." \
    --no-edit --project "$PROJ" >/dev/null
rdm phase create seed --title "Seed" --number 1 --body "seed phase" \
    --no-edit --roadmap existing-rm --project "$PROJ" >/dev/null

# Task to consolidate.
rdm task create consolidate-me --title "Consolidate Me" --body "Body of consolidate-me task." \
    --no-edit --project "$PROJ" >/dev/null

# Fully-terminal archivable roadmap.
rdm roadmap create terminal-rm --title "Terminal Roadmap" --body "A finished roadmap." \
    --no-edit --project "$PROJ" >/dev/null
rdm phase create only --title "Only" --number 1 --body "the only phase" \
    --no-edit --roadmap terminal-rm --project "$PROJ" >/dev/null
rdm phase update phase-1-only --status "done" --no-edit \
    --roadmap terminal-rm --project "$PROJ" >/dev/null

rdm commit -m "seed: groom fixtures" >/dev/null
ok "seeded duplicate pair, tag cluster, stale task, consolidate target, and terminal roadmap"

# ---------------------------------------------------------------------------
say "1. REPORT: assert every signal is surfaced"
# ---------------------------------------------------------------------------

rdm backlog report --older-than 0 --format json --project "$PROJ" >"$TMP/report.json"

section_has stale_tasks '"slug": "dup-a"' "$TMP/report.json" ||
    fail "dup-a must be flagged under stale_tasks"

if ! section_has duplicate_clusters '"slug": "dup-a"' "$TMP/report.json" ||
    ! section_has duplicate_clusters '"slug": "dup-b"' "$TMP/report.json"; then
    fail "duplicate_clusters must contain both dup-a and dup-b"
fi

section_has tag_clusters '"tag": "cluster-tag"' "$TMP/report.json" ||
    fail "tag_clusters must contain a cluster-tag entry"
if ! section_has tag_clusters '"slug": "tag-a"' "$TMP/report.json" ||
    ! section_has tag_clusters '"slug": "tag-b"' "$TMP/report.json"; then
    fail "tag_clusters must contain both tag-a and tag-b"
fi

section_has archivable_roadmaps '"roadmap": "terminal-rm"' "$TMP/report.json" ||
    fail "archivable_roadmaps must contain terminal-rm"

ok "report surfaces stale tasks, the duplicate cluster, the tag cluster, and the archivable roadmap"

# ---------------------------------------------------------------------------
say "2. CONSOLIDATE: fold consolidate-me into existing-rm as a new phase"
# ---------------------------------------------------------------------------

rdm promote consolidate-me --into existing-rm --no-edit --project "$PROJ" \
    >"$TMP/promote.txt" 2>/dev/null

grep -q "Consolidated task 'consolidate-me' → roadmap 'existing-rm' as phase 'phase-2-consolidate-me' (task status: done)" \
    "$TMP/promote.txt" ||
    fail "promote --into must print the consolidation arrow line: $(cat "$TMP/promote.txt")"

rdm roadmap show existing-rm --format json --project "$PROJ" >"$TMP/existing-rm.json"
grep -q '"stem": "phase-2-consolidate-me"' "$TMP/existing-rm.json" ||
    fail "existing-rm must gain phase-2-consolidate-me"

rdm task show consolidate-me --format json --project "$PROJ" >"$TMP/consolidate-me.json"
grep -q '"status": "done"' "$TMP/consolidate-me.json" ||
    fail "consolidate-me must be closed done after consolidation"
grep -q 'Consolidated into roadmap ' "$TMP/consolidate-me.json" ||
    fail "consolidate-me body must carry the consolidation pointer"

ok "consolidate-me folded into existing-rm as phase-2-consolidate-me"

# ---------------------------------------------------------------------------
say "3. MERGE: fold dup-b into dup-a"
# ---------------------------------------------------------------------------

rdm task merge dup-a --from dup-b --no-edit --project "$PROJ" >"$TMP/merge.txt" 2>/dev/null

grep -q "Merged 1 task(s) into 'dup-a'" "$TMP/merge.txt" ||
    fail "task merge must print the merge summary line: $(cat "$TMP/merge.txt")"

rdm task show dup-a --format json --project "$PROJ" >"$TMP/dup-a.json"
grep -q '"bug"' "$TMP/dup-a.json" || fail "dup-a tags must retain bug after merge"
grep -q '"mobile"' "$TMP/dup-a.json" || fail "dup-a tags must gain mobile after merge (union)"
grep -q "## Merged from task \`dup-b\`" "$TMP/dup-a.json" ||
    fail "dup-a body must carry the '## Merged from task \`dup-b\`' heading"

rdm task show dup-b --format json --project "$PROJ" >"$TMP/dup-b.json"
grep -q '"status": "wont-fix"' "$TMP/dup-b.json" || fail "dup-b must be closed wont-fix after merge"
grep -q 'superseded by task/dup-a' "$TMP/dup-b.json" ||
    fail "dup-b close_reason must point at task/dup-a"

ok "dup-b merged into dup-a: tags unioned, provenance recorded, dup-b superseded"

# ---------------------------------------------------------------------------
say "4. RETIRE: mark stale-one wont-fix with a reason"
# ---------------------------------------------------------------------------

rdm task update stale-one --status wont-fix --reason "no longer relevant" \
    --no-edit --project "$PROJ" >/dev/null 2>&1

rdm task show stale-one --format json --project "$PROJ" >"$TMP/stale-one.json"
grep -q '"status": "wont-fix"' "$TMP/stale-one.json" || fail "stale-one must be wont-fix"
grep -q '"no longer relevant"' "$TMP/stale-one.json" ||
    fail "stale-one close_reason must persist the retirement reason"

ok "stale-one retired with a persisted close reason"

# ---------------------------------------------------------------------------
say "5. ARCHIVE: archive the fully-terminal roadmap"
# ---------------------------------------------------------------------------

rdm roadmap archive terminal-rm --project "$PROJ" >"$TMP/archive.txt" 2>/dev/null

grep -q "Archived roadmap 'terminal-rm' from project 'groom-proj'" "$TMP/archive.txt" ||
    fail "roadmap archive must print the confirmation line: $(cat "$TMP/archive.txt")"

rdm roadmap list --archived --format json --project "$PROJ" >"$TMP/archived-list.json"
grep -q '"slug": "terminal-rm"' "$TMP/archived-list.json" ||
    fail "terminal-rm must appear in the archived roadmap list"

rdm roadmap list --format json --project "$PROJ" >"$TMP/active-list.json"
if grep -q '"slug": "terminal-rm"' "$TMP/active-list.json"; then
    fail "terminal-rm must no longer appear in the active roadmap list"
fi

rdm backlog report --older-than 0 --format json --project "$PROJ" >"$TMP/report-after.json"
if section_has archivable_roadmaps '"roadmap": "terminal-rm"' "$TMP/report-after.json"; then
    fail "terminal-rm must drop out of archivable_roadmaps once archived"
fi

ok "terminal-rm archived and dropped from both the active list and the report"

printf '\n\033[1;32mAll backlog-groom-loop checks passed.\033[0m\n'
