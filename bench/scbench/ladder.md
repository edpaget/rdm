# SCBench ablation ladder

The arms in `manifest.toml` form an ablation ladder. Each rung adds one part
of rdm's development lane to the rung below it, so a difference between
adjacent rungs can be attributed to that part. The full lane's effect is
split into tracking, planning with plan review, and the rest of the lane
(the implementer subagent, the verify gate, code review and `rdm:land`).

Every rung is built only from shipped entrypoints: SCBench's own stock
prompts, the shipped `rdm` CLI, and the shipped plugin's skills. No rung uses
a bench-only skill, workflow or flag. A rung that would need a knob rdm does
not ship is either built as a separate rdm feature task first, or dropped. The
dropped rungs are listed below.

## The ladder

Arms run in this order.

| arm | `group` | image | prompt | shipped entrypoints | limits (step / cost / net cost) |
|---|---|---|---|---|---|
| `just-solve` | `control` | stock | upstream `configs/prompts/just-solve.jinja` | none | `[agent]`: 100 / 0 / 0 |
| `anti_slop` | `control` | stock | upstream `configs/prompts/anti_slop.jinja` | none | `[agent]` |
| `plan_first` | `control` | stock | upstream `configs/prompts/plan_first.jinja` | none | `[agent]` |
| `plan-and-test` | `control` | stock | upstream `configs/prompts/plan-and-test.jinja` | none | `[agent]` |
| `tracked` | `rdm` | rdm | `prompts/tracked.jinja` | rdm CLI: `config set`, `roadmap create`, `phase create`, `phase list`, `phase show`, `phase update --status done`, `commit` | 1000 / 0 / 0 |
| `planned` | `rdm` | rdm | `prompts/planned.jinja` | the `tracked` CLI commands, plus `rdm:dispatch-phase --plan-only`, `rdm plan list` / `plan show` and `rdm worktree remove` | 1000 / 0 / 0 |
| `full-lane` | `rdm` | rdm | `prompts/full-lane.jinja` | the `tracked` setup commands, plus `rdm:dispatch-phase` and `rdm:land` | 1000 / 0 / 0 |

`plan_first` and `plan-and-test` are stock prompts shipped by the pinned
SCBench (`af5a9cbc`). They use the same template contract as `just-solve` and
`anti_slop`. `plan-and-test` tells the agent to save its plan as
`<PART N>.md` in the working directory.

The three rdm rungs share their setup word for word. On the first
checkpoint, each one runs `git init`, creates `.venv`, writes `CLAUDE.md`
with a `## Checks` command, sets `dispatch.verify` and creates the roadmap
`solution`. Each checkpoint then becomes one phase, created from the
checkpoint's spec plus the same two acceptance criteria. So the plan repo
and every phase body are the same across the rdm rungs. Only the way phase
N is driven differs:

- **`tracked`**: the agent reads the phase, implements it on `main`, commits,
  and marks it `done` with `rdm phase update N --status done --commit <sha>`.
  There is no plan, review or land stage.
- **`planned`**: the agent runs `rdm:dispatch-phase solution N --plan-only`,
  which plans the phase and plan-reviews it and implements nothing. It then
  reads the approved plan with `rdm plan list` and `rdm plan show`, and runs
  `rdm worktree remove solution --delete-branch`. After that it implements
  the plan on `main` exactly as `tracked` does, commits, and marks the phase
  `done`. The worktree has to go because a plan-only pass leaves the roadmap
  worktree in place at the `main` it was created from. Removing it means the
  next checkpoint plans against the current `main`. The remove succeeds
  because the worktree is clean and its branch has no commits of its own.
- **`full-lane`**: `rdm:dispatch-phase` plans, plan-reviews, implements,
  verifies and code-reviews the phase. Then `rdm:land` lands it on `main`.
  See README.md § "Full-lane arm".

A `done` write needs no gate. rdm's `reviewed` gate is opt-in, and the bench
plan repo does not enable it. `done` is never gated.

## What each comparison attributes

- **control → `tracked`**: the tracking structure. That covers one roadmap,
  one phase per checkpoint with acceptance criteria, a `CLAUDE.md` checks
  command, and committing on `main`.
