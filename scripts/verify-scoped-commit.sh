#!/bin/sh
# Hermetic regression for session-scoped commit / status / discard.
#
# Drives REAL separate processes against temp plan repos to gate every
# acceptance criterion of `plan-repo-concurrency/phase-5-scope-commits-to-changesets`:
#
#   A  two concurrent sessions produce two commits with disjoint path sets
#   B  the same, with no session id set at all (rung 2), plus rung-2
#      cross-process continuity (B2) and rung-4 degradation (B3)
#   C  the `Done:` hook path is scoped — a DISTINCT section, because every
#      other criterion can pass while `apply_done_directives` still sweeps
#   D  every commit primitive call site is on an explicit allowlist
#   E  `rdm init --remote` lands its config commit, a backfilled
#      `.gitattributes` reaches a commit, and a server mutation is attributable
#   F  committed indexes reflect HEAD plus the committing changeset only
#   G  `rdm discard` cannot destroy another session's work, on a disjoint
#      path (G) or a path both sessions journaled — an overwritten write
#      (G3) or a recreated delete (G4) is left in place and reported skipped
#      rather than clobbered
#   H  reads stay shared — no read isolation was introduced
#   I  a commit under a project another session has not landed still lands,
#      with a coherent index
#
# Sections C, D, F, G3/G4 and I carry planted-mutation self-tests proving
# they can fail.
#
# Run after touching rdm-store-git's commit/status/discard paths, the
# `GitStore` scoped entry points, `rdm_core::session::journal`, or the
# `rdm commit`/`rdm status`/`rdm discard` CLI surfaces.
#
# Requires: cargo-built rdm at target/debug/rdm (from this repo). No network.

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

# Clear rdm-related env inherited from the caller's shell so the run never
# touches the developer's real plan repo — and so the rung under test in each
# section is unambiguous. A stray CLAUDE_CODE_SESSION_ID would silently move
# section B from rung 2 to rung 3.
unset RDM_ROOT RDM_PROJECT RDM_STAGE RDM_FORMAT RDM_SESSION
unset CLAUDE_CODE_SESSION_ID CLAUDE_SESSION_ID RDM_HARNESS_SESSION_ID
# An inherited git environment (set whenever this runs under a git hook) would
# point every `git -C <temp-repo>` query below at the invoking repo instead.
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE

# `rdm <entity> create` reads a body from stdin when none is passed inline, so
# an inherited open stdin would hang the run. Detach it once, for this script
# and everything it spawns.
exec </dev/null

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

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# seed_repo <dir>: stand up a plan repo with one project and an initial commit.
#
# Pins RDM_SESSION so the seeding short-circuits at rung 1 before any lease is
# created. Load-bearing, not hygiene: a lease held by this script's own pid
# would be inherited by both driving shells below, merging the very sessions
# the sections exist to prove distinct.
seed_repo() {
    _dir=$1
    mkdir -p "$_dir"
    RDM_SESSION=harness-seed "$RDM_BIN" --root "$_dir" init --default-project demo >/dev/null
    RDM_SESSION=harness-seed "$RDM_BIN" --root "$_dir" commit \
        -m "seed: init plan repo and project" >/dev/null
}

# commit_files <repo> [rev]: the path set of one commit, sorted.
commit_files() {
    git -C "$1" show --name-only --pretty=format: "${2:-HEAD}" |
        grep -v '^$' | sort
}

# head_sha <repo>
head_sha() { git -C "$1" rev-parse HEAD; }

# contains_path <file-listing> <path>
contains_path() { grep -qx "$2" "$1"; }

# assert_absent <listing> <path> <message>
assert_absent() {
    if grep -qx "$2" "$1"; then
        fail "$3 (found '$2' in: $(tr '\n' ' ' <"$1"))"
    fi
}

# index_link_targets <index-file>: every markdown `](target)` in an index, one
# per line. Well-defined because `format_top_level_index` emits one row per
# project whose only link target is `projects/<p>/INDEX.md`.
#
# The replacement below carries a LITERAL newline (a backslash-newline in the
# sed script) rather than `\n`: BSD sed does not expand `\n` on the right-hand
# side, and the silent result would be a helper that extracts nothing and an
# assertion that passes vacuously.
# The trailing `echo` is equally load-bearing: `tr` collapses the file into one
# unterminated line, and a `while read` loop silently drops a final line with
# no newline — which would make the LAST link in the index unchecked.
index_link_targets() {
    {
        tr '\n' ' ' <"$1"
        echo
    } | sed 's/](/\
](/g' | sed -n 's/^](\([^)]*\)).*/\1/p'
}

# assert_no_dangling_links <index-file> <tree-listing>: the committed index
# must never name a path the committed tree does not contain.
assert_no_dangling_links() {
    index_link_targets "$1" >"$TMP/links.$$"
    while read -r _target; do
        [ -n "$_target" ] || continue
        grep -qx "$_target" "$2" ||
            fail "the committed index links '$_target', absent from the committed tree"
    done <"$TMP/links.$$"
    rm -f "$TMP/links.$$"
}

# assert_no_orphan_project_index <tree-listing>: no `projects/<p>/INDEX.md`
# without a sibling `projects/<p>/project.md` in the same tree.
assert_no_orphan_project_index() {
    grep '^projects/[^/]*/INDEX\.md$' "$1" >"$TMP/pidx.$$" || :
    while read -r _idx; do
        [ -n "$_idx" ] || continue
        _manifest=$(echo "$_idx" | sed 's|/INDEX\.md$|/project.md|')
        grep -qx "$_manifest" "$1" ||
            fail "the committed tree holds '$_idx' but not '$_manifest'"
    done <"$TMP/pidx.$$"
    rm -f "$TMP/pidx.$$"
}

# ---------------------------------------------------------------------------
# Section A — two concurrent sessions, two disjoint commits
# ---------------------------------------------------------------------------
say "Section A: two sessions committing produce two commits with disjoint paths"

REPO_A="$TMP/repo-a"
seed_repo "$REPO_A"
BASE_A=$(head_sha "$REPO_A")

# Two long-lived driving shells with distinct explicit ids. Each creates its
# own task and then commits, as two separate `rdm` processes.
for who in alpha beta; do
    {
        echo '#!/bin/sh'
        echo "\"$RDM_BIN\" --root \"$REPO_A\" task create $who-task --title \"$who\" --no-edit --project demo >/dev/null"
        echo "\"$RDM_BIN\" --root \"$REPO_A\" commit -m \"add $who-task\""
        echo ':'
    } >"$TMP/a-$who.sh"
