//! autopilot — pure control logic for the roadmap-driving loop over dispatch-phase.
//!
//! This is the **single source of truth** for the deterministic control core of
//! the `autopilot` workflow: the argument parsing, phase-selection interpretation,
//! outcome interpretation, tier resolution, prompt building, and the bounded drive
//! loop that repeatedly asks `rdm next` for the next actionable phase, dispatches
//! it via the one allowed level of `workflow()` nesting (`dispatch-phase`), and
//! persists the resulting status so the selector steps forward.
//!
//! Because the Claude Code Workflow runtime cannot `import`/`require` (see
//! docs/workflow-schemas.md § "Import spike"), the marked block below is copied
//! BYTE-IDENTICAL into `.claude/workflows/autopilot.js`;
//! `scripts/verify-workflow-autopilot.sh` gates the two copies for byte-equality.
//!
//! Everything the block needs is self-contained (no imports, pure array/string
//! ops, no Date.now / Math.random) and it names NO ambient Workflow global
//! (`agent`/`parallel`/`workflow`/`log`): every side effect is reached through the
//! injected `deps` object, so importing this module in Node — where those globals
//! do not exist — never throws. The `export { … }` at the bottom lives OUTSIDE the
//! markers so it is never copied into the workflow script (whose only permitted
//! export is `meta`). The verify harness imports this module and unit-tests the
//! pure logic, then drives the loop with state-backed fakes — zero LLM calls.

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
  return { roadmap: roadmap, maxPhases: maxPhases, planOnly: planOnly, globalBudget: globalBudget };
}

// selectUnestimated(phaseList) — the stems of phases with NO difficulty and NO
// model tier yet, i.e. the ones the estimate pre-pass must rate.
function selectUnestimated(phaseList) {
  const list = Array.isArray(phaseList) ? phaseList : [];
  return list
    .filter((p) => p && !p.difficulty && !p.model)
    .map((p) => p.stem)
    .filter(Boolean);
}

// difficultyToTier(difficulty) — map an rdm difficulty onto a model tier.
// trivial/easy -> small, moderate -> medium, hard -> large; anything else ->
// medium.
function difficultyToTier(difficulty) {
  if (difficulty === 'trivial' || difficulty === 'easy') return 'small';
  if (difficulty === 'moderate') return 'medium';
  if (difficulty === 'hard') return 'large';
  return 'medium';
}

// resolveTier(model) — a valid tier or the mid-tier default. Call as
// resolveTier(model || 'medium') so an unset tier lands on medium.
function resolveTier(model) {
  if (model === 'small' || model === 'medium' || model === 'large') return model;
  return 'medium';
}

// interpretNext(nextResult) — classify the parsed `rdm next` JSON into a loop
// decision: a `phase` to work, or a `stop` with its reason.
function interpretNext(nextResult) {
  const r = nextResult || {};
  if (r.result === 'phase') {
    return { kind: 'phase', stem: r.stem, number: r.number, model: r.model };
  }
  if (r.result === 'blocked-on-dependencies') {
    return { kind: 'stop', reason: 'blocked-on-dependencies', unmet: r.unmet || [] };
  }
  return { kind: 'stop', reason: 'nothing' };
}

// buildParkReason(stage, note) — a tagged escalation reason string. `stage` is
// 'plan' or 'code'; the tag lets the summary and `rdm review blocked` group
// escalations by which gate produced them.
function buildParkReason(stage, note) {
  return '[' + stage + '] ' + note;
}

