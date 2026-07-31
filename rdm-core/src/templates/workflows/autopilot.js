// autopilot — the roadmap-driving loop over dispatch-phase.
//
// The active driver of rdm's autonomous lane: given ONE roadmap slug, it loops
// over `rdm next`, dispatches each actionable phase by calling the Phase 2
// dispatch-phase workflow via `workflow()` (the one allowed level of nesting),
// interprets the returned OUTCOME, and PERSISTS the resulting status so the
// selector steps forward:
//   reviewed -> advance (rdm phase update --status reviewed)
//   rework   -> re-dispatch against a per-phase budget, then park blocked [code]
//   escalated-> park blocked [plan]
// It bounds itself with a global step budget and `--max-phases`, supports
// `--plan-only` (dispatch stops after its plan gate), always emits a batched
// summary, and NEVER touches `main` — landing is a separate skill (`rdm-land`).
//
// Invoke with args: { roadmap: '<slug>', maxPhases?: <n>, planOnly?: <bool> }.
//
// Its pure control core lives once in `.claude/workflows/lib/autopilot.mjs` and is
// copied BYTE-IDENTICAL into the marked block below (the Workflow runtime cannot
// load helper modules at run time — see docs/workflow-schemas.md § "Import
// spike"); `scripts/verify-workflow-autopilot.sh` gates the two copies for drift.

export const meta = {
  name: 'autopilot',
  description:
    'Drive one rdm roadmap to reviewed: estimate pre-pass, then loop rdm next -> dispatch-phase -> interpret outcome -> advance/park, bounded by a step budget and --max-phases, always summarizing and never touching main',
  // Must list exactly the distinct `phase:` values the real deps' agent() calls
  // emit — verify-workflow-autopilot.sh asserts declared == emitted.
  phases: [
    { title: 'Estimate' },
    { title: 'Fetch' },
    { title: 'Advance' },
    { title: 'Park' },
  ],
}

// The block below is copied BYTE-IDENTICAL from
// .claude/workflows/lib/autopilot.mjs — do NOT edit it here. Edit the lib and
// scripts/verify-workflow-autopilot.sh fails the build on drift.
// >>> autopilot-loop:begin <<<
// Pure, deterministic control logic for the autopilot roadmap-driving loop.
//
// This block is the single source of truth in
// .claude/workflows/lib/autopilot.mjs and is copied BYTE-IDENTICAL into
// .claude/workflows/autopilot.js (the Workflow runtime cannot load modules at run
// time). scripts/verify-workflow-autopilot.sh gates the two copies for drift. No
// Date.now / Math.random — pure array/string ops only. The block names NO ambient
// runtime global (agent/parallel/workflow/log): every side effect is reached
// through the injected `deps` object, so the module imports cleanly in Node.

// DEFAULT_GLOBAL_BUDGET — the maximum total phase dispatches per run, so a
// pathological roadmap can never loop forever even if every phase keeps
// reworking.
const DEFAULT_GLOBAL_BUDGET = 50;

// DEFAULT_MAX_REWORK — the per-phase rework-retry budget: how many times a
// `rework` outcome is re-dispatched before the phase is parked `blocked` with a
// [code] reason.
const DEFAULT_MAX_REWORK = 1;

// DEFAULT_MAX_ADVANCE_ATTEMPTS — how many times a reviewed phase's status
// write-back is retried before the phase is parked, so a reviewed<->advance-fail
// cycle cannot livelock (the global step budget is the ultimate backstop).
const DEFAULT_MAX_ADVANCE_ATTEMPTS = 2;

// DEFAULT_MAX_PARK_ATTEMPTS — how many times a park write is retried before
// giving up on confirmation. park is not itself a dispatch, so the global step
// budget is not a backstop here — this cap is what keeps a persistently-null
// park ack from hanging a single phase's dispatch loop.
const DEFAULT_MAX_PARK_ATTEMPTS = 2;

// parseAutopilotBudget(value, flag) — validate an optional per-run dispatch
// budget override that is forwarded verbatim to dispatch-phase. `null` means
// UNSET: the key is then omitted from the dispatch payload so dispatch-phase
// applies its own default. 0 is meaningful (no reworks — terminate on the first
// blocking review), so it must never be conflated with unset by a falsy check.
// A non-integer string is rejected rather than silently coerced.
function parseAutopilotBudget(value, flag) {
  if (value === null || value === undefined || value === '') return null;
  let n = NaN;
  if (typeof value === 'number') {
    n = value;
  } else if (typeof value === 'string' && /^[+-]?[0-9]+$/.test(value.trim())) {
    n = parseInt(value.trim(), 10);
  }
  if (!Number.isInteger(n) || n < 0 || Object.is(n, -0)) {
    throw new Error('autopilot: ' + flag + ' must be a non-negative integer (got "' + String(value) + '")');
  }
  return n;
}

// parseAutopilotArgs(args) — validate and normalize the run config. A roadmap
// slug is REQUIRED (the loop never roams to another roadmap). maxPhases is a
// positive integer or null (unbounded by phase count). planOnly is a boolean.
// globalBudget defaults to DEFAULT_GLOBAL_BUDGET. It NEVER yields a --land flag —
// landing is a separate skill, out of autopilot's scope.
// Defensive: a caller may stringify the Workflow tool payload, so a JSON-string
// `args` is parsed back into an object. A non-JSON or non-object value falls back
// to {} so the actionable required-slug error surfaces rather than an opaque
// SyntaxError or a TypeError on a primitive.
function parseAutopilotArgs(args) {
  let a = args || {};
  if (typeof a === 'string') {
    try {
      a = JSON.parse(a) || {};
    } catch (e) {
      a = {};
    }
  }
  if (!a || typeof a !== 'object') a = {};
  const roadmap = a.roadmap || '';
  if (!roadmap) {
    throw new Error('autopilot: a roadmap slug is required (pass { roadmap: "<slug>" }) — the loop never roams');
  }
  let maxPhases = null;
  if (a.maxPhases != null && a.maxPhases !== '') {
    const n = parseInt(a.maxPhases, 10);
    if (!(n > 0)) throw new Error('autopilot: --max-phases must be a positive integer');
    maxPhases = n;
  }
  const planOnly = !!a.planOnly;
  let globalBudget = DEFAULT_GLOBAL_BUDGET;
  if (a.globalBudget != null && a.globalBudget !== '') {
    const g = parseInt(a.globalBudget, 10);
    if (g > 0) globalBudget = g;
  }
  // dispatch-phase's two in-run retry budgets, forwarded per run. Unset (null)
  // means "let dispatch-phase apply its own default".
  const maxPlanRevise = parseAutopilotBudget(a.maxPlanRevise, '--max-plan-revise');
  const maxCodeRework = parseAutopilotBudget(a.maxCodeRework, '--max-code-rework');
  // --- Optional caller-supplied hoists (see docs/mechanical-agent-inventory.md).
  // A caller that is already a running agent with the repo in context (the
  // rdm-autopilot skill shim) runs the mechanical command itself and passes the
  // result here, so this workflow never spawns a dedicated subagent for it.
  // Every one is OPTIONAL: absent or malformed falls through to the existing
  // dep call, which is what a direct `Workflow` invocation always does.
  const mechanicalModel =
    typeof a.mechanicalModel === 'string' && a.mechanicalModel.trim() !== '' ? a.mechanicalModel.trim() : null;
  const phaseList = Array.isArray(a.phaseList) ? a.phaseList : null;
  // `next` is consumed ONE-SHOT — see runAutopilot. `rdm next` is what advances
  // the cursor, so reusing a cached result on iteration 2 would re-dispatch the
  // same phase forever.
  const next = a.next && typeof a.next === 'object' ? a.next : null;
  return {
    roadmap: roadmap,
    maxPhases: maxPhases,
    planOnly: planOnly,
    globalBudget: globalBudget,
    maxPlanRevise: maxPlanRevise,
    maxCodeRework: maxCodeRework,
    mechanicalModel: mechanicalModel,
    phaseList: phaseList,
    next: next,
  };
}