done

# Sequential, not concurrent, so both commits are observable in history: the
# property under test is what each commit CONTAINS, and interleaving the two
# would only make the assertion flaky, never stronger. Their changesets are
# built up concurrently either way — session alpha's uncommitted work is on
# disk while beta commits, which is exactly the hazard.
RDM_SESSION=sess-alpha sh "$TMP/a-alpha.sh" >"$TMP/a-alpha.out" 2>&1 ||
    fail "alpha shell failed: $(cat "$TMP/a-alpha.out")"
RDM_SESSION=sess-beta sh "$TMP/a-beta.sh" >"$TMP/a-beta.out" 2>&1 ||
    fail "beta shell failed: $(cat "$TMP/a-beta.out")"

# Interleave check: beta's task was created BEFORE alpha committed? No — the
# stronger arrangement is created below in section F. Here we assert the
# commits themselves.
COMMITS=$(git -C "$REPO_A" rev-list --count "$BASE_A"..HEAD)
[ "$COMMITS" = "2" ] || fail "expected exactly 2 new commits, got $COMMITS"

commit_files "$REPO_A" HEAD >"$TMP/a.head"
commit_files "$REPO_A" HEAD~1 >"$TMP/a.prev"

contains_path "$TMP/a.prev" "projects/demo/tasks/alpha-task.md" ||
    fail "alpha's commit does not contain alpha's task: $(cat "$TMP/a.prev")"
contains_path "$TMP/a.head" "projects/demo/tasks/beta-task.md" ||
    fail "beta's commit does not contain beta's task: $(cat "$TMP/a.head")"
assert_absent "$TMP/a.prev" "projects/demo/tasks/beta-task.md" \
    "alpha's commit swept up beta's uncommitted task"
assert_absent "$TMP/a.head" "projects/demo/tasks/alpha-task.md" \
    "beta's commit re-committed alpha's already-landed task"
ok "each commit contains only its own task file — asserted in BOTH directions"

# Each commit's non-task paths must be its own reconciled indexes and nothing
# else. There is no third session, so anything beyond the two index files and
# the session's own task would be a leak.
for f in "$TMP/a.prev" "$TMP/a.head"; do
    while read -r p; do
        case "$p" in
            projects/demo/tasks/*-task.md | INDEX.md | projects/demo/INDEX.md) ;;
            *) fail "unexpected path '$p' in a scoped commit: $(tr '\n' ' ' <"$f")" ;;
        esac
    done <"$f"
done
ok "each commit contains only its own task plus its own reconciled indexes"

[ -z "$(git -C "$REPO_A" status --porcelain)" ] ||
    fail "tree still dirty after both sessions committed: $(git -C "$REPO_A" status --porcelain)"
ok "both changesets landed — nothing left stranded on disk"

# ---------------------------------------------------------------------------
# Section B — the same, with NO session id set
# ---------------------------------------------------------------------------
say "Section B: disjointness holds with no session id set (rung 2 leases)"

REPO_B="$TMP/repo-b"
seed_repo "$REPO_B"
BASE_B=$(head_sha "$REPO_B")

for who in gamma delta; do
    {
        echo '#!/bin/sh'
        echo "\"$RDM_BIN\" --root \"$REPO_B\" session id --format json"
        echo "\"$RDM_BIN\" --root \"$REPO_B\" task create $who-task --title \"$who\" --no-edit --project demo >/dev/null"
        echo "\"$RDM_BIN\" --root \"$REPO_B\" commit -m \"add $who-task\" >/dev/null"
        echo ':'
    } >"$TMP/b-$who.sh"
done

# Two driving shells are two parents, so rung 2 mints two ids.
env -u RDM_SESSION -u CLAUDE_CODE_SESSION_ID sh "$TMP/b-gamma.sh" >"$TMP/b-gamma.out" 2>&1 ||
    fail "gamma shell failed: $(cat "$TMP/b-gamma.out")"
env -u RDM_SESSION -u CLAUDE_CODE_SESSION_ID sh "$TMP/b-delta.sh" >"$TMP/b-delta.out" 2>&1 ||
    fail "delta shell failed: $(cat "$TMP/b-delta.out")"

ID_G=$(sed -n 's/.*"id":"\([^"]*\)".*/\1/p' "$TMP/b-gamma.out" | head -n 1)
ID_D=$(sed -n 's/.*"id":"\([^"]*\)".*/\1/p' "$TMP/b-delta.out" | head -n 1)
[ -n "$ID_G" ] && [ -n "$ID_D" ] || fail "could not read the two resolved ids"
# Non-vacuity: without this the section could pass because both shells
# committed nothing at all.
[ "$ID_G" != "$ID_D" ] ||
    fail "both shells resolved the SAME id ($ID_G) — the section would pass vacuously"
ok "the two shells resolved distinct ids without any session id set ($ID_G / $ID_D)"

COMMITS_B=$(git -C "$REPO_B" rev-list --count "$BASE_B"..HEAD)
[ "$COMMITS_B" = "2" ] || fail "expected exactly 2 new commits, got $COMMITS_B"
commit_files "$REPO_B" HEAD >"$TMP/b.head"
commit_files "$REPO_B" HEAD~1 >"$TMP/b.prev"
contains_path "$TMP/b.prev" "projects/demo/tasks/gamma-task.md" ||
    fail "gamma's commit does not contain gamma's task"
assert_absent "$TMP/b.prev" "projects/demo/tasks/delta-task.md" \
    "gamma's commit swept up delta's uncommitted task"
assert_absent "$TMP/b.head" "projects/demo/tasks/gamma-task.md" \
    "delta's commit re-committed gamma's already-landed task"
ok "disjointness holds on rung 2, in both directions"

# --- B2: rung-2 continuity across two processes in ONE shell ---------------
say "Section B2: a changeset survives across processes in one parent shell"

REPO_B2="$TMP/repo-b2"
seed_repo "$REPO_B2"
{
    echo '#!/bin/sh'
    echo "\"$RDM_BIN\" --root \"$REPO_B2\" task create carried --title \"Carried\" --no-edit --project demo >/dev/null"
    echo "\"$RDM_BIN\" --root \"$REPO_B2\" commit -m \"add carried\""
    echo ':'
} >"$TMP/b2.sh"
env -u RDM_SESSION -u CLAUDE_CODE_SESSION_ID sh "$TMP/b2.sh" >"$TMP/b2.out" 2>&1 ||
    fail "b2 shell failed: $(cat "$TMP/b2.out")"
