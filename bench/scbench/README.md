# SCBench control-arm runner

`rdm-measure scbench run` drives [SlopCodeBench](https://github.com/SprocketLab/slop-code-bench)
(SCBench) from the pinned manifest in this directory. Each selected manifest
arm gets one `slop-code run` and one `slop-code eval` per seed. The arms are the ablation
ladder described in [`ladder.md`](ladder.md). Four arms are upstream's own
control prompts: `just-solve`, `anti_slop`, `plan_first` and `plan-and-test`.
The `tracked` and `planned` arms add rdm tracking, then rdm's planning lane.
The `full-lane` arm (see "Full-lane arm") runs the full rdm lane.

**Benchmark guide.** [`docs/scbench-benchmark.md`](../../docs/scbench-benchmark.md)
is the operator's guide: prerequisites, how cost scales, triggering
`scbench ladder` on the roadmap branch, reading the report and recording a
baseline. This README is the reference for the runner itself.

> **COST WARNING.** `scbench run` without `--dry-run` runs paid Claude Code
> agents in Docker. Each arm runs every problem in the subset, and every
> checkpoint of each problem, once per seed. Cost scales with
> arms × seeds × problems × checkpoints, and a run takes hours. Use
> `--dry-run` first: it prints the projected run count.

## What is pinned

`manifest.toml` holds every value a result depends on:

- the SCBench commit and the `scb-problems` commit
- the provider and the model (Claude Opus 5.5)
- the thinking level and the Claude Code version installed in the agent image
- the pass policy, the environment and the cost limits
- the problem subset, each problem's checkpoint count
  (`[problems] checkpoints`, from its `config.yaml` at the pin), and the arms

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
    Or keep it in 1Password and run every subcommand through
    `bench/scbench/run.sh <subcommand> ...`, which reads it with `op read`
    from `op://rdm-dev/claude-token/credential` (override with
    `SCBENCH_CLAUDE_TOKEN_REF`) into the runner's environment only, then
    `exec`s the built `rdm-measure`. The token still reaches disk in
    SlopCodeBench's logs; see "Credential scrub" below.
  - **API key**: set `provider = "anthropic"` in the manifest and export
    `ANTHROPIC_API_KEY`. Do not pair `anthropic` with
    `CLAUDE_CODE_OAUTH_TOKEN`; SCBench would pass the token as an API key.
- The host `claude` CLI. The runner records its version. The agent image
  installs the manifest's `claude_code_version` itself.

### Credential scrub

SlopCodeBench logs the whole `docker exec` line, credential included, into
each problem's `infer.log`, and the pinned checkout is not patched. So after
every `slop-code` call the runner makes (`run`, `eval`, and `build-agent`
under `scbench image build`), whatever its exit status, it reads the
provider's credential (`CLAUDE_CODE_OAUTH_TOKEN` or `ANTHROPIC_API_KEY`) from
its own environment and rewrites every regular file under that call's
directory: `<out>/<arm>/seed-<n>/` for a run or eval, `<cache>/scbench-home`
and `<cache>/rdm-image/<sha>/` for an image build. Each occurrence becomes
`***redacted***`. Gzip files are decoded, scrubbed and re-encoded; tar
archives (the exported plan repos) are masked at the value's length, so they
still extract. Symlinks are not followed, and an unset or empty credential
changes nothing.

Not covered: a value inside a zlib-compressed git object within a
plan-repo tar. A `cannot scrub the credential from <path>` line on stderr means that
file could not be read or rewritten and may still hold the credential:
delete it by hand.

## Running

The one trigger is `scbench ladder`: a preflight that fails before any
spend (the credential variable, `uv`, the Docker daemon and, when an arm
runs in the rdm image, `scbench image check`), then `scbench run` over every
manifest arm with the given seeds, then `scbench report`:

```bash
# The projected run count, the preflight it would run, and the plan.
cargo run -q -p rdm-devtools --bin rdm-measure -- scbench ladder \
  --seeds 3 --out bench/scbench/runs/<name> --dry-run

# The paid run.
cargo run -q -p rdm-devtools --bin rdm-measure -- scbench ladder \
  --seeds 3 --out bench/scbench/runs/<name> [--baseline <name>]
```

`--seeds` is required. A preflight failure prints
`error: ladder preflight: ...` before any `slop-code` command and before
`--out` is created. The guide lists each failure and its fix.

`scbench run` runs selected arms directly. From the rdm checkout:

```bash
# Print the plan: provisioning, probes, model copy, and each arm's configs
# and commands. Spawns nothing and creates nothing.
cargo run -q -p rdm-devtools --bin rdm-measure -- scbench run \
  --out bench/scbench/runs/<name> --dry-run

# The paid run: every arm, one seed.
cargo run -q -p rdm-devtools --bin rdm-measure -- scbench run \
  --out bench/scbench/runs/<name>

# Two arms, three seeds each.
cargo run -q -p rdm-devtools --bin rdm-measure -- scbench run \
  --arms just-solve,full-lane --seeds 3 --out bench/scbench/runs/<name>
```

`--out` is required and must be missing or empty. The runner refuses a used
directory, so a finished run is never overwritten.

Other flags:

- `--arms NAME[,NAME...]` (repeatable) runs only those manifest arms, always
  in manifest order whatever order they are listed in; an unknown name is
  refused before anything runs. The default is every arm.
- `--seeds N` runs each selected arm with seeds 1 to N (default 1). The seed
  reaches SCBench as its global `--seed` option:
  `slop-code --seed <n> run ...` and `slop-code --seed <n> eval ...`.
- `--manifest PATH` (default `bench/scbench/manifest.toml`)
- `--cache-dir DIR` (default `bench/scbench/cache`)
- `--uv-bin`, `--docker-bin`, `--claude-bin` to choose the executables

The runner:

1. resolves the rdm SHA from the checkout holding the manifest
2. probes the `uv`, Docker and `claude` versions
3. clones or fetches both pinned repositories under `--cache-dir` and checks
   out each pin
4. copies the model file into the SCBench checkout
5. for seed 1, runs each arm in manifest order, then does the same for seed
   2, and so on; an interrupted run's finished seeds therefore cover every arm

`--dry-run` also prints `seeds`, `selected_arms` and `projection`: the arms,
seeds, per-problem checkpoint counts, `sessions` (arms × seeds × problems,
one `slop-code` problem run each) and `checkpoint_runs`
(arms × seeds × checkpoints).

It stops at the first failing step with a non-zero exit. A unit failure is
reported as `error: arm <name> seed <n>: <run|eval> exited <code>`. Units that
already finished stay on disk.

### Stopping a run

**How to stop.** Press Ctrl-C in the run's terminal, or send `kill -INT` or
`kill -TERM` to the `rdm-measure` pid (under `run.sh` that is `run.sh`'s own
pid, since it `exec`s the runner). The same applies to `scbench ladder`. Do
not use `kill -9`, and do not signal `slop-code` itself: only the runner
knows what to clean up.

**What happens.** Each `slop-code` command runs in its own process group,
with `TMPDIR` set to a fresh scratch directory. On SIGINT, SIGTERM or SIGHUP
the runner:

1. forwards the same signal to `slop-code`'s whole process group (SCBench's
   worker processes included);