// selectUnestimated and the estimate prompt builders (buildEstimateListPrompt /
// buildEstimatorPrompt / buildEstimateWritebackPrompt) now live in the shared
// `estimate-core` block below (single-sourced in lib/estimate.mjs), which
// buildAutopilot's pre-pass reuses. The vestigial JS difficulty->tier map is
// gone entirely — rdm-core owns that policy (Difficulty::model_tier), and the
// loop dispatches on the tier `rdm phase update --difficulty` auto-derives onto
// next.model.

// resolveTier(model) — a valid tier or the mid-tier default. Call as
// resolveTier(model || 'medium') so an unset tier lands on medium.
function resolveTier(model) {
  if (model === 'small' || model === 'medium' || model === 'large') return model;
  return 'medium';
}

// MAX_NEXT_UNWRAP_DEPTH — bounds how many times interpretNext will unwrap a
// string-encoded `result` field before giving up. Guards against a
// pathological/nested chain of JSON-encoded `result` strings looping
// indefinitely or throwing on a cyclic/malicious payload.
const MAX_NEXT_UNWRAP_DEPTH = 3;

// describeRaw(value) — a bounded, safe string rendering of an arbitrary
// fetch:next payload for logging/escalation. Falls back to String(value) if
// JSON.stringify throws (e.g. a cyclic object), and truncates so a huge or
// cyclic payload can never bloat logs or the summary.
function describeRaw(value) {
  let s;
  try {
    s = JSON.stringify(value);
  } catch (e) {
    s = String(value);
  }
  if (typeof s !== 'string') s = String(s);
  const LIMIT = 200;
  if (s.length > LIMIT) return s.slice(0, LIMIT) + '…(truncated)';
  return s;
}

// interpretNext(nextResult) — classify the parsed `rdm next` JSON into a loop
// decision: a `phase` to work, or a `stop` with its reason.
//
// Defensive unwrap: an agent transcribing `rdm next`'s JSON output has been
// observed to double-wrap it — the whole payload re-encoded as a JSON STRING
// inside `result`. This unwraps up to MAX_NEXT_UNWRAP_DEPTH times, applied
// uniformly across all three rdm-next shapes ('phase' | 'nothing' |
// 'blocked-on-dependencies') — the same agent-formatting variance can wrap
// any of them, not just 'phase'.
//
// A malformed/unrecognized/empty result is NEVER classified as 'nothing' —
// that reason is reserved for a genuinely well-formed "no actionable phase"
// answer from `rdm next`. Anything else (non-object input, an unrecognized
// `result` value, a 'phase' result missing `stem`, or an unwrap chain that
// exhausts MAX_NEXT_UNWRAP_DEPTH without bottoming out) stops with the single
// canonical reason 'unparseable' — not fragmented into multiple literals for
// this defect class — carrying a bounded description of the ORIGINAL input
// (before any unwrapping) for the summary/escalation.
function interpretNext(nextResult) {
  let candidate = nextResult;
  for (let depth = 0; depth < MAX_NEXT_UNWRAP_DEPTH; depth++) {
    if (!candidate || typeof candidate !== 'object') break;
    if (candidate.result === 'phase' || candidate.result === 'nothing' || candidate.result === 'blocked-on-dependencies') {
      break;
    }
    if (typeof candidate.result !== 'string') break;
    let parsed;
    try {
      parsed = JSON.parse(candidate.result);
    } catch (e) {
      break;
    }
    if (!parsed || typeof parsed !== 'object' || typeof parsed.result !== 'string') break;
    candidate = parsed;
  }

  const r = candidate && typeof candidate === 'object' ? candidate : {};
  if (r.result === 'phase') {
    if (!r.stem) return { kind: 'stop', reason: 'unparseable', raw: describeRaw(nextResult) };
    return { kind: 'phase', stem: r.stem, number: r.number, model: r.model };
  }
  if (r.result === 'blocked-on-dependencies') {
    return { kind: 'stop', reason: 'blocked-on-dependencies', unmet: r.unmet || [] };
  }
  if (r.result === 'nothing') {
    return { kind: 'stop', reason: 'nothing' };
  }
  return { kind: 'stop', reason: 'unparseable', raw: describeRaw(nextResult) };
}

// buildParkReason(stage, note) — a tagged escalation reason string. `stage` is
// 'plan', 'code', or 'fetch'; the tag lets the summary and `rdm review
// blocked` group escalations by which gate produced them.
function buildParkReason(stage, note) {
  return '[' + stage + '] ' + note;
}

// interpretOutcome(outcome, ctx) — turn a dispatch OUTCOME into the loop's next
// action. reviewed -> advance (or noop-vetted under --plan-only); rework ->
// retry until this loop's per-phase budget is spent, then park; escalated (or
// any unrecognized value) -> park.
//
// `outcome` is the whole OUTCOME OBJECT. The status and the gate-tagged reason
// are read OFF it — dispatch-phase projects them from the canonical review
// source (lib/review.mjs: statusFor / STATUS_MAPPING), so this loop no longer
// restates that map. A bare outcome STRING is still accepted (older callers and
// the pure-helper tests): it carries no policy, so the legacy literals below are
// used as the fallback.
//
// The rework-budget park is this loop's OWN decision — dispatch-phase's rework
// status (`in-progress`) describes a single dispatch, whereas a phase whose
// roadmap-level retry budget is spent must land in the escalation queue as
// `blocked`. That is why the park path uses buildParkReason, not outcome.status.
function interpretOutcome(outcome, ctx) {
  const c = ctx || {};
  const planOnly = !!c.planOnly;
  const reworkCount = c.reworkCount || 0;
  const maxRework = c.maxRework != null ? c.maxRework : DEFAULT_MAX_REWORK;
  const isString = typeof outcome === 'string';
  const o = !isString && outcome ? outcome : {};
  const outcomeStr = isString ? outcome : o.outcome || '';
  // The OUTCOME-supplied reason already carries the canonical `[plan]`/`[code]`
  // gate tag; fall back to this loop's own tagged literal when absent.
  const suppliedReason = typeof o.reason === 'string' && o.reason !== '' ? o.reason : null;
  if (outcomeStr === 'reviewed') {
    // The status to persist comes from the OUTCOME, not from a literal here.
    return planOnly ? { action: 'noop-vetted' } : { action: 'advance', status: o.status || 'reviewed' };
  }
  if (outcomeStr === 'rework') {
    if (reworkCount < maxRework) return { action: 'retry' };
    return { action: 'park', reason: buildParkReason('code', 'rework budget exhausted') };
  }
  if (outcomeStr === 'escalated') {
    return {
      action: 'park',
      reason: suppliedReason || buildParkReason('plan', 'dispatch escalated at the plan gate'),
    };
  }
  return { action: 'park', reason: buildParkReason('code', 'unrecognized dispatch outcome: ' + String(outcomeStr)) };
}

