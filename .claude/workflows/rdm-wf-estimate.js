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
  name: 'rdm-wf-estimate',
  description:
    "Rate an rdm roadmap's unestimated phases: list -> filter -> parallel-rate -> write back difficulty + a ## Estimate audit note (tier derives in core), skipping already-estimated phases",
  // Must list exactly the distinct `phase:` values the real deps' agent() calls
  // emit — verify-workflow-estimate.sh asserts declared == emitted.
  phases: [{ title: 'Estimate' }],
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
// .claude/workflows/rdm-wf-estimate.js by scripts/gen-workflow-estimate.sh (the
// Workflow runtime cannot load modules at run time).
// scripts/verify-workflow-estimate.sh gates the copies for drift. No Date.now /
// Math.random — pure array/string ops only. The block names NO ambient runtime
// global (agent/parallel/workflow/log): every side effect is reached through the
// injected `deps` object, so the module imports cleanly in Node. It NEVER
// reimplements the difficulty->tier mapping — rdm-core owns that
// (Difficulty::model_tier); the writeback sets --difficulty only, and the tier
// is read back from `rdm phase show`.

// --- Environment args: `rdmBin` and `project` --------------------------------
//
// estimate names NO particular rdm executable and NO particular rdm project.
// Both arrive as RUNTIME args and are threaded into every prompt that shells
// out, via the `cfg` object each such prompt builder takes as its trailing
// parameter. This is the contract the retired dispatch-phase engine established,
// reused — NOT a second one: the three helpers below were copied in shape from
// its `lib/dispatch-phase.mjs` (only the thrown-message prefix differs), a file
// `agent-orchestrated-dispatch` phase 7 deleted; the runtime cannot import, so a
// per-consumer copy is expected and this one is now the surviving statement of
// the shape.
// Canonical write-up (rationale, table, why an emit-time placeholder is not
// workable): docs/workflow-schemas.md § "Environment args: `rdmBin` and
// `project`" — not restated here.
//
// Allow-list, in one line: `rdm model resolve` / `rdm commit` / `rdm status` /
// `rdm discard` reject a project flag and must carry NONE; every other
// subcommand this workflow emits (phase list/show/update) is project-scoped and
// takes it. Asserted AS DATA by scripts/verify-workflow-estimate.sh § 9b, not
// by grepping every line.

// projectFlag(cfg) — the ` --project <name>` suffix for a PROJECT-SCOPED
// command, or '' when no project was configured.
function projectFlag(cfg) {
  return cfg && cfg.project ? ' --project ' + cfg.project : '';
}

// resolveRdmBin(value) — resolve the rdm executable to invoke. An ABSENT value
// DEFAULTS to a plain `rdm` on PATH, because a plugin-installed consumer has no
// repo-local build path to pass. The stale-global-build hazard the earlier
// fail-closed stance guarded is real but DOGFOOD-SCOPED to this repo, where
// `RDM_BIN` in `.mise.toml` is the compensating control the calling skill
// resolves (no harness gates it: verify-workflow-dispatch.sh § 9c-dogfood was
// retired with the dispatch engine in agent-orchestrated-dispatch phase 7, and
// CLAUDE.md's development-build rule states it now). A present-but-wrong-
// TYPE value still throws rather than silently degrading to PATH. No existence
// preflight — a plain fallback only. See docs/workflow-schemas.md § "Environment
// args: `rdmBin` and `project`" for the full contract and resolution order.
function resolveRdmBin(value) {
  if (typeof value === 'string' && value.trim() !== '') return value;
  if (value === undefined || value === null || typeof value === 'string') return 'rdm';
  throw new Error(
    'estimate: rdmBin must be a string path to the rdm executable (a repo-local build path, or ' +
      'the sentinel "rdm" to request PATH resolution explicitly). Omit it entirely to default to ' +
      '`rdm` on PATH; a non-string value is a caller bug and is refused rather than guessed.'
  );
}

