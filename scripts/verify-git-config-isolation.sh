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
#   2  the temp-hygiene guarantee from `c647ab0` — no `*__worktrees` directory
#      escapes into TMPDIR — re-measured under the hostile environment, read
#      off section 1's OWN hostile run rather than by re-running
#      scripts/verify-worktree-temp-hygiene.sh. Section 1 already executes
#      every worktree-creating suite with TMPDIR redirected at a scratch
#      directory this script owns, so the leak count is a `find` over a
#      directory that already exists; spawning the other harness again would
#      re-plant its self-test and re-run its suites to measure the same number.
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

# Every run below is additionally scoped with `-p rdm-git -p rdm-cli`. `-E`
# selects which tests RUN; it does not narrow what nextest BUILDS, so without
# the package scope these runs compile every test target in the workspace
# (rdm-server, rdm-tui, rdm-core...) in order to execute tests from two crates
# — and the planted mutation in 1b makes that whole set rebuild. $FILTER names
# no package outside these two, so the scope changes nothing about which tests
# run, only how much is built to run them.

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
#   diff.external         = <script> — defect 1's trigger (phase 20): an
#                                    external diff driver REPLACES git's diff
#                                    machinery, so without `--no-ext-diff` the
#                                    configured script is executed and its
#                                    output (here: nothing) becomes the diff,
#                                    making `unified_diff_argv` report a
#                                    genuinely-modified file as untouched. The
#                                    script touches a marker file, so an
#                                    execution is observable, and exits 0 so the
#                                    failure mode is a silently empty diff
#                                    rather than a loud `external diff died`
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
    # The external diff driver the hostile config points at. Every production
    # `git diff` argv rdm builds was audited before this was added:
    # `rdm-git/src/source.rs` (phase 20) and `rdm-git/src/worktree.rs` carry
    # `--no-ext-diff`, and `rdm-store-git/src/merge.rs` is `--name-only`, which
    # never invokes a driver — so section 1 stays green rather than going red on
    # an unrelated call site.
    cat >"$scratch/hostile-diff-driver.sh" <<'EOF'
#!/bin/sh
: > "$(dirname "$0")/external-diff-ran"
exit 0
EOF
    chmod 755 "$scratch/hostile-diff-driver.sh"
    cat >"$scratch/home/.gitconfig" <<EOF
[user]
	name = Hostile Developer
	email = hostile@example.com
[diff]
	relative = true
	external = $scratch/hostile-diff-driver.sh
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

# Counts `*__worktrees` directories directly inside $1 — the same measure
# scripts/verify-worktree-temp-hygiene.sh makes, applied to the scratch TMPDIR
# a run below already used. `rdm_git::worktree::add` places a worktree as a
# SIBLING of the repo root, so a fixture rooted at its own TempDir puts that
# sibling in TMPDIR, where TempDir::drop never reaches it.
count_leaks() {
    find "$1" -maxdepth 1 -name '*__worktrees' 2>/dev/null | wc -l | tr -d ' '
}

# Runs the implicated suites (with $1/$2/$3 as HOME/GIT_CONFIG_SYSTEM/
# GNUPGHOME overrides, or empty to leave them unset) into scratch dir $4,
# writing a normalized, order-independent PASS/FAIL summary to $5. Returns
# nextest's own exit status.
#
# `CARGO_TERM_COLOR` is pinned to `never` for the same reason this harness
# pins HOME and GIT_CONFIG_SYSTEM: the summary below is parsed out of
# nextest's own output, so an ambient setting that changes that output
# changes the measurement. CI sets `CARGO_TERM_COLOR: always`, which makes
# cargo emit ANSI escapes even into a redirected file, and every per-test
# line then reads `\033[32;1m        PASS\033[0m [ ...` — matching no
# anchored `^\s*PASS\s+\[` pattern. The zero-result guard in section 1
# caught it rather than passing vacuously, but the run was still red.
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
            TMPDIR="$scratch" CARGO_TERM_COLOR=never \
            cargo nextest run -p rdm-git -p rdm-cli -E "$FILTER" \
            >"$scratch/run.log" 2>&1
    else
        TMPDIR="$scratch" CARGO_TERM_COLOR=never \
            cargo nextest run -p rdm-git -p rdm-cli -E "$FILTER" \
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

