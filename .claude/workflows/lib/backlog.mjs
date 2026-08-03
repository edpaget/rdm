//! backlog — pure control logic for the propose-only backlog-grooming pass.
//!
//! This is the **single source of truth** for the deterministic control core of
//! the `rdm-wf-backlog` workflow: argument parsing, the fixed category registry over
//! `rdm backlog report`'s four signal arrays, the per-category analyzer prompt
//! builders (inlining the grooming rules the `rdm-backlog` skill used to carry
//! as prose), the JSON schema every analyzer agent is forced to satisfy, and the
//! orchestration that fetches the report once, fans out one analyzer per
//! populated category in parallel, and consolidates the results into one
//! reviewable, propose-only batch — or short-circuits to "Nothing to groom"
//! when the report carries no signals at all.
//!
//! Because the Claude Code Workflow runtime cannot `import`/`require` (see
//! docs/workflow-schemas.md § "Import spike"), the marked block below is copied
//! BYTE-IDENTICAL into `.claude/workflows/rdm-wf-backlog.js`;
//! `scripts/verify-workflow-backlog.sh` gates the two copies for byte-equality.
//!
//! Everything the block needs is self-contained (no imports, pure array/string
//! ops, no Date.now / Math.random) and it names NO ambient Workflow global
//! (`agent`/`parallel`/`workflow`/`log`): every side effect is reached through the
//! injected `deps` object, so importing this module in Node — where those globals
//! do not exist — never throws. The `export { … }` at the bottom lives OUTSIDE the
//! markers so it is never copied into the workflow script (whose only permitted
//! export is `meta`). The verify harness imports this module and unit-tests the
//! pure logic, then drives the pipeline with fakes — zero LLM calls — and,
//! separately, against a real seeded plan repo to prove zero mutations.
//!
//! ## Non-mutation guarantee
//!
//! This pipeline runs exactly ONE Bash-executing agent — the Stage-0 report
//! fetch (`buildFetchReportPrompt`), whose command template is `rdm backlog
//! report` only (never a mutating verb). Every analyzer agent is explicitly
//! told it is READ-ONLY and must propose text only, never execute a mutating
//! rdm command — mirroring review-refute-fix's "READ-ONLY reviewer" framing.
//! Every proposed action is returned as a `{command, rationale}` string pair
//! for a human to run later; nothing this pipeline does ever executes one.

// >>> backlog-groom:begin <<<
// Pure, deterministic control logic for the backlog-grooming pass.
//
// This block is the single source of truth in .claude/workflows/lib/backlog.mjs
// and is copied BYTE-IDENTICAL into .claude/workflows/rdm-wf-backlog.js (the Workflow
// runtime cannot load modules at run time). scripts/verify-workflow-backlog.sh
// gates the two copies for drift. No Date.now / Math.random — pure array/string
// ops only. The block names NO ambient runtime global (agent/parallel/log):
// every side effect is reached through the injected `deps` object, so the
// module imports cleanly in Node.

// parseBacklogArgs(args) — validate and normalize the run config. Every field
// is optional: `project` (string or null, standard resolution chain applies
// when omitted), `olderThan` (non-negative integer or null — 0 is MEANINGFUL
// and must never be conflated with unset by a falsy check), and `tag` (string
// or null — an explicitly-passed empty string is also meaningful and distinct
// from "not passed", so it is preserved rather than normalized to null).
// Defensive: a caller may stringify the Workflow tool payload, so a
// JSON-string `args` is parsed back into an object. A non-JSON or non-object
// value falls back to {} so every field resolves to its default rather than
// throwing on a primitive.
function parseBacklogArgs(args) {
  let a = args || {};
  if (typeof a === 'string') {
    try {
      a = JSON.parse(a) || {};
    } catch (e) {
      a = {};
    }
  }
  if (!a || typeof a !== 'object') a = {};
  const project = typeof a.project === 'string' && a.project !== '' ? a.project : null;
  let olderThan = null;
  if (a.olderThan !== undefined && a.olderThan !== null && a.olderThan !== '') {
    const n = typeof a.olderThan === 'number' ? a.olderThan : parseInt(a.olderThan, 10);
    if (!Number.isInteger(n) || n < 0) {
      throw new Error('backlog: --older-than must be a non-negative integer (got "' + String(a.olderThan) + '")');
    }
    olderThan = n;
  }
  // typeof-string check (not a truthiness check) so an explicit '' survives —
  // only undefined/null/non-string collapse to "unset".
  const tag = typeof a.tag === 'string' ? a.tag : null;
  return { project: project, olderThan: olderThan, tag: tag };
}

