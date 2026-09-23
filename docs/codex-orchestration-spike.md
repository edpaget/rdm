# Codex execution spike

Phase 2 demonstrates that Codex CLI subprocesses can execute RDM's canonical
review logic and estimate pipeline. Proceed with a CLI-based integration in
phase 3, limited to the demonstrated seams below. This is experimental tooling;
the supported Codex skills remain the manual lane in [codex-support.md](codex-support.md).

## Baseline and method

The 2026-09-15 run used Codex CLI 0.154.0, Node 24.18.0, model
`gpt-6-astra`, and `medium` reasoning effort. These are the operator's installed
model/effort settings, passed explicitly rather than translated from Claude
model names. Authentication reused the existing ChatGPT CLI login in place;
no credentials were copied or included in evidence. Each judgment ran in a
fresh `codex exec --sandbox read-only --ignore-user-config --ephemeral` process.
Personal exec rules were preserved. The outer sandbox could not initialize
Codex's app-server (`Operation not permitted`); normal host approval allowed
initialization while retaining the nested read-only sandbox.

Source baseline and then-main were
`775d2a4c888a6dc13cd21c61f393ff74f55456fd`, in the shared
`roadmap/codex-agent-support` worktree. The experiment ran the working-tree
spike files against that baseline; it is not evidence from a released adapter.
The independent implementation-plan review preceded coding and covered
coherence, architectural fit, intent alignment, and restraint, with no findings.
The operator explicitly selected implementation-plan review rather than another
review of the roadmap/phase definition.

The separate dispatch successor is unlanded. This baseline has document review
targets for roadmaps, phases, and tasks, but no persisted implementation-plan or
change-review target. No successor review gate was exercised or cleared.

## Host choice