# Section 1 ran BEFORE either mutation was planted, so its green is already the
# real tree's and can never be the mutant's — the ordering, not a restatement,
# is what guarantees that. What still has to be proved is that the restore put
# both helpers back exactly, so no mutant source survives into the rest of the
# CI run. `cmp` proves that directly, where a fourth full suite run (plus the
# rebuild the restore forces) would only prove it by inference.
if ! cmp -s "$TMP/rdm-git-support.bak" "$REPO_ROOT/$RDM_GIT_SUPPORT" ||
    ! cmp -s "$TMP/cli-support.bak" "$REPO_ROOT/$CLI_SUPPORT"; then
    fail "1b: the git_test_support helpers were NOT restored byte-for-byte after the
planted mutation — mutant source is still on disk. Restore it from git before
running anything else:
  git checkout -- $RDM_GIT_SUPPORT $CLI_SUPPORT"
fi
ok "1b: both planted mutations were restored byte-for-byte"

# ---------------------------------------------------------------------------
# 2 — worktree temp-hygiene under the hostile environment too
# ---------------------------------------------------------------------------
say "2. no worktree escaped TMPDIR during the hostile run either"

# Read off the runs section 1 already made. $TMP/clean and $TMP/hostile were
# each the TMPDIR of a full pass over the worktree-creating suites, so the two
# counts answer "does a hostile git config change where worktrees land?"
# without executing a single additional test.
CLEAN_LEAKS=$(count_leaks "$TMP/clean")
HOSTILE_LEAKS=$(count_leaks "$TMP/hostile")

# Non-vacuity floor. A zero leak count proves nothing if the runs never built a
# worktree in the first place, and this section — unlike
# scripts/verify-worktree-temp-hygiene.sh's 1b, which plants a TempDir-rooted
# fixture and requires the count to go positive — has no mutation of its own.
# What it can require is that $FILTER actually selected the fixtures 1b proves
# are capable of leaking, so a filter edit that quietly drops them fails here
# instead of reporting a clean zero.
WORKTREE_TESTS=$(grep -c 'worktree' "$TMP/clean.summary" || true)
if [ "$WORKTREE_TESTS" -lt 1 ]; then
    fail "2: the clean run executed no worktree test at all, so its zero leak count is
vacuous — \$FILTER no longer selects rdm-git's worktree suites. See
scripts/verify-worktree-temp-hygiene.sh, whose 1b is what establishes that
those fixtures can leak."
fi

if [ "$CLEAN_LEAKS" -ne 0 ]; then
    find "$TMP/clean" -maxdepth 1 -name '*__worktrees' | head -5 >&2
    fail "2: the CLEAN run leaked $CLEAN_LEAKS '*__worktrees' director(ies) into TMPDIR.
That is scripts/verify-worktree-temp-hygiene.sh's subject, not a config-isolation
failure — fix the fixture rooting there first."
fi
if [ "$HOSTILE_LEAKS" -ne 0 ]; then
    find "$TMP/hostile" -maxdepth 1 -name '*__worktrees' | head -5 >&2
    fail "2: the HOSTILE run leaked $HOSTILE_LEAKS '*__worktrees' director(ies) into
TMPDIR while the clean run leaked none — a worktree path is being resolved from
the ambient git config."
fi
ok "2: neither the clean nor the hostile run leaked a worktree into TMPDIR ($WORKTREE_TESTS worktree tests ran)"

printf '\n\033[1;32mAll git-config isolation checks passed.\033[0m\n'