2. waits up to 10 s (`--stop-grace-secs N`; `0` kills at once), then kills the
   group;
3. removes the Docker containers that mount this run's scratch directory,
   and only those: every container SCBench starts bind-mounts directories
   under it, so another run's containers, even from the same images, are
   left alone;
4. scrubs the unit's `seed-<n>` directory of the credential, as after every
   `slop-code` call;
5. exits 1 with
   `error: arm <name> seed <n>: interrupted by <SIG> during <run|lane|eval> (problems ...)`,
   saying whether `slop-code` exited on the signal or was killed after the
   grace, and how many containers were removed.

A signal between steps (probes, provisioning, the lane's copy of the plan
repos) starts nothing more and is reported the same way.

**What is left behind.** The partial `<out>/<arm>/seed-<n>/`, scrubbed; an
`interrupted` entry on that unit in `rdm-scbench-run.json` (signal, step,
time, whether it was killed, the removed container ids); and in
`scbench report --run <out>` a skipped-run line
`not finished: interrupted by <SIG> during <step>`. Finished units report as
usual. `--out` cannot be reused for a new run; choose a fresh directory.

**What is not covered.** `kill -9` of the runner (nothing can clean up); a
stop during `ladder`'s `image check` container; and a container whose mounts
do not lie under the scratch directory. When the runner cannot list, inspect
or remove containers, or none mounted the scratch directory, it says so on
stderr: check `docker ps` for SlopCodeBench containers.

### Run, then evaluate

Each arm runs `slop-code run --no-evaluate`, then `slop-code eval <run_dir>`
as a separate step. SCBench's `eval` exits 1 with "No problems to evaluate"
when everything is already evaluated, so splitting the two gives exactly one
of each per arm. It also means a failed evaluation can be re-run without
paying for inference again. From the cached SCBench checkout:

