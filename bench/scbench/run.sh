#!/bin/sh
# run.sh: run any `rdm-measure scbench` subcommand with the Claude credential
# loaded from 1Password, e.g.
#
#   bench/scbench/run.sh ladder --seeds 3 --out bench/scbench/runs/first
#   bench/scbench/run.sh run --arms just-solve,full-lane --seeds 1 --out bench/scbench/runs/smoke
#
# CLAUDE_CODE_OAUTH_TOKEN is set to a 1Password secret reference, and `op run`
# replaces it with the secret in the child's environment and masks it in the
# run's live output. SlopCodeBench still writes the token into its logs (the
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
CLAUDE_CODE_OAUTH_TOKEN="$ref" exec op run -- \
    cargo run -q -p rdm-devtools --bin rdm-measure -- scbench "$@"