// parseProjectArg(value) — validate the OPTIONAL project name. Any falsy value
// means "emit no project flag at all". The value is interpolated into a
// Bash-agent prompt, so whitespace and shell metacharacters are rejected rather
// than escaped.
function parseProjectArg(value) {
  if (!value) return '';
  if (typeof value !== 'string' || !/^[A-Za-z0-9._-]+$/.test(value)) {
    throw new Error(
      'estimate: project must be a plain project name matching /^[A-Za-z0-9._-]+$/ (got "' + String(value) + '")'
    );
  }
  return value;
}

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
  // The two ENVIRONMENT axes are resolved AFTER the required-roadmap throw
  // above, not before it. dispatch-phase resolves rdmBin as its very first
  // statement because it has no earlier required field; estimate does, and a
  // payload missing BOTH should surface the actionable "a roadmap slug is
  // required" message for the far more common mis-invocation. Order among the
  // two is still deterministic: rdmBin first (defaulting to `rdm` when absent),
  // then the optional project name. Both are validated HERE, at parse time, so a
  // mis-invocation costs zero tokens.
  const rdmBin = resolveRdmBin(a.rdmBin);
  const project = parseProjectArg(a.project);
  return { roadmap: roadmap, phase: phase, rdmBin: rdmBin, project: project };
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

// `buildEstimateListPrompt`, `buildEstimateWritebackPrompt` and
// `buildEstimateTierPrompt` are GONE with the `estimate:list` /
// `estimate:write:` / `estimate:tier:` agents that ran them. Listing the phases
// is a read the ORCHESTRATOR does (`rdm phase list --format json`, passed as
// `phaseList`); persisting a difficulty is a write it does, from the command
// text `buildEstimateWritebackCommands` returns; and the resulting tier is
// whatever `rdm phase show` reports once that write has landed.

