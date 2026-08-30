# The phase-time verify gate (`dispatch.verify`)

`rdm-wf-dispatch-phase` runs **one** project-supplied command once per implementation
attempt — plus once more when the review's Act step changed code (§ 8) — and interprets its
exit code. A non-zero exit sends the item back through the existing rework budget instead of
reporting it `reviewed`.

This document is canonical for what that command is, how it is resolved, what rdm
deliberately does *not* do with it, and why a phase-time gate does not replace a
commit-time one.

## 1. What `dispatch.verify` is

- **One command string.** Not a list, not a matrix, not a DAG.
- **Project-supplied data.** rdm's resolution and execution path names no language,
  package manager, or test tool. That property is enforced by construction and
  grep-asserted (with an occurrence floor and planted-mutation self-tests) by
  `scripts/verify-workflow-dispatch.sh` § 3-verify.
- **Run once per implementation attempt**, in the item's worktree, after the
  implementer returns and before the code review's verdict is composed — **plus once
  more** when the review's Act step changed code, so a fix that lands after the check
  run cannot ship unverified (§ 8).
- **Interpreted by exit code only.** Zero passes; anything else fails. The command's
  output is captured (tail-truncated) and handed to the rework implementer.

Set it, read it, and remove it with the ordinary config surface:

```bash
rdm config set dispatch.verify "bash scripts/ci.sh"
rdm config get dispatch.verify         # bash scripts/ci.sh  (source: repo config)
rdm config get dispatch.verify --raw   # bash scripts/ci.sh
```

`--raw` prints the value alone — no `(source: …)` annotation, and no output at all when
the key is unset — so a caller can run what it reads without parsing it. That is the form
the dispatch resolution prompt uses; the annotated form is for humans.

It is **repo-only** — a verify command is a property of a project, never of a user — so
`rdm config set --global dispatch.verify …` is rejected with an actionable error, and it
lives in the repo's `rdm.toml` as:

```toml
[dispatch]
verify = "bash scripts/ci.sh"
```

## 2. Resolution precedence

1. The declared `dispatch.verify` key, when set — read with
   `rdm config get dispatch.verify --raw` and used verbatim.
2. Otherwise **discovery**, in order, stopping at the first source that yields anything:
   1. CI configuration under `.github/workflows/` (also `.circleci/config.yml`,
      `.gitlab-ci.yml`)
   2. `docs/principles.md`
   3. `CLAUDE.md` / `AGENTS.md`

   A single-line command is synthesized from whatever checks that source names.
3. Otherwise **escalate**. A dispatch that cannot determine how to verify itself must not
   report success: it returns the existing `escalated` outcome (rdm status `blocked`)
   before planning or implementing anything, with a summary naming the config key and all
   three discovery sources.

A `--plan-only` run does no implementation, so it never escalates on an unresolved verify
command. That `!planOnly` condition is the single point of control on the short-circuit.

## 3. Explicit non-goal: rdm is not a task runner

rdm runs the one command the project declares and reads the exit code. It does **not**:

- decompose the command into steps, reorder them, or run only some of them;
- impose or configure timeouts, parallelism, or retries;
- own per-tool configuration or output formatting;
- retry a failed command inside the same attempt.

All of that stays where it already lives — `hk`, `make`, `just`, a shell script, or CI.
If you want ordering or parallelism, put it in the thing your one command invokes.

## 4. The two layers, and why both exist

Hard enforcement lives in two places, and they are **not** substitutes for each other.

### Commit-time: fast, mandatory, non-bypassable

The project's own **pre-commit hook** is the only layer that blocks a commit regardless of
what any agent decides. This repo's instance is [`hk`](https://hk.jdx.dev/), declared in
`hk.pkl` and wired through `.githooks/pre-commit`; it runs `cargo fmt --check`,
`cargo clippy -- -D warnings`, `cargo nextest run`, plus `shellcheck` and `shfmt` over
staged shell scripts. Recommend a hook like this to make something *truly* mandatory.
rdm neither installs nor requires one.

### Phase-time: slow, once