// advanceReason(stem) — a short deterministic note for the log/summary when a
// phase advances to reviewed.
function advanceReason(stem) {
  return 'phase ' + stem + ' reviewed — advancing';
}

// stepBudgetExhausted(dispatchCount, globalBudget) — has the run hit its global
// dispatch cap?
function stepBudgetExhausted(dispatchCount, globalBudget) {
  return dispatchCount >= globalBudget;
}

// maxPhasesReached(dispatchCount, maxPhases) — has the run hit its --max-phases
// bound? A null bound never trips.
function maxPhasesReached(dispatchCount, maxPhases) {
  if (maxPhases == null) return false;
  return dispatchCount >= maxPhases;
}

// --- Prompt builders (pure strings; the real deps feed these to agents) -------

// buildMechanicalModelPrompt() — a mechanical Bash agent that resolves the
// mechanical dispatch step to a concrete model id, ONCE per run, before any
// other mechanical agent fires. This is deliberately the one dep call in the
// whole run left UNSIZED (mirrors dispatch-phase's Stage-0
// fetch:phase-meta/fetch:task-meta exemption, recorded in
// scripts/verify-workflow-dispatch.sh's AC-MODEL bootstrap whitelist): it is
// the call that produces the model id every other mechanical agent runs on,
// so it cannot know its own model before running. See
// realDeps.resolveMechanicalModel for the corresponding NO-`model:`-key call.
function buildMechanicalModelPrompt() {
  return [
    'You are a mechanical fetch agent. Do not plan or implement anything.',
    'Run exactly this command in the repo root and read its printed output:',
    '  ./target/debug/rdm model resolve mechanical',
    'Return the printed model id verbatim as JSON { "model": "<id>" }.',
    'If the command fails or prints nothing, return { "model": "" }.',
  ].join('\n');
}

// buildFetchNextPrompt(slug) — a mechanical Bash agent that reads `rdm next`.
function buildFetchNextPrompt(slug) {
  return [
    'You are a mechanical fetch agent. Do not plan or implement anything.',
    'Run exactly this command in the repo root and read its JSON output:',
    '  ./target/debug/rdm next --roadmap ' + slug + ' --project rdm --format json',
    "Return the parsed JSON as an object: it has a `result` field (one of 'phase' |",
    "'nothing' | 'blocked-on-dependencies') plus, when result is 'phase', the",
    '`stem`, `number`, `status`, `difficulty`, and `model` fields.',
  ].join('\n');
}

// buildAdvancePrompt(stem, slug, status) — persist an advanced phase's status so
// `rdm next` steps past it. `status` is the OUTCOME-supplied status the canonical
// review mapped this outcome to (see lib/review.mjs's STATUS_MAPPING); it is
// interpolated rather than hardcoded so the map has exactly one home. Callers
// that have no OUTCOME fall back to the reviewed status.
//
// Status write ONLY: it never lands, integrates, or emits a completion directive.
// Writing the land-time completion trailer is `rdm-land`'s job — it synthesizes
// it from the OUTCOME identifiers via `rdm hook done-line` just before the
// rebase, so no autopilot-produced branch ever needs a manual rebase to gain it.
//
// Success is verified with a read-back rather than self-asserted from the
// command's exit code alone (see buildEstimateWritebackPrompt for why).
function buildAdvancePrompt(stem, slug, status) {
  const advanceStatus = status || 'reviewed';
  return [
    'You are a mechanical status agent. Do not plan or implement anything.',
    'The phase has been reviewed. Persist its status so `rdm next` steps past it.',
    'Run exactly this command in the repo root:',
    '  ./target/debug/rdm phase update ' + stem + ' --status ' + advanceStatus + ' --no-edit --roadmap ' + slug + ' --project rdm',
    'Persist status only — integrating the work is a separate, later step handled elsewhere.',
    'Then read back the phase to confirm the write landed:',
    '  ./target/debug/rdm phase show ' + stem + ' --roadmap ' + slug + ' --project rdm --format json',
    'Report ok: true ONLY if the read-back shows status equal to "' + advanceStatus + '"; otherwise report ok: false.',
  ].join('\n');
}

// buildParkPrompt(stem, reason, slug) — park a phase as blocked with an
// escalation reason, so `rdm next` steps past it and the reason is queued.
// Success is verified with a read-back rather than self-asserted from the
// command's exit code alone (see buildEstimateWritebackPrompt for why).
function buildParkPrompt(stem, reason, slug) {
  return [
    'You are a mechanical status agent. Do not plan or implement anything.',
    'Park this phase as blocked so `rdm next` steps past it and the escalation is queued.',
    'Run exactly this command in the repo root:',
    '  ./target/debug/rdm phase update ' + stem + ' --status blocked --reason "' + reason + '" --no-edit --roadmap ' + slug + ' --project rdm',
    'Then read back the phase to confirm the write landed:',
    '  ./target/debug/rdm phase show ' + stem + ' --roadmap ' + slug + ' --project rdm --format json',
    'Report ok: true ONLY if the read-back shows status equal to "blocked"; otherwise report ok: false.',
  ].join('\n');
}

// KNOWN_GOOD_STOP_REASONS — every stop reason that represents a legitimate,
// well-formed run termination. This is an ALLOWLIST (not a denylist keyed on
// 'unparseable') so any FUTURE unrecognized stop reason is also flagged by
// isAbnormalStop rather than silently trusted as a clean finish — mirroring
// the fail-safe-on-unknown convention already used in lib/review.mjs.
const KNOWN_GOOD_STOP_REASONS = ['nothing', 'blocked-on-dependencies', 'budget', 'plan-only-exhausted', 'mechanical-model-unresolved'];

