//! dispatch-phase — pure decision logic for the keystone dispatch pipeline.
//!
//! This is the **single source of truth** for the deterministic decision core of
//! the dispatch-phase workflow: given the surviving findings from the plan-review
//! and code-review gates, decide the phase OUTCOME. Because the Claude Code
//! Workflow runtime cannot `import`/`require` (see docs/workflow-schemas.md
//! § "Import spike"), the marked block below is copied BYTE-IDENTICAL into
//! `.claude/workflows/rdm-wf-dispatch-phase.js`. Unlike the review-refute-fix block —
//! which is stamped by `scripts/gen-workflow-review.sh` — this second block is NOT
//! run through the generator; instead `scripts/verify-workflow-dispatch.sh` gates
//! the two copies for byte-equality, which achieves the same drift protection
//! without teaching the generator a second block.
//!
//! Everything the block needs is self-contained (no imports, pure array/string
//! ops, no Date.now / Math.random). The `export { … }` at the bottom lives
//! OUTSIDE the markers so it is never copied into the workflow script (whose only
//! permitted export is `meta`). The verify harness imports this module and unit-
//! tests the pure logic with fabricated ranked finding arrays — zero LLM calls.
//!
//! The verdict half of the decision core — `classifyOutcome` and its helpers
//! `hasBlocking`, `summarizeFindings`, `codeReviewRounds`, and
//! `DEFAULT_MAX_CODE_REWORK` — was LIFTED into `lib/review.mjs`, the canonical
//! review source, so every surface shares one classifier. In the `.js` consumer
//! those names arrive via the stamped review block (which is positioned BEFORE
//! this block, since `const DEFAULT_MAX_CODE_REWORK` is TDZ-bound and, unlike a
//! function declaration, does not hoist). In Node they arrive via the import
//! below, which lives OUTSIDE the markers and is re-exported for the harness.

import {
  UNREFUTED_DISPOSITION,
  classifyOutcome,
  codeReviewRounds,
  hasBlocking,
  acTableHasGap,
  summarizeFindings,
  statusFor,
  writesCompletion,
  GATE_POLICY,
  deriveSignals,
  DEFAULT_MAX_CODE_REWORK,
  DEFAULT_MAX_REFUTATIONS,
  buildReviewBudget,
  budgetSummaryClause,
  buildReviewCoverage,
  coverageSummaryClause,
} from './review.mjs';

// >>> dispatch-outcome:begin <<<
// Pure, deterministic decision logic for the dispatch-phase pipeline.
//
// This block is the single source of truth in
// .claude/workflows/lib/dispatch-phase.mjs and is copied BYTE-IDENTICAL into
// .claude/workflows/rdm-wf-dispatch-phase.js (the Workflow runtime cannot load modules
// at run time). scripts/verify-workflow-dispatch.sh gates the two copies for
// drift. No Date.now / Math.random — pure array/string ops only.
//
// `hasBlocking`, `summarizeFindings`, `codeReviewRounds`, `classifyOutcome`,
// `statusFor`, `writesCompletion`, `DEFAULT_MAX_CODE_REWORK`,
// `DEFAULT_MAX_REFUTATIONS`, `buildReviewBudget`, `budgetSummaryClause`,
// `buildReviewCoverage`, and `coverageSummaryClause` are NOT declared here: they
// belong to the canonical review source (lib/review.mjs) and reach this block
// from the stamped review block that precedes it in the workflow consumer.

// DEFAULT_MAX_PLAN_REVISE — the in-run plan-revision budget. It is counted
// INDEPENDENTLY of the code-rework budget (DEFAULT_MAX_CODE_REWORK, which lives
// in the review source): a plan that took two revisions consumes no code-rework
// budget, and vice versa.
//
// A budget of N means N reworks AFTER the original attempt, i.e. N + 1 attempts:
//   plan: plan → review → revise 1 → review → revise 2 → review → escalate
//   code: implement → review → rework 1 → review → rework 2 → review → rework
//
// 0 is legal and MEANINGFUL: no reworks at all — terminate on the first blocking
// review. It must never be conflated with "unset" by a falsy check.
const DEFAULT_MAX_PLAN_REVISE = 2;

// --- Environment args: `rdmBin` and `project` --------------------------------
//
// dispatch-phase names NO particular rdm executable and NO particular rdm
// project. Both arrive as RUNTIME args (`rdmBin`, `project`) and are threaded
// into every prompt that shells out, via the `cfg` object each prompt builder
// takes as its trailing parameter. This is the CONTRACT the rest of the
// project-agnostic lane consumes — do not re-derive it elsewhere.
//
// PROJECT-AGNOSTIC ALLOW-LIST — the subcommands that must carry NO project
// flag, because rdm rejects `--project` on them outright:
//
//     rdm model resolve, rdm config get, rdm commit
//     (and rdm status / rdm discard, if added)
//
// EVERY other subcommand this lane emits is PROJECT-SCOPED and takes the flag:
// phase list/show/update, task list/show/create/update, worktree add, next,
// search. A blanket append would produce commands that fail at runtime while
// still satisfying a naive whole-file grep, which is why the allow-list is
// asserted as DATA by scripts/verify-workflow-dispatch.sh § 9b rather than by
// grepping every line.
//
// Canonical write-up: docs/workflow-schemas.md
// § "Environment args: `rdmBin` and `project`".

// projectFlag(cfg) — the ` --project <name>` suffix to append to a
// PROJECT-SCOPED command, or '' when no project was configured (rdm's standard
// resolution chain then applies). Same shape as .claude/workflows/rdm-wf-backlog.js.
function projectFlag(cfg) {
  return cfg && cfg.project ? ' --project ' + cfg.project : '';
}

// resolveRdmBin(value) — resolve the rdm executable to invoke. An ABSENT value
// DEFAULTS to a plain `rdm` on PATH.
//
// This deliberately reverses an earlier fail-closed stance. That stance guarded
// a REAL hazard — inside THIS repo a bare `rdm` is a stale installed build the
// development-build rule forbids — but the hazard is DOGFOOD-SCOPED. A
// downstream consumer who installs the plugin has no repo-local build path to
// pass, and PATH is the right answer for essentially all of them, so charging
// every consumer for this repo's hazard was the wrong trade.
//
// The compensating control now lives where the hazard does: `RDM_BIN` in this
// repo's `.mise.toml` pins the local development build, and the CALLING SKILL
// resolves it before invoking a workflow (the Workflow runtime has no env or
// filesystem access, so this function never reads it). That entry is gated by
// scripts/verify-workflow-dispatch.sh § 9c-dogfood.
//
// An existence preflight remains FORBIDDEN and the default must stay a plain
// fallback, never a probe. A probe would not close the dogfood hazard anyway:
// the stale global EXISTS, so the check passes while running the wrong binary.
//
// A present-but-wrong-TYPE value still throws. Degrading a `rdmBin: 42` typo to
// PATH would reintroduce exactly the silent-wrong-binary hazard this contract
// exists to avoid, and the absent-value default does not need it. Canonical
// write-up, including the three-step resolution order the calling skill
// implements: docs/workflow-schemas.md § "Environment args: `rdmBin` and
// `project`".
function resolveRdmBin(value) {
  if (typeof value === 'string' && value.trim() !== '') return value;
  if (value === undefined || value === null || typeof value === 'string') return 'rdm';
  throw new Error(
    'dispatch-phase: rdmBin must be a string path to the rdm executable (a repo-local build path, ' +
      'or the sentinel "rdm" to request PATH resolution explicitly). Omit it entirely to default to ' +
      '`rdm` on PATH; a non-string value is a caller bug and is refused rather than guessed.'
  );
}