`hk`'s pre-commit here deliberately does **not** run the `scripts/verify-*.sh` harnesses,
because a minutes-long suite has no business in a per-commit gate. That is exactly how the
`review-gate-intent` land-time failures got through: phases 4, 5 and 7 all passed code
review, two harnesses were red at land time, and **every commit had passed the hook**.
Slow verification needs a once-per-phase home, and `dispatch.verify` is it.

**Anti-recommendation:** do *not* close this gap by pushing the slow suite into
pre-commit. That makes every commit pay a minutes-long cost, which is how pre-commit
hooks get bypassed — and a bypassed hook enforces nothing at all.

`rdm-land` keeps its own check run on top of both. A phase-time run cannot establish that
the branch still passes after rebasing onto a moved `main`; that is what the land-time run
is for. The three layers are complementary, not redundant.

## 5. Reconciliation: `rework` here, `blocked` upstream

A failing verification is folded into the round as a synthesized **blocking** finding, so
the untouched classifier resolves it. That means:

- The OUTCOME vocabulary is unchanged: `reviewed | rework | escalated`. No new value, no
  new `classifyOutcome` branch, no new `GATE_POLICY` row, no new `statusFor` mapping.
- Repeated failures consume the **existing** `maxCodeRework` budget — there is no second
  counter — and an exhausted budget returns `rework` (rdm status `in-progress`).
- The `blocked` **park** is written by the invoking loop's existing
  rework-retry-once-then-park policy (the prose `rdm-autopilot` skill's advance/park
  steps, gated by `scripts/verify-skill-autopilot.sh`), not by dispatch itself.

This is the only reading that keeps `maxCodeRework: 0` correct: with a zero budget a
single failing attempt must still be `rework`, never `escalated`.

`escalated` is reserved for the case where no command could be resolved at all (§ 2), which
is a configuration failure rather than a code failure.

## 6. Failure semantics

- **Non-zero exit** → blocking finding → `rework`, with the command named in the OUTCOME
  `summary` and `reason`, and the failing output tail rendered into the rework
  implementer's prompt.
- **Fail-closed.** A verification agent that throws, resolves `null` (an unknown model id),
  or reports no integer exit status is treated as a failure. An unrunnable declared
  verification must never read as a pass.
- **Output is tail-truncated** to the last 4000 characters — failures print last, and an
  unbounded log would blow the rework prompt's context budget.
- **Multi-line values are refused.** The value is interpolated into a Bash-agent prompt; a
  multi-line command belongs in a script that the one declared command invokes.

## 7. Surfacing it to the implementer

The resolved command is rendered into **both** the first-pass and the rework implementer
prompts as available tooling, so it is run proactively rather than discovered as a failure
after the fact. The rework prompt additionally carries the previous round's exit code and
output tail.

## 8. `reviewed` requires a clean worktree

A dispatch that reports an item `reviewed` while work it produced is still uncommitted is a
false green. `rdm-land` rebases before merging, so that work never ships — and if the work
was a fix for a code-review finding, the finding was satisfied by nothing.

Observed on `review-gate-intent` phase 2 (2026-08-28): the code review raised a finding,
marked it `handled: "fixed-inline"`, and the dispatch returned `outcome: "reviewed"`,
`writesCompletion: true` — with the remediation sitting uncommitted in the worktree.

### The mechanism, and the two rejected alternatives