// isAbnormalStop(reason) — true when `reason` is not one of the known-good
// stop reasons above (including any reason this loop has never seen before).
function isAbnormalStop(reason) {
  return KNOWN_GOOD_STOP_REASONS.indexOf(reason) === -1;
}

// budgetHitTag(outcome) — a short marker appended to a completed phase's stem in
// the run summary when that phase's review hit its refutation budget.
//
// Without this, `buildSummary`'s `phases completed (...)` line prints bare stems
// and a REVIEWED phase whose review was bounded would be indistinguishable from
// one that got full coverage — hiding the bound exactly where a reader looks
// first. Pure, deterministic, and empty for every unbounded phase, so an
// unbounded run's summary is byte-unchanged.
function budgetHitTag(outcome) {
  return outcome && outcome.reviewBudget && outcome.reviewBudget.everHit === true ? ' [budget]' : '';
}

// buildSummary(state) — the always-on batched run summary. Lists the phases
// completed in order, the escalations tagged plan/code/fetch with their
// reasons and a pointer at the `rdm review blocked` queue (plus a loud caveat
// when a [fetch]-tagged entry is present, since those never reach that queue —
// see the fetch-stage handling in runAutopilot), the stop reason (loudly
// flagged when abnormal — see isAbnormalStop), and a note that reviewed work
// is left on the roadmap branch and main is never touched.
function buildSummary(state) {
  const s = state || {};
  const roadmap = s.roadmap || '';
  const completed = Array.isArray(s.completed) ? s.completed : [];
  const escalations = Array.isArray(s.escalations) ? s.escalations : [];
  const stopReason = s.stopReason || 'unknown';
  const lines = [];
  lines.push('autopilot summary for roadmap/' + roadmap);
  if (isAbnormalStop(stopReason)) {
    lines.push(
      '*** ABNORMAL TERMINATION (stop reason: ' +
        stopReason +
        ') — the roadmap was NOT driven to exhaustion; do not read this as a completed run.'
    );
  } else {
    lines.push('stop reason: ' + stopReason);
  }
  lines.push('phases completed (' + completed.length + '): ' + (completed.length ? completed.join(', ') : 'none'));
  if (escalations.length) {
    lines.push('escalations awaiting review (' + escalations.length + '):');
    let hasFetchEscalation = false;
    for (const e of escalations) {
      const m = /^\[(\w+)\]/.exec(String((e && e.reason) || ''));
      const stage = m ? m[1] : 'code';
      if (stage === 'fetch') hasFetchEscalation = true;
      lines.push('  - ' + (e && e.stem) + ' [' + stage + ']: ' + ((e && e.reason) || ''));
    }
    lines.push('review the queue: ./target/debug/rdm review blocked --project rdm');
    // [fetch]-tagged entries have no phase stem to park against, so unlike
    // [plan]/[code] entries they are NEVER written to rdm's persisted queue —
    // call this out loudly here (not just in a code comment) so a caller
    // skimming this summary isn't misled by the line above into expecting
    // `rdm review blocked` to list them.
    if (hasFetchEscalation) {
      lines.push(
        'note: [fetch]-tagged entries above are summary-only — no phase is known to park against, so they will NOT appear in the `rdm review blocked` queue; act on them directly from this summary.'
      );
    }
  } else {
    lines.push('escalations awaiting review (0): none');
  }
  lines.push('reviewed work is left on the roadmap/' + roadmap + ' branch; main is never touched.');
  return lines.join('\n');
}

