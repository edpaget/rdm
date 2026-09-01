#!/bin/sh
# Sourced helper library providing a hermetic, deterministic rdm plan-repo
# fixture for test/verify scripts that need real rdm data without touching
# the developer's real plan repo (the one RDM_ROOT normally resolves to).
#
# This file is SOURCED, never executed directly:
#
#   RDM_BIN="$REPO_ROOT/target/debug/rdm"
#   . "$REPO_ROOT/scripts/lib/rdm-plan-fixture.sh"
#   trap 'fixture_teardown' EXIT INT HUP TERM
#   fixture_setup
#   ... drive "$RDM_BIN" --root "$FIXTURE_PLAN" ... ...
#   fixture_teardown
#
# The caller MUST set RDM_BIN (an executable path to the cargo-built rdm
# binary) before calling fixture_setup — this library never derives
# REPO_ROOT/RDM_BIN itself, matching scripts/lib/mechanical-tier-check.sh's
# convention of leaving repo-location resolution to the caller.
#
# Public functions:
#   fixture_setup [project-slug]
#       Stands up a fresh mktemp'd RDM_ROOT, seeds a deterministic set of
#       roadmaps/phases/tasks (see _fixture_seed below), and lands the seed
#       as one git commit. Exports:
#         FIXTURE_ROOT     - the mktemp'd temp directory (parent of everything)
#         FIXTURE_PLAN     - "$FIXTURE_ROOT/plan", the rdm plan repo root
#         FIXTURE_PROJECT  - the project slug (default "fixture-proj")
#       Also reroots HOME/XDG_CONFIG_HOME/XDG_DATA_HOME/XDG_STATE_HOME under
#       FIXTURE_ROOT (saving any prior values for fixture_teardown to
#       restore), unsets every inherited RDM_*-prefixed env var (except
#       RDM_BIN), and exports a fixed
#       git author/committer identity so git commits never fall back to a
#       missing or real global git config.
#   fixture_code_repo [dir-name]
#       Must be called after fixture_setup. Creates a sibling git "code"
#       repo under FIXTURE_ROOT (default dir name "code") on branch `main`
#       with one fixed-message initial commit, so `rdm worktree` has a base
#       ref to branch from. Exports FIXTURE_CODE_REPO.
#   fixture_teardown
#       Removes FIXTURE_ROOT, restores any HOME/XDG_* values fixture_setup
#       overrode (or unsets them if they were unset before). If this library
#       never actually ran a completed fixture_setup (teardown called with no
#       prior setup, or called a second time back-to-back), HOME/XDG_* are
#       left completely untouched — never forced unset. Also unsets every
#       FIXTURE_*/_FIXTURE_* variable this library sets. Safe to call more
#       than once, or with no prior fixture_setup.
#
# Determinism: two fixture_setup calls made on the SAME calendar day produce
# byte-identical `--format json` output for the same rdm commands, once the
# caller redacts exactly this set of fields (nothing else may legitimately
# differ):
#   (a) absolute temp paths — FIXTURE_ROOT, FIXTURE_PLAN, FIXTURE_CODE_REPO,
#       and any path rdm prints beneath them (e.g. a `rdm worktree add`
#       target path);
#   (b) the `created` and `completed` date fields on roadmap/phase/task/
#       review frontmatter — stamped via Local::now().date_naive() in
#       rdm-core/src/ops/task.rs and rdm-core/src/ops/phase.rs, so they
#       track the wall-clock date rather than anything this library
#       controls;
#   (c) any commit-SHA-shaped field — `commit`, `applied_commit`,
#       `review_sha`, or a raw git SHA surfaced by e.g. `rdm status` or
#       `rdm review show`.
# No field outside that list may vary between two same-day runs. This
# library has no standalone committed test file (mirroring
# scripts/lib/mechanical-tier-check.sh); it is exercised by the downstream
# harnesses that source it, plus scratch verification run during
# implementation.
#
# This library never reads or writes the real plan repo: it unsets every
# inherited RDM_*-prefixed variable (including RDM_ROOT, RDM_PROJECT,
# RDM_PLAN_REVIEW, and RDM_REVIEW_AUTHOR — anything rdm-cli/rdm-core reads
# via std::env::var("RDM_..."), except RDM_BIN itself, which fixture_setup
# requires the caller to have already set) before the first rdm invocation
# below. This is a wildcard sweep over the actual environment, not a fixed
# list, so a newly added RDM_* env var in rdm-cli can never silently reopen
# this gap. Every rdm invocation also passes an explicit
# --root "$FIXTURE_PLAN" rather than relying on env propagation.
#
# Failure-path cleanup: fixture_setup does not itself register a trap, so a
# script that aborts (e.g. under `set -e`) between fixture_setup and
# fixture_teardown will leak the mktemp'd FIXTURE_ROOT. Every consumer
# SHOULD register its own cleanup trap immediately after sourcing this file,
# e.g.:
#
#   trap 'fixture_teardown' EXIT INT HUP TERM

