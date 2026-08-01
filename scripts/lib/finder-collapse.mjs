// finder-collapse.mjs — the canonical A/B instrument for the collapsed plan
// finder (`bound-review-fan-out` phase 5).
//
// THE QUESTION
// ------------------------------------------------------------------------
// `plan` mode's DIMENSIONS list has THREE always-on dimensions — `coherence`,
// `architectural-fit` and `restraint` — each dispatched as its own finder agent,
// each paying its own agent context floor, and all three resolving the same
// `FINDINGS_SCHEMA`. Collapsing them into ONE agent holding three lenses would
// cut two agents per review unit. `plan-review.js` fans out per unit with
// `parallel()`, and a roadmap emits its own body as a unit IN ADDITION to one
// per phase, so a five-phase roadmap pays eighteen always-on plan finders.
//
// The risk is DILUTION: one agent asked to hold three review lenses may find
// less per lens than three agents each holding one. That is an empirical
// question, and this module exists to answer it rather than assert it. A
// material per-lens loss is a LEGITIMATE TERMINAL NEGATIVE — see
// `docs/finder-collapse.md`.
//
// SHAPE (mirrors `scripts/lib/refuter-agreement.mjs`, its sibling instrument)
// ------------------------------------------------------------------------
//   * `loadCorpus`     — validate the mined review-unit corpus.
//   * `assessPower`    — is the population big enough to answer the question?
//   * `buildCollapseTrials` — THROWS on an underpowered population unless
//     `allowUnderpowered`, which stamps `noMeasurement: true` and forces the
//     report to print a NO MEASUREMENT banner and suppress the decision line.
//   * arm A prompts come from the REAL exported `findPrompt` + the REAL
//     always-on `DIMENSIONS.plan` entries, injected by the caller. This module
//     NEVER carries its own copy of the production prompt — a drifted copy
//     would measure a strawman.
//   * `buildCollapsedPlanPrompt` — arm B. It lives HERE during the experiment,
//     so a no-ship decision leaves `.claude/workflows/lib/review.mjs`
//     byte-unchanged. On a ship decision it moves into `review.mjs` and this
//     module imports it instead, with the harness asserting byte-equality.
//   * `scoreCollapse`  — per-lens counts, per-lens severity distribution,
//     adjudicated material recall, `concern` attribution validity, per-class
//     token totals, and the SIX-CRITERION decision table.
//   * `auditCollapseDoc` — a corpus-free arithmetic audit of the committed
//     `docs/token-baseline.json` § `planFinderCollapse` figures.
//
// Determinism: no clock, no RNG, no network beyond the injected dispatcher.
// Nothing under `.claude/workflows/` imports this module; it imports nothing
// from the lane at all (the caller injects `findPrompt`/`DIMENSIONS`), so the
// dependency can only ever point one way.

import { spawn } from 'node:child_process';

// --- Corpus vocabulary ------------------------------------------------------

/** Bumped whenever a mined corpus record's shape changes incompatibly. */
export const PLAN_FINDER_CORPUS_SCHEMA_VERSION = 1;

/**
 * The three ALWAYS-ON plan dimensions, in `DIMENSIONS.plan` order.
 *
 * NOTE: `CLAUDE.md` described the always-on plan set as
 * "coherence/architectural-fit" for some time, omitting `restraint`. That prose
 * was stale; `restraint` carries no `when` predicate and has always been
 * always-on. `scripts/verify-finder-collapse.sh` asserts this list matches the
 * real `DIMENSIONS.plan` when-less entries, so the two cannot drift again.
 */
export const ALWAYS_ON_PLAN_LENSES = ['coherence', 'architectural-fit', 'restraint'];

/** The one TRIGGERED plan dimension. Never merged — see docs/finder-collapse.md. */
export const TRIGGERED_PLAN_DIMENSION = 'unit-of-work';

/** Plan target types a review unit may carry, as the unit identity spells them. */
export const PLAN_TARGET_TYPES = ['phase', 'task', 'roadmap'];

/**
 * Same plausibility bound `scripts/lib/refuter-agreement.mjs` applies to a unit
 * identity, so the two instruments bucket the same dispatches into the same
 * units and their figures stay comparable.
 */
export const MAX_UNIT_IDENT_LENGTH = 200;

/**
 * PRE-REGISTERED floor on a usable plan document, fixed before any dispatch.
 *
 * Some recorded plan-review units were dispatched with a FETCH-STATUS LINE in
 * place of the plan body ("Successfully fetched roadmap X ... from the rdm
 * project", 36-102 chars in the pinned window). That is not a plan document: an
 * agent asked to hold three lenses over it would find nothing in either arm, and
 * the unit would dilute the very effect being measured while inflating the
 * apparent corpus. Such units are EXCLUDED AND COUNTED, never padded in.
 *
 * 500 chars sits in the EMPTY BAND between those degenerate targets (max 102)
 * and the smallest real plan document in the window (783) — no unit falls
 * between them, so the threshold is not a tuned parameter.
 */
export const MIN_PLAN_DOC_CHARS = 500;

/** Token classes, matching `scripts/lib/token-report.mjs`'s classification. */
export const TOKEN_CLASSES = ['output', 'uncachedInput', 'cacheWrite', 'cacheRead'];

/** Finding severities, in `SEVERITY_RANK` order. */
export const SEVERITIES = ['blocking', 'concern', 'suggestion'];