grep -q "Nothing" "$TMP/b2.out" &&
    fail "the empty-changeset diagnostic fired — rung 2 did not carry the changeset across processes: $(cat "$TMP/b2.out")"
commit_files "$REPO_B2" HEAD >"$TMP/b2.files"
contains_path "$TMP/b2.files" "projects/demo/tasks/carried.md" ||
    fail "create-then-commit as two processes did not land the file: $(cat "$TMP/b2.files")"
ok "one parent shell's changeset carries across two rdm processes"

# --- B3: rung-4 degradation reports rather than sweeps ---------------------
say "Section B3: an unattributable dirty tree is reported, never swept"

REPO_B3="$TMP/repo-b3"
seed_repo "$REPO_B3"
# Write a file with no rdm process at all: it can belong to no changeset.
mkdir -p "$REPO_B3/projects/demo/tasks"
cat >"$REPO_B3/projects/demo/tasks/orphan.md" <<'EOF'
---
slug: orphan
title: Orphan
status: open
priority: medium
created: 2026-01-01
---

Body.
EOF
env -u RDM_SESSION -u CLAUDE_CODE_SESSION_ID "$RDM_BIN" --root "$REPO_B3" \
    commit -m "should not sweep" >"$TMP/b3.out" 2>&1 ||
    fail "commit failed outright: $(cat "$TMP/b3.out")"
commit_files "$REPO_B3" HEAD >"$TMP/b3.files"
assert_absent "$TMP/b3.files" "projects/demo/tasks/orphan.md" \
    "the scoped commit SWEPT an unattributed path"
grep -q "orphan.md" "$TMP/b3.out" ||
    fail "the unattributed path was not reported: $(cat "$TMP/b3.out")"
grep -q "are not attributed to any changeset" "$TMP/b3.out" ||
    fail "orphan.md was not reported as unattributed: $(cat "$TMP/b3.out")"
# No changeset actually owns this path, so only the whole-tree recovery
# applies — the changeset-targeted hints would point at nothing.
grep -q -- "rdm commit --all" "$TMP/b3.out" ||
    fail "recovery pointer 'rdm commit --all' missing from: $(cat "$TMP/b3.out")"
for hint in "rdm session list" "rdm commit --changeset" "belong to another changeset"; do
    grep -q -- "$hint" "$TMP/b3.out" &&
        fail "unattributed dirt wrongly offered the changeset-targeted hint '$hint': $(cat "$TMP/b3.out")"
done
grep -qx "Nothing to commit." "$TMP/b3.out" &&
    fail "printed the bare 'Nothing to commit.' while the tree was dirty"
ok "unattributed dirt is named as unattributed, with only the --all recovery pointer, and not swept"

# The whole-tree opt-in is the documented recovery and must actually work.
env -u RDM_SESSION "$RDM_BIN" --root "$REPO_B3" commit --all -m "recover" >/dev/null
commit_files "$REPO_B3" HEAD >"$TMP/b3.files2"
contains_path "$TMP/b3.files2" "projects/demo/tasks/orphan.md" ||
    fail "--all did not recover the unattributed path"
ok "--all recovers the unattributed path"

# ---------------------------------------------------------------------------
# Section C — the hook path is scoped (DISTINCT from A/B)
# ---------------------------------------------------------------------------
# Every other criterion in this file can pass while `apply_done_directives`
# still commits the whole tree, because no other section drives the hook.
say "Section C: the Done:-directive hook commits only its own changeset"

REPO_C="$TMP/repo-c"
seed_repo "$REPO_C"
RDM_SESSION=sess-hook-a "$RDM_BIN" --root "$REPO_C" task create hook-task \
    --title "Hook" --no-edit --project demo >/dev/null
RDM_SESSION=sess-hook-a "$RDM_BIN" --root "$REPO_C" commit -m "add hook-task" >/dev/null

# Session B holds an uncommitted task the hook must not touch.
RDM_SESSION=sess-hook-b "$RDM_BIN" --root "$REPO_C" task create bystander \
    --title "Bystander" --no-edit --project demo >/dev/null
[ -f "$REPO_C/projects/demo/tasks/bystander.md" ] ||
    fail "the bystander file was not created, so this section is vacuous"

# A real commit carrying the directive, made from A's lineage, then a real
# `rdm hook post-commit` run against it.
git -C "$REPO_C" commit --allow-empty --quiet \
    -m "chore: land hook-task

Done: task/hook-task"
BEFORE_C=$(head_sha "$REPO_C")
(cd "$REPO_C" && RDM_SESSION=sess-hook-a "$RDM_BIN" --root "$REPO_C" hook post-commit) \
    >"$TMP/c.out" 2>&1 || fail "hook post-commit failed: $(cat "$TMP/c.out")"
AFTER_C=$(head_sha "$REPO_C")
[ "$BEFORE_C" != "$AFTER_C" ] ||
    fail "the hook created no commit, so this section is vacuous"

RDM_SESSION=sess-hook-a "$RDM_BIN" --root "$REPO_C" task show hook-task \
    --project demo --no-body >"$TMP/c.show" 2>&1
grep -qi "done" "$TMP/c.show" || fail "the directive did not land: $(cat "$TMP/c.show")"
ok "the Done: directive landed"

commit_files "$REPO_C" HEAD >"$TMP/c.files"
assert_absent "$TMP/c.files" "projects/demo/tasks/bystander.md" \
    "the hook commit SWEPT session B's uncommitted task"
ok "the hook commit excludes the other session's uncommitted file"

[ -f "$REPO_C/projects/demo/tasks/bystander.md" ] ||
    fail "the hook deleted session B's file from disk"
grep -q "bystander" "$REPO_C/.git/rdm/changesets/sess-hook-b.jsonl" ||
    fail "session B's journal no longer claims its file"
ok "session B's file is still on disk and still in its journal"

# Self-test: a hook that reverted to a whole-tree commit MUST fail this
# section. Simulate exactly that revert by taking a whole-tree commit from
# A's session while B still holds its file, and showing the same assertion
# catches it.
RDM_SESSION=sess-hook-a "$RDM_BIN" --root "$REPO_C" commit --all \
    -m "simulated whole-tree hook" >/dev/null