// buildAutopilot(deps) — returns the async runAutopilot(config) driver. Every
// runtime side effect is reached through `deps`, so the block stays pure and the
// module imports cleanly in Node. Retained loop state is bounded: the latest
// fetchNext result, the current outcome, per-phase rework/advance counters, the
// running dispatch count, the ordered completed[] and escalations[] arrays, and
// (only under --plan-only) a planOnlySeen Set. There is NO normal-mode "seen"
// Set — normal-mode progress is driven by the persisted phase status that
// advance/park write, which `rdm next` reads to step forward.
function buildAutopilot(deps) {
  const d = deps || {};
  const log = d.log || function () {};

  // parkWithRetry(stem, reason, roadmap, model) — park a phase, retrying its
  // write up to DEFAULT_MAX_PARK_ATTEMPTS times and requiring a CONFIRMED ack
  // (ack.ok === true) rather than trusting a thrown/falsy result as success.
  // park is not itself a dispatch, so it has no other backstop — a park that
  // never confirms logs loudly, but the caller still records the escalation
  // and moves on: an unconfirmed park write must never block the run from
  // reaching its final summary.
  async function parkWithRetry(stem, reason, roadmap, model) {
    let parkOk = false;
    for (let attempt = 0; attempt < DEFAULT_MAX_PARK_ATTEMPTS; attempt++) {
      try {
        const ack = await d.park(stem, reason, roadmap, model);
        parkOk = !!ack && ack.ok === true;
      } catch (e) {
        parkOk = false;
      }
      if (parkOk) break;
    }
    if (!parkOk) {
      log(
        'autopilot: park write for ' +
          stem +
          ' returned no confirmation — the escalation is recorded in this summary but the plan-repo status may not reflect it'
      );
    }
    return parkOk;
  }

  return async function runAutopilot(config) {
    const cfg = config || {};
    const roadmap = cfg.roadmap;
    const maxPhases = cfg.maxPhases != null ? cfg.maxPhases : null;
    const planOnly = !!cfg.planOnly;
    const globalBudget = cfg.globalBudget != null ? cfg.globalBudget : DEFAULT_GLOBAL_BUDGET;
    const maxRework = cfg.maxRework != null ? cfg.maxRework : DEFAULT_MAX_REWORK;
    // dispatch-phase's OWN in-run budgets — distinct from maxRework (this loop's
    // roadmap-level re-dispatch budget) and from globalBudget. Only the keys the
    // caller actually set are forwarded, so an unset budget lets dispatch-phase
    // apply its own default rather than receiving an explicit null.
    const budgets = {};
    if (cfg.maxPlanRevise != null) budgets.maxPlanRevise = cfg.maxPlanRevise;
    if (cfg.maxCodeRework != null) budgets.maxCodeRework = cfg.maxCodeRework;

    // Resolve the mechanical model ONCE, before anything else — including the
    // estimate pre-pass. This dep call is deliberately left UNSIZED (mirrors
    // dispatch-phase's Stage-0 fetch:phase-meta/fetch:task-meta exemption): it
    // is the call that produces the model id every other mechanical agent runs
    // on, so it cannot know its own model before running (see
    // buildMechanicalModelPrompt / realDeps.resolveMechanicalModel). An
    // empty/unresolvable result is NOT a silent fallback to the session model —
    // it stops the run immediately and loudly, before any mechanical agent
    // fires, but still returns the always-on batched summary rather than
    // throwing.
    //
    // HOIST: `cfg.mechanicalModel`, when the caller resolved it itself, replaces
    // this dep call entirely. The empty-string fail-closed stop below applies
    // identically to both paths.
    const mechanicalModelRaw =
      typeof cfg.mechanicalModel === 'string' && cfg.mechanicalModel.trim() !== ''
        ? cfg.mechanicalModel
        : await d.resolveMechanicalModel();
    const mechanicalModel = typeof mechanicalModelRaw === 'string' ? mechanicalModelRaw.trim() : '';
    if (!mechanicalModel) {
      log(
        'autopilot: mechanical model could not be resolved (rdm model resolve mechanical returned nothing) — stopping before any mechanical agent runs'
      );
      return buildSummary({ roadmap: roadmap, completed: [], escalations: [], stopReason: 'mechanical-model-unresolved' });
    }

    // Estimate pre-pass — ONCE, before the drive loop. Rate every unestimated
    // phase in a single parallel fan-out, then persist each tier. A wholesale
    // failure or a single missing estimate is tolerated: the phase falls back to
    // the mid tier at dispatch time. estimateList/estimateWriteback are
    // mechanical (fetch/write only) and run on mechanicalModel; parallelEstimate
    // is the difficulty-rating JUDGMENT agent and stays on its own resolved tier.
    // HOIST: `cfg.phaseList`, when the caller ran `rdm phase list` itself,
    // replaces this dep call. selectUnestimated is unchanged either way.
    const phaseList = Array.isArray(cfg.phaseList) ? cfg.phaseList : await d.estimateList(roadmap, mechanicalModel);
    const unestimated = selectUnestimated(phaseList);
    if (unestimated.length) {
      let ests = [];
      try {
        ests = await d.parallelEstimate(unestimated);
      } catch (e) {
        ests = [];
        log('autopilot: estimate pre-pass failed wholesale — unrated phases fall back to mid tier');
      }
      const rated = Array.isArray(ests) ? ests : [];
      for (const est of rated) {
        if (!est || !est.stem || !est.difficulty) continue;
        // Thread the rater's justification so the writeback appends the shared
        // `## Estimate` audit note (same behavior as the standalone estimate
        // workflow — both consume buildEstimateWritebackPrompt from estimate-core).
        try {
          const ack = await d.estimateWriteback(est.stem, est.difficulty, est.justification, roadmap, mechanicalModel);
          if (!ack || ack.ok !== true) {
            log('autopilot: estimate writeback failed for ' + est.stem + ' — it falls back to mid tier');
          }
        } catch (e) {
          log('autopilot: estimate writeback failed for ' + est.stem + ' — it falls back to mid tier');
        }
      }
    }

    // Bounded drive-loop state.
    const completed = [];
    const escalations = [];
    const planOnlySeen = new Set();
    let dispatchCount = 0;
    let stopReason = 'nothing';
    // HOIST (ONE-SHOT): a caller-supplied `rdm next` result covers the FIRST
    // iteration only. Every later iteration must re-read live state, because
    // `rdm next` is what steps the cursor forward once advance/park has
    // persisted a status — reusing a cached result would re-dispatch the same
    // phase forever after a rework.
    let pendingNext = cfg.next && typeof cfg.next === 'object' ? cfg.next : null;

    while (true) {
      if (maxPhasesReached(dispatchCount, maxPhases) || stepBudgetExhausted(dispatchCount, globalBudget)) {
        stopReason = 'budget';
        break;
      }
      const rawNext = pendingNext || (await d.fetchNext(roadmap, mechanicalModel));
      pendingNext = null;
      const next = interpretNext(rawNext);
      if (next.kind === 'stop') {
        stopReason = next.reason;
        if (next.reason === 'unparseable') {
          log('autopilot: fetch:next returned an unparseable payload — raw: ' + (next.raw || ''));
          // Summary-only: no phase stem is known at fetch:next time, so
          // nothing is parked via d.park and this entry will NOT appear in
          // rdm's persisted `rdm review blocked` queue — only in the printed
          // run summary. This is an intentional, scoped limitation: closing
          // it requires an rdm-core schema/status-model change (blocked_phases
          // is phase-only by design), which is out of scope for a
          // workflow-script fix. Tracked as follow-up task
          // surface-fetch-next-escalations-in-blocked-queue.
          escalations.push({
            stem: '(fetch:next)',
            reason: buildParkReason('fetch', 'unparseable rdm-next payload: ' + (next.raw || '')),
          });
        }
        break;
      }
      const stem = next.stem;
      if (planOnly && planOnlySeen.has(stem)) {
        stopReason = 'plan-only-exhausted';
        break;
      }
      // resolveTier(next.model || 'medium') — a mid-tier default covers an unset
      // tier so every dispatch runs on a concrete tier.
      const tier = resolveTier(next.model || 'medium');
      log('autopilot: dispatching ' + stem + ' (tier ' + tier + ', plan-only=' + planOnly + ')');
      let reworkCount = 0;

      // Inner per-phase dispatch loop, bounded by the rework budget.
      while (true) {
        dispatchCount++;
        let outcome;
        try {
          outcome = await d.dispatch(roadmap, stem, planOnly, budgets);
        } catch (e) {
          const reason = buildParkReason('code', 'dispatch failed: ' + ((e && e.message) || 'error'));
          await parkWithRetry(stem, reason, roadmap, mechanicalModel);
          escalations.push({ stem: stem, reason: reason });
          break;
        }
        // The WHOLE OUTCOME object is handed to interpretOutcome so it can read
        // the canonical status/reason policy dispatch-phase projected onto it.
        const decision = interpretOutcome(outcome, { planOnly: planOnly, reworkCount: reworkCount, maxRework: maxRework });

        if (decision.action === 'advance') {
          let advanceOk = false;
          for (let attempt = 0; attempt < DEFAULT_MAX_ADVANCE_ATTEMPTS; attempt++) {
            try {
              const ack = await d.advance(stem, roadmap, decision.status, mechanicalModel);
              advanceOk = !!ack && ack.ok === true;
            } catch (e) {
              advanceOk = false;
            }
            if (advanceOk) break;
            log('autopilot: advance failed for ' + stem + ' (attempt ' + (attempt + 1) + ')');
          }
          if (advanceOk) {
            completed.push(stem + budgetHitTag(outcome));
            log('autopilot: ' + advanceReason(stem));
            break;
          }
          const reason = buildParkReason('code', 'advance to reviewed failed repeatedly');
          await parkWithRetry(stem, reason, roadmap, mechanicalModel);
          escalations.push({ stem: stem, reason: reason });
          break;
        }

        if (decision.action === 'noop-vetted') {
          planOnlySeen.add(stem);
          completed.push(stem + budgetHitTag(outcome));
          log('autopilot: plan-only vetted ' + stem);
          break;
        }

        if (decision.action === 'retry') {
          reworkCount++;
          log('autopilot: rework ' + stem + ' (retry ' + reworkCount + ')');
          continue;
        }

        // decision.action === 'park'
        await parkWithRetry(stem, decision.reason, roadmap, mechanicalModel);
        escalations.push({ stem: stem, reason: decision.reason });
        break;
      }
    }

    return buildSummary({ completed: completed, escalations: escalations, roadmap: roadmap, stopReason: stopReason });
  };
}
// >>> autopilot-loop:end <<<

