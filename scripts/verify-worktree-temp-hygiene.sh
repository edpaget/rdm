#!/bin/sh
# Hermetic regression: worktree-creating tests must not leak worktrees into the
# system temp directory.
#
# `rdm_git::worktree::add` places a worktree at
#
#     repo_root.parent() / "<repo-name>__worktrees" / <item-dir>
#
# — a SIBLING of the repo root. That is correct production layout (a worktree
# for `~/Projects/rdm` belongs in `~/Projects/rdm__worktrees`), but it makes the
# obvious test fixture wrong: a fixture whose git root IS the `TempDir` puts
# that sibling in the system temp directory, where `TempDir::drop` never
# reaches it. Every such run leaves a populated git worktree behind forever.
#
# That was not hypothetical. When this harness was written the developer machine
# had ~44,000 leaked `*__worktrees` directories, 64% of them holding real
# checked-out content, accumulating across months — `cli_worktree` alone leaked
# 15 per run and `cli_gate` 9.
#
# The fix is one line per fixture: root the repo one level DOWN
# (`dir.path().join("repo")`), so the sibling lands inside the `TempDir`.
#
#   1  a full run of every worktree-creating suite leaks nothing
#   1b planted-mutation self-test: a fixture rooted back at the `TempDir` leaks
#
# Hermetic: `TMPDIR` is redirected at a scratch directory, so the count is this
# run's own and never reads (or depends on) whatever is in the real temp dir.
#
# Run after touching `worktree_path`/`add` in `rdm-git/src/worktree.rs`, or any
# worktree test fixture: `rdm-cli/tests/{cli_worktree,cli_gate,cli_verify}.rs`,
# `rdm-git/tests/worktree.rs`.
#
# Requires: cargo + nextest (it builds and runs the real test suites).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

say() { printf '\n\033[1;34m==>\033[0m %s\n' "$*"; }
fail() {
    printf '\n\033[1;31m[FAIL]\033[0m %s\n' "$*" >&2
    exit 1
}
ok() { printf '\033[1;32m[ OK ]\033[0m %s\n' "$*"; }

# The suites that create real worktrees. Keep in sync with the fixtures listed
# in the header. Deliberately both whole crates rather than a list of test
# binaries: section 1 is a leak COUNT, and its sensitivity is exactly the
# number of worktree-creating fixtures it runs, so a net that has to be edited
# whenever a fixture is added is a net that silently stops catching things.
SUITES="-p rdm-git -p rdm-cli"

# Section 1b's planted mutation lives in rdm-git/tests/worktree.rs, an
# integration-test target of rdm-git that nothing else depends on, so only
# rdm-git's own suites can leak because of it. Scoping the mutant run keeps the
# self-test measuring the same thing while sparing a relink of rdm-cli's ~37
# test binaries (which the mutation and the restore would otherwise force
# twice).
MUTANT_SUITES="-p rdm-git"

# Counts `*__worktrees` directories directly inside $1.
count_leaks() {
    find "$1" -maxdepth 1 -name '*__worktrees' 2>/dev/null | wc -l | tr -d ' '
}

# Runs the worktree suites with TMPDIR pointed at a fresh scratch dir and
# echoes how many `*__worktrees` directories were left behind there.
#
# Redirecting TMPDIR is what makes this hermetic: `tempfile` honors it, so both
# the fixtures' TempDirs AND any leaked sibling land in our scratch dir.
leak_delta_for_a_run() {
    scratch=$1
    suites=$2
    mkdir -p "$scratch"
    # shellcheck disable=SC2086
    TMPDIR="$scratch" cargo nextest run $suites >"$TMP/nextest.log" 2>&1 || {
        sed -n '$p' "$TMP/nextest.log" >&2
        fail "the worktree test suites did not pass — fix them before judging leakage"
    }
    count_leaks "$scratch"
}

cd "$REPO_ROOT"

# ---------------------------------------------------------------------------
# 1 — the real tree leaks nothing
# ---------------------------------------------------------------------------
say "1. a full run of the worktree suites leaves no worktree in TMPDIR"

LEAKED=$(leak_delta_for_a_run "$TMP/scratch-clean" "$SUITES")
if [ "$LEAKED" -ne 0 ]; then
    find "$TMP/scratch-clean" -maxdepth 2 -name '*__worktrees' | head -5 >&2
    fail "1: $LEAKED leaked '*__worktrees' director(ies) in TMPDIR after the run.
A worktree fixture is rooted at its TempDir instead of a subdirectory of it, so
rdm_git::worktree::add's sibling worktree escapes TempDir::drop. Root the repo
one level down: let root = dir.path().join(\"repo\")."
fi
ok "1: no worktree escaped TMPDIR ($(count_leaks "$TMP/scratch-clean") leaked)"

# ---------------------------------------------------------------------------
# 1b — planted-mutation self-test: the check can actually fail
# ---------------------------------------------------------------------------
say "1b. planted-mutation self-test — a TempDir-rooted fixture is caught"

FIXTURE="rdm-git/tests/worktree.rs"
cp "$FIXTURE" "$TMP/fixture.bak"
restore_fixture() { cp "$TMP/fixture.bak" "$REPO_ROOT/$FIXTURE"; }
trap 'restore_fixture; rm -rf "$TMP"' EXIT INT HUP TERM

# Revert init_project_repo to the leaking shape: git root == TempDir root.
cat >"$TMP/mutate.py" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read()
old = '''fn init_project_repo() -> SourceRepo {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-b", "main"]);
    std::fs::write(root.join("README.md"), "# project").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "initial commit"]);
    SourceRepo { _dir: dir, root }
}'''
new = '''fn init_project_repo() -> SourceRepo {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    git(&root, &["init", "-b", "main"]);
    std::fs::write(root.join("README.md"), "# project").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "initial commit"]);
    SourceRepo { _dir: dir, root }
}'''
if old not in s:
    sys.exit("planted mutation could not be applied — fixture shape changed; update this harness")
open(p, "w").write(s.replace(old, new))
PY
python3 "$TMP/mutate.py" "$REPO_ROOT/$FIXTURE" || fail "1b: could not plant the mutation"

MUT_LEAKED=$(leak_delta_for_a_run "$TMP/scratch-mutant" "$MUTANT_SUITES")
restore_fixture

if [ "$MUT_LEAKED" -eq 0 ]; then
    fail "1b: the planted TempDir-rooted fixture leaked NOTHING, so section 1 is
vacuous — it would stay green through a real regression. Check that the suites
in \$MUTANT_SUITES actually exercise $FIXTURE's init_project_repo."
fi
ok "1b: the planted regression leaked $MUT_LEAKED director(ies) — section 1 is load-bearing"

# Section 1 ran BEFORE the mutation was planted, so its green is already the
# real tree's and can never be the mutant's — the ordering, not a restatement,
# is what guarantees that. What still has to be proved is that the restore put
# the fixture back exactly, so the mutation cannot survive into the rest of the
# CI run; `cmp` proves that directly, where a third full suite run (plus the
# rebuild the restore forces) would only prove it by inference.
cmp -s "$TMP/fixture.bak" "$REPO_ROOT/$FIXTURE" ||
    fail "1b: $FIXTURE was NOT restored byte-for-byte after the planted mutation —
the mutant fixture is still on disk. Restore it from git before running anything
else: git checkout -- $FIXTURE"
ok "1b: the planted mutation was restored byte-for-byte"

printf '\n\033[1;32mAll worktree temp-hygiene checks passed.\033[0m\n'