function isPlainObject(v) {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

function nonEmptyString(v) {
  return typeof v === 'string' && v.trim() !== '';
}

/**
 * The review unit's IDENTITY — the first line of the plan-mode `findPrompt`
 * target. Rejected (null) when it cannot be a real identity: empty, containing
 * `{` or `"` (the `--implementation-plan` shape is raw pretty-printed JSON), or
 * longer than `MAX_UNIT_IDENT_LENGTH`. A rejected identity is excluded and
 * counted, never bucketed into a fake unit.
 *
 * @param {string} target
 * @returns {string|null}
 */
export function unitIdentOf(target) {
  if (typeof target !== 'string') return null;
  const newlineAt = target.indexOf('\n');
  const first = newlineAt === -1 ? target : target.slice(0, newlineAt);
  if (first.length === 0) return null;
  if (first.indexOf('{') !== -1 || first.indexOf('"') !== -1) return null;
  if (first.length > MAX_UNIT_IDENT_LENGTH) return null;
  return first;
}

/**
 * The plan target type a unit identity declares (`phase <roadmap>/<stem>`,
 * `task task/<slug>`, `roadmap <slug> (body)`), or null when the identity is
 * not one of the three shapes.
 *
 * @param {string} ident
 * @returns {string|null}
 */
export function targetTypeOf(ident) {
  if (typeof ident !== 'string') return null;
  for (const t of PLAN_TARGET_TYPES) {
    if (ident.startsWith(t + ' ')) return t;
  }
  return null;
}

/**
 * The plan DOCUMENT body: everything after the identity line, with the single
 * separating blank line removed. Returns '' when the target carries no body.
 *
 * @param {string} target
 * @returns {string}
 */
export function planDocOf(target) {
  if (typeof target !== 'string') return '';
  const newlineAt = target.indexOf('\n');
  if (newlineAt === -1) return '';
  return target.slice(newlineAt + 1).replace(/^\n/, '');
}

const UNIT_FIELDS = [
  'id',
  'schemaVersion',
  'targetType',
  'targetId',
  'target',
  'planDoc',
  'armA',
  'armAUsage',
  'provenance',
];

/**
 * Validate one mined review-unit record. Unknown top-level keys are REJECTED,
 * so a typo'd hand-edit cannot pass silently and then read as `undefined` in
 * the scorer.
 *
 * @param {unknown} unit
 * @returns {{ ok: boolean, errors: string[] }}
 */
export function validateUnit(unit) {
  const errors = [];
  if (!isPlainObject(unit)) return { ok: false, errors: ['unit is not an object'] };
  for (const key of Object.keys(unit)) {
    if (UNIT_FIELDS.indexOf(key) === -1) errors.push(`unknown top-level key "${key}"`);
  }
  for (const key of UNIT_FIELDS) {
    if (!(key in unit)) errors.push(`missing required key "${key}"`);
  }
  if (!nonEmptyString(unit.id)) errors.push('id must be a non-empty string');
  if (unit.schemaVersion !== PLAN_FINDER_CORPUS_SCHEMA_VERSION) {
    errors.push(`schemaVersion must be ${PLAN_FINDER_CORPUS_SCHEMA_VERSION}, got ${JSON.stringify(unit.schemaVersion)}`);
  }
  if (PLAN_TARGET_TYPES.indexOf(unit.targetType) === -1) {
    errors.push(`targetType must be one of ${PLAN_TARGET_TYPES.join('|')}, got ${JSON.stringify(unit.targetType)}`);
  }
  if (!nonEmptyString(unit.targetId)) errors.push('targetId must be a non-empty string');
  if (!nonEmptyString(unit.target)) errors.push('target must be a non-empty string');
  if (typeof unit.planDoc !== 'string' || unit.planDoc.length < MIN_PLAN_DOC_CHARS) {
    errors.push(`planDoc must be a string of at least ${MIN_PLAN_DOC_CHARS} chars`);
  }
  if (!isPlainObject(unit.armA) || !isPlainObject(unit.armA.byLens)) {
    errors.push('armA must be an object with a byLens map');
  } else {
    for (const lens of ALWAYS_ON_PLAN_LENSES) {
      if (!Array.isArray(unit.armA.byLens[lens])) errors.push(`armA.byLens.${lens} must be an array`);
    }
  }
  if (!isPlainObject(unit.armAUsage)) errors.push('armAUsage must be an object');
  if (!isPlainObject(unit.provenance) || !nonEmptyString(unit.provenance.runId)) {
    errors.push('provenance must be an object carrying a runId');
  }
  return { ok: errors.length === 0, errors };
}

/**
 * Parse a mined corpus JSONL. The FIRST record must be the pinned-window header
 * (`kind: 'header'`); everything after it is a review unit. Duplicate ids are
 * rejected — two units sharing an id would double-count in every statistic.
 *
 * @param {string} text
 * @returns {{ header: object|null, units: object[], errors: string[] }}
 */
export function loadCorpus(text) {
  const units = [];
  const errors = [];
  const seen = new Set();
  let header = null;
  const lines = String(text).split('\n');
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (line === '') continue;
    let parsed;
    try {
      parsed = JSON.parse(line);
    } catch (err) {
      errors.push(`line ${i + 1}: not parseable JSON: ${err.message}`);
      continue;
    }
    if (isPlainObject(parsed) && parsed.kind === 'header') {
      if (header !== null) errors.push(`line ${i + 1}: a corpus carries exactly one header record`);
      else if (!isPlainObject(parsed.window)) errors.push(`line ${i + 1}: header is missing its pinned \`window\``);
      else header = parsed;
      continue;
    }
    const { ok, errors: unitErrors } = validateUnit(parsed);
    if (!ok) {
      for (const e of unitErrors) errors.push(`line ${i + 1} (${parsed && parsed.id}): ${e}`);
      continue;
    }
    if (seen.has(parsed.id)) {
      errors.push(`line ${i + 1}: duplicate id "${parsed.id}"`);
      continue;
    }
    seen.add(parsed.id);
    units.push(parsed);
  }
  if (header === null) errors.push('corpus is missing its header record (the pinned selection window)');
  return { header, units, errors };
}

// --- Power ------------------------------------------------------------------

/**
 * PRE-REGISTERED power floors, fixed before any dispatch.
 *
 *   minUnits      — 8 review units. Below that, one unit's stochastic miss moves
 *                   a per-lens count by more than the decision rule's own
 *                   tolerance, so a "loss" would be noise rather than dilution.
 *   minReplicates — 2 per arm per unit. Finder output is stochastic; a one-shot
 *                   A/B reads a random miss as a per-lens loss.
 *   minTargetTypes — the population must span at least 2 distinct plan target
 *                   types, or the measurement is scoped to one document shape.
 *                   It is 2 and not 3 for a MEASURED reason, not convenience:
 *                   in the pinned window NO `roadmap`-body unit carries a real
 *                   plan document at all — plan-review's fetch step handed those
 *                   units a status line ("Successfully fetched roadmap X …")
 *                   instead of the body, so every one of them falls below
 *                   `MIN_PLAN_DOC_CHARS`. Requiring 3 would make the floor
 *                   unmeetable for a reason that has nothing to do with lens
 *                   dilution. Recorded as a limitation and filed separately.
 *   requiredLenses — all three lenses must have fired somewhere in the
 *                   population, or a silent lens is untestable by construction.
 */
export const POWER_FLOORS = {
  minUnits: 8,
  minReplicates: 2,
  minTargetTypes: 2,
  requiredLenses: ALWAYS_ON_PLAN_LENSES.slice(),
};

/**
 * Is this population big enough to answer the dilution question?
 *
 * Returns `power: 'SUFFICIENT' | 'UNDERPOWERED'` plus every input to that call,
 * so the report can print the arithmetic rather than the conclusion alone.
 *
 * @param {object[]} units
 * @param {{ replicates?: number, floors?: object }} [opts]
 */
export function assessPower(units, opts = {}) {
  const list = Array.isArray(units) ? units : [];
  const floors = { ...POWER_FLOORS, ...(opts.floors || {}) };
  const replicates = opts.replicates === undefined ? floors.minReplicates : opts.replicates;

  const byTargetType = {};
  for (const t of PLAN_TARGET_TYPES) byTargetType[t] = 0;
  const lensesWithFindings = {};
  for (const lens of ALWAYS_ON_PLAN_LENSES) lensesWithFindings[lens] = 0;

  for (const u of list) {
    if (byTargetType[u.targetType] !== undefined) byTargetType[u.targetType] += 1;
    for (const lens of ALWAYS_ON_PLAN_LENSES) {
      const found = (u.armA && u.armA.byLens && u.armA.byLens[lens]) || [];
      if (found.length > 0) lensesWithFindings[lens] += 1;
    }
  }

  const reasons = [];
  if (list.length < floors.minUnits) {
    reasons.push(`only ${list.length} review unit(s) against a floor of ${floors.minUnits}`);
  }
  if (replicates < floors.minReplicates) {
    reasons.push(`only ${replicates} replicate(s) per arm against a floor of ${floors.minReplicates}`);
  }
  const typesPresent = PLAN_TARGET_TYPES.filter((t) => byTargetType[t] > 0);
  if (typesPresent.length < floors.minTargetTypes) {
    reasons.push(
      `only ${typesPresent.length} plan target type(s) present (${typesPresent.join(', ') || 'none'}) ` +
        `against a floor of ${floors.minTargetTypes}`
    );
  }
  const silentLenses = floors.requiredLenses.filter((l) => lensesWithFindings[l] === 0);
  if (silentLenses.length) reasons.push(`no recorded arm-A finding for lens: ${silentLenses.join(', ')}`);

  return {
    power: reasons.length === 0 ? 'SUFFICIENT' : 'UNDERPOWERED',
    unitCount: list.length,
    replicates,
    targetTypesPresent: typesPresent,
    byTargetType,
    lensesWithFindings,
    floors,
    reasons,
  };
}