commit_files "$REPO_C" HEAD >"$TMP/c.files.swept"
contains_path "$TMP/c.files.swept" "projects/demo/tasks/bystander.md" ||
    fail "self-test failed: a whole-tree commit did NOT pick up the bystander, so section C's assertion proves nothing"
ok "self-test: a whole-tree (unscoped) hook IS caught by the same assertion"

# ---------------------------------------------------------------------------
# Section D — every commit primitive call site is on an explicit allowlist
# ---------------------------------------------------------------------------
say "Section D: no unsanctioned caller of a commit primitive"

# The primitives. `create_git_commit` and `git_commit` are `pub(crate)` in
# rdm-store-git, so an out-of-crate caller cannot even compile; this grep
# additionally catches a new IN-crate caller and every caller of the
# whole-tree escape hatch, which the compiler cannot object to.
# Anchored on a `.`/`::` call prefix so a mere *mention* (a doc link, or a
# test named `commit_creates_git_commit`) is not mistaken for a call site.
PRIMITIVES='[.:]\(create_git_commit\|git_commit\|git_commit_changeset\|commit_whole_tree\|commit_changeset\|commit_changeset_id\)('

# The sanctioned sites, by file. Adding a legitimate caller is a deliberate
# edit to this list — which is printed on failure so the reason is obvious.
# The two `rdm-server/tests/` entries are test-seeding callers, which are a
# legitimate whole-tree class rather than an oversight: seeding a fixture
# genuinely wants the sweep.
ALLOWLIST='rdm-store-git/src/commit.rs
rdm-store-git/src/lib.rs
rdm-cli/src/commands/commit.rs
rdm-cli/src/commands/mod.rs
rdm-cli/src/commands/bootstrap.rs
rdm-cli/src/commands/init.rs
rdm-mcp/src/server.rs
rdm-server/src/state.rs
rdm-server/tests/git_history.rs
rdm-server/tests/mutation_policy.rs'

scan_primitives() {
    # $1: root to scan. Prints "file" for every hit outside the allowlist.
    (
        cd "$1" || exit 1
        find . -name '*.rs' -not -path './target/*' | sed 's|^\./||'
    ) | while read -r f; do
        [ -f "$1/$f" ] || continue
        if grep -q "$PRIMITIVES" "$1/$f"; then
            printf '%s\n' "$f"
        fi
    done
}

scan_primitives "$REPO_ROOT" | sort >"$TMP/d.hits"
[ -s "$TMP/d.hits" ] || fail "the primitive grep matched nothing at all — it proves nothing"

printf '%s\n' "$ALLOWLIST" | sort >"$TMP/d.allow"
UNSANCTIONED=$(comm -23 "$TMP/d.hits" "$TMP/d.allow")
if [ -n "$UNSANCTIONED" ]; then
    fail "unsanctioned commit-primitive caller(s):
$UNSANCTIONED

The sanctioned sites are:
$ALLOWLIST

A new committer must be a deliberate addition to this allowlist, and must use
the SCOPED entry point (GitStore::commit_changeset) unless it genuinely wants
the machine-global sweep (GitStore::commit_whole_tree)."
fi
ok "every commit-primitive caller is on the allowlist ($(wc -l <"$TMP/d.hits" | tr -d ' ') file(s))"

# Self-test 1: a planted new caller IS caught.
mkdir -p "$TMP/d-scratch/rdm-cli/src/commands"
cp "$REPO_ROOT/rdm-cli/src/commands/status.rs" "$TMP/d-scratch/rdm-cli/src/commands/status.rs"
printf '\nfn planted() { store.commit_whole_tree("oops"); }\n' \
    >>"$TMP/d-scratch/rdm-cli/src/commands/status.rs"
scan_primitives "$TMP/d-scratch" | sort >"$TMP/d.scratch.hits"
grep -qx 'rdm-cli/src/commands/status.rs' "$TMP/d.scratch.hits" ||
    fail "self-test failed: a planted commit-primitive caller is NOT caught, so section D proves nothing"
ok "self-test: a planted new commit-primitive caller IS caught"

# Self-test 2: the real tree passes (already asserted above, restated so both
# arms of the gate are explicit).
[ -z "$(comm -23 "$TMP/d.hits" "$TMP/d.allow")" ] ||
    fail "self-test failed: the real, unmutated tree does not pass its own gate"
ok "self-test: the real, unmutated tree passes"

# ---------------------------------------------------------------------------
# Section E — Store-bypassing writers still reach a commit
# ---------------------------------------------------------------------------
say "Section E: init --remote, a backfilled .gitattributes, and server writes"

# --- E1: `rdm init --remote` lands its config commit ------------------------
SRC_E="$TMP/repo-e-src"
seed_repo "$SRC_E"
BARE_E="$TMP/repo-e.git"
git clone --quiet --bare "$SRC_E" "$BARE_E"
CLONE_E="$TMP/repo-e-clone"
RDM_SESSION=sess-e "$RDM_BIN" --root "$CLONE_E" init --remote "file://$BARE_E" \
    >"$TMP/e1.out" 2>&1 || fail "init --remote failed: $(cat "$TMP/e1.out")"
commit_files "$CLONE_E" HEAD >"$TMP/e1.files"
contains_path "$TMP/e1.files" "rdm.toml" ||
    fail "init --remote did not land rdm.toml: $(cat "$TMP/e1.files")"
grep -q 'default = "origin"' "$CLONE_E/rdm.toml" ||
    fail "the committed rdm.toml does not carry the remote setting"
ok "rdm init --remote still lands its rdm.toml config commit"

# --- E2: a backfilled `.gitattributes` reaches a commit ---------------------
REPO_E2="$TMP/repo-e2"
seed_repo "$REPO_E2"
# Rewrite HEAD so it predates the merge mapping. Raw git deliberately: every
# rdm command re-ensures the mapping on open, so the removal cannot be made
# through rdm.
git -C "$REPO_E2" rm --cached --quiet .gitattributes
git -C "$REPO_E2" commit --quiet -m "chore: pre-driver repo"
rm -f "$REPO_E2/.gitattributes"
git -C "$REPO_E2" ls-tree --name-only HEAD | grep -q '^\.gitattributes$' &&
    fail "HEAD still carries .gitattributes, so this arm is vacuous"

