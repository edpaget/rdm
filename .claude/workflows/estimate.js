// estimate — headless difficulty-estimation for an rdm roadmap's phases.
//
// Given ONE roadmap slug (optionally narrowed to a single phase number), it
// lists the phases, filters to the ones whose difficulty is UNSET, rates each in
// a parallel() fan-out, writes back the rating — persisting the difficulty AND
// appending a `## Estimate` audit note carrying the justification — and reads the
// core-derived model tier back for the summary. Already-estimated phases are
// skipped, which makes a re-run idempotent.
//
// The model tier is NEVER computed here: the writeback sets `--difficulty` only
// (never `--model`), and rdm-core derives the tier (Difficulty::model_tier);
// the reported tier is whatever `rdm phase show --format json` reports as
// `model`.
//
// Invoke with args: { roadmap: '<slug>', phase?: <number> }.
//
// Its pure estimate core lives once in `.claude/workflows/lib/estimate.mjs` and
// is copied BYTE-IDENTICAL into the marked block below (the Workflow runtime
// cannot load helper modules at run time — see docs/workflow-schemas.md §
// "Import spike"); `scripts/gen-workflow-estimate.sh` stamps it and
// `scripts/verify-workflow-estimate.sh` gates the two copies for drift.

export const meta = {
  name: 'estimate',
  description:
    "Rate an rdm roadmap's unestimated phases: list -> filter -> parallel-rate -> write back difficulty + a ## Estimate audit note (tier derives in core), skipping already-estimated phases",
  // Must list exactly the distinct `phase:` values the real deps' agent() calls
  // emit — verify-workflow-estimate.sh asserts declared == emitted.
  phases: [{ title: 'List' }, { title: 'Estimate' }, { title: 'Writeback' }],
}

// The block below is copied BYTE-IDENTICAL from
// .claude/workflows/lib/estimate.mjs — do NOT edit it here. Edit the lib and run
// scripts/gen-workflow-estimate.sh; scripts/verify-workflow-estimate.sh fails
// the build on drift.
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

// --- Schemas (estimate-specific; see scripts/verify-workflow-estimate.sh) -----

// PHASE_LIST — the parsed `rdm phase list` JSON, wrapped under a `phases` key.
// Anthropic custom tools require input_schema.type === 'object'; a top-level
// `type: 'array'` 400s the StructuredOutput tool, so the array is nested under
// `phases` and unwrapped in the list realDep.
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

// ESTIMATE — one rater agent's difficulty rating + justification for a phase.
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

// ACK — a mechanical write agent's report of whether its command landed.
const ACK_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['ok'],
  properties: {
    ok: { type: 'boolean' },
    detail: { type: 'string' },
  },
}

// TIER — the core-derived model tier read back after a writeback (never
// computed in JS).
const TIER_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['model'],
  properties: {
    model: { type: 'string' },
  },
}

// buildMechanicalModelPrompt() — a mechanical Bash agent that resolves the
// mechanical dispatch step to a concrete model id, ONCE per run, before any
// other mechanical agent fires. This is deliberately the one dep call in the
// whole run left UNSIZED (mirrors dispatch-phase's Stage-0 fetch:phase-meta/
// fetch:task-meta exemption and autopilot's own model:mechanical bootstrap,
// both recorded in their respective verify-workflow-*.sh AC-MODEL bootstrap
// whitelists): it is the call that produces the model id every other
// mechanical agent below runs on, so it cannot know its own model before
// running.
function buildMechanicalModelPrompt() {
  return [
    'You are a mechanical fetch agent. Do not plan or implement anything.',
    'Run exactly this command in the repo root and read its printed output:',
    '  ./target/debug/rdm model resolve mechanical',
    'Return the printed model id verbatim as JSON { "model": "<id>" }.',
    'If the command fails or prints nothing, return { "model": "" }.',
  ].join('\n')
}

// MECHANICAL_MODEL — the resolved `rdm model resolve mechanical` id, from the
// one bootstrap call made before the pipeline runs.
const MECHANICAL_MODEL_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['model'],
  properties: {
    model: { type: 'string' },
  },
}

// --- Driver ------------------------------------------------------------------

const estimateArgs = parseEstimateArgs(args)
const roadmapSlug = estimateArgs.roadmap