// parseProjectArg(value) — validate the OPTIONAL project name. Any falsy value
// (absent / null / '' / 0 / false) means "emit no project flag at all" — never
// ` --project false`. Anything else must be a plain project name: the value is
// interpolated into a Bash-agent prompt, so whitespace and shell metacharacters
// are rejected rather than escaped.
function parseProjectArg(value) {
  if (!value) return '';
  if (typeof value !== 'string' || !/^[A-Za-z0-9._-]+$/.test(value)) {
    throw new Error(
      'dispatch-phase: project must be a plain project name matching /^[A-Za-z0-9._-]+$/ (got "' +
        String(value) +
        '")'
    );
  }
  return value;
}

// parseBudget(value, flag, fallback) — validate a per-run budget override.
// Unset (null/undefined/'') falls back to the caller's default. Anything else
// must be a non-negative integer; a non-integer string is REJECTED rather than
// silently coerced (parseInt('2abc') === 2 is exactly the trap to avoid).
function parseBudget(value, flag, fallback) {
  if (value === null || value === undefined || value === '') return fallback;
  let n = NaN;
  if (typeof value === 'number') {
    n = value;
  } else if (typeof value === 'string' && /^[+-]?[0-9]+$/.test(value.trim())) {
    n = parseInt(value.trim(), 10);
  }
  if (!Number.isInteger(n) || n < 0 || Object.is(n, -0)) {
    throw new Error(
      'dispatch-phase: ' +
        flag +
        ' must be a non-negative integer (got "' +
        String(value) +
        '") — 0 means no reworks, terminate on the first blocking review'
    );
  }
  return n;
}

// parseDispatchArgs(args) — coerce and validate the whole args payload.
//
// The Workflow tool contract forbids stringified args, but LLM callers (rdm-do
// --auto and hand-run single phases) invoke dispatch-phase DIRECTLY and have
// delivered a JSON string; coerce once, then derive every field from it. Budget
// validation runs HERE, at parse time — before any agent() call — so an invalid
// budget can never burn tokens.
function parseDispatchArgs(args) {
  let dispatchArgs = args || {};
  if (typeof dispatchArgs === 'string') {
    try {
      dispatchArgs = JSON.parse(dispatchArgs) || {};
    } catch (e) {
      dispatchArgs = {};
    }
  }
  if (!dispatchArgs || typeof dispatchArgs !== 'object') dispatchArgs = {};
  // The two ENVIRONMENT axes are resolved BEFORE the returned object literal, so
  // validation order is deterministic and independent of property-evaluation
  // order: rdmBin first (defaulting to `rdm` when absent — see resolveRdmBin
  // above), then the optional project name. Like the budgets, both are validated
  // HERE, at parse time, so a mis-invocation costs zero tokens.
  const rdmBin = resolveRdmBin(dispatchArgs.rdmBin);
  const project = parseProjectArg(dispatchArgs.project);
  return {
    roadmap: dispatchArgs.roadmap || '',
    phase: dispatchArgs.phase || '',
    // Task mode: `{ task: <slug> }` dispatches a standalone task instead of a
    // phase — no roadmap, no tier, its own `task/<slug>` worktree.
    task: dispatchArgs.task || '',
    planOnly: !!dispatchArgs.planOnly,
    maxPlanRevise: parseBudget(dispatchArgs.maxPlanRevise, 'maxPlanRevise', DEFAULT_MAX_PLAN_REVISE),
    maxCodeRework: parseBudget(dispatchArgs.maxCodeRework, 'maxCodeRework', DEFAULT_MAX_CODE_REWORK),
    // Per-unit REFUTATION budget (how many gating findings each review round
    // grades), threaded into BOTH review contexts. Validated here, at parse
    // time — before any agent() call — exactly like the two retry budgets, so an
    // invalid value can never burn tokens. 0 is legal and MEANINGFUL (grade
    // nothing; every gating finding passes through un-refuted), and must never
    // be conflated with "unset" by a falsy check.
    maxRefutations: parseBudget(dispatchArgs.maxRefutations, 'maxRefutations', DEFAULT_MAX_REFUTATIONS),
    // --- Optional caller-supplied hoists (see docs/mechanical-agent-inventory.md).
    // A caller that is ALREADY a running agent with the repo in context (the
    // rdm-dispatch-phase / rdm-do --auto shims) can run the mechanical command
    // itself and pass the result here, so the workflow never spawns a dedicated
    // subagent for it. Every one of these is OPTIONAL: absent/malformed simply
    // falls through to the in-workflow agent, which is exactly what a direct
    // `Workflow` invocation (no caller) does.
    phaseMeta: dispatchArgs.phaseMeta && typeof dispatchArgs.phaseMeta === 'object' ? dispatchArgs.phaseMeta : null,
    taskMeta: dispatchArgs.taskMeta && typeof dispatchArgs.taskMeta === 'object' ? dispatchArgs.taskMeta : null,
    // The caller already wrote `--status in-progress` itself and it exited 0, so
    // the workflow's own observability stamp is redundant. NEVER set by a
    // --plan-only invocation (the workflow suppresses the stamp there anyway).
    alreadyInProgress: !!dispatchArgs.alreadyInProgress,
    // --- Environment axes (see the contract block above).
    // `rdmBin` is OPTIONAL — the rdm executable every Bash-agent prompt shells
    // out to, defaulting to a plain `rdm` on PATH when absent. `project` is
    // OPTIONAL and applies ONLY to project-scoped subcommands; '' means "emit
    // no project flag".
    rdmBin: rdmBin,
    project: project,
  };
}