// CATEGORY — the fixed, deterministic order every category is considered in,
// mirroring the four arrays `rdm backlog report --format json` returns. Each
// entry's `analyzerPrompt(items, cfg)` inlines the grooming rules the
// `rdm-backlog` skill used to carry as prose.

// analyzerPreamble(title) — the shared READ-ONLY framing every analyzer
// prompt opens with, mirroring review-refute-fix's "READ-ONLY reviewer"
// instruction: propose text only, never execute a mutating command, and hold
// destructive-if-wrong proposals (retire/merge/archive) to a stricter
// confidence bar than purely additive ones (proposing a new roadmap).
function analyzerPreamble(title) {
  return [
    'You are a READ-ONLY backlog-grooming analyst for the "' + title + '" category.',
    'Propose text ONLY. You must NEVER execute create/update/merge/archive/promote/commit/discard,',
    'or any other mutating rdm command — every action you propose is for a human to run later,',
    'never executed by you. You may run read-only lookups only (e.g. `./target/debug/rdm roadmap',
    'list`, `./target/debug/rdm search`) to inform your analysis.',
    'Hold destructive-if-wrong actions (retire, merge, archive) to a STRICTER confidence bar than',
    'purely additive ones (proposing a new roadmap): if you are not confident, file an open question',
    'instead of guessing — never propose a merge/retire/archive/consolidate action you are unsure of.',
  ].join('\n');
}

// autopilotFramingNote() — shared note attached to every prompt that may
// propose creating/extending a thematic roadmap via `promote`.
function autopilotFramingNote() {
  return [
    'Autopilot-oriented framing: whenever you propose creating or extending a thematic roadmap, the',
    'phase body you propose (the --body/stdin content for `promote`) must be structured with',
    '## Context / ## Steps / ## Acceptance Criteria headings — the same shape every existing phase',
    'body uses — so a later `rdm-estimate` and `rdm-autopilot` can pick the roadmap up immediately.',
  ].join('\n');
}

// projectFlag(cfg) — the ` --project <name>` suffix to append to a proposed
// command, or '' when no project was configured (the standard resolution
// chain then applies).
function projectFlag(cfg) {
  return cfg && cfg.project ? ' --project ' + cfg.project : '';
}

// itemsBlock(items) — the category's raw report items, verbatim, fenced for
// the analyzer to read.
function itemsBlock(items) {
  return ['--- ITEMS (JSON) ---', JSON.stringify(items, null, 2), '--- END ITEMS ---'].join('\n');
}

// outputContract() — the JSON shape every analyzer must return (ANALYSIS_SCHEMA
// below is the enforced version of this contract).
function outputContract() {
  return [
    'Return JSON exactly matching:',
    '{ "proposals": [{"command": "<exact rdm command a human would run>", "rationale": "<why>"}],',
    '  "openQuestions": ["<the ambiguous case and why — NO command attached>"] }',
    'Both arrays may be empty. Never put a command you are not confident about into `proposals` —',
    'file it under `openQuestions` instead.',
  ].join('\n');
}

// promptStaleTasks(items, cfg) — retire-vs-consolidate rules for stale_tasks.
function promptStaleTasks(items, cfg) {
  const proj = projectFlag(cfg);
  return [
    analyzerPreamble('Stale tasks'),
    '',
    'For each task below, decide exactly ONE of:',
    '- Retire, if it reads as superseded or no longer relevant:',
    '    ./target/debug/rdm task update <slug> --status wont-fix --reason "<why it is stale / superseded>" --no-edit' +
      proj,
    '- Consolidate, if it is still valid work that fits a theme, into an existing roadmap:',
    '    ./target/debug/rdm promote <slug> --into <roadmap> --no-edit' + proj,
    '  or into a brand new one (no --no-edit/--body on this form — those apply only to --into):',
    '    ./target/debug/rdm promote <slug> --roadmap-slug <new-slug>' + proj,
    '- Otherwise: file an open question. Do not guess.',
    '',
    autopilotFramingNote(),
    '',
    itemsBlock(items),
    '',
    outputContract(),
  ].join('\n');
}

