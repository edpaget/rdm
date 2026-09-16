# Codex runtime

Phase three adds an explicit Node API and command runner for real review targets
and roadmap estimates. The runtime imports the canonical review and estimate
modules; it does not invoke Claude's workflow engine. Installed Codex skills
remain the four manual skills until phase four integrates these entrypoints.

## Operations and ownership

| Operation | Target | Writes |
| --- | --- | --- |
| `plan-review` | Absolute implementation-plan file | Local evidence only; no plan-review tags or status transitions |
| `code-review` | Clean source checkout, full base/head commit IDs, acceptance criteria | Local evidence only; no review gate advancement |
| `estimate` | Existing roadmap in the explicit project | Preview by default; `apply: true` updates unset difficulty/audit notes and commits only this run's plan changeset |

The caller owns sequencing, authorization and deciding what to do with findings.
A completed review run means execution and coverage completed, not that findings
are absent or the item is approved. Code results include the canonical classified
outcome. Inspect findings and acceptance-criteria evidence before any independent
status decision. The runtime neither lands source nor marks items done.

Judgment roles are finder, refuter and estimator. Every call starts a fresh,
ephemeral, read-only Codex CLI process; finders and refuters have separate
contexts. Mechanical reads and writes use direct RDM argv, never an LLM.
This role is shell/file review, without claims of equivalent Claude permissions,
external connectors, or an autonomous implementation agent.

Role restrictions are fixed by the transport, not configurable model bindings:
user config is ignored, approvals are disabled, web search is disabled, and
apps, plugins, remote plugins, browser/computer use, hooks, multi-agent features,
workspace dependencies and image generation are disabled. Managed/project MCP
configuration remains the trusted host's responsibility. These flags do not
establish universal connector isolation or a filesystem-read allowlist; inspect
that host configuration before running judgment against untrusted material.

## Explicit run specification

Use the repository's Node version (`mise exec node -- …`) and make the Codex CLI
available on `PATH` with working authentication. No Node dependency is added to
the installed Rust binary. From this checkout:

```sh
node scripts/rdm-codex.mjs /absolute/path/to/run-spec.json
```

The runner prints a JSON result on success and exits nonzero on runtime failure.
This example reviews an implementation plan. Model identifiers below are
placeholders: supply exact supported identifiers and the efforts available to
your account; the runtime makes no provider availability discovery or fallback.

```json
{
  "operation": "plan-review",
  "sourceDir": "/absolute/path/to/source-worktree",
  "planRoot": "/absolute/path/to/separate-plan-repo",
  "rdmBin": "/absolute/path/to/source-worktree/scripts/rdm-dev.sh",
  "project": "rdm",
  "session": "codex-your-conversation-id",
  "runDir": "/absolute/private/evidence/new-run-001",
  "planFile": "/absolute/path/to/implementation-plan.md",
  "concurrency": 3,
  "agentTimeoutMs": 180000,
  "rdmTimeoutMs": 120000,
  "host": {
    "capabilities": { "your-supported-model-id": ["medium", "high"] },
    "tiers": {
      "small": { "model": "your-supported-model-id", "effort": "medium" },
      "medium": { "model": "your-supported-model-id", "effort": "high" },
      "large": { "model": "your-supported-model-id", "effort": "high" }
    }
  }
}
```

Source and plan paths must resolve to separate existing git checkout roots.
The source must have a branch and both repositories must have a HEAD commit.
The executable must be absolute and executable. When developing rdm, select
that source checkout's rebuild-before-use `scripts/rdm-dev.sh`.
`runDir` must not exist, and its existing parent must be canonical with no
symlink components. Use an evidence location outside the source checkout so
review evidence does not dirty the reviewed tree. Evidence permissions are
private (directory 0700, files 0600).

The supplied session identifies the caller. Each run mints a distinct
`codex-<UUID>` session, recorded as `manifest.identity.session`; the caller
session is retained as `parentSession`. Runtime writes and commits use only
that owned session, keeping preexisting caller-session changes separate.
Direct commands receive the selected RDM identity, with inherited `RDM_*` and
Git override variables removed. No `--all`, root, changeset or session override
is accepted by the direct-command API.

Concurrency is 1–8 (default 3). Agent timeout is bounded to one hour; direct
RDM timeout is 1–600000 milliseconds (default 120000). Long rebuilds may require
raising the latter explicitly. The JavaScript API also accepts an AbortSignal
as `signal`; it cannot be represented in a JSON file.

### Model policy

The runtime resolves `review-find`, `review-verify`, or estimation's `plan`
step through `rdm model resolve <step> --format json`. Core returns
`{step,tier,model}` and remains responsible for tier policy and the review
floor. Plain CLI output remains unchanged. Codex then maps the resolved tier
to `host.tiers`; shared Claude model preferences are not rewritten.

An optional `tier` supplies a core hint (`small`, `medium`, `large`). For a
phase code review, the item's model tier is used when present. An optional
`host.steps` binding can select a different model/effort for one step, but
must declare the same effective core tier:

```json
{"steps":{"review-verify":{"tier":"large","model":"your-supported-model-id","effort":"high"}}}
```

The selected model must be in `capabilities` and list the requested effort.
Recognized efforts are `minimal`, `low`, `medium`, `high`, and `xhigh`. Claude
aliases are rejected. Missing bindings, unsupported combinations and account
errors stop the run; there is no silent model substitution.

### Review targets

For `plan-review`, set `planFile` to the exact nonempty implementation-plan
file. The canonical implementation-plan driver reviews a content snapshot,
and the runtime rejects a changed file before accepting the result. It does
not perform a high-level roadmap review or persist document review actions.
Parent-intent coverage follows the canonical driver's reported coverage.