// hoistedMetaComplete(meta, isTask) — the ALL-OR-NOTHING guard on a caller-
// supplied phase/task meta payload. A hoisted meta replaces a fetch agent that
// did TWO things: read the item body AND resolve the five per-step model ids.
// Accepting a partial payload would therefore save nothing (the driver would
// still need a model-resolving agent) while actively breaking the run: an
// incomplete `models` map trips the driver's `unresolvedStep` check and
// short-circuits the whole dispatch as a fetchError. So: accept only when the
// body is a non-empty string AND all five model ids are non-empty strings —
// otherwise reject and let the original agent run untouched.
//
// `isTask` selects between the two schemas' requirements. A TASK_META payload
// carries no roadmap/stem/model tier and the driver hard-codes a task's tier to
// `medium`, so body + models are all it needs. A PHASE_META payload is
// different: `meta.model` is the phase's DIFFICULTY TIER, and it is the driver's
// SOLE source for it — unlike `stem`/`roadmap`, which fall back to values the
// top-level args already carry, an absent tier falls back to a hard-coded
// 'medium'. That default is not neutral: `hasBlocking` scales with the tier, and
// a `large` phase silently downgraded to `medium` stops treating a surviving
// `concern` finding as blocking — loosening the gate, the opposite direction
// from the one-directional tightening this gate exists to uphold. So the phase
// case ALSO requires a non-empty string `model`, mirroring PHASE_META_SCHEMA's
// own `required` list; anything short of that falls back to the fetch agent,
// which always supplies it.
function hoistedMetaComplete(meta, isTask) {
  if (!meta || typeof meta !== 'object') return false;
  if (typeof meta.body !== 'string' || String(meta.body).trim() === '') return false;
  // The resolved verification command is part of the all-or-nothing contract for
  // the same reason the model ids are: without it the driver would escalate the
  // dispatch as unverifiable even though the in-workflow fetch agent could have
  // resolved one. A hoist that omits it simply falls back to that agent.
  if (extractVerifyCommand(meta) === '') return false;
  // Phase mode only: the difficulty tier has no recoverable fallback.
  if (!isTask && (typeof meta.model !== 'string' || meta.model.trim() === '')) return false;
  const m = meta.models;
  if (!m || typeof m !== 'object') return false;
  const keys = ['plan', 'implement', 'review_find', 'review_verify', 'mechanical'];
  return keys.filter((k) => typeof m[k] !== 'string' || m[k] === '').length === 0;
}

// >>> verify-gate:begin <<<
// --- Phase-time verification gate --------------------------------------------
//
// The project declares ONE command; this block runs it once per implementation
// attempt and interprets the exit code. rdm is NOT a task runner: timeouts,
// ordering, parallelism, per-tool configuration and output formatting all stay
// in whatever the command invokes. The check set is project-supplied DATA, so
// nothing below names a language, package manager, or test tool — that property
// is satisfied by construction and grep-asserted with an occurrence floor by
// scripts/verify-workflow-dispatch.sh § 3-verify. Canonical write-up:
// docs/verify-gate.md.

// VERIFY_OUTPUT_CAP — how much of a failing command's output survives into the
// rework prompt. The TAIL is kept, not the head: failures print last, and an
// unbounded log would blow the rework implementer's context budget.
const VERIFY_OUTPUT_CAP = 4000;

// truncateVerifyOutput(output) — keep the LAST VERIFY_OUTPUT_CAP characters,
// prefixed with an explicit elision marker so a reader never mistakes a
// truncated tail for the whole log. Non-string input yields ''.
function truncateVerifyOutput(output) {
  if (typeof output !== 'string' || output === '') return '';
  if (output.length <= VERIFY_OUTPUT_CAP) return output;
  return '[...output truncated, showing the last ' + VERIFY_OUTPUT_CAP + ' characters...]\n' +
    output.slice(output.length - VERIFY_OUTPUT_CAP);
}

// normalizeVerifyResult(command, raw, agentFailed) — coerce whatever the verify
// dep resolved into the stable shape the rest of the block consumes:
//   { command, ran, failed, exitCode, output }
//
// FAIL-CLOSED: a thrown dep, a null/undefined resolution (agent() resolves null
// on an unknown model id rather than throwing), and a payload with no integer
// exit code are ALL treated as a failure. An unrunnable declared verification
// must never read as a pass — that is the exact silent-success this gate exists
// to remove.
function normalizeVerifyResult(command, raw, agentFailed) {
  const cmd = typeof command === 'string' ? command : '';
  const r = raw && typeof raw === 'object' ? raw : null;
  const code = r && typeof r.exitCode === 'number' && Number.isInteger(r.exitCode) ? r.exitCode : null;
  const out = r && typeof r.output === 'string' ? r.output : '';
  if (code === null) {
    return {
      command: cmd,
      ran: false,
      failed: true,
      exitCode: -1,
      output: truncateVerifyOutput(
        out ||
          (agentFailed
            ? 'the verification agent threw before reporting an exit status'
            : 'the verification agent reported no usable exit status')
      ),
    };
  }
  return { command: cmd, ran: true, failed: code !== 0, exitCode: code, output: truncateVerifyOutput(out) };
}

// verifyFailureFinding(result) — synthesize a FINDING-shaped object from a
// failed verify result, so the UNTOUCHED classifier resolves the round to
// `rework` with no new OUTCOME value and no classifier branch.
//
// It is added AFTER runReview returns, so it never passes through the refuter
// or the confidence floor — it is a mechanical fact, not a graded judgment.
// The command string lives in `what_fails`, which is exactly the field
// summarizeFindings renders, so the OUTCOME `summary` (and therefore
// outcomePolicy's `reason`) names the command with no change to any shared
// helper.
function verifyFailureFinding(result) {
  const r = result || {};
  const tail = r.output ? '\n--- verification output (tail) ---\n' + r.output : '';
  return {
    id: 'verify-command-failed',
    concern: 'verify',
    severity: 'blocking',
    confidence: 100,
    what_fails: 'the project verification command `' + r.command + '` exited ' + r.exitCode,
    failure_scenario:
      'Running `' + r.command + '` in the item worktree exits ' + r.exitCode + ' instead of 0.' + tail,
    suggested_fix:
      'Make `' + r.command + '` exit 0 from a clean worktree, then re-run it before finishing.',
    unrefuted: true,
    unrefutedReason: 'mechanical',
  };
}

// extractVerifyCommand(meta) — pull the resolved command out of a fetched (or
// caller-hoisted) metadata object. A trimmed non-empty single-line string, or
// '' for anything else (absent, whitespace-only, non-string).
//
// A value containing a newline is REFUSED rather than run: the value is
// interpolated into a Bash-agent prompt, and a multi-line command belongs in a
// script that the one declared command invokes. Refusing yields '', which
// escalates — never a silent partial run.
function extractVerifyCommand(meta) {
  if (!meta || typeof meta !== 'object') return '';
  const v = meta.verify;
  if (typeof v !== 'string') return '';
  const trimmed = v.trim();
  if (trimmed === '' || /[\r\n]/.test(trimmed)) return '';
  return trimmed;
}

