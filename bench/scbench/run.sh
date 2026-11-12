#!/bin/sh
# run.sh: run any `rdm-measure scbench` subcommand with the Claude credential
# loaded from 1Password, e.g.
#
#   bench/scbench/run.sh ladder --seeds 3 --out bench/scbench/runs/first
#   bench/scbench/run.sh run --arms just-solve,full-lane --seeds 1 --out bench/scbench/runs/smoke
#
# CLAUDE_CODE_OAUTH_TOKEN is set to a 1Password secret reference, and `op run`
# replaces it with the secret in the child's environment only: the token is
# never written to disk, and `op run` masks it in the run's output. Override
# the reference with SCBENCH_CLAUDE_TOKEN_REF (for example to a different
# vault). Needs the 1Password CLI (`op`) signed in.
set -eu

ref="${SCBENCH_CLAUDE_TOKEN_REF:-op://rdm-dev/claude-token/credential}"
root="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"

cd "$root"
CLAUDE_CODE_OAUTH_TOKEN="$ref" exec op run -- \
    cargo run -q -p rdm-devtools --bin rdm-measure -- scbench "$@"
