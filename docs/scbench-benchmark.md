# The SlopCodeBench maintainability benchmark

This guide is for the operator who runs rdm's maintainability benchmark: the
pinned [SlopCodeBench](https://github.com/SprocketLab/slop-code-bench)
(SCBench) ablation ladder, reported per arm with confidence intervals and
compared against a checked-in baseline. The tooling is `rdm-measure scbench`
in the repository-only `rdm-devtools` crate. Nothing here is part of the
shipped `rdm` binary.

> **COST WARNING.** `scbench ladder` and `scbench run` without `--dry-run`
> run paid Claude Code agents in Docker, for hours. Always run
> `scbench ladder --dry-run` first to read the projected run count.

## What it measures

SCBench gives an agent a multi-checkpoint programming problem, one
specification change at a time, and measures how the code erodes over the
checkpoints, as well as whether it passes. The manifest
(`bench/scbench/manifest.toml`) pins SCBench, the problem set, the model,
the Claude Code version and the arms. The arms form an ablation ladder
([`bench/scbench/ladder.md`](../bench/scbench/ladder.md)), in this order:

- the `control` arms: upstream's own prompts `just-solve`, `anti_slop`,
  `plan_first` and `plan-and-test`, in SCBench's stock image
- the `rdm` arms, in the rdm-equipped image: `tracked` (rdm tracking only),
  `planned` (tracking plus rdm's planning lane with plan review) and
  `full-lane` (the whole rdm lane through `rdm:dispatch-phase` and
  `rdm:land`)

The question the benchmark answers is whether rdm drives less slop: for each
`rdm` arm against each `control` arm, the report gives each metric's
difference with a confidence interval. The ladder attributes the effect to
tracking, to planning, or to the rest of the lane.

## Prerequisites

- **Docker** with the daemon running, and BuildKit (the default from Docker
  23). The preflight probes `docker version --format {{.Server.Version}}`,
  which fails when the daemon is down.
- **[uv](https://docs.astral.sh/uv/)** and **Python 3.12 or later**. SCBench
  runs through `uv run slop-code`.
- **A credential**, in exactly one of two forms. Never set both:
  - **Subscription** (the manifest's `provider = "claude_code_oauth"`): run
    `claude setup-token` and export the token as `CLAUDE_CODE_OAUTH_TOKEN`.
    SlopCodeBench writes the credential into its logs, and the runner scrubs
    it from the run output; see `bench/scbench/README.md` § "Credential
    scrub".
  - **API key**: set `provider = "anthropic"` in the manifest and export
    `ANTHROPIC_API_KEY`. Do not pair `anthropic` with
    `CLAUDE_CODE_OAUTH_TOKEN`: SCBench would pass the token as an API key.
- **The host `claude` CLI.** The run records its version. The agent images
  install the manifest's `claude_code_version` themselves.
- **The rdm-equipped image**, before any rdm arm runs and again after every
  commit, since the image is keyed by the commit:

  ```bash
  cargo run -q -p rdm-devtools --bin rdm-measure -- scbench image build
  cargo run -q -p rdm-devtools --bin rdm-measure -- scbench image check
  ```

  `image build` refuses a dirty checkout. It is slow (a release build of rdm
  and SCBench's base image) but makes no paid call. See
  [`bench/scbench/README.md`](../bench/scbench/README.md) § "rdm-equipped
  image".
- **Disk.** The pinned checkouts and SCBench's own cache live under
  `bench/scbench/cache/`. Each run's output, including every checkpoint's
  snapshot and agent traces and, for rdm arms, each checkpoint's plan repo,
  lives under `--out`. Both are git-ignored.

## How cost scales with the run count

One `slop-code` problem session runs one problem's checkpoints in order.
The counts are:

- **sessions** = arms × seeds × problems
- **checkpoint runs** = arms × seeds × Σ checkpoints over the subset

Agent cost and time scale with checkpoint runs. The per-problem checkpoint
counts come from the manifest's `[problems] checkpoints`, pinned with the
problem set's SHA. The committed manifest has 7 arms and the subset mvvault
(6 checkpoints) and xjq (5), so 11 checkpoints per arm and seed. At 3 seeds
that is 7 × 3 × 11 = **231 checkpoint runs** in 7 × 3 × 2 = **42 problem
sessions**.

`scbench ladder --dry-run` prints exactly these numbers for the manifest and
seed count, and spawns and creates nothing. Use it before every live run.

To estimate from a finished run, read `cost_usd` in the report per arm and
divide by the arm's checkpoints per seed. Under `claude_code_oauth`, the
cost SCBench reports is the token usage multiplied by the model file's
pricing (`bench/scbench/models/opus-5.5.yaml`). It is not an invoice.

## Triggering the ladder on the roadmap branch

`scbench ladder` is the one command: a preflight, then every manifest arm
over the given seeds, then the report.

1. **Check out the branch and keep it clean.** `image build` refuses a dirty
   tree, because the image is built from the working tree and labelled with
   HEAD.
2. **Build and check the image**: `scbench image build`, then
   `scbench image check`.
3. **Project, then run.**

   ```bash
   cargo run -q -p rdm-devtools --bin rdm-measure -- scbench ladder \
     --seeds 3 --out bench/scbench/runs/<name> --dry-run
   cargo run -q -p rdm-devtools --bin rdm-measure -- scbench ladder \
     --seeds 3 --out bench/scbench/runs/<name>
   ```

   `--seeds` is required, because it is the run's cost choice. `--out` must
   be missing or empty. The ladder also takes `--manifest`, `--cache-dir`,
   `--uv-bin`, `--docker-bin`, `--claude-bin` and `--baseline <name>`.

4. **Read a preflight failure.** The preflight runs cheapest first and stops
   at the first failure, before any `slop-code` command and before `--out`
   is created. Each prints `error: ladder preflight: ...` and exits 1:

   | Message | Fix |
   |---|---|
   | `--baseline: cannot read .../baselines/<name>.json` | Check the name, or save it first with `scbench report --save-baseline` |
   | `CLAUDE_CODE_OAUTH_TOKEN is not set; provider claude_code_oauth needs it` | Run `claude setup-token` and export the token |
   | `ANTHROPIC_API_KEY is not set; provider anthropic needs it` | Export the API key |
   | `provider <p> has no known credential variable` | Set the manifest's `provider` to `claude_code_oauth` or `anthropic` |
   | `uv not found (...)` / `uv is not usable (...)` | Install uv, or pass `--uv-bin` |
   | `the Docker daemon is not reachable (...)` | Start Docker, or pass `--docker-bin` |
   | `rdm image check failed (see above)` | The lines above name each failed probe; rebuild with `scbench image build` |
   | `no SCBench environment at ...; run rdm-measure scbench image build first` | Run `scbench image build` |

   Only whether the credential variable is set is checked. Its value is
   never printed or recorded.

5. **If the run fails part-way**, the error names the unit
   (`arm <name> seed <n>: <run|eval> exited <code>`) and ends with a hint.
   The units that finished are on disk, and
   `scbench report --run bench/scbench/runs/<name>` reports them. Units
   whose eval never finished are listed as skipped, not counted as failures.
   Because runs go seed-major (every arm for seed 1, then seed 2), the
   finished seeds cover every arm. If the run finishes but the report step fails, the
   error says so and ends with the same `scbench report --run` hint.
   To stop a run yourself, press Ctrl-C or send `kill -INT`/`kill -TERM` to
   the `rdm-measure` pid; `bench/scbench/README.md` § "Stopping a run" says
   what it cleans up and what it leaves behind.
6. **Re-run into a fresh `--out`.** The report reads one run directory, so a
   re-run is never merged with an earlier one.

## Reading the report

The ladder ends by printing the markdown report. `scbench report --run <dir>`
produces it again for any run, without any call:

```bash
cargo run -q -p rdm-devtools --bin rdm-measure -- scbench report \
  --run bench/scbench/runs/<name> [--format markdown|json]
```

It always writes two files beside the run record:

- `rdm-scbench-report.json`: the structured report, byte-identical for the
  same run
- `rdm-scbench-report.md`: rendered from the JSON, and what is printed by
  default

### Metrics

Each metric is computed per seed, from SCBench's per-checkpoint files
(`evaluation.json`, `quality_analysis/overall_quality.json` and
`inference_result.json`). It is then summarised over seeds.

| Metric | Per seed | Better |
|---|---|---|
| `pass_rate` | checkpoints passing ÷ Σ over problems of the larger of the manifest's and the observed checkpoint count | higher |
| `cc_max_final` | mean over problems of the highest cyclomatic complexity of any function, at the problem's last checkpoint with quality data | lower |
| `cc_high_count_final` | functions with complexity above 10, at the last checkpoint | lower |
| `mi_mean_final` | mean maintainability index per file, at the last checkpoint | higher |
| `max_nesting_depth_final` | deepest nesting in any function, at the last checkpoint | lower |
| `lint_errors_final` | linter findings, at the last checkpoint | lower |
| `single_use_functions_final` | functions called exactly once, at the last checkpoint | lower |
| `trivial_wrappers_final` | functions that only forward to another, at the last checkpoint | lower |
| `sloc_final` | source lines, at the last checkpoint: context for the counts above, with no direction | — |
| `<metric>_change` | the same metric at the last checkpoint minus at the first, averaged over problems: the erosion across checkpoints | as above |
| `cost_usd` | Σ agent cost over checkpoints | lower |
| `tokens_input`, `tokens_output`, `tokens_cache_read`, `tokens_cache_write`, `tokens_total` | Σ net tokens over checkpoints | lower |
| `steps` | Σ agent steps | lower |
| `wall_clock_seconds` | Σ agent wall-clock, excluding evaluation | lower |
| `lane_failures` | checkpoints the rdm lane did not land (rdm-image arms only) | lower |

A checkpoint passes as SCBench decides it: it has an evaluation, the
evaluation had no infrastructure failure, and the manifest's `pass_policy`
holds. A checkpoint that was never evaluated does not pass.

The report also gives every erosion metric **per checkpoint**, with its
change from the previous checkpoint of the same problem and seed. That is
where erosion over a problem's life shows.

### Confidence intervals

Every figure is the mean over seeds with a 95% percentile bootstrap
interval: the seeds are resampled with replacement 10,000 times. One seed
has no spread, so a single-seed figure has no interval. A degenerate
`[x, x]` would read as certainty. Each statistic draws from its own seeded
generator, so adding an arm or a metric never moves another interval, and a
report is reproducible.

### rdm against control

For every `rdm` arm against every `control` arm (each arm's `group` in the
manifest), the report gives each metric's difference, `rdm − control`, with
an unpaired bootstrap interval: each arm's seeds are resampled
independently. It is unpaired because seed `k` of two arms are independent
agent sessions. SCBench's `--seed` does not fix Claude's sampling.

A difference whose interval excludes 0 is marked `*` (`excludes_zero` in the
JSON). That is a reading aid, not a significance test. The table holds about
two dozen metrics per pair and several pairs, so some intervals will exclude
0 by chance. Read a `*` as a place to look, and confirm it with more seeds.

### Lane failures

For an rdm arm, `lane_failures` counts the checkpoints whose phase did not
reach `done`. `lane_failure_reasons` counts them per reason (`parked`,
`escalated`, `phase-missing`, `unfinished`, `abandoned`, `plan-repo-missing`,
`plan-repo-unreadable`), summed over seeds. See
[`bench/scbench/README.md`](../bench/scbench/README.md) § "Full-lane arm" for
each reason. A lane failure separates "the lane produced worse code" from
"the lane did not finish". File each one a live run exposes as an rdm task
tagged `benchmark`, and fix it in rdm proper, never in bench config.

## Recording a baseline

A baseline is a run's report saved under a name, keyed by five pins:

- the rdm SHA (the checkout under test)
- the SCBench SHA
- the `scb-problems` SHA
- the model
- the Claude Code version

```bash
cargo run -q -p rdm-devtools --bin rdm-measure -- scbench report \
  --run bench/scbench/runs/<name> --save-baseline <baseline>
```

This writes `bench/scbench/baselines/<baseline>.json` beside the run's
manifest. It holds the five pins, the run's report with every per-seed
sample, the source run and the host `claude --version` the run observed.
The name is an operator label matching `[A-Za-z0-9][A-Za-z0-9._-]*`. Saving
is refused when a pin is missing (for a run that failed before resolving the
rdm SHA, say) or when the name already exists: delete an old baseline
deliberately to replace it. Commit the file, so a later commit can be
compared with it.

To compare a later run, pass the name to `report` or to `ladder`:

```bash
cargo run -q -p rdm-devtools --bin rdm-measure -- scbench report \
  --run bench/scbench/runs/<later> --baseline <baseline>
```

The report gains a baseline section. Each arm's current and baseline figures
are shown side by side, with the difference and its interval. The rdm SHA is
expected to differ, since it is the treatment. Any other pin that differs is
flagged as **not comparable**, with both values: the runs measured different
SCBench code, problems, models or Claude Code. Re-baseline then, rather than
reading the difference. Arms present in only one run are listed.

A baseline comparison never gates anything and never changes the exit code.

SCBench's optional LLM judge (`slop-code metrics judge`) is not part of the
report. It adds cost and judge variance on top of the deterministic metrics.

## What only a live run confirms

The tooling is tested hermetically, against fake `uv`, `docker` and `claude`
executables and fixture SCBench output trees built from SCBench's source.
Only the operator's first live ladder confirms:

- that SCBench at the pinned commit writes these files, at these paths, with
  these fields
- that SCBench accepts `--seed` before the subcommand
  (`slop-code --seed <n> run ...`)
- that the committed checkpoint counts match a provisioned problem checkout
- that the rdm image builds, and the lane runs and exports its plan repos in
  the container (see the README's "Full-lane arm")
- the real cost and timing