// verifyToolingLine(command) — the AVAILABLE TOOLING line handed to BOTH the
// first-pass implementer and the rework implementer, so the command is run
// proactively rather than discovered as a failure. '' for an empty command.
function verifyToolingLine(command) {
  const cmd = typeof command === 'string' ? command.trim() : '';
  if (cmd === '') return '';
  return (
    'AVAILABLE TOOLING — this project declares exactly one verification command: `' + cmd + '`. ' +
    'Run it yourself, in full, before you finish. The pipeline runs the same command once after you ' +
    'return, and a non-zero exit sends this item straight back to you as rework.'
  );
}

// verifyFailureClause(result) — the rework-only clause naming the command, its
// exit code, and the truncated output tail, so the rework implementer sees the
// failing output and not merely the command. '' when the verify passed, was
// never run, or is absent.
function verifyFailureClause(result) {
  const r = result || {};
  if (!r || r.failed !== true) return '';
  const tail = r.output ? '\n' + r.output : '\n(no output was captured)';
  return (
    'The verification command `' + r.command + '` exited ' + r.exitCode +
    ' on the prior pass — fix that first; the item cannot be reported reviewed until it exits 0. ' +
    'Its output (tail):' + tail
  );
}

// VERIFY_UNRESOLVED_SUMMARY — the escalation note for a dispatch that could
// determine NO way to verify itself. Names the declared key and all three
// discovery sources so the operator knows exactly what to add.
const VERIFY_UNRESOLVED_SUMMARY =
  'no verification command could be resolved: `dispatch.verify` is unset and nothing was discoverable ' +
  'from .github/workflows/, docs/principles.md, or CLAUDE.md/AGENTS.md — a dispatch that cannot ' +
  'determine how to verify itself must not report success';

// VERIFY_RESOLUTION_LINES — the resolution paragraph BOTH Stage-0 fetch prompts
// append. Declared expression order IS the precedence: the declared key wins;
// otherwise discover, in order, CI config, the principles doc, then the agent
// instruction files; otherwise return an empty string and let the driver
// escalate. Names only file paths and the config key.
function verifyResolutionLines(bin) {
  return [
    'Then resolve this project\'s single VERIFICATION COMMAND — the one command whose exit code says',
    'whether the repository is healthy. Run exactly this command first and read its output:',
    '  ' + bin + ' config get dispatch.verify',
    'If it prints a real value (anything other than "(not set)"), return that value VERBATIM as `verify`.',
    'Otherwise DISCOVER one, checking these sources in order and stopping at the first that yields',
    'anything: (a) the CI configuration under `.github/workflows/` (also `.circleci/config.yml` and',
    '`.gitlab-ci.yml`); (b) `docs/principles.md`; (c) `CLAUDE.md` or `AGENTS.md`. Synthesize ONE',
    'single-line shell command that runs the checks that source names, joined so that any failure',
    'makes the whole command exit non-zero, and return it as `verify`.',
    'If none of the three sources names any checks, return `verify` as an EMPTY STRING. Never invent a',
    'command, never return a multi-line value, and never guess.',
  ];
}

// VERIFY_RESULT — what the mechanical verification agent reports back: the exit
// status of the ONE command it was told to run, plus that command's output tail.
const VERIFY_RESULT_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['exitCode', 'output'],
  properties: {
    exitCode: { type: 'integer' },
    output: { type: 'string' },
  },
};

// buildVerifyPrompt(command, worktreeRef, cfg) — run the resolved command ONCE
// in the item's worktree and report its exit status. Deliberately forbids
// decomposition, reordering, partial runs, retries, and any fix attempt: this
// agent observes, it does not repair. `cfg` is the environment payload
// { rdmBin, project }; `worktree add` is project-scoped and carries the flag.
function buildVerifyPrompt(command, worktreeRef, cfg) {
  const bin = resolveRdmBin(cfg && cfg.rdmBin);
  const proj = projectFlag(cfg);
  return [
    'You are a mechanical verification agent. Do not plan, implement, review, or fix anything, and edit no files.',
    'Find the worktree for this item and work THERE:',
    '  ' + bin + ' worktree add ' + worktreeRef + proj,
    '(it prints the existing path if the worktree already exists) then `cd` into that path.',
    'Run EXACTLY this one command, verbatim, exactly once:',
    '  ' + command,
    'Do not decompose it into parts, do not reorder it, do not run only some of it, do not substitute',
    'a faster equivalent, and do not re-run it on failure.',
    'Return a VERIFY_RESULT object: `exitCode` — the integer exit status you actually observed; and',
    '`output` — the combined stdout and stderr TRUNCATED to the LAST 4000 characters (keep the END,',
    'where failures print). If the command could not be run at all, report a non-zero `exitCode` and',
    'say why in `output`. Never report 0 for a command that failed or that you did not run.',
  ].join('\n');
}
// >>> verify-gate:end <<<

