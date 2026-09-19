#!/bin/sh
# Hermetic regression: the change-review and worktree test suites must
# produce IDENTICAL pass/fail results under a clean developer environment and
# a deliberately HOSTILE one — proving `git_test_support`'s isolation (and
# the guards it exercises, e.g. `--no-relative`/`:(top)` in
# `rdm-git/src/source.rs`'s `unified_diff_argv`) actually works, rather than
# merely that the suites happen to pass when nobody's `~/.gitconfig` gets in
# the way.
#
# `2c55784` fixed a real bug (`unified_diff_argv` returning an empty hunk set
# when `GitSourceRepo` is rooted at a subdirectory of the checkout) that
# passed every test locally yet failed for a real user, because their
# `~/.gitconfig` carried `diff.relative = true` — "a real config users set".
# A suite that never sets that config at all can't tell a working guard from
# a missing one; a suite that always inherits the ambient config can't tell
# a working guard from a lucky developer machine. Section 1 below runs the
# SAME suites twice — once against a clean scratch `HOME`/`GIT_CONFIG_SYSTEM`,
# once against one seeded with several real settings users carry — and
# requires identical results either way. `rdm-git/src/git_test_support.rs`
# and `rdm-cli/tests/git_test_support.rs` are what make that possible: every
# FIXTURE git call is isolated from both, by construction, while the `rdm`
# binary under test (and rdm-git's own read-only/worktree production code,
# neither of which creates a commit in these suites) still receives whatever
# this script's environment provides — so a real regression in the product's
# own guards, not just the fixtures', would show up as a divergence.
#
#   1  the implicated suites pass identically under a clean and a hostile
#      HOME/GIT_CONFIG_SYSTEM
#   1b planted-mutation self-test: stripping the GIT_CONFIG_GLOBAL/SYSTEM
#      isolation lines from BOTH git_test_support.rs copies makes the hostile
#      run diverge from the clean one
#   2  a trailing re-run of scripts/verify-worktree-temp-hygiene.sh, so the
#      temp-hygiene guarantees from `c647ab0` are confirmed leak-free under a
#      hostile environment too, not only a clean one
#
# This script never opens the invoking developer's real `~/.gitconfig` or
# `/etc/gitconfig` — `HOME` and `GIT_CONFIG_SYSTEM` are both redirected at a
# scratch directory it owns, and `GNUPGHOME` is redirected to an empty
# scratch keyring alongside them: the hostile fixture below sets
# `commit.gpgsign = true` deliberately (a setting that must make a fixture
# commit which reaches it FAIL loudly, per `git_test_support`'s isolation,
# rather than silently succeed), and an empty `GNUPGHOME` guarantees that
# failure is fast (`gpg: no default secret key`) rather than an interactive
# signing prompt hanging the run.
#
# Run after touching `rdm-git/src/git_test_support.rs`,
# `rdm-cli/tests/git_test_support.rs`, any of the files that include one of
# them (`rdm-git/src/source.rs`, `rdm-git/src/worktree.rs`,
# `rdm-git/tests/worktree.rs`, `rdm-cli/tests/{cli_worktree,cli_gate,
# cli_verify,cli_review_change}.rs`), or `rdm-git/src/source.rs`'s
# `unified_diff_argv` guards.
#
# Requires: cargo + nextest. Modern git (>= 2.32, for `GIT_CONFIG_GLOBAL`/
# `GIT_CONFIG_SYSTEM` as full-path overrides) — if CI's git predates that,
# this isolation silently no-ops rather than failing loudly.

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

cd "$REPO_ROOT"

# The suites genuinely implicated by change-review/worktree testing:
# rdm-git's source.rs/worktree.rs unit tests plus its `tests/worktree.rs`
# integration test, and the four rdm-cli integration suites that drive real
# project-repo git state. Excludes rdm-git/src/lib.rs's OWN bare `tests`
# module (test names matching `^tests::`, as opposed to `source::tests::`/
# `worktree::tests::`) — its local git() fixture helper is deliberately left
# unisolated (see this phase's file_map: lib.rs gets only the
# `git_test_support` module declaration, no local helper swap), since those
# tests exercise HEAD/log/branch primitives unrelated to this phase's
# diff.relative/worktree-hygiene scope.
FILTER='(package(rdm-git) and not test(/^tests::/)) or (package(rdm-cli) and (binary(cli_worktree) or binary(cli_gate) or binary(cli_verify) or binary(cli_review_change)))'