// The estimate-core block is copied BYTE-IDENTICAL from
// .claude/workflows/lib/estimate.mjs by scripts/gen-workflow-estimate.sh — do
// NOT edit it here. buildAutopilot's estimate pre-pass reuses selectUnestimated
// and buildEstimateWritebackPrompt from it (a compile-time copy, so it adds no
// new workflow() nesting). scripts/verify-workflow-estimate.sh gates it.
// >>> estimate-core:begin <<<
// Pure, deterministic estimate orchestration.
//
// This block is the single source of truth in
// .claude/workflows/lib/estimate.mjs and is copied BYTE-IDENTICAL into
// .claude/workflows/estimate.js, .claude/workflows/autopilot.js, and
// .claude/workflows/lib/autopilot.mjs by scripts/gen-workflow-estimate.sh (the
// Workflow runtime cannot load modules at run time).
// scripts/verify-workflow-estimate.sh gates the copies for drift. No Date.now /
// Math.random — pure array/string ops only. The block names NO ambient runtime
// global (agent/parallel/workflow/log): every side effect is reached through the
// injected `deps` object, so the module imports cleanly in Node. It NEVER
// reimplements the difficulty->tier mapping — rdm-core owns that
// (Difficulty::model_tier); the writeback sets --difficulty only, and the tier
// is read back from `rdm phase show`.

// parseEstimateArgs(args) — validate and normalize the run config. A roadmap
// slug is REQUIRED. `phase` is an optional phase NUMBER (a positive integer) to
// narrow the run to a single phase; unset means "every unestimated phase in the
// roadmap".
// Defensive: a caller may stringify the Workflow tool payload, so a JSON-string
// `args` is parsed back into an object. A non-JSON or non-object value falls
// back to {} so the actionable required-slug error surfaces rather than an
// opaque SyntaxError or a TypeError on a primitive.
function parseEstimateArgs(args) {
  let a = args || {};
  if (typeof a === 'string') {
    try {
      a = JSON.parse(a) || {};
    } catch (e) {
      a = {};
    }
  }
  if (!a || typeof a !== 'object') a = {};
  const roadmap = a.roadmap || '';
  if (!roadmap) {
    throw new Error('estimate: a roadmap slug is required (pass { roadmap: "<slug>" })');
  }
  let phase = null;
  if (a.phase != null && a.phase !== '') {
    const n = parseInt(a.phase, 10);
    if (!(n > 0)) throw new Error('estimate: --phase must be a positive integer phase number');
    phase = n;
  }
  return { roadmap: roadmap, phase: phase };
}

// selectUnestimated(phaseList) — the stems of phases with NO difficulty and NO
// model tier yet, i.e. the ones the estimate pass must rate. Both must be unset:
// a phase with difficulty set but model empty (or vice versa) is treated as
// estimated and skipped.
function selectUnestimated(phaseList) {
  const list = Array.isArray(phaseList) ? phaseList : [];
  return list
    .filter((p) => p && !p.difficulty && !p.model)
    .map((p) => p.stem)
    .filter(Boolean);
}

// buildEstimateListPrompt(slug) — a mechanical Bash agent that lists the phases.
function buildEstimateListPrompt(slug) {
  return [
    'You are a mechanical fetch agent. Do not plan or implement anything.',
    'Run exactly this command in the repo root and read its JSON output:',
    '  ./target/debug/rdm phase list --roadmap ' + slug + ' --project rdm --format json',
    'Return the parsed JSON array verbatim: each element has `number`, `stem`,',
    '`title`, `status`, and — when the phase has been estimated — `difficulty` and',
    '`model`.',
  ].join('\n');
}

// buildEstimatorPrompt(phaseBody) — rate ONE phase's difficulty AND record a
// one-line justification. The argument is the phase body (or, for a Bash-capable
// estimator, a directive naming the command that yields it). Pure: it only
// embeds the argument into the prompt.
function buildEstimatorPrompt(phaseBody) {
  return [
    'You are a difficulty-estimation agent for a single rdm phase.',
    'The phase body (or how to obtain it) is below.',
    '--- PHASE BODY ---',
    phaseBody,
    '--- END PHASE BODY ---',
    'Rate the implementation difficulty as exactly one of: trivial, easy, moderate, hard,',
    'from the scope, risk, and breadth of the work the body describes (a one-line change is',
    'trivial/easy; a self-contained feature is moderate; cross-cutting or high-risk work is hard).',
    'Write a ONE-LINE justification for the rating — it explains the rating, it is not a plan.',
    'Return JSON { "stem": "<the phase stem>", "difficulty": "<trivial|easy|moderate|hard>",',
    '"justification": "<one-line justification>" }.',
  ].join('\n');
}

// buildEstimateWritebackPrompt(stem, difficulty, justification, slug) — persist
// a phase's difficulty AND append a `## Estimate` audit note carrying the
// rating's justification to the phase body. The model tier derives
// automatically, so --model is NEVER set (rdm-core owns difficulty->tier).
//
// The note text and the phase body may contain double-quotes, backticks, `$`,
// or newlines, which would break a naive `--body "..."` interpolation — so the
// agent is instructed to assemble the updated body into a shell variable via a
// QUOTED heredoc (keeping backticks/`$`/punctuation literal) and pass
// `--body "$body"`; rdm's --body is authoritative for Unicode/punctuation.
//
// Success is verified with a read-back rather than self-asserted from the
// command's exit code alone: an unresolvable model id makes the whole agent come
// back empty, not merely non-zero, so the caller needs proof the field landed.
function buildEstimateWritebackPrompt(stem, difficulty, justification, slug) {
  return [
    'You are a mechanical write agent. Do not plan or implement anything.',
    'Persist the phase difficulty AND append an audit note to the phase body.',
    'Do NOT pass --model — the model tier derives automatically from the difficulty.',
    '1. Read the current phase body:',
    '     ./target/debug/rdm phase show ' + stem + ' --roadmap ' + slug + ' --project rdm --format json',
    '   Take the `body` field verbatim.',
    '2. Build the updated body in a shell variable using a QUOTED heredoc so that',
    '   backticks, $, and punctuation stay literal. The updated body is the current',
    '   body, followed by a blank line, then this exact section:',
    '     ## Estimate',
    '',
    '     ' + difficulty + ' — ' + justification,
    '   For example:',
    "     body=$(cat <<'RDM_ESTIMATE_EOF'",
    '     <the current body>',
    '',
    '     ## Estimate',
    '',
    '     ' + difficulty + ' — ' + justification,
    '     RDM_ESTIMATE_EOF',
    '     )',
    '3. Persist the difficulty and the updated body in a single update:',
    '     ./target/debug/rdm phase update ' +
      stem +
      ' --difficulty ' +
      difficulty +
      ' --body "$body" --no-edit --roadmap ' +
      slug +
      ' --project rdm',
    '4. Read the phase back to confirm the write landed:',
    '     ./target/debug/rdm phase show ' + stem + ' --roadmap ' + slug + ' --project rdm --format json',
    'Report ok: true ONLY if the read-back shows difficulty equal to "' +
      difficulty +
      '" and the body now contains the ## Estimate note; otherwise report ok: false.',
  ].join('\n');
}