The official [non-interactive documentation](https://learn.chatgpt.com/docs/non-interactive-mode)
describes JSONL events, structured final outputs, and explicit session resume.
The [SDK documentation](https://learn.chatgpt.com/docs/codex-sdk) describes
thread creation/continuation and points interactive approval clients to app-server.
Both were checked on 2026-09-15. Installed `codex exec --help` and
`codex exec resume --help` confirmed the options used here.

Direct CLI execution is the smallest candidate for this repository's existing
Node workflow tooling. The SDK and a custom app-server client were compared
from documentation, not experimentally benchmarked. Neither is required for the
read-only judgment and caller-owned fixture writes demonstrated here. Revisit
app-server if production approval interaction needs it.

## Experiments and results

[Recorded evidence](codex-orchestration-evidence.json) includes fixture base/head
and diff hash, every call's label/thread/timing/usage, coverage, findings,
refutation accounting, estimates, and the separate resume probe. Full private
JSONL remains in the local temporary evidence directory printed by the runner.
The published record omits raw tool transcripts and local plan paths.

| Capability | Observed result | Disposition |
| --- | --- | --- |
| Canonical code review | Three finders (AC, correctness, tests), followed by three fresh refuters, detected planted subtraction in `add`; AC failed and outcome was `rework`. Complete selected coverage. | Go for integration of shared review logic; standalone production driver remains work. |
| Canonical plan driver | Real `runPlanReviewDriver` in implementation-plan mode ran coherence, architectural-fit and restraint. One candidate was refuted; outcome `reviewed`. | Go for report-only driver integration. Persisted target writes are unproven. |
| Estimate | Two live ratings were written through the real development CLI to an isolated plan repo; tiers were read back from core. The pre-estimated phase was unchanged. Second run made zero agent calls and zero writes. | Go for direct mechanical CLI wiring around the canonical pipeline. |
| Backlog | Injectable pipeline exists; production fetch/model wiring remains in the engine. | Driver integration required; no live backlog claim. |
| Document | Shared module contains decision helpers; full gather/synthesis/write driver remains inline. | Driver extraction required; no live document claim. |
| Recovery | Fresh processes were independent. Separate persisted-session probe resumed the same thread and recalled a marker. | Conversation continuation demonstrated; workflow replay and production write recovery are not supported by this spike. |
| Prose orchestration | No production dispatch or autonomous skill entrypoint executed. | Retain manual lane until phases 3–4 implement and verify the host integration. |

The plan driver's implementation-plan branch intentionally has no parent-intent
channel. Its canonical missing-intent suggestion is preserved in the evidence;
three of three selected dimensions is not a claim that intent alignment ran.
Likewise, code fixture coverage is three selected dimensions, not all possible
dimensions for an arbitrary change.

All 12 judgment calls had distinct thread IDs. Peak concurrency was two; the
runner checked that every code finder finished before any code refuter began.
The fixture remained clean. From first call start to final call completion the
experiment took 148.688 seconds. Summed raw usage across those independent calls
was 520,117 input tokens (315,648 cached) and 3,714 output tokens. These include
host context/tool turns and are not a per-prompt or fixed-input billing promise.
Actual billed cost and context-window occupancy were not exposed. There is no
claim of performance parity or superiority over the retiring dispatch engine.

The resume probe reported input/output usage of 15,777/20 initially and
31,590/40 after resume, with 15,616 cached input tokens on resume. Treat these
as raw reported usage observations, not an assumption about per-turn versus
cumulative billing. It proves memory of one marker in the same conversation;
it does not prove cached-prefix workflow replay or exactly-once side effects.
Finders and refuters never reuse that probe's session.

## Failure handling and limits

Credential-free tests use a fake executable to inject malformed/truncated JSONL,
failed turns, duplicate/missing completion, schema errors, invalid models,
authentication/rate failures, process death, timeout, cancellation, and excessive
output. These are synthetic failures, not observed provider outages. Tests
verify child-group termination, waiting for shutdown, bounded concurrency, and
resume identity. The real CLI probe separately established that the schema
translation and fresh-thread protocol work with this installed version.

Canonical schemas have optional properties; Codex strict output schemas require
all properties. The experiment makes optional fields nullable on the wire,
removes only those optional nulls, then validates the original closed schema.
Unsupported schema keywords fail closed. This is a deliberately limited schema
contract, not a general JSON Schema implementation.

No child receives permission to mutate the experiment's source repo. Only the
parent performs explicit argv-based fixture CLI writes. Rating targets and the
entire batch are validated before any estimate write; unknown or incomplete
ratings cannot partially update a valid sibling. Estimates use core-derived
tiers and never write shared Claude/Codex model preferences.

Incomplete review coverage, absent AC tables, refuter failure, and budget
overflow fail the experiment even if current canonical policy would report an
otherwise clean result. The canonical policy is unchanged. A single review
round is passed explicitly to classification so an unperformed rework round
cannot be mistaken for a clean review. That correction followed the initial
live run and has a dedicated failing-then-passing regression test.

Recovery is stop-and-inspect: the runner awaits child shutdown and reports
failure without automatically retrying writes. Retain the printed temporary
fixture and its call evidence, inspect actual RDM state, and start a new
isolated experiment. A partial fixture is never production completion. Phase 3
must design run/side-effect ownership and readback before enabling recovery
against user data. SIGKILL or machine failure cannot promise cleanup.

Only one model/effort pair was tested live. CLI flags provide the execution seam;
host-specific per-step configuration and validation of supported combinations
remain phase 3. Read-only shell execution is not equivalent to Claude's
`rdm-mechanical` tool restriction (Bash plus StructuredOutput), and filesystem
sandboxing alone does not establish restrictions on every connector/tool.

## Remaining driver work

- `review.mjs` exports the canonical find/refute pipeline. The standalone code
  engine still owns diff acquisition, classification/reporting, and optional
  status mutation inline.
- `plan-review.mjs` exports its full driver, but persisted-target fetch, act,
  notes, and gate writes still require mechanical host translation. The live
  report-only test does not validate those effects.
- `estimate.mjs` and `backlog.mjs` expose pipelines; the engines retain bootstrap
  and mechanical dependency wrappers. This spike replaces estimate's mechanical
  operations only for an isolated fixture.
- `document.mjs` exports helpers. Its engine still contains the driver, prompts,
  and schemas; injecting four functions into the helper module cannot port it.

Phase 3 should integrate these seams only as supported by evidence, preserving
fresh review contexts, explicit cwd/session identity, direct mechanical reads,
permission boundaries, and safe recovery. Phase 4 should enable each entrypoint
only after its actual behavior is verified. Both must consume successor
persisted-review/core-gate interfaces when landed; no permanent Codex version
of the retiring dispatch engine is needed.

## Reproduction and checks

From this checkout, carry the explicit development environment required by
[Codex support](codex-support.md). The live command uses saved CLI authentication
and performs model calls; the test command does not.

```sh
cargo nextest run -p rdm-cli --test codex_runtime --test codex_estimate
node --test scripts/lib/codex-spike-process.test.mjs
node scripts/run-codex-orchestration-spike.mjs gpt-6-astra medium
```

The review/estimate tests moved to Rust in phase 4; the legacy process suite
remains visible pending its migration. See the [case map](codex-test-migration.md).
The live results below remain historical evidence, not a claim of a new live run.

The runner prints its temporary evidence directory and leaves fixtures for
inspection. The resume probe uses exported `runCodex`: make a call with
`persistSession: true`, then a second call with `resumeThreadId` equal to its
returned thread ID, the same schema/model/effort, and a prompt asking for the
previous marker. Raw outputs are included separately in the published record.

Validation included the 32 new deterministic tests, the canonical review and
estimate harnesses, `cargo fmt --check`, `cargo clippy -- -D warnings`, and 137
filtered `rdm-core` agent-config tests. The estimate harness initially failed
its existing real-binary assertion (one of two expected estimates returned);
an immediate rerun passed without any harness or canonical-source change.
Its transient failure is recorded rather than silently relabeled a first-pass
success. New tests are wired into CI. No shell or generated policy file changed.