// promptDuplicateClusters(items, cfg) — survivor-pick rules for duplicate_clusters.
function promptDuplicateClusters(items, cfg) {
  const proj = projectFlag(cfg);
  return [
    analyzerPreamble('Duplicate clusters'),
    '',
    'For each cluster below, pick a survivor — state the rule you used (most complete body, or',
    'earliest `created`) — and fold the rest into it:',
    '    ./target/debug/rdm task merge <survivor> --from <other1> --from <other2> --no-edit' + proj,
    'If no survivor is clearly best, file an open question instead. Do not guess.',
    '',
    itemsBlock(items),
    '',
    outputContract(),
  ].join('\n');
}

// promptTagClusters(items, cfg) — existing-roadmap-check rules for tag_clusters.
function promptTagClusters(items, cfg) {
  const proj = projectFlag(cfg);
  return [
    analyzerPreamble('Tag clusters'),
    '',
    'A cluster of related tasks sharing one tag is a consolidation candidate. First check',
    '(read-only) whether a thematic roadmap already covers it:',
    '    ./target/debug/rdm roadmap list' + proj,
    '    ./target/debug/rdm search <tag> --type roadmap' + proj,
    'If one exists, propose one `promote --into` per task:',
    '    ./target/debug/rdm promote <slug> --into <existing-roadmap> --no-edit' + proj,
    'If none exists, propose a create-then-fold sequence: first (no --no-edit/--body on this form —',
    'those apply only to --into):',
    '    ./target/debug/rdm promote <first-slug> --roadmap-slug <new-thematic-slug>' + proj,
    'then for every remaining task in the cluster:',
    '    ./target/debug/rdm promote <slug> --into <that-new-slug> --no-edit' + proj,
    'Never propose both --into and --roadmap-slug for the same task — they are mutually exclusive.',
    '',
    autopilotFramingNote(),
    '',
    itemsBlock(items),
    '',
    outputContract(),
  ].join('\n');
}

// promptArchivableRoadmaps(items, cfg) — archive rationale for archivable_roadmaps.
function promptArchivableRoadmaps(items, cfg) {
  const proj = projectFlag(cfg);
  return [
    analyzerPreamble('Archivable roadmaps'),
    '',
    'Each roadmap below is already all-terminal (every phase done/wont-fix) but not yet archived.',
    'For each one, propose:',
    '    ./target/debug/rdm roadmap archive <roadmap>' + proj,
    'with rationale "all phases terminal, not yet archived." Never add --force — these candidates',
    'never need it; --force exists only to override an INCOMPLETE roadmap, which is not this case.',
    '',
    itemsBlock(items),
    '',
    outputContract(),
  ].join('\n');
}

const CATEGORY = [
  { key: 'stale_tasks', arrayField: 'stale_tasks', title: 'Stale tasks', analyzerPrompt: promptStaleTasks },
  {
    key: 'duplicate_clusters',
    arrayField: 'duplicate_clusters',
    title: 'Duplicate clusters',
    analyzerPrompt: promptDuplicateClusters,
  },
  { key: 'tag_clusters', arrayField: 'tag_clusters', title: 'Tag clusters', analyzerPrompt: promptTagClusters },
  {
    key: 'archivable_roadmaps',
    arrayField: 'archivable_roadmaps',
    title: 'Archivable roadmaps',
    analyzerPrompt: promptArchivableRoadmaps,
  },
];

