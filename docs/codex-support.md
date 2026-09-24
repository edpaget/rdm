# Codex support: manual skills and explicit runtime

Phase one enables manual, review-gated work. The [phase-three runtime](codex-runtime.md)
adds explicit implementation-plan review, pinned code review, and roadmap
estimates with durable evidence and conservative recovery. Phase four repairs
the shared interfaces and distributes that explicit runtime. Automated skill
dispatch remains later integration work.

The [phase-two execution spike](codex-orchestration-spike.md) records experimental
Codex execution of shared review and estimate logic. It does not enable the
withheld production skills. The runtime is a separate explicit entrypoint;
the skill support matrix below remains the manual lane.

## Installation and discovery

```sh
rdm agent-config codex --project my-project --out /path/to/source
rdm agent-config codex --skills --project my-project --out /path/to/source
```

The first command writes `AGENTS.md`; inspect existing instructions before
emission because the generator overwrites its output files. The second writes
four `.agents/skills/<name>/SKILL.md` files and reports the seven withheld
skills. It also installs the explicit runtime and shared modules under
`.agents/rdm-runtime`; see [runtime setup](codex-runtime.md). This does not enable
automated skills, install Claude workflows/custom agents/plugins, or remove
unrelated or previously installed skills. Node and authenticated Codex are
needed only when executing runtime judgment, not for ordinary CLI use.

`--user` writes instructions to `$CODEX_HOME/AGENTS.md` (default
`~/.codex/AGENTS.md`) but skills to `~/.agents/skills` and the runtime to
`~/.agents/rdm-runtime`, independently of
`CODEX_HOME`. rdm preserves relative environment paths: a relative `CODEX_HOME`
is resolved against the invoking process's working directory. Prefer absolute
values when switching checkouts. The existing `agents-md` platform retains its previous behavior;
use `codex` for Codex-specific paths. Codex plugin generation by rdm is not
implemented: use `--skills --out`, not the Claude plugin tree.

Codex discovers repository skills from the current directory up to the git
root. Start a fresh session in the emitted repository, inspect `/skills`,
then explicitly request `$rdm-roadmap` or `$rdm-do` with a scoped task. In
noninteractive mode, include that same explicit skill request in the prompt.
An `AGENTS.override.md` can replace `AGENTS.md` at its level. Duplicate skill
names in project, user, or plugin locations are not merged: inspect the
selected path and use the repository `.agents/skills` copy for dogfooding.
For example, explicitly request `$rdm-roadmap` **from the absolute path to
the repository's `.agents/skills/rdm-roadmap/SKILL.md`**, not just the bare
name. Installed plugin skills may be namespaced (the fixture below exposes
`rdm-coexistence:rdm-roadmap`); user/repository copies can still share a name.
Do not uninstall other copies without permission. Restart if newly generated
skills are not visible.