# Opening the repo backfills the file; the very next scoped commit must carry
# it even though no Store batch ever wrote it.
RDM_SESSION=sess-e2 "$RDM_BIN" --root "$REPO_E2" task create e2-task \
    --title "E2" --no-edit --project demo >/dev/null
RDM_SESSION=sess-e2 "$RDM_BIN" --root "$REPO_E2" commit -m "add e2-task" >/dev/null
commit_files "$REPO_E2" HEAD >"$TMP/e2.files"
contains_path "$TMP/e2.files" ".gitattributes" ||
    fail "the backfilled .gitattributes never reached a commit: $(cat "$TMP/e2.files")"
ok "a backfilled .gitattributes reaches a scoped commit"

# --- E3: a store-bypassing server-shaped write is attributable -------------
# `rdm-server`'s shipped default is staging-only: its writes are journaled to
# a startup changeset and reconciled with `rdm commit --changeset <id>`. Drive
# exactly that contract with the real binary: a mutation under a known
# changeset id, committed later by id from a DIFFERENT session.
#
# This arm gates the *reconciliation* half — that a changeset written by one
# process lands intact from another. The HTTP half (that a server mutation
# reaches a commit under `--autocommit`, and fails loudly under the
# staging-only default: a per-mutation stderr WARN plus an `X-Rdm-Staged`
# response header carrying this exact command) is gated by
# `rdm-server/tests/mutation_policy.rs`, which needs a bound listener and so
# lives with the Rust tests rather than here. CI runs both.
REPO_E3="$TMP/repo-e3"
seed_repo "$REPO_E3"
RDM_SESSION=server-session "$RDM_BIN" --root "$REPO_E3" task create server-task \
    --title "Server" --no-edit --project demo >/dev/null
# Staging-only: nothing committed yet, and the pending work is visible.
RDM_SESSION=other-session "$RDM_BIN" --root "$REPO_E3" status >"$TMP/e3.status" 2>&1
grep -q "belong to other changesets" "$TMP/e3.status" ||
    fail "the pending server changeset is not loudly reported: $(cat "$TMP/e3.status")"
commit_files "$REPO_E3" HEAD >"$TMP/e3.before"
assert_absent "$TMP/e3.before" "projects/demo/tasks/server-task.md" \
    "the staging-only default committed on its own"
ok "the staging-only default leaves the write staged and loudly reported"

RDM_SESSION=other-session "$RDM_BIN" --root "$REPO_E3" \
    commit --changeset server-session -m "reconcile server changeset" \
    >"$TMP/e3.out" 2>&1 || fail "commit --changeset failed: $(cat "$TMP/e3.out")"
commit_files "$REPO_E3" HEAD >"$TMP/e3.after"
contains_path "$TMP/e3.after" "projects/demo/tasks/server-task.md" ||
    fail "the documented reconciliation command did not land the server write: $(cat "$TMP/e3.after")"
ok "the documented reconciliation command lands the server write"

# ---------------------------------------------------------------------------
# Section F — committed indexes reflect HEAD plus the committing changeset
# ---------------------------------------------------------------------------
say "Section F: the committed INDEX.md is HEAD plus this changeset only"

REPO_F="$TMP/repo-f"
seed_repo "$REPO_F"
RDM_SESSION=sess-f-a "$RDM_BIN" --root "$REPO_F" roadmap create alpha-map \
    --title "Alpha Map" --no-edit --project demo >/dev/null
RDM_SESSION=sess-f-b "$RDM_BIN" --root "$REPO_F" roadmap create beta-map \
    --title "Beta Map" --no-edit --project demo >/dev/null

# On disk the shared index now carries BOTH — that is the defect this scoping
# exists to contain.
grep -q "alpha-map" "$REPO_F/projects/demo/INDEX.md" ||
    fail "the on-disk index does not carry alpha, so this section is vacuous"
grep -q "beta-map" "$REPO_F/projects/demo/INDEX.md" ||
    fail "the on-disk index does not carry beta, so this section is vacuous"

RDM_SESSION=sess-f-a "$RDM_BIN" --root "$REPO_F" commit -m "add alpha-map" >/dev/null
git -C "$REPO_F" show "HEAD:projects/demo/INDEX.md" >"$TMP/f.committed"
grep -q "alpha-map" "$TMP/f.committed" ||
    fail "A's commit's index does not contain A's row: $(cat "$TMP/f.committed")"
grep -q "beta-map" "$TMP/f.committed" &&
    fail "A's commit's index LEAKED B's uncommitted row: $(cat "$TMP/f.committed")"
ok "the committed index carries A's row and NOT B's"

grep -q "beta-map" "$REPO_F/projects/demo/INDEX.md" ||
    fail "the on-disk index lost B's row — a scoped commit must not rewrite disk"
ok "the on-disk index still carries both rows"

RDM_SESSION=sess-f-b "$RDM_BIN" --root "$REPO_F" commit -m "add beta-map" >/dev/null
git -C "$REPO_F" show "HEAD:projects/demo/INDEX.md" >"$TMP/f.committed2"
grep -q "alpha-map" "$TMP/f.committed2" ||
    fail "B's commit dropped A's already-landed row: $(cat "$TMP/f.committed2")"
grep -q "beta-map" "$TMP/f.committed2" ||
    fail "B's commit does not contain B's own row: $(cat "$TMP/f.committed2")"
ok "B's later commit carries BOTH rows — HEAD now includes A"

[ -z "$(git -C "$REPO_F" status --porcelain)" ] ||
    fail "the reconciled index does not match disk after both commits: $(git -C "$REPO_F" status --porcelain)"
ok "the reconciled indexes converge on the on-disk state"

# Self-test: a derived blob taken FROM DISK would carry B's row into A's
# commit. Prove the assertion above can see that, using a whole-tree commit
# (which is exactly "take the derived blob from disk") as the stand-in.
REPO_F2="$TMP/repo-f2"
seed_repo "$REPO_F2"
RDM_SESSION=sess-f2-a "$RDM_BIN" --root "$REPO_F2" roadmap create alpha-map \
    --title "Alpha Map" --no-edit --project demo >/dev/null
RDM_SESSION=sess-f2-b "$RDM_BIN" --root "$REPO_F2" roadmap create beta-map \
    --title "Beta Map" --no-edit --project demo >/dev/null