/**
 * The RUN population, chosen deterministically and BEFORE any dispatch: sort
 * every qualifying unit by id, bucket by target type, then take units
 * round-robin across the buckets in `PLAN_TARGET_TYPES` order until `n` are
 * chosen. Round-robin guarantees the scarce target types (one roadmap body, one
 * task) are represented rather than crowded out by the phase bucket.
 *
 * Pure and total — no clock, no RNG, no post-hoc selection. `n >= units.length`
 * returns every unit.
 *
 * @param {object[]} units
 * @param {number} n
 * @returns {object[]}
 */
export function selectRunUnits(units, n) {
  const list = (Array.isArray(units) ? units : []).slice().sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  if (!Number.isInteger(n) || n < 1) throw new Error(`selectRunUnits: n must be a positive integer, got ${JSON.stringify(n)}`);
  const buckets = PLAN_TARGET_TYPES.map((t) => list.filter((u) => u.targetType === t));
  const chosen = [];
  let progressed = true;
  while (chosen.length < n && progressed) {
    progressed = false;
    for (const b of buckets) {
      if (chosen.length >= n) break;
      const next = b.shift();
      if (next) {
        chosen.push(next);
        progressed = true;
      }
    }
  }
  // Stable output order regardless of the interleave, so a trial plan built
  // twice from the same corpus is byte-identical.
  return chosen.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
}

// --- Arm B: the collapsed prompt --------------------------------------------

/**
 * Arm B's prompt: ONE finder holding all three always-on lenses.
 *
 * A MINIMAL DELTA from the real `findPrompt`: same READ-ONLY stance, same
 * "Review target" line, same plan-mode diff hint, the same
 * `PLAN_SEVERITY_CALIBRATION` paragraph injected exactly once, the same
 * evidence instruction, and the same FINDINGS-schema closing instruction. The
 * ONLY change is that the single-dimension sentence becomes an enumerated list
 * of the three lenses, plus the three instructions the merge makes necessary.
 * Any wording change beyond that would be a confound — the A/B varies the number
 * of agents, and that only.
 *
 * `lenses` are the REAL `DIMENSIONS.plan` always-on entries, injected by the
 * caller, so this builder carries no copy of the production `focus` text.
 *
 * @param {{key: string, title: string, focus: string}[]} lenses
 * @param {{ target?: string, calibration?: string }} context
 * @returns {string}
 */
export function buildCollapsedPlanPrompt(lenses, context) {
  const ctx = context || {};
  const target = ctx.target || '(the target described in your working directory)';
  const list = Array.isArray(lenses) ? lenses : [];
  const keys = list.map((d) => d.key);
  const lines = [
    'You are a READ-ONLY reviewer. Do not edit any files.',
    'Review target: ' + target + '.',
    'Inspect the plan document text.',
    'You hold ' + list.length + ' review lenses. Review the plan under EVERY one of them:',
  ];
  list.forEach((dim, i) => {
    lines.push(String(i + 1) + '. ' + dim.title + ' (' + dim.key + '): ' + dim.focus);
  });
  if (ctx.calibration) lines.push(ctx.calibration);
  lines.push(
    'Review every lens INDEPENDENTLY and do not trade one off against another: a clean verdict under one lens is not evidence about any other, and a finding under one lens never excuses skipping another.',
    "Each finding's `concern` MUST be exactly one of: " + keys.join(', ') + '.',
    'Never emit `concern: ' +
      TRIGGERED_PLAN_DIMENSION +
      '` — that lens belongs to a separate reviewer and is not yours to report.',
    'Report only findings you can back with concrete evidence. One strong finding beats five weak ones.',
    'Return JSON matching the FINDINGS schema: a `findings` array, each with id, concern, location, severity (blocking|concern|suggestion), confidence (0-100), what_fails, why, recommendation.',
    'Return an empty `findings` array ONLY when ALL of the lenses above are clean.'
  );
  return lines.join('\n');
}

// --- Trials -----------------------------------------------------------------

/**
 * Build the trial plan for both arms.
 *
 * THROWS on an underpowered population — an A/B whose population failed the
 * pre-registered floor is not a result and must never be dispatched as if it
 * were. `opts.allowUnderpowered` overrides the throw and stamps
 * `noMeasurement: true`, which `formatReport` turns into a NO MEASUREMENT
 * banner with the decision line SUPPRESSED (the same mechanism
 * `buildBatchTrials` uses in the sibling instrument).
 *
 * @param {{ header: object|null, units: object[] }} corpus
 * @param {{ findPrompt: Function, planDimensions: object[], calibration?: string,
 *   replicates?: number, model?: string, runUnits?: number,
 *   allowUnderpowered?: boolean }} opts
 * @returns {{ trials: object[], prompts: Map<string,string>, power: object,
 *   noMeasurement: boolean, runUnits: object[] }}
 */
export function buildCollapseTrials(corpus, opts = {}) {
  const units = (corpus && corpus.units) || [];
  const replicates = opts.replicates === undefined ? POWER_FLOORS.minReplicates : opts.replicates;
  if (!Number.isInteger(replicates) || replicates < 1) {
    throw new Error(`replicates must be a positive integer, got ${JSON.stringify(opts.replicates)}`);
  }
  if (typeof opts.findPrompt !== 'function') {
    throw new Error(
      'buildCollapseTrials requires the REAL findPrompt from .claude/workflows/lib/review.mjs — ' +
        'this instrument never carries its own copy of the production prompt'
    );
  }
  const lenses = Array.isArray(opts.planDimensions) ? opts.planDimensions : [];
  const lensKeys = lenses.map((d) => d.key);
  for (const expected of ALWAYS_ON_PLAN_LENSES) {
    if (lensKeys.indexOf(expected) === -1) {
      throw new Error(
        `buildCollapseTrials: planDimensions is missing the always-on lens "${expected}" ` +
          `(got ${lensKeys.join(', ') || 'none'}) — arm A must be the REAL always-on set`
      );
    }
  }

  const runUnitCount = opts.runUnits === undefined ? POWER_FLOORS.minUnits : opts.runUnits;
  const runUnits = selectRunUnits(units, Math.min(runUnitCount, Math.max(units.length, 1)));
  const power = assessPower(runUnits, { replicates });

  if (power.power !== 'SUFFICIENT' && !opts.allowUnderpowered) {
    throw new Error(
      'POWER: UNDERPOWERED — ' +
        power.reasons.join('; ') +
        '. Refusing to dispatch: an A/B built from this population would measure dilution rather than evidence. ' +
        'Pass --allow-underpowered to build the plan anyway; the report is then stamped NO MEASUREMENT and prints no decision.'
    );
  }

  const trials = [];
  const prompts = new Map();
  for (const unit of runUnits) {
    for (let r = 1; r <= replicates; r++) {
      for (const dim of lenses) {
        const trialId = `${unit.id}|A|${dim.key}|r${r}`;
        trials.push({
          trialId,
          unitId: unit.id,
          targetType: unit.targetType,
          arm: 'A',
          lens: dim.key,
          replicate: r,
          model: opts.model || null,
        });
        prompts.set(trialId, opts.findPrompt('plan', dim, { target: unit.target }));
      }
      const trialId = `${unit.id}|B|collapsed|r${r}`;
      trials.push({
        trialId,
        unitId: unit.id,
        targetType: unit.targetType,
        arm: 'B',
        lens: null,
        replicate: r,
        model: opts.model || null,
      });
      prompts.set(
        trialId,
        buildCollapsedPlanPrompt(lenses, { target: unit.target, calibration: opts.calibration })
      );
    }
  }

  return {
    trials,
    prompts,
    power,
    noMeasurement: power.power !== 'SUFFICIENT',
    runUnits,
  };
}