Paths and discovery were checked against the official
[skills documentation](https://learn.chatgpt.com/docs/build-skills) and
[AGENTS.md documentation](https://learn.chatgpt.com/docs/agent-configuration/agents-md)
on 2026-09-15. Installation/discovery is distinct from successful workflow
execution; see the support matrix below.

## Developing rdm itself

The checked-in `AGENTS.md` is repository guidance, not generated downstream
CLI documentation. The four local skills are generated from the downstream
templates, with `--project rdm --principles-file docs/principles.md`:

```sh
sh scripts/gen-codex-skills.sh
```

That command refreshes generated runtime templates from their canonical
production modules before building. Validate Codex distribution with
`cargo nextest run -p rdm-core -E 'test(codex)'` and
`cargo nextest run -p rdm-cli --test cli_agent_config --test codex_distribution`.
The Rust tests check generated-local drift and execute installed production
runtime code in an unrelated source/plan fixture. Standalone shell verification
harnesses are not Codex acceptance gates.

Set explicit values before launching Codex, or repeat them on every shell
tool invocation. Replace the paths and choose a unique session ID once for
the conversation:

```sh
export RDM_ROOT=/absolute/path/to/rdm-plan-repo
export RDM_PROJECT=rdm
export RDM_SESSION=codex-your-unique-conversation-id
export RDM_BIN=/absolute/path/to/rdm/scripts/rdm-dev.sh
"$RDM_BIN" session id
"$RDM_BIN" roadmap show codex-agent-support --project rdm
```

The wrapper uses Cargo to rebuild the checkout it lives in before every
command. Build failures stop execution; no stale installed binary fallback
is used. It honors Cargo's target configuration and never assumes all
worktrees share `target/debug`. It requires an existing, readable absolute
plan path and a stable session identity. Set these explicitly rather than
relying on this repository's personal `.mise.toml` defaults. Cargo, the
repository toolchain, and normal write access to build output are required.

To start a phase, use `"$RDM_BIN" worktree add <roadmap> --project rdm --format
json`, inspect its returned path and existing work, then set `RDM_BIN` to that
worktree's `scripts/rdm-dev.sh`. Keep `RDM_ROOT` and `RDM_SESSION` unchanged.
Use explicit working directories in tool calls. Check `session id` across two
separate calls if the harness creates a new shell each time. `rdm status` and
`rdm commit` take no project flag; never use `--all` to collect another
session's changes.

An external plan repo or sibling worktree may lie outside the host's writable
roots. Request narrowly scoped normal approval when needed. An unreadable
plan repo is not permission to create a replacement, silently switch projects,
or disable sandboxing. Missing toolchain, authentication, or approval should
be reported with the failed command and the action needed to continue.

## Support matrix

“Workflow” describes the existing Claude surface, not a Codex capability.
Distributed Claude ships eleven skills but only one workflow engine; local
Claude has five production engines and a separate spike. Eight local skills
depend on workflow behavior. The manual Codex `rdm-do` is a deliberate bounded
alternative, not a claim that its Claude finalization workflow was ported.

Audit sources: distributed prose lives in
`rdm-core/src/templates/skill-<name>-cli.md`; local Claude entrypoints are
`.claude/skills/rdm-<name>/SKILL.md`. Codex templates live in
`rdm-core/src/templates/codex/rdm-<name>.md` and generate the local
`.agents/skills` copies. Claude's one shipped engine is
`rdm-wf-review-refute-fix` (`rdm-wf-dispatch-phase` was a second until
`agent-orchestrated-dispatch` phase 7 retired it in favour of the prose
`rdm-dispatch-phase` orchestrator); its four additional
local engines are `rdm-wf-plan-review`, `rdm-wf-estimate`, `rdm-wf-backlog`,
and `rdm-wf-document`. Their local files are under `.claude/workflows`;
shared review specifications originate in `.claude/workflows/lib/review.mjs`
and are stamped into generated blocks, not independently rewritten for Codex.
Claude tool names (`Workflow`, `Agent`, `Bash`, `Read`), `$ARGUMENTS`, and
permission-mode flags are replaced by ordinary Codex shell/file tools,
explicit user task text, and host-managed approvals in the manual templates.

| Skill | Distributed Claude | Local Claude | Codex phase one |
| --- | --- | --- | --- |
| rdm-roadmap | Prose | Prose | Manual planning; independent review handoff |
| rdm-do | Dispatch/review workflow | Dispatch/review workflow | Manual implementation → needs-review |
| rdm-revise | Prose | Prose | Document revision with per-comment provenance |
| rdm-land | Prose | Prose | Explicit reviewed landing |
| rdm-review | Review workflow | Review workflow | Withheld; independent human/working host |
| rdm-plan-review | Prose review recipe | Plan-review workflow | Withheld; independent human/working host |
| rdm-estimate | Prose recipe | Estimate workflow | Withheld; inspect/estimate manually |
| rdm-backlog | Prose recipe | Backlog workflow | Withheld; inspect and propose manually |
| rdm-document | Prose recipe | Document workflow | Withheld; author docs manually |
| rdm-dispatch-phase | Prose orchestrator (one Workflow call; the plan gate is a human approve review, since `rdm-wf-plan-review` is not shipped) | Prose orchestrator (two Workflow calls) | Withheld; one manual rdm-do item. The procedure is prose, but its code-review stage — and, locally, its plan-review stage too — are `Workflow` calls Codex has no runtime for. |
| rdm-autopilot | Prose loop over the orchestrator | Estimate workflow + prose orchestrator | Withheld; one manual rdm-do item |

Existing Pi generation is unchanged. Native Codex review, agent delegation,
or external tools must not be represented as the unported canonical workflow.
The manual implementation skill provides a precise review handoff and leaves
status `needs-review` until independent review evidence exists. Landing is
never implied by permission to implement.

## Verification and first improvement candidate

Rust distribution tests cover fresh project and user emission, frontmatter,
manual-lane host boundaries, plugin rejection, generated-local drift, and
explicit runtime execution against foreign source/plan repositories. These
deterministic checks do not substitute for a live Codex discovery/invocation
smoke test; record the tested CLI version and its observed result separately.

On 2026-09-15, a fresh Codex CLI 0.154.0 read-only `exec` session discovered
the repository `rdm-do` in its skill catalog, explicitly invoked it, read the
generated skill and `AGENTS.md`, and correctly described the `needs-review`
handoff without mutating source or plans. The outer host required approval
to initialize the CLI; the child session retained its read-only sandbox.
This proves discovery and instruction-following for the manual entrypoint,
not an automated review runtime or a completed end-to-end autonomous cycle.

A second fresh CLI session explicitly used `rdm-roadmap` for inspection,
invoked `scripts/rdm-dev.sh`, preserved the supplied session identity, and
successfully read the real Codex roadmap and task backlog without document
mutations. The wrapper was also invoked from the primary checkout's working
directory and from the RDM-created linked worktree, resolving the same
explicit plan repository and the selected worktree's development build.
Main remains unchanged until this branch is landed. Codex app/IDE invocation
has not been exercised here; documented discovery rules are not claims of
runtime parity.

### Reproducing the coexistence check

The opt-in `scripts/verify-codex-coexistence.mjs` creates an isolated temporary
HOME and CODEX_HOME, installs a local skills-only plugin, seeds a distinguishable
same-name user skill, and emits the real repository skill. It asks Codex's
actual skill catalog to confirm all three enabled copies, then (when given a
login file) starts a fresh read-only CLI session that explicitly selects the
repository path. It verifies the selected path and plan-review gate, and checks
the user skill, plugin source/cache, marketplace, and repository skill bytes
were preserved. Fixtures and non-secret evidence remain in the printed temp
directory; the private login copy is deleted even when the live check fails.
Catchable interrupts (SIGINT/SIGTERM) and the two-minute timeout stop the child
process group and delete the copy too. The live `codex exec` call runs under the
repository-only Rust runner `rdm-smoke` (crate `rdm-devtools`, never shipped),
which creates the copy (mode 0600) just before spawning and removes it on every
catchable path; `cargo nextest run -p rdm-devtools` exercises success, non-zero
exit, timeout, output cap, SIGINT and SIGTERM with real fixture processes and
non-secret markers. The script builds `rdm-smoke` with cargo on demand; set
`RDM_SMOKE_BIN` to use a prebuilt one. Standalone invocation:
`cargo run -q -p rdm-devtools --bin rdm-smoke -- run --timeout-secs 120 --private-copy SRC:DEST -- <program> <args>`.
SIGKILL or machine failure cannot guarantee cleanup: if
that occurs, remove only `config/auth.json` inside the printed test directory
before keeping or sharing the evidence. The temporary root is private.
No real user/plugin installation is modified.

```sh
# Discovery/preservation only; no login copied and no live model invocation:
node scripts/verify-codex-coexistence.mjs /absolute/path/to/rdm /absolute/path/to/codex
# Full live check: explicitly opt in to copying this login into the temp config:
node scripts/verify-codex-coexistence.mjs /absolute/path/to/rdm /absolute/path/to/codex /absolute/path/to/auth.json
```

This is also an optional arm of the existing distribution harness: set
`RDM_CODEX_BIN` to the Codex executable and optionally `RDM_CODEX_AUTH_FILE`
to the login file. Without the latter it reports live invocation as **not run**,
not as passed. The default hermetic harness needs neither Codex nor an account.

The full check passed with Codex CLI 0.154.0 on 2026-09-15: the real catalog
contained the enabled repository, user, and installed-plugin copies; a fresh
session read the requested repository SKILL.md, reported its independent
plan-review gate, and left all protected bytes unchanged. The helper retains
`catalog.json`, `events.jsonl`, `answer.json`, and `result.json` for inspection.
This verifies the explicit-path CLI recipe, not automatic precedence or app/IDE
selection behavior. Fixture manifests follow the official
[plugin packaging guidance](https://learn.chatgpt.com/docs/build-plugins).

A bounded early dogfood candidate is the stalled `agent-orchestrated-dispatch`
worktree mismatch: inspect the shared roadmap branch and phase-specific
worktree before diagnosing why a reviewer saw no implementation. The
`roadmap/phase` worktree reference appears capable of selecting an empty phase
checkout while work exists on the shared roadmap branch; this is a diagnostic
lead, not a proven historical cause. Do not mutate that roadmap or create a
permanent fork of its retiring engine as part of this bootstrap phase.

The read-only snapshot on 2026-09-15 found the shared
`roadmap/agent-orchestrated-dispatch` at
`2c55784d8658621f453ea8a567abca3570d1f2b2`, while the phase-four worktree
`phase-agent-orchestrated-dispatch-phase-4-change-review-target` was still at
`00a9aa045543b625c05e3f790919996e89a90afc` (the then-main base). A future
diagnostic should pin and compare those actual heads, verify the review target
and gate probe resolve the implementation worktree, and stop before any
recovery mutation unless separately authorized.