```bash
cd bench/scbench/cache/slop-code-bench
SCBENCH_PROBLEMS_PATH="$PWD/../scb-problems" SCBENCH_HOME="$PWD/../scbench-home" \
  uv run slop-code --seed <n> eval <out>/<arm>/seed-<n>/output --overwrite
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
      agent.yaml              # the generated agent config, shared by the seeds
      docker.j2               # rdm arms only: the rendered agent template
      prompt.jinja            # prompt_from = "manifest" arms: the rendered prompt
      seed-<n>/               # one per seed
        run.yaml              # the generated run config (save_dir is seed-<n>)
        output/               # SCBench's own run directory, layout untouched
        plan-repos/<problem>/<checkpoint>/  # rdm arms only: each checkpoint's plan repo
```

The run record `rdm-scbench-run.json` sits beside the arm directories, never
inside SCBench's `output/` trees. It holds:

- `schema: 2`, `seeds` (`[1, ..., n]`) and `selected_arms`
- every manifest pin, and the manifest path
- the rdm SHA
- the observed `uv`, Docker and `claude` versions
- the model file's source, installed path and SHA-256
- one entry per unit (arm and seed) in run order, with its `seed`, `group`,
  generated configs and run directory, and `finished`: the time its eval
  exited 0, `null` until then
- for an rdm arm, each checkpoint's copied plan repo and lane
  classification (`lane`, see "Full-lane arm")
- every command spawned, with its working directory, `SCBENCH_*`
  environment, exit code and timestamps

It is rewritten after every command, so a failed run still records the
command that failed.

## Reporting a run

```bash
cargo run -q -p rdm-devtools --bin rdm-measure -- scbench report \
  --run bench/scbench/runs/<name> [--format markdown|json] \
  [--save-baseline <name>] [--baseline <name>]
```