// runPlanGate(config, deps) — the bounded plan stage. Author a plan, review it,
// and revise up to `config.maxRevise` times, breaking early the moment a review
// comes back with no blockers. Returns
// { fetchError, stage, planDoc, findings, reviewCount, reviseCount }.
//
// Every side effect is reached through the injected `deps` (d.plan / d.revise /
// d.review), so this block names NO ambient runtime global and the module
// imports cleanly in Node — the lib/autopilot.mjs precedent, which is what makes
// the budget loop testable at all.
//
// `d.review(planDoc)` is a `runReview` from the canonical review source and
// therefore resolves `{ survivors, acTable }`, not a bare array — both call
// sites below destructure it. `acTable` is discarded: plan mode never sets it
// (the `ac` dimension does not exist there), and `hasBlocking`'s
// `Array.isArray(findings)` guard would otherwise silently see a non-array and
// report no blocking findings at all, permanently defeating the plan gate's
// escalation path.
//
// agent() RESOLVES to null on an unknown/unavailable model id rather than
// throwing (spike consequence 3), so BOTH the initial plan and EVERY revise
// result are null-guarded. The revise guard runs before the reassignment and
// before the next review, so a null doc is never reviewed as an empty plan and
// never clobbers the last good one.
async function runPlanGate(config, deps) {
  const c = config || {};
  const d = deps || {};
  const maxRevise = c.maxRevise != null ? c.maxRevise : DEFAULT_MAX_PLAN_REVISE;
  const tier = c.tier;
  let planDoc = await d.plan();
  if (planDoc === null || planDoc === undefined) {
    return { fetchError: true, stage: 'plan', planDoc: null, findings: [], reviewCount: 0, reviseCount: 0 };
  }
  let reviewResult = (await d.review(planDoc)) || {};
  let findings = reviewResult.survivors || [];
  // The per-round refutation-budget accounting the review pipeline returns as
  // its third field. Captured per round so a plan gate that hit its bound is
  // visible even when a later round did not.
  const budgetRounds = [reviewResult.budget || null];
  // The per-round DIMENSION-PARTICIPATION accounting the review pipeline returns
  // as its fourth field, captured exactly like budgetRounds so a round that lost
  // a dimension stays visible even when a later round ran clean.
  const coverageRounds = [reviewResult.coverage || null];
  let reviewCount = 1;
  let reviseCount = 0;
  for (let i = 0; i < maxRevise; i++) {
    if (!hasBlocking(findings, tier)) break;
    const revised = await d.revise(planDoc, findings);
    reviseCount++;
    if (revised === null || revised === undefined) {
      return {
        fetchError: true,
        stage: 'revise',
        planDoc: planDoc,
        findings: findings,
        budgetRounds: budgetRounds,
        budget: budgetRounds[budgetRounds.length - 1],
        coverageRounds: coverageRounds,
        coverage: coverageRounds[coverageRounds.length - 1],
        reviewCount: reviewCount,
        reviseCount: reviseCount,
      };
    }
    planDoc = revised;
    reviewResult = (await d.review(planDoc)) || {};
    findings = reviewResult.survivors || [];
    budgetRounds.push(reviewResult.budget || null);
    coverageRounds.push(reviewResult.coverage || null);
    reviewCount++;
  }
  return {
    fetchError: false,
    stage: null,
    planDoc: planDoc,
    findings: findings,
    budgetRounds: budgetRounds,
    budget: budgetRounds[budgetRounds.length - 1],
    coverageRounds: coverageRounds,
    coverage: coverageRounds[coverageRounds.length - 1],
    reviewCount: reviewCount,
    reviseCount: reviseCount,
  };
}

// runCodeGate(config, deps) — the bounded code stage. Implement, review, and
// rework up to `config.maxRework` times, breaking early on a clean review.
// Returns { findings, rounds, acRounds, reworkCount, reviewCount, actResult }
// where `rounds`/`acRounds` are the per-round review findings / AC tables in
// order (always at least one entry each).
//
// `d.review()` is a `runReview` from the canonical review source and therefore
// resolves `{ survivors, acTable }`, not a bare array — every round destructures
// it. The rework-loop continuation checks BOTH `hasBlocking` and
// `acTableHasGap`: an AC-only gap (no blocking finding at all) must still
// consume the rework budget instead of exiting after round 1 and reporting
// `rework` without ever attempting a fix.
//
// No null guard is needed here: `implement` returns no document the pipeline
// consumes, and the review pipeline already converts an all-null finder sweep
// into a blocking finding.
//
// Act step: once the loop settles on a CLEAN final round (no blocking finding,
// no AC-table gap) with non-empty survivors, the optional `d.act` dep is
// invoked exactly once to incorporate them by size (small → fixed inline,
// large → filed as a task — see buildCodeActPrompt). This never runs on a
// still-blocking/AC-gapped round, whatever caused it (still-blocking findings
// and unresolved AC gaps are handled by the rework/status machinery, not this
// step — "never fix large changes inline" stays intact). A missing `act` dep or
// a thrown Act call is swallowed: concern/suggestion findings are non-gating by
// the module's own severity contract, so a failed fix-attempt must never
// change the outcome.
//
// Verify gate: `config.verifyCommand` is the project's single resolved
// verification command. It is run through the optional `d.verify` dep at exactly
// ONE call site (implementAndVerify below), immediately after every
// `d.implement(...)` and before the corresponding `d.review()`, so it fires
// exactly once per implementation attempt on the first pass and on every rework
// round alike. A failure is FAIL-CLOSED (a throw, a null resolution, or a
// missing exit code all count) and is folded into the round as a synthesized
// blocking finding, so the UNTOUCHED classifier resolves it to `rework` — no new
// OUTCOME value, and no second budget: the existing `maxRework` bound is what
// terminates a repeatedly failing verification.
//
// Rework notes: `d.implement` is called with `null` for the first pass and
// `{ findings, acTable, verify }` on every rework pass — NEVER a bare findings
// array.
// The AC table is a structured side-channel decoupled from `findings` (a FAIL
// criterion need not also appear as a finding), so without also passing
// `acTable` an AC-only-gap rework (empty `findings`) would hand the
// implementer zero information about what to fix.
async function runCodeGate(config, deps) {
  const c = config || {};
  const d = deps || {};
  const maxRework = c.maxRework != null ? c.maxRework : DEFAULT_MAX_CODE_REWORK;
  const tier = c.tier;
  const verifyCommand = typeof c.verifyCommand === 'string' ? c.verifyCommand.trim() : '';
  // Per-round verify results, parallel to `rounds`, plus the number of times the
  // single call site actually fired. Returned so the harness can assert the
  // one-run-per-attempt contract from the OUTSIDE, without instrumenting a fake.
  const verifyRounds = [];
  let verifyCalls = 0;
  // THE SINGLE VERIFY CALL SITE. Implement and verify are deliberately fused
  // into one helper so the first pass and every rework round route through the
  // same line: no branch can skip it, and no retry loop can multiply it.
  const implementAndVerify = async (notes) => {
    await d.implement(notes);
    if (typeof d.verify !== 'function' || verifyCommand === '') {
      verifyRounds.push(null);
      return null;
    }
    verifyCalls++;
    let raw = null;
    let agentFailed = false;
    try {
      raw = await d.verify(verifyCommand);
    } catch (e) {
      agentFailed = true;
    }
    const normalized = normalizeVerifyResult(verifyCommand, raw, agentFailed);
    verifyRounds.push(normalized);
    return normalized;
  };
  // foldVerify(survivors, verifyResult) — prepend the synthesized blocking
  // finding when the round's verification failed. A NEW array every time, so the
  // per-round records in `rounds` never alias each other.
  const foldVerify = (survivors, verifyResult) => {
    const list = Array.isArray(survivors) ? survivors : [];
    if (!verifyResult || verifyResult.failed !== true) return list;
    return [verifyFailureFinding(verifyResult)].concat(list);
  };
  let verifyResult = await implementAndVerify(null);
  let reviewResult = (await d.review()) || {};
  let findings = foldVerify(reviewResult.survivors || [], verifyResult);
  let acTable = reviewResult.acTable != null ? reviewResult.acTable : null;
  const rounds = [findings];
  const acRounds = [acTable];
  // Per-round refutation-budget accounting, parallel to `rounds`/`acRounds`. The
  // budget re-applies per round, so a round-1 hit that was resolved by round 2
  // stays visible via `everHit` in the OUTCOME.
  const budgetRounds = [reviewResult.budget || null];
  // Per-round dimension-participation accounting, parallel to budgetRounds. A
  // dimension that failed in round 1 stays visible even if round 2 ran complete.
  // It is RECORDED ONLY — the rework-loop continuation below deliberately does
  // NOT consult it, so an incomplete round never consumes an extra rework.
  const coverageRounds = [reviewResult.coverage || null];
  let reworkCount = 0;
  for (let i = 0; i < maxRework; i++) {
    if (!hasBlocking(findings, tier) && !acTableHasGap(acTable)) break;
    // The PRIOR round's verify result rides along in the rework notes (the
    // argument is evaluated before the reassignment below), so the rework
    // implementer sees the failing output, not just the command.
    verifyResult = await implementAndVerify({ findings: findings, acTable: acTable, verify: verifyResult });
    reworkCount++;
    reviewResult = (await d.review()) || {};
    findings = foldVerify(reviewResult.survivors || [], verifyResult);
    acTable = reviewResult.acTable != null ? reviewResult.acTable : null;
    rounds.push(findings);
    acRounds.push(acTable);
    budgetRounds.push(reviewResult.budget || null);
    coverageRounds.push(reviewResult.coverage || null);
  }
  let actResult = null;
  const isClean = !hasBlocking(findings, tier) && !acTableHasGap(acTable);
  if (isClean && findings.length > 0) {
    actResult = d.act ? await d.act(findings).catch(() => null) : null;
  }
  return {
    findings: findings,
    rounds: rounds,
    acRounds: acRounds,
    budgetRounds: budgetRounds,
    coverageRounds: coverageRounds,
    coverage: coverageRounds[coverageRounds.length - 1],
    reworkCount: reworkCount,
    reviewCount: rounds.length,
    verifyCommand: verifyCommand,
    verifyRounds: verifyRounds,
    verifyCalls: verifyCalls,
    verifyResult: verifyResult,
    actResult: actResult,
  };
}

