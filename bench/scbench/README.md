# SCBench control-arm runner

`rdm-measure scbench run` drives [SlopCodeBench](https://github.com/SprocketLab/slop-code-bench)
(SCBench) from the pinned manifest in this directory. Each manifest arm gets
one `slop-code run` and one `slop-code eval`. The arms here are upstream's
own control prompts, `just-solve` and `anti_slop`. rdm's arms are measured
against them later.

> **COST WARNING.** `scbench run` without `--dry-run` runs paid Claude Code
> agents in Docker. Each arm runs every problem in the subset, and every
> checkpoint of each problem. Cost scales with arms × problems × checkpoints,
> and a run takes hours. Use `--dry-run` first.

## What is pinned

`manifest.toml` holds every value a result depends on:

- the SCBench commit and the `scb-problems` commit
- the provider and the model (Claude Opus 5.5)
- the thinking level and the Claude Code version installed in the agent image
- the pass policy, the environment and the cost limits
- the problem subset and the arms

The runner builds the `slop-code` run config and agent config from the
manifest alone. No SCBench default decides anything, so the manifest always
describes the configuration that would run.

`models/opus-5.5.yaml` defines Opus 5.5 for SCBench. The pinned SCBench has
no Opus 5.5 entry and loads models only from its own `configs/models/`, so
the runner copies this file there. It is the only addition rdm makes to the
SCBench checkout.

## Prerequisites

- Docker, with the daemon running. The runner probes
  `docker version --format {{.Server.Version}}`, which fails when the daemon
  is down.
- [`uv`](https://docs.astral.sh/uv/) and Python 3.12 or later.
- A credential:
  - **Subscription** (the manifest's `provider = "claude_code_oauth"`): run
    `claude setup-token` and export the token as `CLAUDE_CODE_OAUTH_TOKEN`.
  - **API key**: set `provider = "anthropic"` in the manifest and export
    `ANTHROPIC_API_KEY`. Do not pair `anthropic` with
    `CLAUDE_CODE_OAUTH_TOKEN`; SCBench would pass the token as an API key.
- The host `claude` CLI. The runner records its version. The agent image
  installs the manifest's `claude_code_version` itself.

## Running

From the rdm checkout:

```bash
# Print the plan: provisioning, probes, model copy, and each arm's configs
# and commands. Spawns nothing and creates nothing.
cargo run -q -p rdm-devtools --bin rdm-measure -- scbench run \
  --out bench/scbench/runs/<name> --dry-run

# The paid run.
cargo run -q -p rdm-devtools --bin rdm-measure -- scbench run \
  --out bench/scbench/runs/<name>
```

`--out` is required and must be missing or empty. The runner refuses a used
directory, so a finished run is never overwritten.

Other flags:

- `--manifest PATH` (default `bench/scbench/manifest.toml`)
- `--cache-dir DIR` (default `bench/scbench/cache`)
- `--uv-bin`, `--docker-bin`, `--claude-bin` to choose the executables

The runner:

1. resolves the rdm SHA from the checkout holding the manifest
2. probes the `uv`, Docker and `claude` versions
3. clones or fetches both pinned repositories under `--cache-dir` and checks
   out each pin
4. copies the model file into the SCBench checkout
5. runs each arm in manifest order

It stops at the first failing step with a non-zero exit. An arm failure is
reported as `error: arm <name>: <run|eval> exited <code>`. Arms that already
finished stay on disk.

### Run, then evaluate

Each arm runs `slop-code run --no-evaluate`, then `slop-code eval <run_dir>`
as a separate step. SCBench's `eval` exits 1 with "No problems to evaluate"
when everything is already evaluated, so splitting the two gives exactly one
of each per arm. It also means a failed evaluation can be re-run without
paying for inference again. From the cached SCBench checkout:

```bash
cd bench/scbench/cache/slop-code-bench
SCBENCH_PROBLEMS_PATH="$PWD/../scb-problems" SCBENCH_HOME="$PWD/../scbench-home" \
  uv run slop-code eval <out>/<arm>/output --overwrite
```

## Where output lands

```
bench/scbench/
  cache/                      # git-ignored
    slop-code-bench/          # the SCBench checkout, at the pin
    scb-problems/             # the problem checkout (SCBENCH_PROBLEMS_PATH)
    scbench-home/             # SCBench's own cache (SCBENCH_HOME)
  runs/<name>/                # --out; git-ignored under bench/scbench/runs
    rdm-scbench-run.json      # the run record
    <arm>/
      agent.yaml              # the generated agent config
      run.yaml                # the generated run config
      output/                 # SCBench's own run directory, layout untouched
```

The run record `rdm-scbench-run.json` sits beside the arm directories, never
inside SCBench's `output/` trees. It holds:

- every manifest pin, and the manifest path
- the rdm SHA
- the observed `uv`, Docker and `claude` versions
- the model file's source, installed path and SHA-256
- each arm's generated configs and run directory
- every command spawned, with its working directory, `SCBENCH_*`
  environment, exit code and timestamps

It is rewritten after every command, so a failed run still records the
command that failed.

## Cost figures under a subscription

Under `claude_code_oauth`, the cost SCBench reports is the token usage
multiplied by the model file's pricing. It is not an invoice.