// --- Dispatch ---------------------------------------------------------------

/** Promise wrapper over `spawn`, collecting stdout/stderr. Never rejects. */
function runProcess(cmd, argv, opts = {}) {
  return new Promise((resolve) => {
    let child;
    try {
      child = spawn(cmd, argv, { cwd: opts.cwd, stdio: ['pipe', 'pipe', 'pipe'] });
    } catch (err) {
      resolve({ status: null, stdout: '', stderr: '', error: err });
      return;
    }
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (d) => {
      stdout += d;
    });
    child.stderr.on('data', (d) => {
      stderr += d;
    });
    child.on('error', (err) => resolve({ status: null, stdout, stderr, error: err }));
    child.on('close', (status) => resolve({ status, stdout, stderr, error: null }));
    if (opts.input !== undefined) child.stdin.end(opts.input);
    else child.stdin.end();
  });
}

function tryParseJson(text) {
  if (typeof text !== 'string') return null;
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

/**
 * Pull the FIRST balanced JSON object out of a prose/fenced response body.
 * Deliberately tolerant: a finder that wraps its JSON in prose or a code fence
 * still parses; one that emits no `findings` array at all stays null and is
 * bucketed as an ERROR — never coerced to `[]`, which would read as "this arm
 * found the document clean" and silently manufacture a dilution result.
 */
export function extractFindingsPayload(text) {
  const direct = tryParseJson(text);
  if (direct && Array.isArray(direct.findings)) return direct.findings;
  if (typeof text !== 'string') return null;
  for (let i = 0; i < text.length; i++) {
    if (text[i] !== '{') continue;
    let depth = 0;
    let inStr = false;
    let esc = false;
    for (let j = i; j < text.length; j++) {
      const ch = text[j];
      if (inStr) {
        if (esc) esc = false;
        else if (ch === '\\') esc = true;
        else if (ch === '"') inStr = false;
        continue;
      }
      if (ch === '"') inStr = true;
      else if (ch === '{') depth += 1;
      else if (ch === '}') {
        depth -= 1;
        if (depth === 0) {
          const parsed = tryParseJson(text.slice(i, j + 1));
          if (parsed && Array.isArray(parsed.findings)) return parsed.findings;
          i = j;
          break;
        }
      }
    }
  }
  return null;
}

/**
 * Pull `{ findings, usage, toolCalls }` out of a `claude -p --output-format json`
 * body. Exported so the harness can drive the real parsing path without
 * spawning anything.
 */
export function parseClaudeFinderResult(body) {
  const usage = {};
  for (const c of TOKEN_CLASSES) usage[c] = 0;
  const u = (body && body.usage) || {};
  usage.output = Number(u.output_tokens || 0);
  usage.uncachedInput = Number(u.input_tokens || 0);
  usage.cacheWrite = Number(u.cache_creation_input_tokens || 0);
  usage.cacheRead = Number(u.cache_read_input_tokens || 0);

  let toolCalls = 0;
  let findings = null;
  const messages = Array.isArray(body && body.messages) ? body.messages : [];
  for (const m of messages) {
    const content = m && m.message && Array.isArray(m.message.content) ? m.message.content : [];
    for (const block of content) {
      if (block && block.type === 'tool_use') {
        toolCalls += 1;
        if (block.name === 'StructuredOutput' && block.input && Array.isArray(block.input.findings)) {
          findings = block.input.findings;
        }
      }
    }
  }
  if (findings === null) findings = extractFindingsPayload(body && body.result);
  if (typeof (body && body.num_tool_uses) === 'number' && toolCalls === 0) toolCalls = body.num_tool_uses;
  return {
    findings,
    usage,
    toolCalls,
    error: findings === null ? 'no `findings` array in the response' : null,
  };
}

/**
 * Dispatch one trial through `claude -p`, returning its findings and usage.
 *
 * ASYNC on purpose — a synchronous spawn would block Node's single thread and
 * serialize a run that is meant to be concurrent. Never throws for a per-trial
 * failure; a MISSING BINARY is a setup error and DOES throw, because that is not
 * a per-trial outcome and must not be laundered into N empty results.
 *
 * @param {object} trial
 * @param {string} prompt
 * @param {{ cwd?: string, spawnImpl?: Function }} [opts]
 */
export async function dispatchTrial(trial, prompt, opts = {}) {
  const run = opts.spawnImpl || runProcess;
  const model = trial.model || 'opus';
  const res = await run('claude', ['-p', '--model', model, '--output-format', 'json'], {
    cwd: opts.cwd,
    input: prompt,
  });
  if (res.error && res.error.code === 'ENOENT') {
    throw new Error(
      'the `claude` binary was not found on PATH. run-finder-collapse dispatches real agents through ' +
        '`claude -p`; install/authenticate the CLI, or use --dry-run / --dispatch-stub / --score.'
    );
  }
  if (res.error) return { findings: null, error: `claude failed to start: ${res.error.message}`, usage: {}, toolCalls: 0 };
  if (res.status !== 0) {
    return {
      findings: null,
      error: `claude exited ${res.status}: ${String(res.stderr || '').trim().slice(0, 400)}`,
      usage: {},
      toolCalls: 0,
    };
  }
  const body = tryParseJson(res.stdout);
  if (body === null) return { findings: null, error: 'claude returned a non-JSON body', usage: {}, toolCalls: 0 };
  return parseClaudeFinderResult(body);
}

/**
 * Execute the trial plan. Dependency-injected so the harness can drive the whole
 * path with no subprocess and no spend.
 *
 * Results are written into an INDEXED slot, never pushed, so output order is the
 * trial-plan order regardless of completion order — concurrency must not make a
 * run non-reproducible.
 */
export async function runCollapseTrials(trials, prompts, dispatch, opts = {}) {
  const log = opts.log || (() => {});
  const concurrency = Math.max(1, opts.concurrency || 1);
  const results = new Array(trials.length);
  let next = 0;

  async function worker() {
    for (;;) {
      const i = next++;
      if (i >= trials.length) return;
      const t = trials[i];
      // A THROWN dispatcher is fatal (a missing binary, a bad model) — that is a
      // setup error, not a per-trial failure, and must not be swallowed.
      const outcome = await dispatch(t, prompts.get(t.trialId));
      results[i] = {
        trialId: t.trialId,
        unitId: t.unitId,
        targetType: t.targetType,
        arm: t.arm,
        lens: t.lens,
        replicate: t.replicate,
        findings: outcome && Array.isArray(outcome.findings) ? outcome.findings : null,
        error: (outcome && outcome.error) || null,
        usage: (outcome && outcome.usage) || {},
        toolCalls: (outcome && outcome.toolCalls) || 0,
      };
      log(
        `trial ${t.trialId}: ${
          results[i].findings === null ? 'ERROR ' + results[i].error : results[i].findings.length + ' finding(s)'
        }`
      );
    }
  }

  await Promise.all(Array.from({ length: Math.min(concurrency, trials.length) }, worker));
  return results;
}

// --- Adjudication -----------------------------------------------------------

const ADJUDICATION_FIELDS = [
  'unitId',
  'arm',
  'replicate',
  'lens',
  'findingId',
  'material',
  'matchedInOtherArm',
  'matchedFindingId',
  'rationale',
  'adjudicatedAgainstCommit',
];

/**
 * Parse the hand adjudication JSONL. Every record is validated; a malformed one
 * is an error rather than a silently-`undefined` `material` flag (which would
 * read as "not material" and quietly move the decision).
 *
 * @param {string} text
 * @returns {{ rows: object[], errors: string[] }}
 */
export function loadAdjudication(text) {
  const rows = [];
  const errors = [];
  const seen = new Set();
  const lines = String(text).split('\n');
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (line === '') continue;
    let row;
    try {
      row = JSON.parse(line);
    } catch (err) {
      errors.push(`line ${i + 1}: not parseable JSON: ${err.message}`);
      continue;
    }
    if (!isPlainObject(row)) {
      errors.push(`line ${i + 1}: not an object`);
      continue;
    }
    for (const key of Object.keys(row)) {
      if (ADJUDICATION_FIELDS.indexOf(key) === -1) errors.push(`line ${i + 1}: unknown key "${key}"`);
    }
    for (const key of ADJUDICATION_FIELDS) {
      if (!(key in row)) errors.push(`line ${i + 1}: missing required key "${key}"`);
    }
    if (row.arm !== 'A' && row.arm !== 'B') errors.push(`line ${i + 1}: arm must be "A" or "B"`);
    if (typeof row.material !== 'boolean') errors.push(`line ${i + 1}: material must be a boolean`);
    if (typeof row.matchedInOtherArm !== 'boolean') errors.push(`line ${i + 1}: matchedInOtherArm must be a boolean`);
    if (ALWAYS_ON_PLAN_LENSES.indexOf(row.lens) === -1) {
      errors.push(`line ${i + 1}: lens must be one of ${ALWAYS_ON_PLAN_LENSES.join('|')}, got ${JSON.stringify(row.lens)}`);
    }
    if (!nonEmptyString(row.adjudicatedAgainstCommit)) {
      errors.push(`line ${i + 1}: adjudicatedAgainstCommit must be a non-empty string — an adjudication with no pinned tree is not reproducible`);
    }
    const key = `${row.unitId}|${row.arm}|${row.replicate}|${row.findingId}`;
    if (seen.has(key)) errors.push(`line ${i + 1}: duplicate adjudication for ${key}`);
    seen.add(key);
    rows.push(row);
  }
  return { rows, errors };
}