// estimateListCommand(slug, cfg) — the read-only command that produces the phase
// list this pipeline consumes, returned as TEXT so a caller that omitted
// `phaseList` is told exactly what to run.
function estimateListCommand(slug, cfg) {
  return resolveRdmBin(cfg && cfg.rdmBin) + ' phase list --roadmap ' + slug + projectFlag(cfg) + ' --format json';
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

// buildEstimateWritebackCommands(stem, difficulty, justification, slug, cfg) —
// the ORDERED shell commands that persist one phase's difficulty AND append its
// `## Estimate` audit note, then read the phase back so the caller can see the
// core-derived tier. Returned as DATA; nothing here runs them.
//
// RUNNABLE AS EMITTED — paste the list into one shell session and it does what
// it says, with nothing for the caller to substitute first. That is the whole
// contract of a returned command ladder, and it is why `--append-body` exists:
// `--body` is whole-document-authoritative, so persisting the note through it
// would mean the CALLER reading the current body and handing it back, which is
// both a document crossing a model boundary and a clobber waiting for a dropped
// line. `--append-body` adds the note in rdm-core without anyone re-transmitting
// what is already there, so the emitted text can never destroy a body.
//
// The note is captured through a QUOTED heredoc, never interpolated into a
// command line, so backticks, `$` and punctuation ride through literally.
// `--model` is deliberately absent: the tier derives from the difficulty in
// rdm-core, and the last command is what reads it back.
function buildEstimateWritebackCommands(stem, difficulty, justification, slug, cfg) {
  const bin = resolveRdmBin(cfg && cfg.rdmBin);
  const proj = projectFlag(cfg);
  const show = bin + ' phase show ' + stem + ' --roadmap ' + slug + proj + ' --format json';
  const note = '## Estimate\n\n' + difficulty + ' — ' + justification;
  return [
    "RDM_ESTIMATE_NOTE=$(cat <<'RDM_ESTIMATE_EOF'\n" + note + '\nRDM_ESTIMATE_EOF\n)',
    '  ' + bin + ' phase update ' + stem + ' --difficulty ' + difficulty +
      ' --append-body "$RDM_ESTIMATE_NOTE" --no-edit --roadmap ' + slug + proj,
    '  ' + show,
  ];
}

// buildEstimatePipeline(deps) — returns the async runEstimate(config) driver.
// Every runtime side effect is reached through `deps`, so the block stays pure
// and the module imports cleanly in Node. The flow: take the CALLER-SUPPLIED
// phase list -> filter to the unestimated (optionally narrowed to one phase
// number) -> parallel-rate each -> build each one's writeback command text ->
// return a DETERMINISTIC summary object. THE ONLY AGENT IT DISPATCHES IS THE
// RATER, which is judgment; the list read and the writeback belong to the
// orchestrator, and the resulting tier is what `rdm phase show` reports once the
// returned commands have run. A phase whose
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

    const phaseList = Array.isArray(cfg.phaseList) ? cfg.phaseList : [];

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
        // The writeback is COMMAND TEXT, not a dispatch. There is no ack to read
        // and therefore no `tier` here: the tier is whatever `rdm phase show`
        // reports after the caller has run these commands, which is the last one
        // in the list.
        const commands = buildEstimateWritebackCommands(r.stem, r.difficulty, justification, roadmap, cfg);
        estimated.push({
          stem: r.stem,
          difficulty: r.difficulty,
          justification: justification,
          writebackCommands: commands,
          writebackScript: commands.join('\n'),
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
  lines.push('rated, writeback commands returned for you to run (' + estimated.length + '):');
  if (estimated.length) {
    for (const e of estimated) {
      lines.push('  - ' + e.stem + ': ' + e.difficulty + ' — ' + (e.justification || ''));
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

// `PHASE_LIST_SCHEMA`, `ACK_SCHEMA`, `TIER_SCHEMA`, `MECHANICAL_MODEL_SCHEMA`
// and `buildMechanicalModelPrompt` are GONE with the four mechanical agents
// whose returns they shaped. The rater's ESTIMATE schema above is the only one
// left, because the rater is the only agent left.

// --- Driver ------------------------------------------------------------------

const estimateArgs = parseEstimateArgs(args)
const roadmapSlug = estimateArgs.roadmap
// The environment payload threaded into every prompt that shells out.
// parseEstimateArgs already resolved (and validated) both axes above, before
// any agent() call, so a mis-invocation costs zero tokens.
const estimateCfg = { rdmBin: estimateArgs.rdmBin, project: estimateArgs.project }

// coerceRawArgs(a) — the same JSON-string tolerance parseEstimateArgs applies,
// so a stringified payload still surfaces the optional caller-supplied hoists
// below. Never throws: anything unusable yields {} and every hoist read then
// falls through to its agent.
function coerceRawArgs(a) {
  let raw = a || {}
  if (typeof raw === 'string') {
    try {
      raw = JSON.parse(raw) || {}
    } catch (e) {
      raw = {}
    }
  }
  if (!raw || typeof raw !== 'object') raw = {}
  return raw
}
// Optional caller-supplied hoists (see docs/mechanical-agent-inventory.md). The
// rdm-estimate shim is already a running agent with the repo in context, so it
// runs `rdm model resolve mechanical` / `rdm phase list --format json` itself
// and passes the results here — this workflow then spawns no subagent for them.
// Both are OPTIONAL: absent or malformed falls through to the original agent,
// which is what a direct `Workflow` invocation always does.
const rawEstimateArgs = coerceRawArgs(args)

// THE ONLY AGENT THIS ENGINE DISPATCHES IS THE RATER. The `model:mechanical`,
// `estimate:list`, `estimate:write:` and `estimate:tier:` agents are gone: the
// ORCHESTRATOR runs `rdm phase list` itself and passes the parsed array as
// `phaseList`, and the writeback comes back as command text for it to run.
if (!Array.isArray(rawEstimateArgs.phaseList)) {
  const cmd = estimateListCommand(roadmapSlug, estimateCfg)
  const msg =
    'estimate: no `phaseList` supplied — run `' +
    cmd +
    '` yourself and pass the parsed JSON array as `phaseList`. This engine reads nothing.'
  log(msg)
  return { roadmap: roadmapSlug, estimated: [], skipped: [], deferred: [], fetchError: true, listCommand: cmd }
}

// Real deps close over the ambient Workflow globals (agent/parallel/log). These
// live OUTSIDE the copied block; the block itself names no ambient global.
const realDeps = {
  log: function (msg) {
    log(msg)
  },
  // parallelRate is the difficulty-rating JUDGMENT agent, and the only agent
  // here. It reads each phase body itself, from the command its prompt names.
  parallelRate: async function (stems) {
    return parallel(
      stems.map(function (stem) {
        return function () {
          return agent(
            buildEstimatorPrompt(
              'Run `' +
                resolveRdmBin(estimateCfg && estimateCfg.rdmBin) +
                ' phase show ' +
                stem +
                ' --roadmap ' +
                roadmapSlug +
                projectFlag(estimateCfg) +
                ' --format json` and use the returned `body` field as the phase body.'
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
}

// parseEstimateArgs already enforced a non-empty roadmap slug (it throws
// otherwise), so roadmapSlug is guaranteed set here.
const summary = await buildEstimatePipeline(realDeps)(
  Object.assign({}, estimateArgs, { phaseList: rawEstimateArgs.phaseList, rdmBin: estimateCfg.rdmBin, project: estimateCfg.project })
)
log(buildEstimateSummaryText(summary))

return summary