# Builds a fresh scratch HOME containing a hostile ~/.gitconfig, plus a
# separate hostile file for GIT_CONFIG_SYSTEM. Both are deliberately never
# the real ~/.gitconfig or /etc/gitconfig.
#
#   user.name/email       = set    — a real ~/.gitconfig always has these; a
#                                    scratch HOME that omits them is not
#                                    "hostile", it is simply incomplete, and
#                                    breaks any real commit that (unlike this
#                                    phase's isolated fixtures, which stamp
#                                    identity via GIT_AUTHOR_*/GIT_COMMITTER_*
#                                    env vars) resolves identity from config
#   diff.relative         = true  — the `2c55784` regression's own trigger
#   init.defaultBranch    = weird — a nonstandard default, in case anything
#                                    relies on "main" without asking for it
#   advice.detachedHead   = true  — noisy advice text some git subcommands
#                                    would otherwise leak onto stderr
#   core.pager            = false — would otherwise page long output and
#                                    block on a pty that isn't there
#   commit.gpgsign        = true  — must make a fixture commit that reaches
#                                    it fail loudly (see the header comment)
build_hostile_env() {
    scratch=$1
    mkdir -p "$scratch/home" "$scratch/gnupghome"
    chmod 700 "$scratch/gnupghome"
    cat >"$scratch/home/.gitconfig" <<'EOF'
[user]
	name = Hostile Developer
	email = hostile@example.com
[diff]
	relative = true
[init]
	defaultBranch = weird
[advice]
	detachedHead = true
[core]
	pager = false
[commit]
	gpgsign = true
EOF
    cat >"$scratch/system-gitconfig" <<'EOF'
[diff]
	relative = true
EOF
}

# A milder hostile environment for section 2 below: `build_hostile_env`
# minus `commit.gpgsign` AND `init.defaultBranch` (which several unrelated
# `rdm bootstrap` fixtures assume is left at "main" when they don't pass
# `-b main` explicitly). Section 2 re-runs
# `verify-worktree-temp-hygiene.sh`'s own unscoped `-p rdm-git -p rdm-cli`
# suites (every fixture in both crates, not just the ones this phase
# isolates) purely to confirm the worktree-leak count stays zero under a
# hostile config — a filesystem check unrelated to either setting. Reusing
# the full hostile env there would fail the ~30 OTHER, unrelated `cli_*.rs`
# fixtures this phase deliberately leaves untouched (see AC4), which would
# only prove those files are out of scope, not that leak counting is broken.
build_benign_hostile_env() {
    scratch=$1
    mkdir -p "$scratch/home"
    cat >"$scratch/home/.gitconfig" <<'EOF'
[user]
	name = Hostile Developer
	email = hostile@example.com
[diff]
	relative = true
[advice]
	detachedHead = true
[core]
	pager = false
EOF
    cat >"$scratch/system-gitconfig" <<'EOF'
[diff]
	relative = true
EOF
}

# Runs the implicated suites (with $1/$2/$3 as HOME/GIT_CONFIG_SYSTEM/
# GNUPGHOME overrides, or empty to leave them unset) into scratch dir $4,
# writing a normalized, order-independent PASS/FAIL summary to $5. Returns
# nextest's own exit status.
run_suites() {
    home_override=$1
    system_override=$2
    gnupg_override=$3
    scratch=$4
    summary=$5
    mkdir -p "$scratch"
    set +e
    if [ -n "$home_override" ]; then
        HOME="$home_override" GIT_CONFIG_SYSTEM="$system_override" GNUPGHOME="$gnupg_override" \
            TMPDIR="$scratch" cargo nextest run -E "$FILTER" \
            >"$scratch/run.log" 2>&1
    else
        TMPDIR="$scratch" cargo nextest run -E "$FILTER" \
            >"$scratch/run.log" 2>&1
    fi
    status=$?
    set -e
    # Normalize each per-test result line to "<status> <test-name>", drop
    # timing/index, and treat a LEAK the same as a PASS: nextest's own
    # subprocess-reaping detection is flaky under parallel load (observed
    # independently of any config, hostile or not) and is not a signal this
    # harness is trying to measure.
    grep -E '^[[:space:]]*(PASS|FAIL|LEAK|ABORT|TIMEOUT)[[:space:]]+\[' "$scratch/run.log" |
        sed -E 's/^[[:space:]]*(PASS|FAIL|LEAK|ABORT|TIMEOUT)[[:space:]]+\[[^]]*\][[:space:]]+\([^)]*\)[[:space:]]+(.*)$/\2 \1/' |
        sed -E 's/ LEAK$/ PASS/; s/ (ABORT|TIMEOUT)$/ FAIL/' |
        sort >"$summary"
    return $status
}

# ---------------------------------------------------------------------------
# 1 — identical results under a clean and a hostile environment
# ---------------------------------------------------------------------------
say "1. the implicated suites pass identically under a clean and a hostile HOME/GIT_CONFIG_SYSTEM"

run_suites "" "" "" "$TMP/clean" "$TMP/clean.summary" || {
    tail -40 "$TMP/clean/run.log" >&2
    fail "1: the suites did not pass under a CLEAN environment — fix them before judging isolation"
}
CLEAN_COUNT=$(wc -l <"$TMP/clean.summary" | tr -d ' ')
[ "$CLEAN_COUNT" -gt 0 ] || fail "1: no test results were parsed out of the clean run's log — harness is broken"
ok "1: clean run — $CLEAN_COUNT test results captured"

