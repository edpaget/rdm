# SCBench control-arm runner

`rdm-measure scbench run` drives [SlopCodeBench](https://github.com/SprocketLab/slop-code-bench)
(SCBench) from the pinned manifest in this directory. Each manifest arm gets
one `slop-code run` and one `slop-code eval`. Two arms are upstream's own
control prompts, `just-solve` and `anti_slop`. The `full-lane` arm (see
"Full-lane arm") runs the full rdm lane and is measured against them.

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
      docker.j2               # rdm arms only: the rendered agent template
      prompt.jinja            # prompt_from = "manifest" arms: the rendered prompt
      run.yaml                # the generated run config
      output/                 # SCBench's own run directory, layout untouched
      plan-repos/<problem>/<checkpoint>/  # rdm arms only: each checkpoint's plan repo
```

The run record `rdm-scbench-run.json` sits beside the arm directories, never
inside SCBench's `output/` trees. It holds:

- every manifest pin, and the manifest path
- the rdm SHA
- the observed `uv`, Docker and `claude` versions
- the model file's source, installed path and SHA-256
- each arm's generated configs and run directory
- for an rdm arm, each checkpoint's copied plan repo and lane
  classification (`lane`, see "Full-lane arm")
- every command spawned, with its working directory, `SCBENCH_*`
  environment, exit code and timestamps

It is rewritten after every command, so a failed run still records the
command that failed.

## rdm-equipped image

An arm runs in the rdm-equipped agent image when it sets `image = "rdm"`:

```toml
[[arm]]
name = "just-solve-rdm"
prompt = "configs/prompts/just-solve.jinja"
image = "rdm"
```

Arms without `image` run in SCBench's stock `claude_code` image. An rdm arm
needs the manifest's `[rdm]` table, which names the plan repo's path in the
container (`plan_root`) and its default project.

Build the image and check it before the first run with an rdm arm, and again
after every commit:

```bash
cargo run -q -p rdm-devtools --bin rdm-measure -- scbench image build
cargo run -q -p rdm-devtools --bin rdm-measure -- scbench image check
```

Neither makes a paid call, but `build` is slow: it runs a release build of
rdm and builds SCBench's base image. Both take `--manifest`, `--cache-dir` and
`--docker-bin`; `build` also takes `--uv-bin`.

`image build`:

1. refuses a dirty checkout, because the build context is the working tree
   and the image would otherwise claim a HEAD it was not built from
2. provisions the pinned SCBench checkout, and refuses a `plan_root` inside
   the environment's graded directory
3. builds `agent/Dockerfile` with the checkout as context: a
   `rust:1.94-bookworm` stage builds `rdm` for the Docker daemon's own
   platform, and the result is the payload image
   `rdm-scbench-payload:<HEAD>`, labelled `org.rdm.scbench.rdm-sha=<HEAD>`.
   It holds `/opt/rdm/bin/rdm`, the `claude-rdm` wrapper, a copy of the
   checkout's `plugins/rdm` and `/opt/rdm/RDM_SHA`
4. writes the rendered `agent/docker.j2` and the rdm agent config under
   `cache/rdm-image/<HEAD>/`
5. runs `slop-code docker build-agent ... --force-build`, which builds exactly
   the agent image a run uses

BuildKit is required, for the cache mounts and `Dockerfile.dockerignore`. It
is the default from Docker 23.

`image check` runs three probes in the agent image. The SHA recorded in
`/opt/rdm/RDM_SHA` must equal the checkout's HEAD. `rdm --version` must run.
And `claude --plugin-dir /opt/rdm/plugin plugin details rdm` must list every
skill in the checkout's `plugins/rdm/skills`. It prints each failure and exits
1 when any probe failed.

Some choices follow from how the pinned SCBench works:

- **Why `--plugin-dir` instead of `claude plugin install`.** SCBench mounts a
  fresh host directory over `~/.claude` at run time, which hides anything
  `plugin install` put there. The image carries the checkout's `plugins/rdm`
  byte for byte, and the rdm agent config passes
  `--plugin-dir /opt/rdm/plugin`. That loads the same skills a marketplace
  install provides.
- **Why `version` carries `-rdm.<sha>`.** SCBench names its agent image
  `slop-code:claude_code-<version>-<environment>` and reuses an image of that
  name without rebuilding it. The rdm agent config's `version` is
  `<claude_code_version>-rdm.<HEAD>`, so an rdm image never shares a name with
  the stock image, and each commit gets its own. The template installs the
  manifest's `claude_code_version` directly, so both images run the same
  Claude Code.
- **Where the plan repo lives.** At `plan_root` (`/rdm-plan`), created empty in
  the image. SCBench grades only the environment's `docker.workdir`
  (`/workspace`), so plan-repo files never count toward the metrics; the
  runner refuses a `plan_root` inside it and records the graded directory in
  each rdm arm's run record. The agent binary, `claude-rdm`, runs `rdm init`
  there when no plan repo exists yet, then `claude`. Each problem runs in its
  own container, so each problem starts with a fresh plan repo that its later
  checkpoints reuse.

`image check` passing in the hermetic tests shows only that rdm builds the
planned commands. The first real `image build` and `image check` are what
confirm the Dockerfile builds, the payload copies in, and `claude` lists the
plugin inside the container.

## Full-lane arm

The `full-lane` arm is the full rdm development lane:

```toml
[[arm]]
name = "full-lane"
prompt = "prompts/full-lane.jinja"   # relative to this manifest
prompt_from = "manifest"
image = "rdm"
step_limit = 1000
cost_limit = 0
net_cost_limit = 0
```

**Checkpoints become phases.** Each problem gets one roadmap, `solution`, in
its own plan repo. Each checkpoint becomes one phase of it, and the
checkpoint's spec is the phase body, followed by two generic acceptance
criteria. SCBench passes the template only `spec` and `is_continuation`, and
no problem or checkpoint name. So the first checkpoint
(`is_continuation` false) creates the roadmap and phase 1. Each continuation
reads `rdm phase list` and appends the next number.

**Two entrypoints.** The agent drives each phase with the shipped plugin's
`rdm:dispatch-phase solution <N> --project bench`, unattended: plan, plan
review, implement, verify, code review and triage. When that ends
`reviewed`, the agent runs `rdm:land solution/<stem>`. `rdm:autopilot` is not
used, because the shipped plugin's autopilot has no `--land` flag. The prompt
tells the agent never to write code itself, and to stop when either
entrypoint does not succeed.

**The graded checkout.** The agent's working directory (`/workspace`) is the
program's primary git checkout. `rdm:land` fast-forwards its `main`, and
SCBench grades that directory, so each checkpoint ends with the landed
result. There is no bench-side copy or merge. rdm puts the item worktrees at
`/workspace__worktrees/`, which the image creates beside the graded
directory. The graded tree also gains `.git/`, `.gitignore` and `CLAUDE.md`.
None of these is Python, so the erosion metrics never read them.

**Checks.** On the first checkpoint the agent sets
`rdm config set dispatch.verify "<workspace>/.venv/bin/python -m pytest -q"`
over its own tests. It also writes the same command under `## Checks` in
`CLAUDE.md`, because `rdm:land` aborts when none of its discovery sources
names a check. The command uses the primary checkout's absolute venv path,
because a worktree has no `.venv`. The agent never sees SCBench's hidden
tests.

**Limits.** An arm's `step_limit`, `cost_limit` and `net_cost_limit` override
`[agent]`'s for that arm only, and an arm that omits one inherits it. The
lane's main session needs far more than the controls' 100 turns. `0` makes
cost unlimited, and 1000 steps is a finite backstop against a runaway loop.
Size these from the first live run.

**Plan-repo copy.** The plan repo lives at `plan_root`, outside the graded
directory, and the container is removed after each problem. So after every
`claude` call, the wrapper tars the plan repo (working tree and `.git`) to
`$HOME/.claude/projects/-workspace/rdm-plan/<n>.tar`, with `<n>` counting up
from `000001`. SCBench copies every new file under the Claude trace directory
`projects/-workspace/` into the checkpoint's
`output/<problem>/<checkpoint>/agent/workspace/`. This relies on the pinned
SCBench's trace export (`traces.copy_new_trace_files`); it is not a
documented SCBench interface. After the arm's `slop-code run` and before its
`eval`, the runner extracts each checkpoint's latest tar with `tar -xf` to
`<out>/<arm>/plan-repos/<problem>/<checkpoint>/`. That is next to SCBench's
`output/`, never inside it. A problem's last checkpoint copy is its final
plan repo.

**Classification.** The runner reads each copy through rdm-core, as
`rdm phase list` does. A checkpoint's phase is the one that is new since the
previous checkpoint. The runner writes each checkpoint into the run record's
`lane`, and prints `<landed>/<total> checkpoints landed`:

| Observed | `outcome` | `reason` | `detail` |
|---|---|---|---|
| the phase is `done` | `landed` | | |
| no new phase | `lane-failure` | `phase-missing` | |
| `blocked` | `lane-failure` | `parked` | the phase's blocked reason |
| `reviewed` | `lane-failure` | `escalated` | `rdm:land` escalated or did not run |
| `not-started` / `in-progress` / `needs-review` | `lane-failure` | `unfinished` | the status |
| `wont-fix` | `lane-failure` | `abandoned` | |
| no export in the checkpoint | `lane-failure` | `plan-repo-missing` | |
| the plan repo cannot be read | `lane-failure` | `plan-repo-unreadable` | rdm-core's error |

This separates "the lane produced worse code" from "the lane did not
finish". The operator files each lane failure a live run exposes as an rdm
task tagged `benchmark`. The fix goes into rdm proper, never into bench
config.

The hermetic tests cover the rendered prompt, the arm's invocation, the copy
and the classification, all against fakes and fixture plan repos. Only a live
run confirms the rest:

- that Claude Code loads the plugin's skills under `--print` in the container
- that the trace export copies the tars
- that `rdm worktree add` and `rdm:land` work at `/workspace`
- that 1000 steps is enough

## Cost figures under a subscription

Under `claude_code_oauth`, the cost SCBench reports is the token usage
multiplied by the model file's pricing. It is not an invoice.