- **`tracked` → `planned`**: rdm's planner and its plan review. Both rungs
  implement in the main session on `main`. `planned` adds only the plan-only
  pass and the worktree cleanup.
- **`just-solve` → `plan_first` / `plan-and-test`**: any planning at all,
  using upstream's own prompts. Compare this with `tracked` → `planned` to
  separate the value of "planning" from the value of rdm's reviewed plan.
- **`planned` → `full-lane`**: the rest of the lane, which is the implementer
  subagent, the verify gate, code review with triage, and `rdm:land`'s
  checks. Both rungs run the same `dispatch-phase` planner and plan review.

Confounds to keep in mind:

- `planned` implements in the main session. `full-lane` implements in a
  subagent, in its own worktree. So `planned` → `full-lane` also measures the
  change of implementer context.
- The rdm rungs run with 1000 steps and the controls with SCBench's 100. A
  limit is a backstop, not a treatment. The rdm rungs' extra rdm commands,
  and `planned`'s whole plan-only pass, run in the main session, so a
  truncation would read as `unfinished`. The operator resizes every limit
  from the first live run.
- The rdm rungs' graded tree also holds `.git/`, `.gitignore` and
  `CLAUDE.md`. `plan-and-test`'s graded tree holds its `<PART N>.md` files.
  None of these is Python, so the erosion metrics never read them.

## How checkpoints are classified

Every `image = "rdm"` arm gets the plan-repo copy and the classification in
README.md § "Full-lane arm". The classifier has no arm-specific code. A
checkpoint is `landed` when its new phase reached `done`. For `tracked` and
`planned`, that is the agent's own `done` write after committing to `main`.
For `full-lane`, it is `rdm:land`'s. The other states map the same way for
every rung: a `planned` plan-only pass that parks the phase is `parked`, a
session cut off at the step limit is `unfinished`, and a checkpoint with no
new phase is `phase-missing`.

## The `group` tag

`group` is the treatment, and `image` is the environment. The benchmark
report compares runs by `group`. When an arm omits `group`, it follows `image`:
`rdm` for the rdm image, `control` for the stock one. `group = "rdm"` on a
stock-image arm is refused, because an rdm treatment needs rdm installed.

## Dropped rungs

- **Plan without plan review.** No shipped entrypoint produces an unreviewed
  plan unattended. `dispatch-phase` always runs the plan review, and its
  `--interactive` mode waits for a human approval that SCBench's unattended
  session cannot give. An agent hand-writing `rdm plan create` would not be
  rdm's planner. That rung is roughly `plan_first` plus tracking, which the
  ladder already measures. A "skip the plan review" knob would weaken the
  lane's gate for users only to serve the benchmark, so no task is filed.
- **`planned` plus code review through `rdm:review`, then `rdm:land`.**
  `rdm:review` is interactive: it presents its report for discussion. Its
  unattended form is `dispatch-phase`, which is the full lane. `full-lane`
  already measures review.
- **`rdm:autopilot`.** It has no `--land` flag, so nothing would reach the
  graded `main`. Over a roadmap driven one phase at a time, it dispatches
  exactly what `dispatch-phase` does. It would be `full-lane` without the
  land.
- **`rdm:do --auto`.** It is a shim onto `dispatch-phase`, so it is
  identical to `full-lane`.
- **An image-only control** (the `just-solve` prompt in the rdm image). The
  manifest can express it today, but it is not in the ladder. It would cost
  a full arm to measure only whether having rdm installed changes anything
  when the prompt never mentions it. To add it, append:

  ```toml
  [[arm]]
  name = "just-solve-rdm"
  group = "control"
  prompt = "configs/prompts/just-solve.jinja"
  image = "rdm"
  ```

## What only a live run confirms

The hermetic tests cover each arm's invocation and the rendered rung
prompts. Only a live run shows:

- that the agent follows each rung's prompt
- that `dispatch-phase --plan-only` completes unattended under `--print`
- that `rdm worktree remove` succeeds after a plan-only pass in the container
- that 1000 steps suits `tracked` and `planned`
- that the upstream controls behave in Claude Code as they do upstream