**The pick: the step that applies an inline fix commits its own change.** The Act step's
prompt now tells the fixer to fix it, re-run the declared verification command, `git add`
only the files it changed, and `git commit` with a message body carrying one
`Review-Finding: <id>` line per finding that commit closes. The short sha comes back on the
`handled` entry (`CODE_ACT_SCHEMA`'s optional `commit`) and is stamped onto the finding as
`handledCommit`, so **the finding and the commit that closed it are both recoverable from
the OUTCOME** — on the `reviewed` branch and on the `rework` branch alike.

The terminal cleanliness assertion below is *enforcement* of that invariant, not the
remediation.

Rejected, with reasons:

- **Amending the implementation commit.** Destroys attribution: that commit predates the
  finding, so the record would claim the fix shipped as part of the original
  implementation. The act prompt explicitly forbids amending.
- **A bulk "commit whatever is dirty" step after the act step.** Unattributable — one
  nameless commit for every finding at once — and it would sweep unrelated dirt (an
  operator's own edits in a shared per-roadmap worktree) into that commit.

### The fixed ordering

Inside `runCodeGate`, the terminal tail runs in exactly this order:

1. the bounded rework loop (implement → verify → review, `maxCodeRework` times);
2. the **Act** step — fix the finding *and commit it*;
3. a **post-act re-verify**, whenever the Act step changed code;
4. the **cleanliness probe** (`clean:check`) — `git status --porcelain` in the item worktree;
5. **fold** any failure into the FINAL round.

Step 3 is what makes the ordering matter: a fix applied *after* the check run would
otherwise escape verification entirely. It reuses the same single `d.verify` call site the
rest of the gate uses, so §§ 1/6's "no branch can skip it, no retry loop can multiply it"
property survives.

`actChangedCode` decides whether step 3 fires, and is **fail-closed**: act invoked but its
result null, thrown, or carrying no `handled` array ⇒ re-verify anyway. An unknown act
outcome must never be read as "nothing changed".

### Probe semantics

- The `clean:check` agent runs `git status --porcelain` **once** in the item's worktree and
  returns its output verbatim. It observes; it does not repair. Editing, staging,
  committing, stashing, `reset --hard` and `clean -fdx` are all explicitly forbidden — a
  probe that rewarded a destructive shortcut would be worse than no probe.
- `parseWorktreeStatus` is **fail-closed**, mirroring `normalizeVerifyResult`: a non-object,
  a missing or non-string `porcelain`, a thrown probe, or text that parses to no path all
  read as *not clean*. An unobservable worktree is never a clean one.
- An **absent** `d.clean` dep is a **skip**, not a failure — the same precedent the absent
  `d.verify` dep sets. Non-vacuity is bought statically:
  `scripts/verify-workflow-dispatch.sh` § 3-clean asserts the shipped driver actually binds
  the probe, with planted-mutation self-tests behind each assertion.
- A rename entry (`R  old -> new`) reports the **destination**; paths are never split on
  whitespace; a trailing newline never produces a phantom path; and a wholesale-dirty tree
  is capped at 20 reported paths with an `…and N more` tail so the OUTCOME summary (and
  therefore the `rdm review blocked` queue line) cannot be blown.
- The probe is scoped to the **item worktree only**. A LARGE finding filed with
  `rdm task create` mutates the plan repo, which is a different git repository; landing that
  batch stays the caller's `rdm commit`.

### Why a post-act failure folds instead of re-entering the rework loop

Both terminal failures — a failing post-act re-verify and a dirty tree — are folded into the
**final** round as mechanical blocking findings (`verifyFailureFinding` /
`dirtyWorktreeFinding`), so the *untouched* classifier resolves the unit to `rework`. No new
OUTCOME value, no classifier branch.

They deliberately do **not** re-enter the rework loop. `maxCodeRework` already bounded the
implement/verify/review rounds; a second entry point would silently multiply that budget.
The fold **replaces** the last entry of `rounds` rather than pushing a new one, because
`acRounds`/`budgetRounds`/`coverageRounds` are index-parallel to it and `reviewCount` is
derived from its length — a push would manufacture a phantom review round.

### Scope

`--plan-only` is unaffected by construction: the driver returns at its `if (planOnly)`
branch before `runCodeGate` is reached, so neither the act step, the post-act re-verify, nor
the probe can fire. `Done:`-trailer writing is likewise untouched — `rdm-land` remains the
sole land-time writer, synthesizing the directive from the OUTCOME via
`rdm hook done-line`. This gate only makes `rdm-land`'s existing "the worktree is clean"
precondition satisfiable by construction rather than by luck.

Pre-existing dirt the dispatch did not create will now force `rework`. That is the intended
direction — `reviewed` must mean landable — and the finding names the paths so an operator
can tell instantly that the dirt was not the dispatch's.

## See also

- [`workflow-schemas.md`](workflow-schemas.md) § "Verify gate" — the result schema and the
  `verify:run` label.
- [`escalation-protocol.md`](escalation-protocol.md) — the budgets this gate reuses.
- [`autonomous-loop.md`](autonomous-loop.md) — where the `blocked` park is written.