// buildEstimateTierPrompt(stem, slug) — read the core-derived model tier back
// from rdm-core after a writeback, for the summary. The tier is NEVER computed
// in JS: it is whatever `rdm phase show`'s `model` field reports (rdm-core's
// Difficulty::model_tier is authoritative).
function buildEstimateTierPrompt(stem, slug) {
  return [
    'You are a mechanical fetch agent. Do not plan or implement anything.',
    'Run exactly this command in the repo root and read its JSON output:',
    '  ./target/debug/rdm phase show ' + stem + ' --roadmap ' + slug + ' --project rdm --format json',
    'Return JSON { "model": "<the phase JSON `model` field verbatim, or empty string if unset>" }.',
  ].join('\n');
}

// buildEstimatePipeline(deps) — returns the async runEstimate(config) driver.
// Every runtime side effect is reached through `deps`, so the block stays pure
// and the module imports cleanly in Node. The flow: list the phases -> filter to
// the unestimated (optionally narrowed to one phase number) -> parallel-rate
// each -> per-phase write back the difficulty + audit note -> read the
// core-derived tier back -> return a DETERMINISTIC summary object. A phase whose
// difficulty is already set is filtered out by selectUnestimated, so it is never
// rated or written (which is what makes a re-run idempotent). A rater result
// that is null or omits stem/difficulty is skipped with a log line rather than
// dereferenced.
//
// The summary distinguishes TWO non-rated populations, because conflating them
// misreports state after a `--phase`-narrowed run:
//   * `skipped`  — phases that ALREADY carry a difficulty/model (genuinely
//                  already estimated; correctly left untouched forever).
//   * `deferred` — phases still unestimated but excluded from THIS run only by
//                  the `phase` narrow (they still need rating on a later pass).
// On an un-narrowed run `deferred` is always empty.
function buildEstimatePipeline(deps) {
  const d = deps || {};
  const log = d.log || function () {};

  return async function runEstimate(config) {
    const cfg = config || {};
    const roadmap = cfg.roadmap || '';
    const onlyNumber = cfg.phase != null ? cfg.phase : null;

    const listed = await d.list(roadmap);
    const phaseList = Array.isArray(listed) ? listed : [];

    // Every phase that still needs rating, BEFORE the optional phase narrow.
    const unestimatedStems = selectUnestimated(phaseList);
    const unestimatedSet = new Set(unestimatedStems);

    let targetStems = unestimatedStems.slice();
    // Narrow to a single phase NUMBER when requested. An already-estimated
    // target is already absent from targetStems, so this degenerates to a no-op.
    if (onlyNumber != null) {
      const wanted = new Set(
        phaseList
          .filter((p) => p && p.number === onlyNumber)
          .map((p) => p.stem)
          .filter(Boolean)
      );
      targetStems = targetStems.filter((s) => wanted.has(s));
    }
    // Deterministic order for both the fan-out and the summary.
    targetStems = targetStems.slice().sort();
    const targetSet = new Set(targetStems);

    const allStems = phaseList.map((p) => p && p.stem).filter(Boolean);
    // `skipped` is ONLY the genuinely-already-estimated phases (difficulty/model
    // set). Phases still unestimated but excluded by the phase narrow go into
    // `deferred`, so a narrowed run never mislabels them as already estimated.
    const skipped = allStems.filter((s) => !unestimatedSet.has(s)).slice().sort();
    const deferred = unestimatedStems.filter((s) => !targetSet.has(s)).slice().sort();

    const estimated = [];
    if (targetStems.length) {
      let rated = [];
      try {
        rated = await d.parallelRate(targetStems);
      } catch (e) {
        rated = [];
        log('estimate: rating pass failed wholesale — nothing was written back');
      }
      const ratedArr = (Array.isArray(rated) ? rated : [])
        .filter((r) => r && r.stem && r.difficulty)
        .slice()
        .sort((a, b) => (a.stem < b.stem ? -1 : a.stem > b.stem ? 1 : 0));
      for (const r of ratedArr) {
        const justification = typeof r.justification === 'string' ? r.justification : '';
        let ack = null;
        try {
          ack = await d.writeback(r.stem, r.difficulty, justification, roadmap);
        } catch (e) {
          ack = null;
        }
        if (!ack || ack.ok !== true) {
          log('estimate: writeback failed for ' + r.stem + ' — difficulty not persisted');
          continue;
        }
        let tier = '';
        try {
          tier = await d.showTier(r.stem, roadmap);
        } catch (e) {
          tier = '';
        }
        estimated.push({
          stem: r.stem,
          difficulty: r.difficulty,
          justification: justification,
          tier: typeof tier === 'string' ? tier : '',
        });
      }
    }

    return { roadmap: roadmap, estimated: estimated, skipped: skipped, deferred: deferred };
  };
}

// buildEstimateSummaryText(summary) — a human-readable rendering of the
// deterministic summary object, for the log/skill output. Pure and
// order-preserving (the object's arrays are already sorted).
function buildEstimateSummaryText(summary) {
  const s = summary || {};
  const roadmap = s.roadmap || '';
  const estimated = Array.isArray(s.estimated) ? s.estimated : [];
  const skipped = Array.isArray(s.skipped) ? s.skipped : [];
  const deferred = Array.isArray(s.deferred) ? s.deferred : [];
  const lines = [];
  lines.push('estimate summary for roadmap/' + roadmap);
  lines.push('estimated (' + estimated.length + '):');
  if (estimated.length) {
    for (const e of estimated) {
      lines.push('  - ' + e.stem + ': ' + e.difficulty + ' (tier ' + (e.tier || 'unknown') + ') — ' + (e.justification || ''));
    }
  } else {
    lines.push('  none');
  }
  lines.push('skipped, already estimated (' + skipped.length + '): ' + (skipped.length ? skipped.join(', ') : 'none'));
  // Only surfaced after a `--phase`-narrowed run leaves other unestimated
  // phases untouched; empty (and omitted) on a full run.
  if (deferred.length) {
    lines.push('deferred, still unestimated — not targeted this run (' + deferred.length + '): ' + deferred.join(', '));
  }
  return lines.join('\n');
}
// >>> estimate-core:end <<<