// ANALYSIS_SCHEMA — the JSON schema every analyzer agent is forced to satisfy.
const ANALYSIS_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['proposals', 'openQuestions'],
  properties: {
    proposals: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['command', 'rationale'],
        properties: {
          command: { type: 'string', minLength: 1 },
          rationale: { type: 'string', minLength: 1 },
        },
      },
    },
    openQuestions: {
      type: 'array',
      items: { type: 'string', minLength: 1 },
    },
  },
};

// buildFetchReportPrompt(cfg) — the ONE Bash-executing agent prompt in the
// whole pipeline. `cmd` is seeded from a literal read-only command and only
// ever grows by appending flag text — never a mutating verb — so the
// executable command template stays provably read-only by construction.
function buildFetchReportPrompt(cfg) {
  const c = cfg || {};
  let cmd = './target/debug/rdm backlog report --format json';
  if (c.olderThan != null) cmd += ' --older-than ' + c.olderThan;
  if (typeof c.tag === 'string' && c.tag !== '') cmd += ' --tag ' + c.tag;
  if (c.project) cmd += ' --project ' + c.project;
  return [
    'You are a mechanical fetch agent. Do not plan, analyze, or mutate anything.',
    'Run exactly this command in the repo root and read its JSON output:',
    '  ' + cmd,
    'Return the parsed JSON verbatim as an object with four arrays: `stale_tasks` (slug, title,',
    'status, created, age_days), `duplicate_clusters` (members: slug/title), `tag_clusters` (tag,',
    'tasks: slug/title), and `archivable_roadmaps` (roadmap, title, phase_count). Any array missing',
    'from the command output should be returned as an empty array.',
  ].join('\n');
}

// parseBacklogReport(raw) — validate/default the four signal arrays from
// `rdm backlog report --format json`. A string is JSON-parsed; a
// null/non-object result is a genuine FETCH ERROR (thrown), never laundered
// into an empty report — an empty report is a report that parsed fine but
// carries no signals, a fetch failure is something else entirely.
function parseBacklogReport(raw) {
  let r = raw;
  if (typeof r === 'string') {
    try {
      r = JSON.parse(r);
    } catch (e) {
      throw new Error('backlog: report fetch did not return valid JSON');
    }
  }
  if (!r || typeof r !== 'object') {
    throw new Error('backlog: report fetch returned no data');
  }
  return {
    stale_tasks: Array.isArray(r.stale_tasks) ? r.stale_tasks : [],
    duplicate_clusters: Array.isArray(r.duplicate_clusters) ? r.duplicate_clusters : [],
    tag_clusters: Array.isArray(r.tag_clusters) ? r.tag_clusters : [],
    archivable_roadmaps: Array.isArray(r.archivable_roadmaps) ? r.archivable_roadmaps : [],
  };
}

// selectCategories(report) — the populated categories, in CATEGORY order.
// Deterministic: no Date.now / Math.random anywhere in the selection.
function selectCategories(report) {
  const r = report || {};
  return CATEGORY.filter((cat) => Array.isArray(r[cat.arrayField]) && r[cat.arrayField].length > 0);
}

// normalizeAnalysis(result) — defend against a malformed/absent analyzer
// result. `null` (a crashed or null-resolved analyzer) propagates as null so
// the caller can degrade that category to an open question rather than
// aborting the whole run.
function normalizeAnalysis(result) {
  if (!result) return null;
  return {
    proposals: Array.isArray(result.proposals) ? result.proposals : [],
    openQuestions: Array.isArray(result.openQuestions) ? result.openQuestions : [],
  };
}