// --- Scoring ----------------------------------------------------------------

/**
 * PRE-REGISTERED decision rule, fixed before any paid dispatch (see
 * docs/finder-collapse.md § Decision rule). Every threshold is a constant here
 * so the doc's DECISION line can be COPIED from the instrument's output rather
 * than hand-reasoned.
 */
export const DECISION_RULE = {
  // (2) per lens: arm B may lose at most this many adjudicated material
  // findings, AND at most this share of arm A's.
  maxPerLensMaterialLoss: 1,
  maxPerLensMaterialLossShare: 0.15,
  // (3) extinction: a lens where arm A produced at least this many material
  // findings must not go to zero in arm B.
  extinctionArmAFloor: 2,
  // (4) arm B's material `blocking` count may fall short by at most this many.
  maxBlockingShortfall: 1,
  // (5) share of arm-B findings whose `concern` is a valid lens key.
  minAttributionValidity: 0.95,
  // (6) mean tokens per unit must fall by at least this share. NEVER a ship on
  // its own — the decision requires ALL six.
  minTokenReduction: 0.2,
  // The adjudicated replicate. Hand adjudication covers this replicate in full
  // for BOTH arms; the remaining replicates supply variance on the mechanical
  // measures. Stated in the doc's Method and Limitations.
  adjudicatedReplicate: 1,
};

function emptySeverityCounts() {
  const out = {};
  for (const s of SEVERITIES) out[s] = 0;
  out.other = 0;
  return out;
}

function countSeverity(counts, finding) {
  const sev = finding && finding.severity;
  if (SEVERITIES.indexOf(sev) === -1) counts.other += 1;
  else counts[sev] += 1;
}

function emptyUsage() {
  const out = {};
  for (const c of TOKEN_CLASSES) out[c] = 0;
  return out;
}

function addUsage(into, usage) {
  for (const c of TOKEN_CLASSES) into[c] += Number((usage && usage[c]) || 0);
}

function totalOf(usage) {
  let n = 0;
  for (const c of TOKEN_CLASSES) n += usage[c];
  return n;
}

function ratio(numerator, denominator) {
  return denominator === 0 ? null : numerator / denominator;
}

/**
 * Score a completed A/B run.
 *
 * Everything per-lens is reported PER LENS and never blended: a cross-lens mean
 * would hide exactly the extinction the phase is looking for.
 * `findBlendedLensKeys` gates that structurally.
 *
 * @param {{ trials: object[] }} run - a saved trials file
 * @param {object[]} adjudication - rows from `loadAdjudication`
 * @param {{ rule?: object, noMeasurement?: boolean, power?: object }} [opts]
 */