RDM_SESSION=sess-f2-a "$RDM_BIN" --root "$REPO_F2" commit --all -m "disk-sourced index" >/dev/null
git -C "$REPO_F2" show "HEAD:projects/demo/INDEX.md" >"$TMP/f2.committed"
grep -q "beta-map" "$TMP/f2.committed" ||
    fail "self-test failed: a disk-sourced derived blob does NOT leak the other row, so section F proves nothing"
ok "self-test: a disk-sourced derived blob IS caught by the same assertion"

# ---------------------------------------------------------------------------
# Section G — discard cannot destroy another session's work
# ---------------------------------------------------------------------------
say "Section G: a scoped discard leaves another session's work intact"

REPO_G="$TMP/repo-g"
seed_repo "$REPO_G"
RDM_SESSION=sess-g-a "$RDM_BIN" --root "$REPO_G" roadmap create gone-map \
    --title "Gone" --no-edit --project demo >/dev/null
RDM_SESSION=sess-g-b "$RDM_BIN" --root "$REPO_G" roadmap create kept-map \
    --title "Kept" --no-edit --project demo >/dev/null

A_FILE="$REPO_G/projects/demo/roadmaps/gone-map/roadmap.md"
B_FILE="$REPO_G/projects/demo/roadmaps/kept-map/roadmap.md"
[ -f "$A_FILE" ] && [ -f "$B_FILE" ] || fail "fixture files missing, section G is vacuous"
B_BEFORE=$(cksum <"$B_FILE")

RDM_SESSION=sess-g-a "$RDM_BIN" --root "$REPO_G" discard --force >"$TMP/g.out" 2>&1 ||
    fail "discard failed: $(cat "$TMP/g.out")"

[ ! -f "$A_FILE" ] || fail "A's own file survived its discard"
ok "A's file is gone"
[ -f "$B_FILE" ] || fail "the scoped discard DESTROYED B's uncommitted file"
[ "$(cksum <"$B_FILE")" = "$B_BEFORE" ] || fail "B's file was modified by A's discard"
ok "B's file is still on disk, byte-identical"
grep -q "kept-map" "$REPO_G/.git/rdm/changesets/sess-g-b.jsonl" ||
    fail "B's journal no longer claims its file"
ok "B's journal still lists its file"
grep -q "kept-map" "$REPO_G/projects/demo/INDEX.md" ||
    fail "the regenerated index dropped B's still-uncommitted row"
grep -q "gone-map" "$REPO_G/projects/demo/INDEX.md" &&
    fail "the regenerated index still carries the discarded roadmap"
ok "the on-disk index kept B's row and dropped A's"

RDM_SESSION=sess-g-b "$RDM_BIN" --root "$REPO_G" commit -m "add kept-map" >"$TMP/g.commit" 2>&1 ||
    fail "B could not commit after A's discard: $(cat "$TMP/g.commit")"
git -C "$REPO_G" show "HEAD:projects/demo/INDEX.md" >"$TMP/g.index"
grep -q "kept-map" "$TMP/g.index" || fail "B's commit landed a wrong index: $(cat "$TMP/g.index")"
grep -q "gone-map" "$TMP/g.index" && fail "B's commit resurrected A's discarded roadmap"
ok "B can still commit afterwards, with a correct index"

# Negative arm: the whole-tree opt-in DOES destroy, and warns first.
REPO_G2="$TMP/repo-g2"
seed_repo "$REPO_G2"
RDM_SESSION=sess-g2-a "$RDM_BIN" --root "$REPO_G2" roadmap create a-map \
    --title "A" --no-edit --project demo >/dev/null
RDM_SESSION=sess-g2-b "$RDM_BIN" --root "$REPO_G2" roadmap create b-map \
    --title "B" --no-edit --project demo >/dev/null
RDM_SESSION=sess-g2-a "$RDM_BIN" --root "$REPO_G2" discard --force --all \
    >"$TMP/g2.out" 2>"$TMP/g2.err" || fail "whole-tree discard failed: $(cat "$TMP/g2.err")"
[ ! -f "$REPO_G2/projects/demo/roadmaps/b-map/roadmap.md" ] ||
    fail "--all did NOT destroy the other session's work, so the negative arm proves nothing"
grep -q "sess-g2-b" "$TMP/g2.err" ||
    fail "--all did not name the changeset it was about to destroy: $(cat "$TMP/g2.err")"
ok "--all destroys other sessions' work and names them first"

# --- G3: overlapping-path arm — A writes twice, B overwrites once after ----
# A's own path-set scoping (G above) only protects a DISJOINT path. Here A
# and B both journal writes to the SAME never-committed path: A's discard
# must skip it, not clobber it.
say "Section G3: discard leaves a path another session overwrote since"

REPO_G3="$TMP/repo-g3"
seed_repo "$REPO_G3"
RDM_SESSION=sess-g3-a "$RDM_BIN" --root "$REPO_G3" task create shared \
    --title "Shared" --no-edit --project demo >/dev/null
RDM_SESSION=sess-g3-a "$RDM_BIN" --root "$REPO_G3" commit -m "seed shared task" >/dev/null
RDM_SESSION=sess-g3-a "$RDM_BIN" --root "$REPO_G3" task update shared \
    --body "A's first edit" --no-edit --project demo >/dev/null
RDM_SESSION=sess-g3-a "$RDM_BIN" --root "$REPO_G3" task update shared \
    --body "A's second edit" --no-edit --project demo >/dev/null
RDM_SESSION=sess-g3-b "$RDM_BIN" --root "$REPO_G3" task update shared \
    --body "B's edit" --no-edit --project demo >/dev/null

G3_FILE="$REPO_G3/projects/demo/tasks/shared.md"
grep -q "B's edit" "$G3_FILE" || fail "fixture: B's overwrite did not land, section G3 is vacuous"
G3_BEFORE=$(cksum <"$G3_FILE")

RDM_SESSION=sess-g3-a "$RDM_BIN" --root "$REPO_G3" discard --force \
    >"$TMP/g3.out" 2>&1 || fail "A's discard failed: $(cat "$TMP/g3.out")"
[ "$(cksum <"$G3_FILE")" = "$G3_BEFORE" ] ||
    fail "A's discard overwrote B's content on the shared path"
ok "B's overwrite survives A's discard, byte-identical"
grep -q "skipped" "$TMP/g3.out" || fail "A's discard did not report a skipped path: $(cat "$TMP/g3.out")"
grep -q "shared.md" "$TMP/g3.out" || fail "A's discard did not name the skipped path: $(cat "$TMP/g3.out")"
ok "A's discard reports the skipped path by name"