// --- Schemas (autopilot-specific; see docs/workflow-schemas.md) --------------

// NEXT — the parsed `rdm next` JSON a mechanical Bash agent returns.
const NEXT_SCHEMA = {
  type: 'object',
  required: ['result'],
  additionalProperties: false,
  properties: {
    result: { type: 'string' },
    roadmap: { type: 'string' },
    number: { type: 'integer' },
    stem: { type: 'string' },
    status: { type: 'string' },
    difficulty: { type: 'string' },
    model: { type: 'string' },
    unmet: { type: 'array', items: { type: 'string' } },
  },
}

// PHASE_LIST — the parsed `rdm phase list` JSON for the estimate pre-pass,
// wrapped under a `phases` key. Anthropic custom tools require
// input_schema.type === 'object'; a top-level `type: 'array'` 400s, so the
// array is nested under `phases` and unwrapped in the estimateList realDep.
const PHASE_LIST_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['phases'],
  properties: {
    phases: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['stem', 'status'],
        properties: {
          number: { type: 'integer' },
          stem: { type: 'string' },
          title: { type: 'string' },
          status: { type: 'string' },
          tags: { type: 'array', items: { type: 'string' } },
          difficulty: { type: 'string' },
          model: { type: 'string' },
        },
      },
    },
  },
}

// ESTIMATE — one estimator agent's difficulty rating + justification for a
// single phase. The justification is persisted by the writeback as a
// `## Estimate` audit note (single-sourced with the standalone estimate
// workflow via estimate-core's buildEstimateWritebackPrompt).
const ESTIMATE_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['stem', 'difficulty', 'justification'],
  properties: {
    stem: { type: 'string' },
    difficulty: { type: 'string', enum: ['trivial', 'easy', 'moderate', 'hard'] },
    justification: { type: 'string' },
  },
}

// ACK — a mechanical write agent's report of whether its command exited 0.
const ACK_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['ok'],
  properties: {
    ok: { type: 'boolean' },
    detail: { type: 'string' },
  },
}

// MECHANICAL_MODEL — the resolved `rdm model resolve mechanical` id, from the
// one bootstrap call realDeps.resolveMechanicalModel makes before any other
// mechanical agent runs.
const MECHANICAL_MODEL_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['model'],
  properties: {
    model: { type: 'string' },
  },
}

// --- Driver ------------------------------------------------------------------
const cfg = parseAutopilotArgs(args)
const roadmapSlug = cfg.roadmap

// Real deps close over the ambient Workflow globals (agent/parallel/log and the
// one nested workflow() call). These live OUTSIDE the copied block; the block
// itself names no ambient global.
const realDeps = {
  log: function (msg) {
    log(msg)
  },
  // resolveMechanicalModel — the one bootstrap call in the whole run left
  // deliberately UNSIZED (no `model:` key), mirroring dispatch-phase's Stage-0
  // fetch:phase-meta/fetch:task-meta exemption: this IS the call that produces
  // the model id every other mechanical agent below runs on, so it cannot know
  // its own model before running. scripts/verify-workflow-autopilot.sh's
  // AC-MODEL-style sweep whitelists this label by name for exactly that reason
  // — do not add a `model:` key here.
  resolveMechanicalModel: async function () {
    const r = await agent(buildMechanicalModelPrompt(), {
      label: 'model:mechanical',
      phase: 'Fetch',
      schema: MECHANICAL_MODEL_SCHEMA,
    })
    return r && typeof r.model === 'string' ? r.model.trim() : ''
  },
  estimateList: async function (slug, model) {
    // The in-block prompt builder buildEstimateListPrompt ("Return the parsed
    // JSON array verbatim") is deliberately left unchanged: the StructuredOutput
    // tool schema — not the prompt text — governs the agent's output shape, and
    // editing that prompt would land inside the byte-copied autopilot-loop block
    // and force a matching lib/autopilot.mjs edit + re-stamp (out of scope here).
    // We wrap the array under `phases` in PHASE_LIST_SCHEMA and unwrap it here so
    // the in-block selectUnestimated still receives a plain array.
    const r = await agent(buildEstimateListPrompt(slug), {
      label: 'estimate:list',
      phase: 'Fetch',
      schema: PHASE_LIST_SCHEMA,
      model: model,
    })
    return (r && r.phases) || []
  },
  // parallelEstimate is the difficulty-rating JUDGMENT agent — it stays on the
  // tier resolved for it elsewhere and deliberately receives NO mechanical
  // model argument.
  parallelEstimate: async function (unestimated) {
    return parallel(
      unestimated.map(function (stem) {
        return function () {
          return agent(
            buildEstimatorPrompt(
              'Run `./target/debug/rdm phase show ' +
                stem +
                ' --roadmap ' +
                roadmapSlug +
                ' --project rdm --format json` and use the returned `body` field as the phase body.'
            ),
            { label: 'estimate:' + stem, phase: 'Estimate', schema: ESTIMATE_SCHEMA }
          ).then(function (r) {
            return { stem: (r && r.stem) || stem, difficulty: r && r.difficulty, justification: r && r.justification }
          })
        }
      })
    )
  },
  estimateWriteback: async function (stem, difficulty, justification, slug, model) {
    return agent(buildEstimateWritebackPrompt(stem, difficulty, justification, slug), {
      label: 'estimate:write:' + stem,
      phase: 'Estimate',
      schema: ACK_SCHEMA,
      model: model,
    })
  },
  fetchNext: async function (slug, model) {
    return agent(buildFetchNextPrompt(slug), {
      label: 'fetch:next',
      phase: 'Fetch',
      schema: NEXT_SCHEMA,
      model: model,
    })
  },
  dispatch: async function (slug, stem, planOnly, budgets) {
    // Forward dispatch-phase's two in-run retry budgets ONLY when this run set
    // them, so an unset budget lets dispatch-phase apply its own default rather
    // than receiving an explicit null.
    const b = budgets || {}
    const payload = { roadmap: slug, phase: stem, planOnly: planOnly }
    if (b.maxPlanRevise != null) payload.maxPlanRevise = b.maxPlanRevise
    if (b.maxCodeRework != null) payload.maxCodeRework = b.maxCodeRework
    return workflow('dispatch-phase', payload)
  },
  // `status` comes from the dispatch OUTCOME (the canonical review's status
  // mapping), not from a literal in this file.
  advance: async function (stem, slug, status, model) {
    return agent(buildAdvancePrompt(stem, slug, status), {
      label: 'advance:' + stem,
      phase: 'Advance',
      schema: ACK_SCHEMA,
      model: model,
    })
  },
  park: async function (stem, reason, slug, model) {
    return agent(buildParkPrompt(stem, reason, slug), {
      label: 'park:' + stem,
      phase: 'Park',
      schema: ACK_SCHEMA,
      model: model,
    })
  },
}

const summary = await buildAutopilot(realDeps)(cfg)
log(summary)
return summary