// Real deps close over the ambient Workflow globals (agent/parallel/log). These
// live OUTSIDE the copied block; the block itself names no ambient global. Every
// agent() result is guarded against null (an unresolvable model resolves agent()
// to null rather than throwing) before it is dereferenced.
let mechanicalModel = ''
const realDeps = {
  log: function (msg) {
    log(msg)
  },
  // resolveMechanicalModel — the one bootstrap call in the whole run left
  // deliberately UNSIZED (no `model:` key), mirroring dispatch-phase's Stage-0
  // exemption and autopilot's model:mechanical precedent: this IS the call
  // that produces the model id estimate:list/estimate:write/estimate:tier
  // below run on, so it cannot know its own model before running.
  // scripts/verify-workflow-estimate.sh's mechanical-tier sweep whitelists
  // this label by name for exactly that reason — do not add a `model:` key
  // here.
  resolveMechanicalModel: async function () {
    const r = await agent(buildMechanicalModelPrompt(), {
      label: 'model:mechanical',
      phase: 'List',
      schema: MECHANICAL_MODEL_SCHEMA,
    })
    return r && typeof r.model === 'string' ? r.model.trim() : ''
  },
  list: async function (slug) {
    // The StructuredOutput tool schema — not the prompt text — governs the
    // agent's output shape; the in-block prompt says "Return the parsed JSON
    // array verbatim", so we wrap it under `phases` in PHASE_LIST_SCHEMA and
    // unwrap here, keeping the in-block selectUnestimated fed a plain array.
    const r = await agent(buildEstimateListPrompt(slug), {
      label: 'estimate:list',
      phase: 'List',
      schema: PHASE_LIST_SCHEMA,
      model: mechanicalModel,
    })
    return (r && r.phases) || []
  },
  // parallelRate is the difficulty-rating JUDGMENT agent — it stays on the
  // session/default tier and reads each phase body via a Bash directive.
  parallelRate: async function (stems) {
    return parallel(
      stems.map(function (stem) {
        return function () {
          return agent(
            buildEstimatorPrompt(
              'Run `./target/debug/rdm phase show ' +
                stem +
                ' --roadmap ' +
                roadmapSlug +
                ' --project rdm --format json` and use the returned `body` field as the phase body.'
            ),
            { label: 'estimate:rate:' + stem, phase: 'Estimate', schema: ESTIMATE_SCHEMA }
          ).then(function (r) {
            if (!r) return null
            return { stem: r.stem || stem, difficulty: r.difficulty, justification: r.justification }
          })
        }
      })
    )
  },
  writeback: async function (stem, difficulty, justification, slug) {
    return agent(buildEstimateWritebackPrompt(stem, difficulty, justification, slug), {
      label: 'estimate:write:' + stem,
      phase: 'Writeback',
      schema: ACK_SCHEMA,
      model: mechanicalModel,
    })
  },
  // showTier reads the core-derived tier back — the tier is whatever rdm-core
  // put on the `model` field, never a JS mapping.
  showTier: async function (stem, slug) {
    const r = await agent(buildEstimateTierPrompt(stem, slug), {
      label: 'estimate:tier:' + stem,
      phase: 'Writeback',
      schema: TIER_SCHEMA,
      model: mechanicalModel,
    })
    return r && typeof r.model === 'string' ? r.model : ''
  },
}

// Resolve the mechanical model ONCE, before the pipeline runs — including
// before estimate:list, the pipeline's first mechanical call. An unresolved
// result stops the run before any mechanical agent fires, rather than
// silently falling through to an unpinned list/writeback/tier-read call.
const mechanicalModelRaw = await realDeps.resolveMechanicalModel()
mechanicalModel = typeof mechanicalModelRaw === 'string' ? mechanicalModelRaw.trim() : ''
if (!mechanicalModel) {
  log(
    'estimate: mechanical model could not be resolved (rdm model resolve mechanical returned nothing) — stopping before any mechanical agent runs'
  )
  return { roadmap: roadmapSlug, estimated: [], skipped: [], deferred: [], fetchError: true }
}

// parseEstimateArgs already enforced a non-empty roadmap slug (it throws
// otherwise), so roadmapSlug is guaranteed set here.
const summary = await buildEstimatePipeline(realDeps)(estimateArgs)
log(buildEstimateSummaryText(summary))
return summary