// consolidateBatch(perCategory) — render ONE ordered, reviewable batch: a
// "### <title>" subsection per category that actually produced a proposal
// (a category whose analyzer crashed or proposed nothing is never given an
// empty header), followed by ONE merged "## Open questions" section
// aggregating every category's open questions (plus a note for any category
// whose analyzer failed outright). Always ends with the Open questions
// section, even when empty ("None.") — mirroring the old skill's rule that
// the section is always present.
function consolidateBatch(perCategory) {
  const items = Array.isArray(perCategory) ? perCategory : [];
  const lines = ['## Grooming plan', ''];
  const openQuestions = [];
  for (const entry of items) {
    const cat = entry && entry.cat;
    if (!cat) continue;
    const result = entry.result;
    if (!result) {
      openQuestions.push('[' + cat.title + '] analysis failed for this category — review manually');
      continue;
    }
    const proposals = Array.isArray(result.proposals) ? result.proposals : [];
    if (proposals.length) {
      lines.push('### ' + cat.title);
      lines.push('');
      for (const p of proposals) {
        const command = (p && p.command) || '';
        const rationale = (p && p.rationale) || '';
        lines.push('- `' + command + '` — ' + rationale);
      }
      lines.push('');
    }
    const oq = Array.isArray(result.openQuestions) ? result.openQuestions : [];
    for (const q of oq) {
      openQuestions.push('[' + cat.title + '] ' + q);
    }
  }
  lines.push('## Open questions');
  lines.push('');
  if (openQuestions.length) {
    for (const q of openQuestions) lines.push('- ' + q);
  } else {
    lines.push('None.');
  }
  return lines.join('\n');
}

// buildBacklogPipeline(deps) — returns the async runBacklog(args) driver.
// deps.fetchReport(cfg) has NO ambient equivalent (there is nothing in the
// Workflow runtime that could stand in for "shell out to `rdm backlog
// report`") and is therefore REQUIRED; deps.agent/deps.parallel default to
// the ambient Workflow globals when omitted, exactly like review.mjs's
// buildReviewPipeline, so the module still imports cleanly in Node (where
// those globals do not exist) as long as the harness injects fakes.
function buildBacklogPipeline(deps) {
  const d = deps || {};
  const _agent = d.agent || (typeof agent !== 'undefined' ? agent : undefined);
  const _parallel = d.parallel || (typeof parallel !== 'undefined' ? parallel : undefined);
  const _log = d.log || (typeof log !== 'undefined' ? log : function () {});
  const _fetchReport = d.fetchReport;
  if (!_fetchReport) {
    throw new Error('backlog: deps.fetchReport is required (no ambient equivalent — inject a report-fetching function)');
  }
  if (!_agent || !_parallel) {
    throw new Error('backlog: missing agent/parallel (pass deps outside the Workflow runtime)');
  }

  return async function runBacklog(args) {
    const cfg = parseBacklogArgs(args);
    // A fetch failure (bad command, non-JSON, no output) is allowed to THROW
    // here, uncaught — it must surface as a fetch error, never be laundered
    // into a false "Nothing to groom".
    const rawReport = await _fetchReport(cfg);
    const report = parseBacklogReport(rawReport);
    const categories = selectCategories(report);
    if (categories.length === 0) {
      return { groomed: false, summary: 'Nothing to groom — the backlog report returned no signals' };
    }
    const perCategory = await _parallel(
      categories.map((cat) => () =>
        _agent(cat.analyzerPrompt(report[cat.arrayField], cfg), {
          label: 'analyze:' + cat.key,
          schema: ANALYSIS_SCHEMA,
        })
          .then((result) => ({ cat: cat, result: normalizeAnalysis(result) }))
          .catch(() => {
            _log('backlog: analyzer for "' + cat.key + '" failed — degrading to an open question');
            return { cat: cat, result: null };
          })
      )
    );
    const summary = consolidateBatch(perCategory);
    _log(
      'backlog: analyzed ' + categories.length + ' populated categor' + (categories.length === 1 ? 'y' : 'ies')
    );
    return { groomed: true, summary: summary };
  };
}
// >>> backlog-groom:end <<<

// Node-only exports for the verify harness. NOT part of the copied block — the
// marker END is above this line, so a copy never carries these.
export {
  parseBacklogArgs,
  CATEGORY,
  ANALYSIS_SCHEMA,
  analyzerPreamble,
  autopilotFramingNote,
  projectFlag,
  itemsBlock,
  outputContract,
  promptStaleTasks,
  promptDuplicateClusters,
  promptTagClusters,
  promptArchivableRoadmaps,
  buildFetchReportPrompt,
  parseBacklogReport,
  selectCategories,
  normalizeAnalysis,
  consolidateBatch,
  buildBacklogPipeline,
};