// JSON Schema the code-lane Act step is forced to satisfy: one disposition per
// surviving finding it was asked to incorporate. Mirrors the STAMP_ACK_SCHEMA
// pattern (a small, verifiable acknowledgement) rather than free text.
const CODE_ACT_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['handled'],
  properties: {
    handled: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['id', 'action'],
        properties: {
          id: { type: 'string', minLength: 1 },
          // `skipped` is what an un-refuted (non-gating) finding's disposition
          // rule actually resolves to when the change would be major — without
          // it the act step has no way to record "deliberately not done", and
          // would have to misreport a skip as one of the other two actions.
          action: { type: 'string', enum: ['fixed-inline', 'filed-as-task', 'skipped'] },
          taskSlug: { type: 'string' },
          reason: { type: 'string' },
        },
      },
    },
  },
};

// buildCodeActPrompt(kind, roadmapOrTask, ident, worktreeRef, survivors, cfg) —
// the code-lane Act step: an already-verified surviving finding is incorporated
// by SIZE, not severity (severity already decided the outcome — this decides
// whether/how the finding is acted on). Modeled directly on
// lib/plan-review.mjs's buildActPrompt, but code-review findings are fixed
// inline in the worktree (no whole-document authoritative-body rewrite) and
// large ones are filed with `rdm task create`, not a plan-doc note.
//
// `cfg` is the environment payload `{ rdmBin, project }`. `task create` is a
// PROJECT-SCOPED subcommand, so it carries the project flag.
function buildCodeActPrompt(kind, roadmapOrTask, ident, worktreeRef, survivors, cfg) {
  const bin = resolveRdmBin(cfg && cfg.rdmBin);
  const proj = projectFlag(cfg);
  const target = kind === 'task' ? 'task/' + ident : roadmapOrTask + '/' + ident;
  // Provenance is MIXED once the review passes a non-gating finding through
  // un-refuted, so the LEADING claim has to be conditional: an unconditional
  // "these survived refutation" would be a false statement about part of the
  // payload, not merely an incomplete one. The all-verified branch stays
  // byte-identical to the pre-pass-through prompt.
  const list = Array.isArray(survivors) ? survivors : [];
  const hasUnrefuted = list.some((f) => f && f.unrefuted);
  const lines = hasUnrefuted
    ? [
        'You are acting on code-review findings of MIXED provenance for ' + target +
          ' (worktree: ' + worktreeRef + ').',
        'None of them gates (the reviewed outcome is already decided). A finding WITHOUT `unrefuted: true` ' +
          'survived an independent refuter; a finding WITH it was never graded by one.',
      ]
    : [
        'You are acting on ALREADY-VERIFIED code-review findings for ' + target + ' (worktree: ' + worktreeRef + ').',
        'These findings survived refutation and are non-gating (the reviewed outcome is already decided).',
      ];
  lines.push(
    JSON.stringify(survivors, null, 2),
    'For EACH finding, decide SMALL vs LARGE:',
    '- SMALL — localized, low-risk, no new acceptance criterion (a typo, a missing doc comment, a tightened ' +
      'error message, an extra test). Fix it directly in the worktree at ' + worktreeRef +
      ' and re-run the relevant tests. Do not create a separate landing commit — the fix folds into the ' +
      'eventual land-time commit.',
    '- LARGE — new modules, cross-cutting changes, or anything that would warrant its own acceptance ' +
      'criterion. Do NOT edit code for these: file it with `' + bin + ' task create <slug> --title ' +
      '"Code review finding: <desc>" --body "<details>" --tags code-review --no-edit' + proj + '`.'
  );
  if (hasUnrefuted) {
    lines.push(UNREFUTED_DISPOSITION);
  }
  lines.push(
    hasUnrefuted
      ? 'Return JSON matching the CODE_ACT schema: a `handled` array with ONE entry per finding you were given — ' +
          'id, action (fixed-inline|filed-as-task|skipped), taskSlug when you filed a task, and a one-line ' +
          '`reason` when you skipped one under the rule above.'
      : 'Return JSON matching the CODE_ACT schema: a `handled` array with ONE entry per finding you were given — ' +
          'id, action (fixed-inline|filed-as-task), and taskSlug when you filed a task.'
  );
  return lines.join('\n');
}

// OUTCOME_REASON_PREFIX — which gate a non-clean outcome came out of.
// dispatch-phase's escalations originate at the PLAN gate (classifyOutcome only
// returns 'escalated' from a blocking plan finding, or from a fetch failure
// before any code exists), so they are tagged `[plan]`; an unresolved code
// rework is tagged `[code]`. This deliberately differs from the canonical
// STATUS_MAPPING.reasonPrefix (`[code]`), which describes the INTERACTIVE review
// surface, where an escalation comes out of the code gate. The tag names which
// gate escalated, not which module produced the string.
const OUTCOME_REASON_PREFIX = { escalated: '[plan]', rework: '[code]' };