For `code-review`, replace `planFile` with full hexadecimal `base` and `head`
commit IDs, plus either explicit `target` text containing acceptance criteria
or an item reference:

```json
{"item":{"type":"phase","roadmap":"codex-agent-support","phase":"phase-3-host-agnostic-runtime-adapter"}}
```

The head must equal checkout HEAD, base must be its ancestor, the range must
have a nonempty diff, and the worktree must be clean including untracked files.
Item reviews read the actual phase body and validate its registered roadmap
worktree and branch. The runtime derives dimensions from changed files/diff,
executes canonical finder/refuter orchestration, and rechecks checkout and
item snapshots. Missing dimension coverage, missing AC evidence, refuter errors,
and refutation overflow are incomplete runs, never clean approvals.

### Estimates

For `estimate`, replace the review fields with `roadmap` and optionally
`apply: true`. Preview returns proposals without modifying plan documents.
Apply validates the complete judgment batch before any writes, rechecks each
phase, preserves tags and existing body, and writes only still-unset difficulty
plus the canonical audit note. It reads back persisted values/core tier and
commits only the runtime-owned session. Already estimated phases are skipped.
An interrupted update, commit, failed readback or later drift stops execution
and requires inspection before another apply run.

## Evidence and recovery

Each fresh run contains `manifest.json`, an append-only fsynced `journal.jsonl`,
per-call private Codex event/stderr evidence, and a final `result.json` when
finalization succeeds. The manifest records source root, branch, HEAD, plan
HEAD, binary, project, caller/owned sessions, lifecycle and uncertainty. It also
records the runner PID, parent PID, hostname, executable/argv and approximate
process start time for interrupted-run inspection.
Journal records associate agent calls with thread IDs and usage, model
resolution, target snapshots, and direct command argv/output. Source HEAD
and plan commits are separate evidence. Keep artifacts private: prompts and
outputs can contain source or plan text despite diagnostic redaction.

A crash can leave `status: running` and a write intent without acknowledgement.
Treat that as uncertain even if `uncertainWrites` is still false. Explicit
failed writes and postwrite readback failures set uncertainty. The runtime
never reopens a run directory, replays mutations, or claims successful
completion after an uncertain write.

Before inspecting mutations or starting another run, stop the old runner.
Read `manifest.runner`, confirm you are on its recorded hostname, and inspect
the recorded PID (replace `12345` below with that value):

```sh
ps -p 12345 -o pid=,ppid=,lstart=,command=
```

Compare executable/command, parent and process start time with the manifest.
The recorded start time is approximate; PID reuse means a matching number
alone is insufficient. If the process is still the recorded runner, send
`kill -TERM 12345`, then repeat `ps` until that runner has exited. Do not signal
an unrelated process whose PID was reused. If identity is ambiguous, inspect
further rather than retrying the operation concurrently.

During an active Codex call, the transport handles SIGTERM/SIGINT by cancelling
and terminating the child process group. A hard crash or SIGKILL may bypass
that cleanup. The runner PID and model thread IDs do not prove absence of
stray child processes: inspect process command lines and groups for the
recorded checkout before restarting, and resolve surviving related work
explicitly. A direct RDM subprocess may also finish a mutation after runner
termination, so wait for related work to stop before readback.

Inspect `manifest.json` and every `write-intent` / `write-acknowledged` /
`write-uncertain` event. Using the **owned session from that manifest**, set:

```sh
export RDM_BIN=/absolute/path/to/recorded/source/scripts/rdm-dev.sh
export RDM_ROOT=/absolute/path/to/recorded/plan-repo
export RDM_PROJECT=recorded-project
export RDM_SESSION=codex-owned-session-from-manifest
"$RDM_BIN" session id --format json
"$RDM_BIN" session journal --format json
"$RDM_BIN" status
"$RDM_BIN" phase show phase-stem --roadmap roadmap-slug --project "$RDM_PROJECT" --format json
git -C "$RDM_ROOT" log -5 --oneline
```

Compare actual phase body/difficulty/tags, session-owned paths and recent plan
commits with the recorded intended writes and before/after plan HEADs. Resolve
any remaining staged changes deliberately under that exact session; do not
commit all changes or adopt unrelated sessions. A commit may have succeeded
before its acknowledgement was lost, so inspect history before deciding a
commit is needed. Restart with a new `runDir` only after reconciling prior
side effects. Read-only review failures can be rerun against freshly verified
targets with a new evidence directory.

## API and boundaries

`runRuntime(spec)` in `scripts/lib/codex-runtime.mjs` is the command runner's
API. `createRun(spec)` exposes `identity`, `session`, `runDir`,
`rdm(args, {json, mutating})`, `record(type, data)`, `finish(result)`, and
`fail(error)`. Callers supply exact argv including `--format json` when needed.
`json` parses stdout; `mutating` records durable intent before execution.
Adapters that detect uncertainty after a successful process must throw an
error with `uncertainWrites: true`, which `fail` persists. Existing run
evidence cannot be reused and terminal contexts reject more commands.

Unsupported: autonomous dispatch/autopilot, source edits or landing,
automated review gate/status writes, plan-target auto-act/tag persistence,
backlog/document drivers, interrupted-write replay, and production skill
entrypoints beyond the manual lane. These are explicit capability boundaries,
not claims of complete workflow parity.

Credential-free tests run with:

```sh
node --test scripts/lib/codex-spike-*.test.mjs scripts/lib/codex-runtime*.test.mjs
```

These tests exercise contracts and isolated fixtures. Live authenticated
runs have separate evidence; passing fixtures alone does not establish a
successful account/model invocation or completed independent review.