export function scoreCollapse(run, adjudication, opts = {}) {
  const rule = { ...DECISION_RULE, ...(opts.rule || {}) };
  const trials = (run && run.trials) || [];
  const adj = Array.isArray(adjudication) ? adjudication : [];

  const unitIds = [...new Set(trials.map((t) => t.unitId))].sort();
  const replicates = [...new Set(trials.map((t) => t.replicate))].sort((a, b) => a - b);

  // --- Per-lens finding counts and severity distribution (all replicates) ---
  const byLens = {};
  for (const lens of ALWAYS_ON_PLAN_LENSES) {
    byLens[lens] = {
      A: { findings: 0, observations: 0, severity: emptySeverityCounts(), perObservation: [] },
      B: { findings: 0, observations: 0, severity: emptySeverityCounts(), perObservation: [] },
    };
  }
  // Arm-B findings whose `concern` is not a valid lens key. They are counted for
  // criterion 5 and are NEVER remapped onto a plausible lens — that would
  // fabricate attribution and inflate whichever lens received them.
  const attribution = { total: 0, valid: 0, invalid: [], claimedUnitOfWork: 0 };
  const errors = { A: 0, B: 0 };

  for (const t of trials) {
    if (t.findings === null) {
      errors[t.arm] = (errors[t.arm] || 0) + 1;
      continue;
    }
    if (t.arm === 'A') {
      const slot = byLens[t.lens];
      if (!slot) continue;
      slot.A.observations += 1;
      slot.A.findings += t.findings.length;
      slot.A.perObservation.push(t.findings.length);
      for (const f of t.findings) countSeverity(slot.A.severity, f);
    } else {
      for (const lens of ALWAYS_ON_PLAN_LENSES) byLens[lens].B.observations += 1;
      const perLensThisTrial = {};
      for (const lens of ALWAYS_ON_PLAN_LENSES) perLensThisTrial[lens] = 0;
      for (const f of t.findings) {
        attribution.total += 1;
        const concern = f && f.concern;
        if (ALWAYS_ON_PLAN_LENSES.indexOf(concern) !== -1) {
          attribution.valid += 1;
          byLens[concern].B.findings += 1;
          perLensThisTrial[concern] += 1;
          countSeverity(byLens[concern].B.severity, f);
        } else {
          if (concern === TRIGGERED_PLAN_DIMENSION) attribution.claimedUnitOfWork += 1;
          attribution.invalid.push({ trialId: t.trialId, findingId: (f && f.id) || null, concern: concern ?? null });
        }
      }
      for (const lens of ALWAYS_ON_PLAN_LENSES) byLens[lens].B.perObservation.push(perLensThisTrial[lens]);
    }
  }

  const perLens = {};
  for (const lens of ALWAYS_ON_PLAN_LENSES) {
    const a = byLens[lens].A;
    const b = byLens[lens].B;
    perLens[lens] = {
      armA: {
        findings: a.findings,
        observations: a.observations,
        meanPerObservation: ratio(a.findings, a.observations),
        severity: a.severity,
        perObservation: a.perObservation,
      },
      armB: {
        findings: b.findings,
        observations: b.observations,
        meanPerObservation: ratio(b.findings, b.observations),
        severity: b.severity,
        perObservation: b.perObservation,
      },
    };
  }

  // --- Adjudicated material findings (the adjudicated replicate only) -------
  const adjRows = adj.filter((r) => r.replicate === rule.adjudicatedReplicate);
  const severityOf = new Map();
  for (const t of trials) {
    if (t.findings === null) continue;
    for (const f of t.findings) severityOf.set(`${t.unitId}|${t.arm}|${t.replicate}|${(f && f.id) || ''}`, f);
  }

  // ADJUDICATION COVERAGE. Criteria 2-4 are computed from hand-adjudicated
  // material findings, so an EMPTY or PARTIAL adjudication makes them pass
  // VACUOUSLY (0 material in both arms is "no loss"). That would let a run with
  // no adjudication at all emit `ship-collapsed`, which is exactly the
  // false-pass this instrument exists to prevent. Coverage is therefore
  // computed first and gates those three criteria outright.
  const adjudicatedIds = new Set(adjRows.map((r) => `${r.unitId}|${r.arm}|${r.replicate}|${r.findingId}`));
  const coverage = { expected: 0, adjudicated: 0, missing: [] };
  for (const t of trials) {
    if (t.replicate !== rule.adjudicatedReplicate || t.findings === null) continue;
    for (const f of t.findings) {
      coverage.expected += 1;
      const key = `${t.unitId}|${t.arm}|${t.replicate}|${(f && f.id) || ''}`;
      if (adjudicatedIds.has(key)) coverage.adjudicated += 1;
      else if (coverage.missing.length < 20) coverage.missing.push(key);
    }
  }
  coverage.complete = coverage.expected > 0 && coverage.adjudicated === coverage.expected;

  const material = {};
  for (const lens of ALWAYS_ON_PLAN_LENSES) {
    material[lens] = { A: { count: 0, blocking: 0, matched: 0 }, B: { count: 0, blocking: 0, matched: 0 } };
  }
  for (const r of adjRows) {
    if (!material[r.lens] || (r.arm !== 'A' && r.arm !== 'B')) continue;
    if (!r.material) continue;
    const slot = material[r.lens][r.arm];
    slot.count += 1;
    if (r.matchedInOtherArm) slot.matched += 1;
    const f = severityOf.get(`${r.unitId}|${r.arm}|${r.replicate}|${r.findingId}`);
    if (f && f.severity === 'blocking') slot.blocking += 1;
  }

  // --- Tokens, per class, per arm ------------------------------------------
  const tokens = { A: emptyUsage(), B: emptyUsage() };
  const dispatches = { A: 0, B: 0 };
  for (const t of trials) {
    if (!tokens[t.arm]) continue;
    dispatches[t.arm] += 1;
    addUsage(tokens[t.arm], t.usage);
  }
  // A UNIT-OBSERVATION is one (unit, replicate) pair — the thing a production
  // run actually pays for. Reporting per-dispatch would flatter arm B by
  // construction (it makes one dispatch where arm A makes three), so both arms
  // are normalized to the same denominator.
  const unitObservations = unitIds.length * replicates.length;
  const perUnit = {
    A: ratio(totalOf(tokens.A), unitObservations),
    B: ratio(totalOf(tokens.B), unitObservations),
  };
  const meanInputPerDispatch = {
    A: ratio(tokens.A.uncachedInput + tokens.A.cacheWrite + tokens.A.cacheRead, dispatches.A),
    B: ratio(tokens.B.uncachedInput + tokens.B.cacheWrite + tokens.B.cacheRead, dispatches.B),
  };
  const tokenDelta = perUnit.A === null || perUnit.B === null || perUnit.A === 0 ? null : (perUnit.A - perUnit.B) / perUnit.A;

  // --- The six criteria ----------------------------------------------------
  const criteria = [];
  const powerSufficient = !opts.noMeasurement && (!opts.power || opts.power.power === 'SUFFICIENT');
  criteria.push({
    id: 1,
    name: 'power',
    detail: 'the run population clears the pre-registered floors',
    pass: powerSufficient,
    observed: opts.power ? opts.power.power : powerSufficient ? 'SUFFICIENT' : 'UNDERPOWERED',
  });

  const perLensLoss = {};
  let c2 = true;
  for (const lens of ALWAYS_ON_PLAN_LENSES) {
    const a = material[lens].A.count;
    const b = material[lens].B.count;
    const loss = a - b;
    const share = a === 0 ? 0 : loss / a;
    const ok = coverage.complete && loss <= rule.maxPerLensMaterialLoss && share <= rule.maxPerLensMaterialLossShare;
    perLensLoss[lens] = { armA: a, armB: b, loss, lossShare: share, pass: ok };
    if (!ok) c2 = false;
  }
  criteria.push({
    id: 2,
    name: coverage.complete ? 'per-lens material recall' : 'per-lens material recall (NOT ADJUDICATED)',
    detail: `every lens loses at most ${rule.maxPerLensMaterialLoss} adjudicated material finding and at most ${Math.round(
      rule.maxPerLensMaterialLossShare * 100
    )} percentage points`,
    pass: c2,
    observed: perLensLoss,
  });

  const extinction = {};
  let c3 = true;
  for (const lens of ALWAYS_ON_PLAN_LENSES) {
    const a = material[lens].A.count;
    const b = material[lens].B.count;
    const extinct = a >= rule.extinctionArmAFloor && b === 0;
    extinction[lens] = { armA: a, armB: b, extinct };
    if (extinct || !coverage.complete) c3 = false;
  }
  criteria.push({
    id: 3,
    name: 'no lens extinction',
    detail: `no lens goes to zero material findings in arm B where arm A produced at least ${rule.extinctionArmAFloor}`,
    pass: c3,
    observed: extinction,
  });

  let blockingA = 0;
  let blockingB = 0;
  for (const lens of ALWAYS_ON_PLAN_LENSES) {
    blockingA += material[lens].A.blocking;
    blockingB += material[lens].B.blocking;
  }
  const c4 = coverage.complete && blockingB >= blockingA - rule.maxBlockingShortfall;
  criteria.push({
    id: 4,
    name: 'no severity downgrade',
    detail: `arm B's adjudicated material blocking count is within ${rule.maxBlockingShortfall} of arm A's`,
    pass: c4,
    observed: { armA: blockingA, armB: blockingB, shortfall: blockingA - blockingB },
  });

  const attributionValidity = ratio(attribution.valid, attribution.total);
  const c5 = attribution.total === 0 ? false : attributionValidity >= rule.minAttributionValidity;
  criteria.push({
    id: 5,
    name: 'concern attribution validity',
    detail: `at least ${Math.round(rule.minAttributionValidity * 100)} % of arm-B findings carry a valid lens key`,
    pass: c5,
    observed: {
      total: attribution.total,
      valid: attribution.valid,
      validity: attributionValidity,
      claimedUnitOfWork: attribution.claimedUnitOfWork,
      invalid: attribution.invalid,
    },
  });

  const c6 = tokenDelta !== null && tokenDelta >= rule.minTokenReduction;
  criteria.push({
    id: 6,
    name: 'tokens materially lower',
    detail: `mean tokens per unit-observation fall by at least ${Math.round(
      rule.minTokenReduction * 100
    )} % — NEVER a ship on its own`,
    pass: c6,
    observed: { perUnitArmA: perUnit.A, perUnitArmB: perUnit.B, reduction: tokenDelta },
  });

  const allPass = criteria.every((c) => c.pass);
  const decision = opts.noMeasurement ? 'no-measurement' : allPass ? 'ship-collapsed' : 'no-ship';

  return {
    instrument: 'scripts/lib/finder-collapse.mjs',
    decision,
    noMeasurement: !!opts.noMeasurement,
    unitCount: unitIds.length,
    replicates: replicates.length,
    adjudicatedReplicate: rule.adjudicatedReplicate,
    adjudicatedRows: adjRows.length,
    adjudicationCoverage: coverage,
    dispatchErrors: errors,
    perLens,
    material,
    attribution: {
      total: attribution.total,
      valid: attribution.valid,
      validity: attributionValidity,
      claimedUnitOfWork: attribution.claimedUnitOfWork,
      invalid: attribution.invalid,
    },
    tokens: {
      byArm: tokens,
      dispatches,
      unitObservations,
      meanPerUnitObservation: perUnit,
      meanInputPerDispatch,
      reduction: tokenDelta,
    },
    criteria,
    rule,
  };
}