# _fixture_rdm <args...> — invoke the caller-provided rdm binary rooted at
# the fixture plan repo. Internal; not part of the public API.
_fixture_rdm() {
    "$RDM_BIN" --root "$FIXTURE_PLAN" "$@"
}

# _fixture_seed — populate the deterministic roadmap/phase/task set.
# Fully fixed literal slugs/titles/numbers/tags, fixed sequential order, no
# nondeterministic input ($RANDOM, PIDs, `date`-derived literals) — the only
# volatile output this can ever produce is the created/completed dates
# rdm-core stamps itself (documented above, not something this function can
# suppress).
_fixture_seed() {
    _fixture_rdm roadmap create sample-roadmap \
        --title "Sample Roadmap" \
        --body "A sample roadmap for exercising list/show/search/next/tree." \
        --no-edit --project "$FIXTURE_PROJECT" >/dev/null

    _fixture_rdm phase create seed-one --title "Seed One" --number 1 \
        --body "First phase: already done." \
        --no-edit --roadmap sample-roadmap --project "$FIXTURE_PROJECT" >/dev/null
    _fixture_rdm phase create seed-two --title "Seed Two" --number 2 \
        --body "Second phase: in progress." \
        --no-edit --roadmap sample-roadmap --project "$FIXTURE_PROJECT" >/dev/null
    _fixture_rdm phase create seed-three --title "Seed Three" --number 3 \
        --body "Third phase: not started." \
        --no-edit --roadmap sample-roadmap --project "$FIXTURE_PROJECT" >/dev/null

    _fixture_rdm phase update 1 --status "done" --no-edit \
        --roadmap sample-roadmap --project "$FIXTURE_PROJECT" >/dev/null
    _fixture_rdm phase update 2 --status in-progress --no-edit \
        --roadmap sample-roadmap --project "$FIXTURE_PROJECT" >/dev/null

    _fixture_rdm task create fixture-task-open --title "Fixture Task Open" \
        --tags bug --body "An open task tagged bug." \
        --no-edit --project "$FIXTURE_PROJECT" >/dev/null
    _fixture_rdm task create fixture-task-active --title "Fixture Task Active" \
        --tags ui --body "An in-progress task tagged ui." \
        --no-edit --project "$FIXTURE_PROJECT" >/dev/null
    _fixture_rdm task create fixture-task-done --title "Fixture Task Done" \
        --tags bug --body "A done task tagged bug." \
        --no-edit --project "$FIXTURE_PROJECT" >/dev/null

    _fixture_rdm task update fixture-task-active --status in-progress \
        --no-edit --project "$FIXTURE_PROJECT" >/dev/null
    _fixture_rdm task update fixture-task-done --status "done" \
        --no-edit --project "$FIXTURE_PROJECT" >/dev/null
}