// outcomePolicy(outcome, kind, summary) — the gate/completion policy owned by the
// canonical review source, projected onto the OUTCOME contract so no consumer
// has to restate the map:
//   status           — the rdm status this outcome maps to for `kind`
//                      ('phase' | 'task'), straight from statusFor().
//   writesCompletion — MAY this outcome's surface write the land-time completion
//                      directive? Expressed ONLY as a boolean, never as the
//                      directive literal: this block is stamped verbatim into
//                      workflow scripts, and the dispatch harness forbids that
//                      literal anywhere in a stamped region. The land-time writer
//                      (`rdm-land`) turns this boolean plus the OUTCOME's
//                      identifiers into the real trailer via `rdm hook done-line`.
//   reason           — a gate-tagged park/escalation note; empty on a clean review.
function outcomePolicy(outcome, kind, summary) {
  const prefix = OUTCOME_REASON_PREFIX[outcome];
  return {
    status: statusFor(outcome, kind),
    writesCompletion: writesCompletion(outcome),
    reason: prefix ? prefix + ' ' + summary : '',
  };
}

// annotateHandled(findings, actResult) — realize the guideline "for each
// finding, state how it was handled (fixed-inline / filed-as-task)" for the
// mechanical code lane: stamp a `handled` field onto each finding from the
// matching `actResult.handled` entry (by `id`), defaulting to `'unhandled'`
// when the Act step wasn't run, failed, or didn't report that specific
// finding. Pure and order-preserving; a no-op (returns `findings` unchanged)
// when `actResult` carries no usable `handled` array.
function annotateHandled(findings, actResult) {
  if (!actResult || !Array.isArray(actResult.handled)) return findings;
  const actionById = {};
  actResult.handled.forEach((h) => {
    if (h && h.id) actionById[h.id] = h.action;
  });
  return findings.map((f) => ({ ...f, handled: (f && f.id && actionById[f.id]) || 'unhandled' }));
}

// buildOutcome — the OUTCOME contract { roadmap, phase, outcome, status,
// writesCompletion, summary, reason, findings }. fetchError short-circuits to
// escalated. Never emits a land-time completion directive — it emits the
// `writesCompletion` boolean instead, and `rdm-land` writes the trailer.
function buildOutcome(input) {
  const i = input || {};
  const roadmap = i.roadmap;
  const phase = i.phase;
  const tier = i.tier;
  if (i.fetchError === true) {
    const failSummary = 'phase fetch failed';
    const failPolicy = outcomePolicy('escalated', 'phase', failSummary);
    return {
      roadmap: roadmap,
      phase: phase,
      outcome: 'escalated',
      status: failPolicy.status,
      writesCompletion: failPolicy.writesCompletion,
      summary: failSummary,
      reason: failPolicy.reason,
      reviewBudget: buildReviewBudget(i.budgetRounds, i.planBudget),
      reviewCoverage: buildReviewCoverage(i.coverageRounds, i.planCoverage),
      findings: [],
    };
  }
  // A dispatch that could determine NO verification command escalates rather
  // than skipping verification. Structured exactly like the fetchError branch
  // above and reusing the SAME `escalated` value via outcomePolicy — it adds no
  // OUTCOME value, no classifier branch, and no GATE_POLICY row.
  if (i.verifyUnresolved === true) {
    const verifyPolicy = outcomePolicy('escalated', 'phase', VERIFY_UNRESOLVED_SUMMARY);
    return {
      roadmap: roadmap,
      phase: phase,
      outcome: 'escalated',
      status: verifyPolicy.status,
      writesCompletion: verifyPolicy.writesCompletion,
      summary: VERIFY_UNRESOLVED_SUMMARY,
      reason: verifyPolicy.reason,
      reviewBudget: buildReviewBudget(i.budgetRounds, i.planBudget),
      reviewCoverage: buildReviewCoverage(i.coverageRounds, i.planCoverage),
      findings: [],
    };
  }
  const planFindings = i.planFindings || [];
  const acRounds = i.acRounds || [];
  const lastAcTable = acRounds.length ? acRounds[acRounds.length - 1] : null;
  const classifierInput = {
    planFindings: planFindings,
    codeFindings: i.codeFindings,
    codeFindingsAfterRework: i.codeFindingsAfterRework,
    codeReviews: i.codeReviews,
    maxRework: i.maxRework,
    tier: tier,
    acTable: lastAcTable,
  };
  const outcome = classifyOutcome(classifierInput);
  // The LAST code-review round is what both the rework and reviewed payloads
  // report — never a stale earlier pass, whatever the rework budget was.
  const rounds = codeReviewRounds(classifierInput);
  const lastRound = rounds[rounds.length - 1] || [];
  let findings;
  let summary;
  if (outcome === 'escalated') {
    findings = planFindings;
    summary = 'plan gate escalated: ' + summarizeFindings(planFindings);
  } else if (outcome === 'rework') {
    findings = lastRound;
    // An AC-only gap can force `rework` with an EMPTY lastRound findings
    // array (no blocking finding at all) — summarizeFindings([]) would then
    // misleadingly read "no surviving findings". Name the real cause instead.
    summary =
      lastRound.length === 0 && acTableHasGap(lastAcTable)
        ? 'code rework unresolved: unmet acceptance criteria in AC table'
        : 'code rework unresolved: ' + summarizeFindings(lastRound);
  } else {
    findings = annotateHandled(lastRound, i.actResult);
    summary = 'phase reviewed clean: ' + summarizeFindings(lastRound);
  }
  // The bound is appended to the summary in ALL THREE branches, and only when a
  // round actually hit it — a run that stayed under budget keeps a
  // byte-unchanged summary. Because outcomePolicy derives `reason` from
  // `summary`, a parked/escalated budget-hit unit surfaces it in the
  // `rdm review blocked` queue for free.
  const reviewBudget = buildReviewBudget(i.budgetRounds, i.planBudget);
  summary = summary + budgetSummaryClause(reviewBudget);
  // The coverage clause is appended in ALL THREE branches too, immediately AFTER
  // the budget clause so the ordering is fixed and deterministic for a run that
  // hit both. It is EMPTY when every round ran every dimension, so a healthy
  // run's summary stays byte-unchanged. Because outcomePolicy derives `reason`
  // from `summary`, a parked/escalated unit whose review lost a dimension
  // surfaces that in the `rdm review blocked` queue for free.
  //
  // This is the ONLY thing coverage does here: it never reaches classifyOutcome's
  // input, and no branch gates on it. Recorded, never gated on.
  const reviewCoverage = buildReviewCoverage(i.coverageRounds, i.planCoverage);
  summary = summary + coverageSummaryClause(reviewCoverage);
  const policy = outcomePolicy(outcome, 'phase', summary);
  return {
    roadmap: roadmap,
    phase: phase,
    outcome: outcome,
    status: policy.status,
    writesCompletion: policy.writesCompletion,
    summary: summary,
    reason: policy.reason,
    reviewBudget: reviewBudget,
    reviewCoverage: reviewCoverage,
    findings: findings,
  };
}