/**
 * Structural gate: no reported figure may BLEND the three lenses. A single
 * cross-lens recall number is exactly what would hide an extinct lens behind two
 * healthy ones, which is the failure mode this phase exists to detect.
 *
 * Returns the JSON paths of offending keys — a rate-shaped key name that does
 * NOT sit under a per-lens segment.
 *
 * @param {unknown} value
 * @param {string} [pathPrefix]
 * @returns {string[]}
 */
export function findBlendedLensKeys(value, pathPrefix = '$') {
  const RATE_KEY = /(recall|accuracy|materialShare|lossShare)/i;
  // A THRESHOLD is not an observation. `decisionRule` / `rule` hold the
  // pre-registered constants, and criterion 2 applies its threshold PER LENS by
  // construction, so a rate-shaped key there is not a blended measurement.
  // Everything else is measured output and must be lens-scoped.
  const THRESHOLD_PARENTS = ['decisionRule', 'rule'];
  const offenders = [];
  const walk = (node, p, exempt) => {
    if (Array.isArray(node)) {
      node.forEach((v, i) => walk(v, `${p}[${i}]`, exempt));
      return;
    }
    if (!isPlainObject(node)) return;
    for (const [k, v] of Object.entries(node)) {
      const childExempt = exempt || ALWAYS_ON_PLAN_LENSES.indexOf(k) !== -1 || THRESHOLD_PARENTS.indexOf(k) !== -1;
      if (RATE_KEY.test(k) && !childExempt) offenders.push(`${p}.${k}`);
      walk(v, `${p}.${k}`, childExempt);
    }
  };
  walk(value, pathPrefix, false);
  return offenders;
}

// --- Report -----------------------------------------------------------------

function pct(x) {
  return x === null || x === undefined ? 'n/a' : `${(x * 100).toFixed(1)} %`;
}

function num(x, digits = 2) {
  return x === null || x === undefined ? 'n/a' : Number(x).toFixed(digits);
}

/**
 * Render a scored run. On a `no-measurement` report the DECISION line is
 * SUPPRESSED and replaced by a NO MEASUREMENT banner — an underpowered run must
 * never read as a pass.
 *
 * @param {object} report
 * @param {'text'|'json'} [format]
 * @returns {string}
 */
export function formatReport(report, format = 'text') {
  if (format === 'json') return JSON.stringify(report, null, 2);
  const out = [];
  out.push('Collapsed plan finder A/B — three always-on lenses in one agent vs three agents');
  out.push('');
  out.push(
    `Units ${report.unitCount}  replicates ${report.replicates}  adjudicated replicate ${report.adjudicatedReplicate} (${report.adjudicatedRows} row(s))`
  );
  out.push(`Dispatch errors: arm A ${report.dispatchErrors.A || 0}, arm B ${report.dispatchErrors.B || 0}`);
  out.push('');
  out.push('Per-lens findings (mean per unit-observation; NEVER blended across lenses)');
  out.push('  lens                  armA    armB   |  armA sev (b/c/s)   armB sev (b/c/s)');
  for (const lens of ALWAYS_ON_PLAN_LENSES) {
    const l = report.perLens[lens];
    const sa = l.armA.severity;
    const sb = l.armB.severity;
    out.push(
      `  ${lens.padEnd(20)} ${num(l.armA.meanPerObservation).padStart(5)}   ${num(l.armB.meanPerObservation).padStart(5)}   |  ` +
        `${sa.blocking}/${sa.concern}/${sa.suggestion}`.padEnd(18) +
        `${sb.blocking}/${sb.concern}/${sb.suggestion}`
    );
  }
  out.push('');
  const cov = report.adjudicationCoverage || { expected: 0, adjudicated: 0, complete: false };
  out.push(
    'Adjudicated MATERIAL findings (replicate ' +
      report.adjudicatedReplicate +
      '; coverage ' +
      cov.adjudicated +
      '/' +
      cov.expected +
      (cov.complete ? '' : ' — INCOMPLETE, criteria 2-4 cannot pass') +
      ')'
  );
  out.push('  lens                  armA    armB    loss   loss%');
  for (const lens of ALWAYS_ON_PLAN_LENSES) {
    const m = report.material[lens];
    const loss = m.A.count - m.B.count;
    out.push(
      `  ${lens.padEnd(20)} ${String(m.A.count).padStart(5)}   ${String(m.B.count).padStart(5)}   ${String(loss).padStart(5)}   ` +
        pct(m.A.count === 0 ? 0 : loss / m.A.count)
    );
  }
  out.push('');
  out.push(
    `Attribution: ${report.attribution.valid}/${report.attribution.total} arm-B findings carry a valid lens key (${pct(
      report.attribution.validity
    )}); ${report.attribution.claimedUnitOfWork} claimed unit-of-work`
  );
  out.push(
    `Tokens per unit-observation: arm A ${num(report.tokens.meanPerUnitObservation.A, 0)}, arm B ${num(
      report.tokens.meanPerUnitObservation.B,
      0
    )} (${pct(report.tokens.reduction)} lower)`
  );
  out.push(
    `  by class — arm A ` +
      TOKEN_CLASSES.map((c) => `${c}=${report.tokens.byArm.A[c]}`).join(' ') +
      `\n  by class — arm B ` +
      TOKEN_CLASSES.map((c) => `${c}=${report.tokens.byArm.B[c]}`).join(' ')
  );
  out.push('');
  out.push('Decision criteria (ALL six required; criterion 6 alone is NEVER a ship)');
  for (const c of report.criteria) {
    out.push(`  ${c.pass ? 'PASS' : 'FAIL'}  (${c.id}) ${c.name} — ${c.detail}`);
  }
  out.push('');
  if (report.noMeasurement) {
    out.push('NO MEASUREMENT — the run population was underpowered.');
    out.push('No decision line is printed: an underpowered A/B is not a result, and must never read as a pass.');
  } else {
    out.push(`DECISION: ${report.decision}`);
  }
  return out.join('\n');
}

