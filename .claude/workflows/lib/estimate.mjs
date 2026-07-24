//! estimate — the single canonical estimate orchestration for every rdm surface.
//!
//! This module is the **one source of truth** for the estimate core:
//! **list phases → filter to the unestimated → parallel-rate each → write back
//! the difficulty (and an audit note) → read the core-derived tier back**. Two
//! surfaces consume the SAME marked block, so a change to the estimate loop
//! lands once and behaves identically everywhere:
//!
//!   * the standalone `.claude/workflows/estimate.js` workflow (a human running
//!     it directly, or the `rdm-estimate` skill shim);
//!   * the `.claude/workflows/autopilot.js` estimate pre-pass, which reuses
//!     `selectUnestimated` / `buildEstimateWritebackPrompt` from the same block.
//!
//! Because the Claude Code Workflow runtime cannot `import`/`require` (proven by
//! the P1 import spike — see docs/workflow-schemas.md § "Import spike"), the
//! marked `estimate-core` block below is copied BYTE-IDENTICAL into every
//! consumer by `scripts/gen-workflow-estimate.sh`; its `--check` mode fails the
//! build on drift, and `scripts/verify-workflow-estimate.sh` gates it.
//!
//! **Tier resolution stays in `rdm-core`.** This block never reimplements the
//! difficulty→tier mapping (`Difficulty::model_tier`, rdm-core/src/model.rs is
//! authoritative): the writeback sets `--difficulty` only and NEVER `--model`,
//! and the summary reads the core-derived tier back from `rdm phase show`.
//!
//! Everything the block needs is self-contained (no imports, pure array/string
//! ops, no Date.now / Math.random) and it names NO ambient Workflow global
//! (`agent`/`parallel`/`workflow`/`log`): every side effect is reached through
//! the injected `deps` object, so importing this module in Node — where those
//! globals do not exist — never throws. The `export { … }` at the bottom lives
//! OUTSIDE the markers so it is never copied into a workflow script.

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

// Node-only exports for the verify harness. NOT part of the copied block — the
// marker END is above this line, so a copy never carries these.
export {
  parseEstimateArgs,
  selectUnestimated,
  buildEstimateListPrompt,
  buildEstimatorPrompt,
  buildEstimateWritebackPrompt,
  buildEstimateTierPrompt,
  buildEstimatePipeline,
  buildEstimateSummaryText,
};