`scbench report` makes no call. It reads the run record and each finished
unit's SCBench checkpoint tree (`evaluation.json`,
`quality_analysis/overall_quality.json` and `inference_result.json` per
checkpoint, plus the token totals of the final `result` event in
`agent/stdout.jsonl` when the checkpoint has one), then writes `rdm-scbench-report.json` and
`rdm-scbench-report.md` beside the run record and prints the chosen format
(default markdown). Per arm it gives the checkpoint pass rate (under the
manifest's `pass_policy`), the erosion metrics at the last checkpoint and
their change since the first, every erosion metric per checkpoint with its
change from the previous one, cost, tokens, agent wall-clock and the
lane-failure count. Each figure carries a 95% bootstrap interval over seeds.
It also gives every `rdm` arm's difference from every `control` arm, with an
interval. A unit whose eval never finished is listed under `skipped_runs`
rather than counted. A phase 1 to 4 run record (one seed, no `schema`) is
still read.

`--save-baseline <name>` also writes `baselines/<name>.json` beside the
run's manifest, keyed by five pins: the rdm, SCBench and `scb-problems` SHAs,
the model and the Claude Code version. It is refused when a pin is missing
or the name already exists. `--baseline <name>` renders the run beside that
baseline and flags any non-rdm pin that differs. Neither changes the exit
code.

## rdm-equipped image

An arm runs in the rdm-equipped agent image when it sets `image = "rdm"`:

```toml
[[arm]]
name = "just-solve-rdm"
group = "control"
prompt = "configs/prompts/just-solve.jinja"
image = "rdm"
```

This arm is a control even though it runs in the rdm image, because its
prompt never uses rdm; without `group` it would default to `rdm`.

Arms without `image` run in SCBench's stock `claude_code` image. An rdm arm
needs the manifest's `[rdm]` table, which names the plan repo's path in the
container (`plan_root`) and its default project.

Build the image and check it before the first run with an rdm arm, and again
after every commit. The wrapper `agent/claude-rdm.sh` is part of the image,
so any change to it changes the rdm SHA: run `scbench image build` again
before the next rdm-arm run, because `image check` refuses the stale image:

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
  there when no plan repo exists yet, then `claude`, resuming it until the
  checkpoint settles (see "Full-lane arm" § "Headless resumes"). The wrapper
  never writes stdout itself: stdout carries only `claude`'s stream, and the
  wrapper's and rdm's own output go to stderr. Each problem runs in its
  own container, so each problem starts with a fresh plan repo that its later
  checkpoints reuse.

`image check` passing in the hermetic tests shows only that rdm builds the
planned commands. The first real `image build` and `image check` are what
confirm the Dockerfile builds, the payload copies in, and `claude` lists the
plugin inside the container.

## Ablation ladder

Each arm carries a `group`, `rdm` or `control`, which says whether it is an
rdm treatment. The report compares runs by `group`. An arm without `group`
takes it from `image`: `rdm` for the rdm image, `control` for the stock one.
`group = "rdm"` on a stock-image arm is refused. The committed manifest sets
`group` on every arm. The dry-run plan and the run record carry each arm's
resolved `group`. [`ladder.md`](ladder.md) lists every rung, the shipped
entrypoints it uses, what each comparison attributes, and the rungs that were
dropped and why.

## Full-lane arm

The `full-lane` arm is the full rdm development lane:

```toml
[[arm]]
name = "full-lane"
group = "rdm"
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

**Headless resumes.** SCBench runs `claude --print` once per checkpoint, and
the lane backgrounds subagents and Workflows. Claude Code's print mode stops
background tasks 10 minutes after the last turn, which ended every full-lane
checkpoint of the first smoke run with nothing committed, reviewed or
landed. The wrapper handles this for every rdm arm (`tracked`, `planned`,
`full-lane`); the controls keep upstream's configuration.

- **The ceiling.** The wrapper exports
  `CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS=7200000` (2 h) unless it is already
  set. It is finite, not `0` ("wait indefinitely"), so a background shell
  that never exits cannot hold a checkpoint forever.
- **The settle rule.** After every `claude` exit the wrapper reads
  `rdm phase list --roadmap solution --format json`, through rdm and never
  the files. The checkpoint's phase is the highest-numbered phase of
  `$RDM_ROADMAP` (`solution`, from the agent config's `env`), numbered above
  the highest phase before the checkpoint started. That baseline is kept
  under the container's `$TMPDIR` and reused by SCBench's own `--continue`
  retry. The checkpoint has settled when its phase is `done`, `blocked` or
  `wont-fix`. A full-lane phase left `reviewed` has not settled.
- **The resume.** While the checkpoint has not settled, the wrapper resumes
  the same session: `claude --continue` with every original argument before
  `--` (model, output format, `--max-turns`, permission mode, `--plugin-dir`)
  and a fixed continuation prompt after it. That prompt says the previous
  process's background work was stopped, that this is the same checkpoint
  (so no second phase is created), to check the phase's state and redo any
  step whose result never arrived in the foreground, and never to end a turn
  while a background task is still running. The wrapper exits with the last
  `claude`'s status.
- **The cap.** At most `RDM_RESUME_MAX` resumes per checkpoint attempt,
  default 6: one per lane stage that runs in the background or can be cut
  (planner, plan review, implementer, verify, code review, land). `0`
  disables resuming. The default lives in the wrapper, so the rdm SHA pins
  it. Changing it means editing the wrapper and rebuilding the image.
- **The export.** Every `claude` exit, each resume included, writes a new
  plan-repo tar, so a checkpoint's latest export is its final state.
- **Where the resume lines land.** The wrapper's `claude-rdm: phase <N> is
  <status>; resuming (<k>/<max>)` and `claude-rdm: checkpoint not settled
  after <max> resume(s)` lines go to stderr, which SCBench writes to
  `<checkpoint>/agent/stderr.log`.
- **Usage.** Every resume continues one session, so its final `result`
  carries the session's cumulative `total_cost_usd` and `modelUsage`.
  SCBench keeps the last result's cost and counts every process's assistant
  messages as steps, so the report's `cost_usd` and `steps` cover every
  resume. The report reads tokens from the final result's `modelUsage` in
  `<checkpoint>/agent/stdout.jsonl` (see "Reporting a run").
- **Limits.** `step_limit` reaches Claude Code as `--max-turns`, per `claude`
  process. Each resume keeps it, so an rdm-arm checkpoint can run up to
  `(1 + RDM_RESUME_MAX) × step_limit` main-thread turns (7 × 1000), while a
  control runs one process of 100. SCBench's own retry multiplies both by up
  to 3. The guide documents this as a confound against the controls; the
  report's `steps` and `cost_usd` show the actual spend.

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
- that Claude Code 2.1.322 honours `CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS`,
  that `--continue` resumes the checkpoint's main session in `/workspace`,
  that the resumed process restores the session's cost, that the agent
  follows the continuation prompt, and that 2 h and 6 resumes suit real lane
  stages

## Cost figures under a subscription

Under `claude_code_oauth`, the cost SCBench reports is the token usage
multiplied by the model file's pricing. It is not an invoice.