RDM_SESSION=sess-g3-b "$RDM_BIN" --root "$REPO_G3" commit -m "land B's edit" \
    >"$TMP/g3.commit" 2>&1 || fail "B could not commit after A's discard: $(cat "$TMP/g3.commit")"
git -C "$REPO_G3" show "HEAD:projects/demo/tasks/shared.md" | grep -q "B's edit" ||
    fail "B's commit did not land B's content"
ok "B's commit still lands with B's content"

# --- G4: delete-then-recreate arm -------------------------------------------
# A creates+commits a roadmap, then deletes it (uncommitted). B recreates a
# roadmap at the same slug/path with different content. A's discard must
# leave B's recreated file in place, not revert it to A's original HEAD
# content.
say "Section G4: discard leaves a path another session recreated after a delete"

REPO_G4="$TMP/repo-g4"
seed_repo "$REPO_G4"
RDM_SESSION=sess-g4-a "$RDM_BIN" --root "$REPO_G4" roadmap create shared-map \
    --title "A's roadmap" --no-edit --project demo >/dev/null
RDM_SESSION=sess-g4-a "$RDM_BIN" --root "$REPO_G4" commit -m "seed shared-map" >/dev/null
RDM_SESSION=sess-g4-a "$RDM_BIN" --root "$REPO_G4" roadmap delete shared-map \
    --force --project demo >/dev/null
RDM_SESSION=sess-g4-b "$RDM_BIN" --root "$REPO_G4" roadmap create shared-map \
    --title "B's roadmap" --no-edit --project demo >/dev/null

G4_FILE="$REPO_G4/projects/demo/roadmaps/shared-map/roadmap.md"
grep -q "B's roadmap" "$G4_FILE" || fail "fixture: B's recreate did not land, section G4 is vacuous"
G4_BEFORE=$(cksum <"$G4_FILE")

RDM_SESSION=sess-g4-a "$RDM_BIN" --root "$REPO_G4" discard --force \
    >"$TMP/g4.out" 2>&1 || fail "A's discard failed: $(cat "$TMP/g4.out")"
[ -f "$G4_FILE" ] || fail "A's discard destroyed B's recreated roadmap.md"
[ "$(cksum <"$G4_FILE")" = "$G4_BEFORE" ] ||
    fail "A's discard reverted B's recreated content instead of leaving it in place"
ok "B's recreated file survives A's discard, byte-identical"
grep -q "skipped" "$TMP/g4.out" || fail "A's discard did not report a skipped path: $(cat "$TMP/g4.out")"
grep -q "shared-map" "$TMP/g4.out" || fail "A's discard did not name the skipped path: $(cat "$TMP/g4.out")"
ok "A's discard reports the recreated path by name"

# Self-test: without the guard, a discard-shaped restore-to-HEAD clobbers a
# path another session also claims. `--all` is exactly that shape (an
# unconditional restore-to-HEAD, with no per-path content check) and stands
# in for the pre-fix scoped behavior on this ONE shared path, the same way
# section F's self-test uses `commit --all` as a stand-in for a disk-sourced
# derived blob.
REPO_G3S="$TMP/repo-g3-selftest"
seed_repo "$REPO_G3S"
RDM_SESSION=sess-g3s-a "$RDM_BIN" --root "$REPO_G3S" task create shared \
    --title "Shared" --no-edit --project demo >/dev/null
RDM_SESSION=sess-g3s-a "$RDM_BIN" --root "$REPO_G3S" commit -m "seed shared task" >/dev/null
RDM_SESSION=sess-g3s-a "$RDM_BIN" --root "$REPO_G3S" task update shared \
    --body "A's edit" --no-edit --project demo >/dev/null
RDM_SESSION=sess-g3s-b "$RDM_BIN" --root "$REPO_G3S" task update shared \
    --body "B's edit" --no-edit --project demo >/dev/null
G3S_FILE="$REPO_G3S/projects/demo/tasks/shared.md"
RDM_SESSION=sess-g3s-a "$RDM_BIN" --root "$REPO_G3S" discard --force --all \
    >/dev/null 2>&1 || fail "self-test: whole-tree discard failed unexpectedly"
grep -q "B's edit" "$G3S_FILE" &&
    fail "self-test failed: the unconditional-restore stand-in did NOT clobber B's edit, so section G3's assertion proves nothing"
ok "self-test: an unconditional restore-to-HEAD IS caught by section G3's assertion"

REPO_G4S="$TMP/repo-g4-selftest"
seed_repo "$REPO_G4S"
RDM_SESSION=sess-g4s-a "$RDM_BIN" --root "$REPO_G4S" roadmap create shared-map \
    --title "A's roadmap" --no-edit --project demo >/dev/null
RDM_SESSION=sess-g4s-a "$RDM_BIN" --root "$REPO_G4S" commit -m "seed shared-map" >/dev/null
RDM_SESSION=sess-g4s-a "$RDM_BIN" --root "$REPO_G4S" roadmap delete shared-map \
    --force --project demo >/dev/null
RDM_SESSION=sess-g4s-b "$RDM_BIN" --root "$REPO_G4S" roadmap create shared-map \
    --title "B's roadmap" --no-edit --project demo >/dev/null
G4S_FILE="$REPO_G4S/projects/demo/roadmaps/shared-map/roadmap.md"
RDM_SESSION=sess-g4s-a "$RDM_BIN" --root "$REPO_G4S" discard --force --all \
    >/dev/null 2>&1 || fail "self-test: whole-tree discard failed unexpectedly"
grep -q "B's roadmap" "$G4S_FILE" &&
    fail "self-test failed: the unconditional-restore stand-in did NOT clobber B's recreated content, so section G4's assertion proves nothing"
ok "self-test: an unconditional restore-to-HEAD IS caught by section G4's assertion"

# ---------------------------------------------------------------------------
# Section H — reads stay shared
# ---------------------------------------------------------------------------
say "Section H: reads are NOT isolated — A can read B's uncommitted item"

REPO_H="$TMP/repo-h"
seed_repo "$REPO_H"
RDM_SESSION=sess-h-b "$RDM_BIN" --root "$REPO_H" task create shared-read \
    --title "Shared Read" --no-edit --project demo >/dev/null

RDM_SESSION=sess-h-a "$RDM_BIN" --root "$REPO_H" task show shared-read \
    --project demo >"$TMP/h.show" 2>&1 ||
    fail "session A could not READ session B's uncommitted task: $(cat "$TMP/h.show")"