// interpretOutcome(outcomeStr, ctx) — turn a dispatch OUTCOME string into the
// loop's next action. reviewed -> advance (or noop-vetted under --plan-only);
// rework -> retry until the per-phase budget is spent, then park [code];
// escalated (or any unrecognized value) -> park. dispatch-phase's escalations
// originate at the plan gate, so they are tagged [plan].
function interpretOutcome(outcomeStr, ctx) {
  const c = ctx || {};
  const planOnly = !!c.planOnly;
  const reworkCount = c.reworkCount || 0;
  const maxRework = c.maxRework != null ? c.maxRework : DEFAULT_MAX_REWORK;
  if (outcomeStr === 'reviewed') {
    return planOnly ? { action: 'noop-vetted' } : { action: 'advance' };
  }
  if (outcomeStr === 'rework') {
    if (reworkCount < maxRework) return { action: 'retry' };
    return { action: 'park', reason: buildParkReason('code', 'rework budget exhausted') };
  }
  if (outcomeStr === 'escalated') {
    return { action: 'park', reason: buildParkReason('plan', 'dispatch escalated at the plan gate') };
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

// buildEstimatorPrompt(phaseBody) — rate ONE phase's difficulty. The argument is
// the phase body (or, for a Bash-capable estimator, a directive naming the
// command that yields it). Pure: it only embeds the argument into the prompt.
function buildEstimatorPrompt(phaseBody) {
  return [
    'You are a difficulty-estimation agent for a single rdm phase.',
    'The phase body (or how to obtain it) is below.',
    '--- PHASE BODY ---',
    phaseBody,
    '--- END PHASE BODY ---',
    'Rate the implementation difficulty as exactly one of: trivial, easy, moderate, hard.',
    'Return JSON { "stem": "<the phase stem>", "difficulty": "<trivial|easy|moderate|hard>" }.',
  ].join('\n');
}

// buildEstimateWritebackPrompt(stem, difficulty, slug) — persist a phase's
// difficulty; the model tier derives automatically, so --model is never set.
function buildEstimateWritebackPrompt(stem, difficulty, slug) {
  return [
    'You are a mechanical write agent. Do not plan or implement anything.',
    'Persist the phase difficulty (the model tier derives automatically — do NOT',
    'pass --model). Run exactly this command in the repo root:',
    '  ./target/debug/rdm phase update ' + stem + ' --difficulty ' + difficulty + ' --no-edit --roadmap ' + slug + ' --project rdm',
    'Report whether the command exited 0.',
  ].join('\n');
}

// buildAdvancePrompt(stem, slug) — persist a reviewed phase's status so
// `rdm next` steps past it. Status write ONLY: it never lands, integrates, or
// emits a completion directive — that is a separate, later step.
function buildAdvancePrompt(stem, slug) {
  return [
    'You are a mechanical status agent. Do not plan or implement anything.',
    'The phase has been reviewed. Persist its status so `rdm next` steps past it.',
    'Run exactly this command in the repo root:',
    '  ./target/debug/rdm phase update ' + stem + ' --status reviewed --no-edit --roadmap ' + slug + ' --project rdm',
    'Persist status only — integrating the work is a separate, later step handled elsewhere.',
    'Report whether the command exited 0.',
  ].join('\n');
}

// buildParkPrompt(stem, reason, slug) — park a phase as blocked with an
// escalation reason, so `rdm next` steps past it and the reason is queued.
function buildParkPrompt(stem, reason, slug) {
  return [
    'You are a mechanical status agent. Do not plan or implement anything.',
    'Park this phase as blocked so `rdm next` steps past it and the escalation is queued.',
    'Run exactly this command in the repo root:',
    '  ./target/debug/rdm phase update ' + stem + ' --status blocked --reason "' + reason + '" --no-edit --roadmap ' + slug + ' --project rdm',
    'Report whether the command exited 0.',
  ].join('\n');
}

// buildSummary(state) — the always-on batched run summary. Lists the phases
// completed in order, the escalations tagged plan/code with their reasons and a
// pointer at the `rdm review blocked` queue, the stop reason, and a note that
// reviewed work is left on the roadmap branch and main is never touched.
function buildSummary(state) {
  const s = state || {};
  const roadmap = s.roadmap || '';
  const completed = Array.isArray(s.completed) ? s.completed : [];
  const escalations = Array.isArray(s.escalations) ? s.escalations : [];
  const stopReason = s.stopReason || 'unknown';
  const lines = [];
  lines.push('autopilot summary for roadmap/' + roadmap);
  lines.push('stop reason: ' + stopReason);
  lines.push('phases completed (' + completed.length + '): ' + (completed.length ? completed.join(', ') : 'none'));
  if (escalations.length) {
    lines.push('escalations awaiting review (' + escalations.length + '):');
    for (const e of escalations) {
      const stage = String((e && e.reason) || '').indexOf('[plan]') === 0 ? 'plan' : 'code';
      lines.push('  - ' + (e && e.stem) + ' [' + stage + ']: ' + ((e && e.reason) || ''));
    }
    lines.push('review the queue: ./target/debug/rdm review blocked --project rdm');
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
  return async function runAutopilot(config) {
    const cfg = config || {};
    const roadmap = cfg.roadmap;
    const maxPhases = cfg.maxPhases != null ? cfg.maxPhases : null;
    const planOnly = !!cfg.planOnly;
    const globalBudget = cfg.globalBudget != null ? cfg.globalBudget : DEFAULT_GLOBAL_BUDGET;
    const maxRework = cfg.maxRework != null ? cfg.maxRework : DEFAULT_MAX_REWORK;

    // Estimate pre-pass — ONCE, before the drive loop. Rate every unestimated
    // phase in a single parallel fan-out, then persist each tier. A wholesale
    // failure or a single missing estimate is tolerated: the phase falls back to
    // the mid tier at dispatch time.
    const phaseList = await d.estimateList(roadmap);
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
        try {
          await d.estimateWriteback(est.stem, est.difficulty, roadmap);
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

    while (true) {
      if (maxPhasesReached(dispatchCount, maxPhases) || stepBudgetExhausted(dispatchCount, globalBudget)) {
        stopReason = 'budget';
        break;
      }
      const next = interpretNext(await d.fetchNext(roadmap));
      if (next.kind === 'stop') {
        stopReason = next.reason;
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
          outcome = await d.dispatch(roadmap, stem, planOnly);
        } catch (e) {
          const reason = buildParkReason('code', 'dispatch failed: ' + ((e && e.message) || 'error'));
          await d.park(stem, reason, roadmap);
          escalations.push({ stem: stem, reason: reason });
          break;
        }
        const outcomeStr = (outcome && outcome.outcome) || '';
        const decision = interpretOutcome(outcomeStr, { planOnly: planOnly, reworkCount: reworkCount, maxRework: maxRework });

        if (decision.action === 'advance') {
          let advanceOk = false;
          for (let attempt = 0; attempt < DEFAULT_MAX_ADVANCE_ATTEMPTS; attempt++) {
            try {
              const ack = await d.advance(stem, roadmap);
              advanceOk = !ack || ack.ok !== false;
            } catch (e) {
              advanceOk = false;
            }
            if (advanceOk) break;
            log('autopilot: advance failed for ' + stem + ' (attempt ' + (attempt + 1) + ')');
          }
          if (advanceOk) {
            completed.push(stem);
            log('autopilot: ' + advanceReason(stem));
            break;
          }
          const reason = buildParkReason('code', 'advance to reviewed failed repeatedly');
          await d.park(stem, reason, roadmap);
          escalations.push({ stem: stem, reason: reason });
          break;
        }

        if (decision.action === 'noop-vetted') {
          planOnlySeen.add(stem);
          completed.push(stem);
          log('autopilot: plan-only vetted ' + stem);
          break;
        }

        if (decision.action === 'retry') {
          reworkCount++;
          log('autopilot: rework ' + stem + ' (retry ' + reworkCount + ')');
          continue;
        }

        // decision.action === 'park'
        await d.park(stem, decision.reason, roadmap);
        escalations.push({ stem: stem, reason: decision.reason });
        break;
      }
    }

    return buildSummary({ completed: completed, escalations: escalations, roadmap: roadmap, stopReason: stopReason });
  };
}
// >>> autopilot-loop:end <<<

// Node-only exports for the verify harness. NOT part of the copied block — the
// marker END is above this line, so a copy never carries these.
export {
  DEFAULT_GLOBAL_BUDGET,
  DEFAULT_MAX_REWORK,
  DEFAULT_MAX_ADVANCE_ATTEMPTS,
  parseAutopilotArgs,
  selectUnestimated,
  difficultyToTier,
  resolveTier,
  interpretNext,
  buildParkReason,
  interpretOutcome,
  advanceReason,
  stepBudgetExhausted,
  maxPhasesReached,
  buildFetchNextPrompt,
  buildEstimateListPrompt,
  buildEstimatorPrompt,
  buildEstimateWritebackPrompt,
  buildAdvancePrompt,
  buildParkPrompt,
  buildSummary,
  buildAutopilot,
};
