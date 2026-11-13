#!/bin/sh
# run.sh: run any `rdm-measure scbench` subcommand with the Claude credential
# read from 1Password, e.g.
#
#   bench/scbench/run.sh ladder --seeds 3 --out bench/scbench/runs/first
#   bench/scbench/run.sh run --arms just-solve,full-lane --seeds 1 --out bench/scbench/runs/smoke
#
# It builds rdm-measure first, reads the secret once with `op read` into
# CLAUDE_CODE_OAUTH_TOKEN in the runner's environment only, then `exec`s the
# built binary. No `op` or `cargo` process stays between the operator and the
# runner: run.sh's pid is the runner's, so Ctrl-C in this terminal, or
# `kill -INT`/`kill -TERM` on that pid, stops the run cleanly (slop-code is
# signalled, the containers this run started are removed; see README.md
# "Stopping a run"). SlopCodeBench still writes the token into its logs (the
# `docker exec` line in each problem's infer.log), so after every slop-code
# call `rdm-measure scbench` rewrites every file under that unit's directory
# to replace it with ***redacted***, naming on stderr any file it could not
# rewrite. Override the reference with SCBENCH_CLAUDE_TOKEN_REF (for example
# to a different vault). Needs the 1Password CLI (`op`) signed in.
#
# SCBench talks to Docker through the Python SDK, which only reads DOCKER_HOST
# or /var/run/docker.sock, not the docker CLI's active context. Docker Desktop
# on macOS may create only ~/.docker/run/docker.sock, so when DOCKER_HOST is
# unset it is taken from the active context's endpoint.
set -eu

ref="${SCBENCH_CLAUDE_TOKEN_REF:-op://rdm-dev/claude-token/credential}"
root="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"

if [ -z "${DOCKER_HOST:-}" ]; then
    DOCKER_HOST="$(docker context inspect --format '{{.Endpoints.docker.Host}}')"
    export DOCKER_HOST
fi

cd "$root"
cargo build -q -p rdm-devtools --bin rdm-measure
CLAUDE_CODE_OAUTH_TOKEN="$(op read "$ref")"
export CLAUDE_CODE_OAUTH_TOKEN
exec "${CARGO_TARGET_DIR:-$root/target}/debug/rdm-measure" scbench "$@"