# fixture_setup [project-slug]
fixture_setup() {
    if [ -z "${RDM_BIN:-}" ] || [ ! -x "$RDM_BIN" ]; then
        echo "fixture_setup: RDM_BIN is not set to an executable rdm binary — run 'cargo build' first and set RDM_BIN=<repo>/target/debug/rdm" >&2
        return 1
    fi

    if [ -n "${FIXTURE_ROOT:-}" ] && [ -d "$FIXTURE_ROOT" ]; then
        echo "fixture_setup: FIXTURE_ROOT ($FIXTURE_ROOT) is already set and still exists — call fixture_teardown before calling fixture_setup again" >&2
        return 1
    fi

    FIXTURE_ROOT=$(mktemp -d) || {
        echo "fixture_setup: mktemp -d failed" >&2
        return 1
    }
    FIXTURE_PLAN="$FIXTURE_ROOT/plan"
    FIXTURE_PROJECT="${1:-fixture-proj}"
    mkdir -p "$FIXTURE_PLAN"

    # Save the caller's HOME/XDG_* (once per setup/teardown pair, so a
    # re-entrant setup call that somehow skipped teardown can't clobber the
    # originals) before rerooting them under FIXTURE_ROOT.
    if [ -z "${_FIXTURE_ENV_SAVED:-}" ]; then
        if [ "${HOME+x}" = "x" ]; then
            _FIXTURE_SAVED_HOME_SET=1
            _FIXTURE_SAVED_HOME="$HOME"
        else
            _FIXTURE_SAVED_HOME_SET=0
        fi
        if [ "${XDG_CONFIG_HOME+x}" = "x" ]; then
            _FIXTURE_SAVED_XDG_CONFIG_HOME_SET=1
            _FIXTURE_SAVED_XDG_CONFIG_HOME="$XDG_CONFIG_HOME"
        else
            _FIXTURE_SAVED_XDG_CONFIG_HOME_SET=0
        fi
        if [ "${XDG_DATA_HOME+x}" = "x" ]; then
            _FIXTURE_SAVED_XDG_DATA_HOME_SET=1
            _FIXTURE_SAVED_XDG_DATA_HOME="$XDG_DATA_HOME"
        else
            _FIXTURE_SAVED_XDG_DATA_HOME_SET=0
        fi
        if [ "${XDG_STATE_HOME+x}" = "x" ]; then
            _FIXTURE_SAVED_XDG_STATE_HOME_SET=1
            _FIXTURE_SAVED_XDG_STATE_HOME="$XDG_STATE_HOME"
        else
            _FIXTURE_SAVED_XDG_STATE_HOME_SET=0
        fi
        _FIXTURE_ENV_SAVED=1
    fi

    HOME="$FIXTURE_ROOT/home"
    XDG_CONFIG_HOME="$FIXTURE_ROOT/xdg-config"
    XDG_DATA_HOME="$FIXTURE_ROOT/xdg-data"
    XDG_STATE_HOME="$FIXTURE_ROOT/xdg-state"
    export HOME XDG_CONFIG_HOME XDG_DATA_HOME XDG_STATE_HOME
    mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_STATE_HOME"

    # Never let an inherited RDM_*-prefixed env var steer a fixture command
    # at the real plan repo — every invocation below also passes --root
    # explicitly regardless. Sweep the actual environment for every RDM_*
    # name (RDM_ROOT, RDM_PROJECT, RDM_PLAN_REVIEW, RDM_REVIEW_AUTHOR,
    # RDM_FORMAT, RDM_PLAN_REPO*, RDM_SERVER_QUICK_FILTERS,
    # RDM_TEST_STALL_HOOK_MS, and any future addition) rather than hardcoding
    # a list that a new rdm-cli env var could silently fall outside of.
    # RDM_BIN is preserved — fixture_setup itself requires it.
    for _fixture_env_name in $(env | LC_ALL=C sed -n 's/^\(RDM_[A-Za-z0-9_]*\)=.*/\1/p'); do
        case "$_fixture_env_name" in
            RDM_BIN) ;;
            *) unset "$_fixture_env_name" ;;
        esac
    done
    unset _fixture_env_name

    # Fixed git identity so a git commit made in the plan repo or a code
    # repo never falls back to a missing/real global git config.
    GIT_AUTHOR_NAME="fixture-bot"
    GIT_AUTHOR_EMAIL="fixture@example.invalid"
    GIT_COMMITTER_NAME="fixture-bot"
    GIT_COMMITTER_EMAIL="fixture@example.invalid"
    export GIT_AUTHOR_NAME GIT_AUTHOR_EMAIL GIT_COMMITTER_NAME GIT_COMMITTER_EMAIL

    _fixture_rdm init --default-project "$FIXTURE_PROJECT" >/dev/null
    _fixture_seed
    _fixture_rdm commit -m "seed: fixture data" >/dev/null
}