grep -q "Shared Read" "$TMP/h.show" ||
    fail "the read returned no content: $(cat "$TMP/h.show")"
ok "rdm task show crosses the session boundary"

RDM_SESSION=sess-h-a "$RDM_BIN" --root "$REPO_H" search "Shared Read" \
    --project demo >"$TMP/h.search" 2>&1 || fail "search failed: $(cat "$TMP/h.search")"
grep -q "shared-read" "$TMP/h.search" ||
    fail "search did not find another session's uncommitted item: $(cat "$TMP/h.search")"
ok "rdm search crosses the session boundary — no read isolation was introduced"

# ---------------------------------------------------------------------------
# Section I — a commit under a project another session has not landed
# ---------------------------------------------------------------------------
say "Section I: B commits under a project A created but never committed"

REPO_I="$TMP/repo-i"
seed_repo "$REPO_I"

# A creates a SECOND project and leaves it staged.
RDM_SESSION=sess-i-a "$RDM_BIN" --root "$REPO_I" project create alt \
    --title "Alt" >"$TMP/i.a.out" 2>&1 ||
    fail "A could not create the project: $(cat "$TMP/i.a.out")"

# Vacuity guards: the manifest must be on disk and NOT in HEAD, or the whole
# section proves nothing.
[ -f "$REPO_I/projects/alt/project.md" ] ||
    fail "fixture: A's project.md was never written, section I is vacuous"
git -C "$REPO_I" ls-tree -r --name-only HEAD >"$TMP/i.tree.before"
assert_absent "$TMP/i.tree.before" "projects/alt/project.md" \
    "fixture: A's project already landed, section I is vacuous"

RDM_SESSION=sess-i-b "$RDM_BIN" --root "$REPO_I" task create b-task \
    --title "B" --no-edit --project alt >"$TMP/i.b.out" 2>&1 ||
    fail "B could not create its task: $(cat "$TMP/i.b.out")"

RDM_SESSION=sess-i-b "$RDM_BIN" --root "$REPO_I" commit -m "land b" >"$TMP/i.out" 2>&1 ||
    fail "B's commit aborted over A's uncommitted project: $(cat "$TMP/i.out")"
if grep -q "project not found" "$TMP/i.out"; then
    fail "the misleading 'project not found' error survived: $(cat "$TMP/i.out")"
fi
ok "B's commit landed instead of aborting with 'project not found'"

git -C "$REPO_I" ls-tree -r --name-only HEAD >"$TMP/i.tree"
git -C "$REPO_I" show "HEAD:INDEX.md" >"$TMP/i.index"

contains_path "$TMP/i.tree" "projects/alt/tasks/b-task.md" ||
    fail "B's own document did not land: $(tr '\n' ' ' <"$TMP/i.tree")"
assert_absent "$TMP/i.tree" "projects/alt/project.md" \
    "A's uncommitted manifest was swept into B's commit"
assert_absent "$TMP/i.tree" "projects/alt/INDEX.md" \
    "an orphan project index landed in B's commit"
assert_no_dangling_links "$TMP/i.index" "$TMP/i.tree"
assert_no_orphan_project_index "$TMP/i.tree"
ok "the committed tree and index are coherent: no dangling row, no orphan index"

if grep -q 'projects/alt/INDEX.md' "$TMP/i.index"; then
    fail "the root index names a project index the commit does not contain"
fi
ok "accepted trade: B's own document is absent from the index B commits — the divergence is one-directional (tree ⊇ index) and heals below"

# Healing arm: the deferred rows return the moment A commits.
RDM_SESSION=sess-i-a "$RDM_BIN" --root "$REPO_I" commit -m "land alt" >"$TMP/i.heal.out" 2>&1 ||
    fail "A could not commit afterwards: $(cat "$TMP/i.heal.out")"

git -C "$REPO_I" ls-tree -r --name-only HEAD >"$TMP/i.tree2"
git -C "$REPO_I" show "HEAD:INDEX.md" >"$TMP/i.index2"
contains_path "$TMP/i.tree2" "projects/alt/project.md" ||
    fail "A's manifest did not land: $(tr '\n' ' ' <"$TMP/i.tree2")"
contains_path "$TMP/i.tree2" "projects/alt/INDEX.md" ||
    fail "the deferred project index did not return: $(tr '\n' ' ' <"$TMP/i.tree2")"
grep -q 'projects/alt/INDEX.md' "$TMP/i.index2" ||
    fail "the deferred root-index row did not return: $(cat "$TMP/i.index2")"
git -C "$REPO_I" show "HEAD:projects/alt/INDEX.md" >"$TMP/i.altindex"
grep -q 'b-task' "$TMP/i.altindex" ||
    fail "B's task did not reappear in the reconciled project index: $(cat "$TMP/i.altindex")"
assert_no_dangling_links "$TMP/i.index2" "$TMP/i.tree2"
assert_no_orphan_project_index "$TMP/i.tree2"
[ -z "$(git -C "$REPO_I" status --porcelain)" ] ||
    fail "the reconciled indexes did not converge on disk: $(git -C "$REPO_I" status --porcelain)"
ok "the deferred rows return once the owning session lands its project"

# Self-test arm 1: a planted dangling row must be caught. Run in a subshell,
# because `fail` exits.
cp "$TMP/i.index" "$TMP/i.index.corrupt"
printf '| [ghost](projects/ghost/INDEX.md) | 0 | 0 | no roadmaps |\n' >>"$TMP/i.index.corrupt"
if (assert_no_dangling_links "$TMP/i.index.corrupt" "$TMP/i.tree") >/dev/null 2>&1; then
    fail "self-test: the dangling-link check is blind to a planted row"
fi
ok "self-test: a planted dangling row IS caught"

# Self-test arm 2: a planted orphan project index must be caught.
cp "$TMP/i.tree" "$TMP/i.tree.corrupt"
printf 'projects/ghost/INDEX.md\n' >>"$TMP/i.tree.corrupt"
if (assert_no_orphan_project_index "$TMP/i.tree.corrupt") >/dev/null 2>&1; then
    fail "self-test: the orphan-project-index check is blind to a planted entry"
fi
ok "self-test: a planted orphan project index IS caught"

printf '\n\033[1;32mAll scoped-commit checks passed.\033[0m\n'