// --- Audit ------------------------------------------------------------------

/**
 * CORPUS-FREE arithmetic audit of `docs/token-baseline.json` §
 * `planFinderCollapse`. Proves the committed figures are internally consistent
 * with no corpus, no trials file and no adjudication present — so the committed
 * numbers can be gated in CI without any of the measurement inputs.
 *
 * @param {object} section
 * @returns {{ ok: boolean, errors: string[], checks: string[] }}
 */
export function auditCollapseDoc(section) {
  const errors = [];
  const checks = [];
  if (!isPlainObject(section)) return { ok: false, errors: ['planFinderCollapse section is missing or not an object'], checks };

  if (!isPlainObject(section.window) || !nonEmptyString(section.window.until)) {
    errors.push('window.until must pin the selection window');
  } else checks.push(`window pinned at ${section.window.until}`);

  const DECISIONS = ['ship-collapsed', 'no-ship', 'no-measurement'];
  if (DECISIONS.indexOf(section.decision) === -1) {
    errors.push(`decision must be one of ${DECISIONS.join('|')}, got ${JSON.stringify(section.decision)}`);
  } else checks.push(`decision = ${section.decision}`);

  if (!isPlainObject(section.perLens)) {
    errors.push('perLens must be an object keyed by lens');
  } else {
    for (const lens of ALWAYS_ON_PLAN_LENSES) {
      const l = section.perLens[lens];
      if (!isPlainObject(l)) {
        errors.push(`perLens.${lens} is missing`);
        continue;
      }
      for (const arm of ['armA', 'armB']) {
        const a = l[arm];
        if (!isPlainObject(a)) {
          errors.push(`perLens.${lens}.${arm} is missing`);
          continue;
        }
        const sev = a.severity;
        if (!isPlainObject(sev)) {
          errors.push(`perLens.${lens}.${arm}.severity is missing`);
          continue;
        }
        const sum = SEVERITIES.reduce((n, s) => n + Number(sev[s] || 0), 0) + Number(sev.other || 0);
        if (sum !== Number(a.findings)) {
          errors.push(
            `perLens.${lens}.${arm}: severity counts sum to ${sum} but findings is ${a.findings}`
          );
        } else checks.push(`perLens.${lens}.${arm}: severity sums to findings (${sum})`);
        if (a.observations > 0) {
          const mean = a.findings / a.observations;
          if (Math.abs(mean - Number(a.meanPerObservation)) > 0.005) {
            errors.push(
              `perLens.${lens}.${arm}: meanPerObservation ${a.meanPerObservation} != ${a.findings}/${a.observations}`
            );
          }
        }
      }
    }
  }

  if (!isPlainObject(section.attribution)) {
    errors.push('attribution must be an object');
  } else {
    const { total, valid, validity } = section.attribution;
    if (total > 0 && Math.abs(valid / total - Number(validity)) > 0.005) {
      errors.push(`attribution.validity ${validity} != ${valid}/${total}`);
    } else checks.push(`attribution.validity matches ${valid}/${total}`);
    if (Number(valid) > Number(total)) errors.push('attribution.valid exceeds attribution.total');
  }

  if (!isPlainObject(section.tokens)) {
    errors.push('tokens must be an object');
  } else {
    for (const arm of ['A', 'B']) {
      const usage = section.tokens.byArm && section.tokens.byArm[arm];
      if (!isPlainObject(usage)) {
        errors.push(`tokens.byArm.${arm} is missing`);
        continue;
      }
      for (const c of TOKEN_CLASSES) {
        if (typeof usage[c] !== 'number') errors.push(`tokens.byArm.${arm}.${c} must be a number`);
      }
    }
    const uo = Number(section.tokens.unitObservations || 0);
    if (uo > 0 && isPlainObject(section.tokens.byArm)) {
      for (const arm of ['A', 'B']) {
        const usage = section.tokens.byArm[arm];
        if (!isPlainObject(usage)) continue;
        const total = TOKEN_CLASSES.reduce((n, c) => n + Number(usage[c] || 0), 0);
        const mean = total / uo;
        const claimed = section.tokens.meanPerUnitObservation && section.tokens.meanPerUnitObservation[arm];
        if (Math.abs(mean - Number(claimed)) > 1) {
          errors.push(`tokens.meanPerUnitObservation.${arm} ${claimed} != ${total}/${uo}`);
        } else checks.push(`tokens.meanPerUnitObservation.${arm} matches its class sum`);
      }
    }
  }

  if (!Array.isArray(section.criteria) || section.criteria.length !== 6) {
    errors.push('criteria must be an array of exactly the six pre-registered criteria');
  } else {
    const allPass = section.criteria.every((c) => c && c.pass === true);
    if (section.decision === 'ship-collapsed' && !allPass) {
      errors.push('decision is ship-collapsed but not every criterion passed');
    }
    if (section.decision === 'no-ship' && allPass) {
      errors.push('decision is no-ship but every criterion passed');
    }
    const tokenOnly = section.criteria.filter((c) => c && c.pass).map((c) => c.id);
    if (section.decision === 'ship-collapsed' && tokenOnly.length === 1 && tokenOnly[0] === 6) {
      errors.push('a ship decision resting on criterion 6 alone is forbidden by the pre-registered rule');
    }
    checks.push(`criteria table holds six entries; decision agrees with it`);
  }

  const cov = section.adjudicationCoverage;
  if (!isPlainObject(cov)) {
    errors.push('adjudicationCoverage must be recorded — criteria 2-4 pass vacuously without it');
  } else {
    if (cov.adjudicated > cov.expected) errors.push('adjudicationCoverage.adjudicated exceeds expected');
    const complete = cov.expected > 0 && cov.adjudicated === cov.expected;
    if (complete !== cov.complete) {
      errors.push(`adjudicationCoverage.complete (${cov.complete}) disagrees with ${cov.adjudicated}/${cov.expected}`);
    }
    if (section.decision === 'ship-collapsed' && !cov.complete) {
      errors.push('a ship decision requires a COMPLETE adjudication — criteria 2-4 are vacuous otherwise');
    }
    checks.push(`adjudication coverage ${cov.adjudicated}/${cov.expected}`);
  }

  const blended = findBlendedLensKeys(section);
  if (blended.length) errors.push(`blended cross-lens rate key(s): ${blended.join(', ')}`);
  else checks.push('no blended cross-lens rate key');

  return { ok: errors.length === 0, errors, checks };
}