# fixture_code_repo [dir-name]
fixture_code_repo() {
    if [ -z "${FIXTURE_ROOT:-}" ] || [ ! -d "$FIXTURE_ROOT" ]; then
        echo "fixture_code_repo: FIXTURE_ROOT is not set — call fixture_setup before fixture_code_repo" >&2
        return 1
    fi

    dir_name="${1:-code}"
    FIXTURE_CODE_REPO="$FIXTURE_ROOT/$dir_name"
    export FIXTURE_CODE_REPO

    git init --quiet -b main "$FIXTURE_CODE_REPO"
    (cd "$FIXTURE_CODE_REPO" && git commit --quiet --allow-empty -m "chore: initial")
}

# fixture_teardown
fixture_teardown() {
    if [ -n "${FIXTURE_ROOT:-}" ]; then
        rm -rf "${FIXTURE_ROOT:?}"
    fi

    # Only restore/unset HOME/XDG_* if THIS library actually saved them in a
    # completed fixture_setup call. If _FIXTURE_ENV_SAVED was never set
    # (fixture_teardown called with no prior fixture_setup, or called a
    # second time back-to-back after the first teardown already cleared it),
    # leave the caller's HOME/XDG_* completely untouched — do not unset them.
    if [ "${_FIXTURE_ENV_SAVED:-0}" = "1" ]; then
        if [ "${_FIXTURE_SAVED_HOME_SET:-0}" = "1" ]; then
            HOME="$_FIXTURE_SAVED_HOME"
            export HOME
        else
            unset HOME
        fi
        if [ "${_FIXTURE_SAVED_XDG_CONFIG_HOME_SET:-0}" = "1" ]; then
            XDG_CONFIG_HOME="$_FIXTURE_SAVED_XDG_CONFIG_HOME"
            export XDG_CONFIG_HOME
        else
            unset XDG_CONFIG_HOME
        fi
        if [ "${_FIXTURE_SAVED_XDG_DATA_HOME_SET:-0}" = "1" ]; then
            XDG_DATA_HOME="$_FIXTURE_SAVED_XDG_DATA_HOME"
            export XDG_DATA_HOME
        else
            unset XDG_DATA_HOME
        fi
        if [ "${_FIXTURE_SAVED_XDG_STATE_HOME_SET:-0}" = "1" ]; then
            XDG_STATE_HOME="$_FIXTURE_SAVED_XDG_STATE_HOME"
            export XDG_STATE_HOME
        else
            unset XDG_STATE_HOME
        fi
    fi

    unset FIXTURE_ROOT FIXTURE_PLAN FIXTURE_PROJECT FIXTURE_CODE_REPO
    unset _FIXTURE_ENV_SAVED
    unset _FIXTURE_SAVED_HOME _FIXTURE_SAVED_HOME_SET
    unset _FIXTURE_SAVED_XDG_CONFIG_HOME _FIXTURE_SAVED_XDG_CONFIG_HOME_SET
    unset _FIXTURE_SAVED_XDG_DATA_HOME _FIXTURE_SAVED_XDG_DATA_HOME_SET
    unset _FIXTURE_SAVED_XDG_STATE_HOME _FIXTURE_SAVED_XDG_STATE_HOME_SET
}