// buildTaskOutcome — the task-shaped OUTCOME contract { task, outcome, status,
// writesCompletion, summary, reason, findings }. A task is keyed by slug and
// belongs to no roadmap, so it emits a `task` identifier instead of
// `roadmap`/`phase`; the decision core (classifyOutcome / hasBlocking /
// summarizeFindings / outcomePolicy) is shared UNCHANGED with the phase path.
// Tasks always dispatch at the fixed `medium` tier, so the `large`
// gate-tightening in hasBlocking never applies to them. `escalated` maps to the
// `blocked` TASK status — never downgraded to `in-progress`. fetchError
// short-circuits to escalated. Never emits a land-time completion directive.
function buildTaskOutcome(input) {
  const i = input || {};
  const task = i.task;
  const tier = i.tier;
  if (i.fetchError === true) {
    const failSummary = 'task fetch failed';
    const failPolicy = outcomePolicy('escalated', 'task', failSummary);
    return {
      task: task,
      outcome: 'escalated',
      status: failPolicy.status,
      writesCompletion: failPolicy.writesCompletion,
      summary: failSummary,
      reason: failPolicy.reason,
      reviewBudget: buildReviewBudget(i.budgetRounds, i.planBudget),
      reviewCoverage: buildReviewCoverage(i.coverageRounds, i.planCoverage),
      findings: [],
    };
  }
  // A dispatch that could determine NO verification command escalates rather
  // than skipping verification. Structured exactly like the fetchError branch
  // above and reusing the SAME `escalated` value via outcomePolicy — it adds no
  // OUTCOME value, no classifier branch, and no GATE_POLICY row.
  if (i.verifyUnresolved === true) {
    const verifyPolicy = outcomePolicy('escalated', 'task', VERIFY_UNRESOLVED_SUMMARY);
    return {
      task: task,
      outcome: 'escalated',
      status: verifyPolicy.status,
      writesCompletion: verifyPolicy.writesCompletion,
      summary: VERIFY_UNRESOLVED_SUMMARY,
      reason: verifyPolicy.reason,
      reviewBudget: buildReviewBudget(i.budgetRounds, i.planBudget),
      reviewCoverage: buildReviewCoverage(i.coverageRounds, i.planCoverage),
      findings: [],
    };
  }
  const planFindings = i.planFindings || [];
  const acRounds = i.acRounds || [];
  const lastAcTable = acRounds.length ? acRounds[acRounds.length - 1] : null;
  const classifierInput = {
    planFindings: planFindings,
    codeFindings: i.codeFindings,
    codeFindingsAfterRework: i.codeFindingsAfterRework,
    codeReviews: i.codeReviews,
    maxRework: i.maxRework,
    tier: tier,
    acTable: lastAcTable,
  };
  const outcome = classifyOutcome(classifierInput);
  const rounds = codeReviewRounds(classifierInput);
  const lastRound = rounds[rounds.length - 1] || [];
  let findings;
  let summary;
  if (outcome === 'escalated') {
    findings = planFindings;
    summary = 'plan gate escalated: ' + summarizeFindings(planFindings);
  } else if (outcome === 'rework') {
    findings = lastRound;
    // See buildOutcome's identical AC-only-gap note: an empty lastRound with a
    // gapped AC table must not read as "no surviving findings".
    summary =
      lastRound.length === 0 && acTableHasGap(lastAcTable)
        ? 'code rework unresolved: unmet acceptance criteria in AC table'
        : 'code rework unresolved: ' + summarizeFindings(lastRound);
  } else {
    findings = annotateHandled(lastRound, i.actResult);
    summary = 'task reviewed clean: ' + summarizeFindings(lastRound);
  }
  // See buildOutcome's identical note: appended in all three branches, only on
  // an actual hit.
  const reviewBudget = buildReviewBudget(i.budgetRounds, i.planBudget);
  summary = summary + budgetSummaryClause(reviewBudget);
  // The coverage clause is appended in ALL THREE branches too, immediately AFTER
  // the budget clause so the ordering is fixed and deterministic for a run that
  // hit both. It is EMPTY when every round ran every dimension, so a healthy
  // run's summary stays byte-unchanged. Because outcomePolicy derives `reason`
  // from `summary`, a parked/escalated unit whose review lost a dimension
  // surfaces that in the `rdm review blocked` queue for free.
  //
  // This is the ONLY thing coverage does here: it never reaches classifyOutcome's
  // input, and no branch gates on it. Recorded, never gated on.
  const reviewCoverage = buildReviewCoverage(i.coverageRounds, i.planCoverage);
  summary = summary + coverageSummaryClause(reviewCoverage);
  const policy = outcomePolicy(outcome, 'task', summary);
  return {
    task: task,
    outcome: outcome,
    status: policy.status,
    writesCompletion: policy.writesCompletion,
    summary: summary,
    reason: policy.reason,
    reviewBudget: reviewBudget,
    reviewCoverage: reviewCoverage,
    findings: findings,
  };
}
// >>> dispatch-outcome:end <<<

// Node-only exports for the verify harness. NOT part of the copied block — the
// marker END is above this line, so a copy never carries these.
export {
  DEFAULT_MAX_PLAN_REVISE,
  DEFAULT_MAX_CODE_REWORK,
  DEFAULT_MAX_REFUTATIONS,
  buildReviewBudget,
  budgetSummaryClause,
  buildReviewCoverage,
  coverageSummaryClause,
  parseBudget,
  projectFlag,
  resolveRdmBin,
  parseProjectArg,
  parseDispatchArgs,
  hoistedMetaComplete,
  runPlanGate,
  runCodeGate,
  codeReviewRounds,
  hasBlocking,
  acTableHasGap,
  summarizeFindings,
  classifyOutcome,
  statusFor,
  writesCompletion,
  GATE_POLICY,
  deriveSignals,
  outcomePolicy,
  OUTCOME_REASON_PREFIX,
  CODE_ACT_SCHEMA,
  buildCodeActPrompt,
  annotateHandled,
  buildOutcome,
  buildTaskOutcome,
  VERIFY_OUTPUT_CAP,
  VERIFY_RESULT_SCHEMA,
  VERIFY_UNRESOLVED_SUMMARY,
  truncateVerifyOutput,
  normalizeVerifyResult,
  verifyFailureFinding,
  extractVerifyCommand,
  verifyToolingLine,
  verifyFailureClause,
  verifyResolutionLines,
  buildVerifyPrompt,
};