build_hostile_env "$TMP/hostile-fixture"
if run_suites "$TMP/hostile-fixture/home" "$TMP/hostile-fixture/system-gitconfig" \
    "$TMP/hostile-fixture/gnupghome" "$TMP/hostile" "$TMP/hostile.summary"; then
    :
else
    tail -40 "$TMP/hostile/run.log" >&2
    fail "1: the suites did NOT pass under the HOSTILE environment — a fixture or a
production guard is reading the developer's ambient git config. See
rdm-git/src/git_test_support.rs and rdm-cli/tests/git_test_support.rs."
fi

if ! diff -u "$TMP/clean.summary" "$TMP/hostile.summary" >"$TMP/1.diff"; then
    cat "$TMP/1.diff" >&2
    fail "1: the clean and hostile runs produced DIFFERENT per-test results (see diff
above) — isolation is leaking the hostile HOME/GIT_CONFIG_SYSTEM into a
fixture or a production code path."
fi
ok "1: clean and hostile runs produced identical results ($CLEAN_COUNT tests)"

# ---------------------------------------------------------------------------
# 1b — planted-mutation self-test: strip the isolation, hostile run diverges
# ---------------------------------------------------------------------------
say "1b. planted-mutation self-test — stripping GIT_CONFIG_GLOBAL/SYSTEM isolation is caught"

RDM_GIT_SUPPORT="rdm-git/src/git_test_support.rs"
CLI_SUPPORT="rdm-cli/tests/git_test_support.rs"
cp "$RDM_GIT_SUPPORT" "$TMP/rdm-git-support.bak"
cp "$CLI_SUPPORT" "$TMP/cli-support.bak"
restore_support() {
    cp "$TMP/rdm-git-support.bak" "$REPO_ROOT/$RDM_GIT_SUPPORT"
    cp "$TMP/cli-support.bak" "$REPO_ROOT/$CLI_SUPPORT"
}
trap 'restore_support; rm -rf "$TMP"' EXIT INT HUP TERM

strip_isolation() {
    target=$1
    # Remove the two isolating env lines and the GIT_CONFIG_GLOBAL match arm,
    # leaving every other line (including GIT_AUTHOR/COMMITTER identity)
    # intact — a minimal mutation, not a rewrite.
    python3 - "$target" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read()
old = '''        .env("GIT_CONFIG_SYSTEM", "/dev/null");
    match global_override {
        Some(path) => cmd.env("GIT_CONFIG_GLOBAL", path),
        None => cmd.env("GIT_CONFIG_GLOBAL", "/dev/null"),
    };'''
new = '''        ;
    let _ = global_override;'''
if old not in s:
    sys.exit(f"planted mutation could not be applied to {p} — helper shape changed; update this harness")
open(p, "w").write(s.replace(old, new))
PY
}

strip_isolation "$RDM_GIT_SUPPORT" || fail "1b: could not plant the mutation in $RDM_GIT_SUPPORT"
strip_isolation "$CLI_SUPPORT" || fail "1b: could not plant the mutation in $CLI_SUPPORT"

if run_suites "$TMP/hostile-fixture/home" "$TMP/hostile-fixture/system-gitconfig" \
    "$TMP/hostile-fixture/gnupghome" "$TMP/mutant-hostile" "$TMP/mutant-hostile.summary"; then
    MUTANT_DIVERGED=0
else
    MUTANT_DIVERGED=1
fi
if [ "$MUTANT_DIVERGED" -eq 0 ] && diff -q "$TMP/clean.summary" "$TMP/mutant-hostile.summary" >/dev/null 2>&1; then
    restore_support
    fail "1b: with the isolation lines stripped, the hostile run still matched the
clean one — section 1 is vacuous. Check that the implicated suites actually
exercise git_test_support::git for every fixture commit."
fi
restore_support
ok "1b: stripping the isolation made the hostile run diverge — section 1 is load-bearing"

# Restated on the real tree, so a passing run is never the mutant's.
run_suites "" "" "" "$TMP/clean-again" "$TMP/clean-again.summary" ||
    fail "1b: the restored tree failed under a clean environment — restore failed"
diff -q "$TMP/clean.summary" "$TMP/clean-again.summary" >/dev/null 2>&1 ||
    fail "1b: the restored tree's results differ from the first clean run — restore failed"
ok "1b: restored tree passes identically again"

# ---------------------------------------------------------------------------
# 2 — worktree temp-hygiene, re-run under the hostile environment too
# ---------------------------------------------------------------------------
say "2. scripts/verify-worktree-temp-hygiene.sh, re-run under a hostile environment"

build_benign_hostile_env "$TMP/hygiene-hostile-fixture"
HOME="$TMP/hygiene-hostile-fixture/home" \
    GIT_CONFIG_SYSTEM="$TMP/hygiene-hostile-fixture/system-gitconfig" \
    sh "$SCRIPT_DIR/verify-worktree-temp-hygiene.sh" ||
    fail "2: verify-worktree-temp-hygiene.sh failed under a hostile environment"
ok "2: worktree temp-hygiene holds under a hostile environment too"

printf '\n\033[1;32mAll git-config isolation checks passed.\033[0m\n'
