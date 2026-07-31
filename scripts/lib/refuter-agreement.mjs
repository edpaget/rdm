// refuter-agreement.mjs — the canonical corpus/scoring module behind the
// refuter-agreement harness.
//
// WHY THIS EXISTS
// ------------------------------------------------------------------------
// Refuters run on the most expensive tier everywhere: `review-verify` resolves
// to opus at the default and `large` tiers, and `plan-review.js` passes no
// models at all (so its finders and refuters inherit the ambient, opus-class
// session model). An initial 8-finding A/B suggested a cheaper tier might be
// viable but was far too small to move a gate on — and its ONE disagreement was
// the expensive failure mode: a mechanically-true-but-not-a-defect finding that
// the cheaper tier kept and the expensive tier refuted.
//
// This module is the reusable instrument that replaces that anecdote: a
// validated corpus schema, deterministic trial construction, and a scorer that
// reports FALSE NEGATIVES and FALSE POSITIVES over structurally different
// denominators — never blended — alongside per-tier token cost, so the
// already-measured "cheaper model, same tokens" effect stays visible.
//
// WHAT IT IS NOT
// ------------------------------------------------------------------------
// It is NOT wired into the lane's hot path. Nothing under `.claude/workflows/`
// imports it; it imports FROM `.claude/workflows/lib/review.mjs` (via the
// runner's dependency injection) and never the other way round. It is invoked
// by hand or by `scripts/verify-refuter-agreement.sh`.
//
// It also does NOT measure price. Every token figure here is VOLUME. A
// re-tiering argument is a price-per-token argument and must be labelled as
// such — this roadmap's headline metric is tokens, and the initial A/B already
// showed the cheaper tier spending MORE of them (52.9k vs 48.4k).
//
// Determinism: no clock, no RNG, no network. Same corpus and trials in, same
// report out. (The gate greps this file for either forbidden global, so this
// note names neither literally.)

import crypto from 'node:crypto';

/** Bumped whenever the corpus record shape changes incompatibly. */
export const CORPUS_SCHEMA_VERSION = 1;

/**
 * The closed set of ground-truth classes. `validateCorpusItem` rejects anything
 * outside it, so a typo cannot silently create a new bucket that then vanishes
 * from every per-class breakdown.
 *
 * - `real-defect` — the finding is correct and the code/plan is wrong.
 * - `mechanically-true-not-a-defect` — the stated fact holds, but a documented
 *   exception, a deliberate design, or a governing artifact means it is not a
 *   defect. THE DIVERGENCE CLASS: this is where the tiers disagreed.
 * - `false-premise` — cites a file, symbol, or behavior that does not exist.
 * - `stale-fact` — was true once, superseded by a later commit.
 * - `misread-scope` — true somewhere, but not at the cited location.
 * - `style-preference` — a taste call dressed as a defect.
 */
export const GROUND_TRUTH_CLASSES = [
  'real-defect',
  'mechanically-true-not-a-defect',
  'false-premise',
  'stale-fact',
  'misread-scope',
  'style-preference',
];

/** The divergence class the corpus is deliberately weighted toward. */
export const DIVERGENCE_CLASS = 'mechanically-true-not-a-defect';

/** Legal `groundTruth.authority` values. */
export const AUTHORITIES = ['authoritative', 'judgement-call'];

/** Legal `mode` values — the two `buildReviewPipeline` modes. */
export const MODES = ['code', 'plan'];

/** Legal `provenance.kind` values. */
export const PROVENANCE_KINDS = ['mined', 'constructed'];

/**
 * Severities that still spawn a refuter. Phase 6 landed
 * `NON_GATING_SEVERITIES = ['suggestion']` in `.claude/workflows/lib/review.mjs`,
 * so a `suggestion` finding is never refuted again — historical `suggestion`
 * items are mineable but tiering-IRRELEVANT and must not reach a headline rate.
 */
export const GATING_SEVERITIES = ['blocking', 'concern'];

/** Severities recorded as historical-only; excluded from the headline rates. */
export const HISTORICAL_ONLY_SEVERITIES = ['suggestion'];

/** Corpus floors, asserted by `scripts/verify-refuter-agreement.sh`. */
export const MIN_CORPUS_SIZE = 45;
/** The divergence class is deliberately over-represented, far above incidence. */
export const MIN_DIVERGENCE_CLASS_SHARE = 0.35;
/** Mined items must stay the primary source; constructed items only top up. */
export const MIN_MINED_SHARE = 0.6;
/** At least half the corpus must be settled by a citable artifact. */
export const MIN_AUTHORITATIVE_SHARE = 0.5;

/** The four token classes `scripts/lib/token-report.mjs` uses, in report order. */
export const TOKEN_CLASSES = ['output', 'uncachedInput', 'cacheWrite', 'cacheRead'];

/**
 * Fields every `finding` must carry. Deliberately narrower than the full
 * `FINDING` contract: real historical findings occasionally omit `why` or
 * `recommendation`, and rejecting those would quietly bias the corpus toward
 * tidy findings. The six required here are the ones a refuter cannot grade
 * without.
 */
const FINDING_FIELDS = ['id', 'concern', 'location', 'severity', 'confidence', 'what_fails'];

const TOP_LEVEL_FIELDS = [
  'id',
  'schemaVersion',
  'mode',
  'dim',
  'target',
  'finding',
  'promptSha256',
  'promptDrift',
  'provenance',
  'groundTruth',
];

/**
 * An `authoritative` item's evidence must point at something a reader can go
 * and check: a source path, a git sha, or a document section/AC reference.
 * A bare assertion ("clearly not a bug") is a judgement call, not authority.
 */
export const AUTHORITATIVE_EVIDENCE_RE =
  /(\.rs\b|\.mjs\b|\.js\b|\.sh\b|\.md\b|\.json\b|\.toml\b|\b[0-9a-f]{7,40}\b|§|\bAC-|\bAC\d)/;

/** sha256 hex of a UTF-8 string. */
export function sha256(text) {
  return crypto.createHash('sha256').update(String(text), 'utf8').digest('hex');
}

// --- Corpus schema --------------------------------------------------------

function isPlainObject(v) {
  return v !== null && typeof v === 'object' && !Array.isArray(v);
}

function nonEmptyString(v) {
  return typeof v === 'string' && v.trim() !== '';
}

/**
 * Validate one corpus record against the schema.
 *
 * Every field is required and unknown top-level keys are REJECTED, so a
 * malformed hand-edit (a typo'd key that would then read as `undefined` in the
 * scorer) cannot pass silently.
 *
 * @param {unknown} item
 * @returns {{ ok: boolean, errors: string[] }}
 */
export function validateCorpusItem(item) {
  const errors = [];
  if (!isPlainObject(item)) return { ok: false, errors: ['item is not an object'] };

  for (const key of Object.keys(item)) {
    if (TOP_LEVEL_FIELDS.indexOf(key) === -1) errors.push(`unknown top-level key "${key}"`);
  }
  for (const key of TOP_LEVEL_FIELDS) {
    if (!(key in item)) errors.push(`missing required key "${key}"`);
  }

  if (!nonEmptyString(item.id)) errors.push('id must be a non-empty string');
  if (item.schemaVersion !== CORPUS_SCHEMA_VERSION) {
    errors.push(`schemaVersion must be ${CORPUS_SCHEMA_VERSION}, got ${JSON.stringify(item.schemaVersion)}`);
  }
  if (MODES.indexOf(item.mode) === -1) errors.push(`mode must be one of ${MODES.join('|')}, got ${JSON.stringify(item.mode)}`);
  if (!isPlainObject(item.dim) || !nonEmptyString(item.dim.key)) errors.push('dim must be an object with a non-empty key');
  if (!nonEmptyString(item.target)) errors.push('target must be a non-empty string');
  if (!nonEmptyString(item.promptSha256) || !/^[0-9a-f]{64}$/.test(item.promptSha256)) {
    errors.push('promptSha256 must be a 64-char lowercase hex digest');
  }
  if (typeof item.promptDrift !== 'boolean') errors.push('promptDrift must be a boolean');

  // finding
  if (!isPlainObject(item.finding)) {
    errors.push('finding must be an object');
  } else {
    for (const f of FINDING_FIELDS) {
      if (!(f in item.finding)) errors.push(`finding.${f} is required`);
    }
    if (!nonEmptyString(item.finding.severity)) errors.push('finding.severity must be a non-empty string');
    if (typeof item.finding.confidence !== 'number') errors.push('finding.confidence must be a number');
  }

  // provenance
  if (!isPlainObject(item.provenance)) {
    errors.push('provenance must be an object');
  } else {
    const kind = item.provenance.kind;
    if (PROVENANCE_KINDS.indexOf(kind) === -1) {
      errors.push(`provenance.kind must be one of ${PROVENANCE_KINDS.join('|')}, got ${JSON.stringify(kind)}`);
    } else if (kind === 'mined') {
      for (const f of ['projectSlug', 'sessionId', 'runId', 'agentId', 'workflow']) {
        if (!nonEmptyString(item.provenance[f])) errors.push(`mined provenance.${f} must be a non-empty string`);
      }
      const hv = item.provenance.historicalVerdict;
      if (!isPlainObject(hv) || typeof hv.refuted !== 'boolean') {
        errors.push('mined provenance.historicalVerdict must be an object with a boolean refuted');
      }
    } else {
      if (!nonEmptyString(item.provenance.builtAgainstCommit)) {
        errors.push('constructed provenance.builtAgainstCommit must be a non-empty string');
      }
      if (!nonEmptyString(item.provenance.rationale)) {
        errors.push('constructed provenance.rationale must be a non-empty string');
      }
    }
  }

  // groundTruth — never null in the CORPUS (the miner emits null; adjudication fills it in).
  if (!isPlainObject(item.groundTruth)) {
    errors.push('groundTruth must be an object (the miner emits null; adjudicate before checking in)');
  } else {
    if (typeof item.groundTruth.defect !== 'boolean') errors.push('groundTruth.defect must be a boolean');
    if (GROUND_TRUTH_CLASSES.indexOf(item.groundTruth.class) === -1) {
      errors.push(`groundTruth.class must be one of ${GROUND_TRUTH_CLASSES.join('|')}, got ${JSON.stringify(item.groundTruth.class)}`);
    }
    if (AUTHORITIES.indexOf(item.groundTruth.authority) === -1) {
      errors.push(`groundTruth.authority must be one of ${AUTHORITIES.join('|')}, got ${JSON.stringify(item.groundTruth.authority)}`);
    }
    if (!nonEmptyString(item.groundTruth.evidence)) {
      errors.push('groundTruth.evidence must be a non-empty string');
    } else if (item.groundTruth.authority === 'authoritative' && !AUTHORITATIVE_EVIDENCE_RE.test(item.groundTruth.evidence)) {
      errors.push('an authoritative item\'s evidence must cite a concrete artifact (a source path, a 7+ hex sha, or a §/AC- reference)');
    }
    // Ground truth is adjudicated against A PINNED TREE, and that tree is the
    // one a replay run actually reads — not the tree the historical refuter
    // read. A finding that was a real defect then and is fixed now is
    // `stale-fact`/`defect: false` here, because that is the honest answer for
    // a refuter reading THIS tree. Recording the commit is what makes the
    // judgement re-checkable.
    if (!/^[0-9a-f]{7,40}$/.test(String(item.groundTruth.adjudicatedAgainstCommit || ''))) {
      errors.push('groundTruth.adjudicatedAgainstCommit must be a 7-40 char hex commit sha');
    }
    // A `real-defect` class and `defect: false` are contradictory; so is any
    // non-`real-defect` class with `defect: true`. Catch the mismatch here
    // rather than letting it silently skew a rate.
    const shouldBeDefect = item.groundTruth.class === 'real-defect';
    if (typeof item.groundTruth.defect === 'boolean' && item.groundTruth.defect !== shouldBeDefect) {
      errors.push(
        `groundTruth.defect (${item.groundTruth.defect}) contradicts class "${item.groundTruth.class}" ` +
          `(only "real-defect" carries defect: true)`
      );
    }
  }

  return { ok: errors.length === 0, errors };
}

/**
 * Parse a JSONL corpus. Blank lines are skipped; a duplicate `id` is rejected
 * (two items sharing an id would double-count in every per-item statistic).
 *
 * @param {string} text
 * @returns {{ items: object[], errors: string[] }}
 */
export function loadCorpus(text) {
  const items = [];
  const errors = [];
  const seen = new Set();
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
    const { ok, errors: itemErrors } = validateCorpusItem(parsed);
    if (!ok) {
      for (const e of itemErrors) errors.push(`line ${i + 1} (${parsed && parsed.id}): ${e}`);
      continue;
    }
    if (seen.has(parsed.id)) {
      errors.push(`line ${i + 1}: duplicate id "${parsed.id}"`);
      continue;
    }
    seen.add(parsed.id);
    items.push(parsed);
  }
  return { items, errors };
}

// --- Prompt reconstruction ------------------------------------------------

/**
 * Recover the `refutePrompt` INPUTS from a historical prompt string.
 *
 * `refutePrompt(mode, dim, finding, context)` builds exactly:
 *
 *   You are a READ-ONLY refuter. Do not edit any files.
 *   A prior reviewer raised this <dim.key> finding against <target>:
 *   <JSON.stringify(finding, null, 2)>
 *   Start from the stance: this is NOT a real issue unless the <code|plan> proves otherwise. ...
 *
 * `<target>` is interpolated INLINE on the header line and IS ROUTINELY
 * MULTI-LINE — the `--implementation-plan` plan-review target is a whole
 * pretty-printed plan document. Recovering the target by taking "the rest of
 * the first line" therefore silently TRUNCATES it, and a truncated target
 * cannot regenerate byte-identically. The target instead ends at the `:` that
 * immediately precedes the newline before the finding's own line-anchored
 * opening brace — which is exactly the brace `extractFinding` already locates,
 * so both instruments are anchored to the same structure.
 *
 * @param {string} prompt
 * @param {{ extractFinding: (prompt: string) => object|null,
 *   matchBrace: (text: string, start: number) => number }} deps
 * @returns {{ mode: string|null, dimKey: string|null, target: string|null, finding: object|null }}
 */
export function reconstructRefuteInputs(prompt, deps = {}) {
  const { extractFinding, matchBrace } = deps;
  const out = { mode: null, dimKey: null, target: null, finding: null };
  if (typeof prompt !== 'string') return out;

  const HEADER = 'A prior reviewer raised this ';
  const AGAINST = ' finding against ';
  const headerAt = prompt.indexOf(HEADER);
  const findingAt = typeof matchBrace === 'function' ? findFindingStart(prompt, matchBrace) : -1;
  if (headerAt !== -1) {
    const againstAt = prompt.indexOf(AGAINST, headerAt);
    if (againstAt !== -1) {
      out.dimKey = prompt.slice(headerAt + HEADER.length, againstAt);
      const targetFrom = againstAt + AGAINST.length;
      if (findingAt > targetFrom) {
        // Drop the ':' and the '\n' that `[...].join('\n')` put between the
        // header line and the finding JSON.
        let targetTo = findingAt - 1;
        if (prompt[targetTo - 1] === ':') targetTo -= 1;
        out.target = prompt.slice(targetFrom, targetTo);
      } else {
        // No recoverable finding brace — fall back to the single-line read so
        // dimKey/target are still populated for diagnostics.
        const lineEnd = prompt.indexOf('\n', targetFrom);
        const rest = lineEnd === -1 ? prompt.slice(targetFrom) : prompt.slice(targetFrom, lineEnd);
        out.target = rest.endsWith(':') ? rest.slice(0, -1) : rest;
      }
    }
  }

  if (prompt.indexOf('unless the code proves otherwise') !== -1) out.mode = 'code';
  else if (prompt.indexOf('unless the plan proves otherwise') !== -1) out.mode = 'plan';

  if (typeof extractFinding === 'function') out.finding = extractFinding(prompt);
  return out;
}

/**
 * Index of the finding object's opening `{` in a refuter prompt, using the same
 * sentinel + brace-match anchoring `extractFinding` uses. Returns -1 when the
 * prompt is not shaped like a refuter prompt.
 */
export function findFindingStart(prompt, matchBrace) {
  const sentinel = '\nStart from the stance:';
  const sentinelAt = prompt.indexOf(sentinel);
  if (sentinelAt === -1) return -1;
  const closeAt = sentinelAt - 1;
  if (prompt[closeAt] !== '}') return -1;
  for (let i = 0; i <= closeAt; i++) {
    if (prompt[i] !== '{') continue;
    if (i !== 0 && prompt[i - 1] !== '\n') continue;
    if (matchBrace(prompt, i) !== closeAt) continue;
    return i;
  }
  return -1;
}

/**
 * Regenerate a corpus item's prompt through the REAL `refutePrompt`.
 *
 * `refutePrompt` is injected rather than imported so this module never
 * hard-depends on `.claude/workflows/lib/review.mjs` — keeping the harness a
 * strict consumer of the lane and never a dependency of it.
 *
 * @param {object} item
 * @param {(mode, dim, finding, context) => string} refutePrompt
 * @returns {string}
 */
export function regeneratePrompt(item, refutePrompt) {
  if (typeof refutePrompt !== 'function') throw new Error('regeneratePrompt requires the real refutePrompt function');
  return refutePrompt(item.mode, item.dim, item.finding, { target: item.target });
}

/**
 * Compare a regenerated prompt against the recorded `promptSha256`.
 * A mismatch is REPORTED, never silently accepted — it means `refutePrompt` was
 * edited after the corpus was mined, so replay is no longer byte-identical to
 * history. The item is still runnable; it just loses that guarantee.
 *
 * @returns {{ id: string, drifted: boolean, expected: string, actual: string }}
 */
export function checkPromptFidelity(item, refutePrompt) {
  const actual = sha256(regeneratePrompt(item, refutePrompt));
  return { id: item.id, drifted: actual !== item.promptSha256, expected: item.promptSha256, actual };
}

// --- Trial construction ---------------------------------------------------

/**
 * Build the flat, deterministic, stably-ordered trial plan: one entry per
 * `corpusId × tier × replicateIndex`, in corpus order then tier order then
 * replicate order. No clock, no shuffling — the same inputs always produce the
 * same plan in the same order, so a run is reproducible and diffable.
 *
 * @param {object[]} corpus
 * @param {{ tiers: string[], replicates?: number }} opts
 * @returns {Array<{ trialId: string, corpusId: string, tier: string, replicate: number }>}
 */
export function buildTrials(corpus, opts = {}) {
  const tiers = Array.isArray(opts.tiers) ? opts.tiers : [];
  const replicates = opts.replicates === undefined ? 2 : opts.replicates;
  if (tiers.length === 0) throw new Error('buildTrials requires at least one tier');
  if (!Number.isInteger(replicates) || replicates < 1) {
    throw new Error(`replicates must be a positive integer, got ${JSON.stringify(opts.replicates)}`);
  }
  const trials = [];
  for (const item of corpus) {
    for (const tier of tiers) {
      for (let r = 1; r <= replicates; r++) {
        trials.push({ trialId: `${item.id}|${tier}|${r}`, corpusId: item.id, tier, replicate: r });
      }
    }
  }
  return trials;
}

// --- Batch power: can this corpus answer the batching question at all? -----
//
// THE GROUPING KEY MUST CARRY THE REVIEW-UNIT IDENTITY.
//
// `buildReviewPipeline` is invoked ONCE PER REVIEW UNIT (one dispatch-phase
// code-review stage, one plan-review phase unit), never once per workflow run.
// A batched refuter dispatch is therefore exactly "one review unit's gating
// findings for ONE dimension". Grouping the corpus by `(runId, mode, dim)`
// merges findings raised against DIFFERENT review units inside the same run —
// a shape the pipeline can never produce — and inflates the apparent batch
// size. That coarse key is the same one phase 1 measured and rejected for
// `refuterFanout` (run wf_55af7324-87c: 96 refuters at one `phaseIndex` across
// 9 distinct review units). See docs/refuter-batching.md § Corpus power for the
// superseded figures and why they are void.

/** Minimum group size at which a batched dispatch can exhibit anchoring at all. */
export const MIN_BATCH_GROUP_SIZE = 3;

/**
 * Pre-registered floors for the batched arm. Derivation (recorded so they read
 * as pre-registered rather than post-hoc): at the floor the bounded run is
 * 6 batched dispatches × 2 replicates (12) + 18 per-finding dispatches × 2
 * replicates (36) = 48 dispatches, just under the ~55 ceiling the recorded
 * 533k-tokens-per-Opus-dispatch mean allows.
 */
export const MIN_QUALIFYING_BATCH_GROUPS = 6;
/** @see MIN_QUALIFYING_BATCH_GROUPS */
export const MIN_QUALIFYING_BATCH_ITEMS = 18;

/**
 * Agent-index gap above which two members of one (runId, unit, mode, dim) group
 * are treated as belonging to SEPARATE dispatches (a REWORK re-review is a
 * second find→refute chain the four-part key would otherwise silently merge).
 * A heuristic, deliberately loose: `refuterFanout.refuterCountsByUnit` puts p50
 * at 8.5 refuters per unit, so one round's members sit within a couple of dozen
 * agent slots while a re-review lands far beyond that.
 */
export const ROUND_SPLIT_GAP = 24;

const MAX_UNIT_IDENT_LENGTH = 200;

/**
 * Review-unit identity for a corpus item, from its already-extracted `target`.
 *
 * The rule is phase 1's, applied to the same structure: plan-mode targets are
 * `phase <roadmap>/<stem>` + a blank line + the body, so the FIRST LINE is the
 * identity; code-mode targets are a single bare line. A target that is itself
 * pretty-printed JSON (the `--implementation-plan` shape) or an implausibly
 * long single line is REJECTED rather than captured as a fake identity, and its
 * item is excluded from grouping entirely.
 *
 * NOTE — DELIBERATE LOCAL RESTATEMENT. The canonical predicate is
 * `isPlausibleUnitIdent` in `scripts/measure-refuter-severity.mjs`. It is not
 * imported here because that module is a CLI and importing it would invert the
 * dependency (that instrument imports from this one). The two copies cannot
 * drift: `scripts/verify-refuter-agreement.sh` § 2c-equivalence imports BOTH and
 * asserts they agree on every committed corpus item's target first line.
 *
 * @param {object} item
 * @returns {string|null}
 */
export function unitIdentOf(item) {
  if (!isPlainObject(item)) return null;
  const target = typeof item.target === 'string' ? item.target : '';
  const newlineAt = target.indexOf('\n');
  const first = newlineAt === -1 ? target : target.slice(0, newlineAt);
  if (first.length === 0) return null;
  if (first.indexOf('{') !== -1 || first.indexOf('"') !== -1) return null;
  if (first.length > MAX_UNIT_IDENT_LENGTH) return null;
  return first;
}

/**
 * The four-part batch-group key: `runId|unitIdent|mode|dim.key`.
 * Returns null when the item cannot be placed in a real dispatch — no run id
 * (a `constructed` item) or no recoverable unit identity.
 *
 * @param {object} item
 * @returns {string|null}
 */
export function batchGroupKeyFor(item) {
  if (!isPlainObject(item)) return null;
  const runId = item.provenance && item.provenance.runId;
  if (!nonEmptyString(runId)) return null;
  const unitIdent = unitIdentOf(item);
  if (unitIdent === null) return null;
  if (!nonEmptyString(item.mode)) return null;
  const dimKey = item.dim && item.dim.key;
  if (!nonEmptyString(dimKey)) return null;
  return `${runId}|${unitIdent}|${item.mode}|${dimKey}`;
}

function splitGroupByRound(members, gap) {
  // Every member must carry an agentIndex, or the group's real round structure
  // is unknown and the group size stands as an UPPER BOUND.
  if (!members.every((m) => Number.isInteger(m.provenance && m.provenance.agentIndex))) return null;
  const sorted = members.slice().sort((a, b) => a.provenance.agentIndex - b.provenance.agentIndex);
  const rounds = [[sorted[0]]];
  for (let i = 1; i < sorted.length; i++) {
    const delta = sorted[i].provenance.agentIndex - sorted[i - 1].provenance.agentIndex;
    if (delta > gap) rounds.push([]);
    rounds[rounds.length - 1].push(sorted[i]);
  }
  return rounds;
}

/**
 * Group a corpus (or a set of mined CANDIDATES — this function never reads
 * `groundTruth`) into the batches a real dispatch could form, and report whether
 * the resulting population has the power to answer the anchoring question.
 *
 * THREE EXCLUSIONS, each separately counted, applied before grouping:
 *
 *  - `constructed` items — no run id and no real review unit, so they cannot
 *    belong to any dispatch a production run could produce.
 *  - `HISTORICAL_ONLY_SEVERITIES` (`suggestion`) — never dispatched to a refuter
 *    since `workflow-token-reduction` phase 6, so never a batch member.
 *  - items whose unit identity is unrecoverable — excluded, never bucketed into
 *    a fake unit (the identical policy `refuterFanout.refuterCountsByUnit` used,
 *    so the two measurements stay comparable).
 *
 * @param {object[]} corpus
 * @param {{ minGroupSize?: number, roundSplitGap?: number,
 *   minQualifyingGroups?: number, minQualifyingItems?: number }} [opts]
 */
export function groupCorpusForBatching(corpus, opts = {}) {
  const minGroupSize = opts.minGroupSize === undefined ? MIN_BATCH_GROUP_SIZE : opts.minGroupSize;
  if (!Number.isInteger(minGroupSize) || minGroupSize < 2) {
    throw new Error(`minGroupSize must be an integer >= 2, got ${JSON.stringify(opts.minGroupSize)}`);
  }
  const roundSplitGap = opts.roundSplitGap === undefined ? ROUND_SPLIT_GAP : opts.roundSplitGap;
  const minQualifyingGroups =
    opts.minQualifyingGroups === undefined ? MIN_QUALIFYING_BATCH_GROUPS : opts.minQualifyingGroups;
  const minQualifyingItems =
    opts.minQualifyingItems === undefined ? MIN_QUALIFYING_BATCH_ITEMS : opts.minQualifyingItems;

  const items = Array.isArray(corpus) ? corpus : [];
  let constructedExcluded = 0;
  let nonGatingExcluded = 0;
  let unrecoverableUnitExcluded = 0;

  const buckets = new Map();
  for (const item of items) {
    if (item && item.provenance && item.provenance.kind === 'constructed') {
      constructedExcluded += 1;
      continue;
    }
    const severity = item && item.finding && item.finding.severity;
    if (HISTORICAL_ONLY_SEVERITIES.indexOf(severity) !== -1) {
      nonGatingExcluded += 1;
      continue;
    }
    const key = batchGroupKeyFor(item);
    if (key === null) {
      unrecoverableUnitExcluded += 1;
      continue;
    }
    let bucket = buckets.get(key);
    if (!bucket) {
      bucket = [];
      buckets.set(key, bucket);
    }
    bucket.push(item);
  }

  // Stable key sort — no clock, no insertion-order dependence.
  const keys = [...buckets.keys()].sort();
  const groups = [];
  let roundSplits = 0;
  let agentIndexedGroups = 0;
  for (const key of keys) {
    const members = buckets.get(key);
    const parts = key.split('|');
    const runId = parts[0];
    const dimKey = parts[parts.length - 1];
    const mode = parts[parts.length - 2];
    const unitIdent = parts.slice(1, parts.length - 2).join('|');
    const rounds = splitGroupByRound(members, roundSplitGap);
    if (rounds) {
      agentIndexedGroups += 1;
      if (rounds.length > 1) roundSplits += rounds.length - 1;
    }
    const emit = rounds || [members];
    emit.forEach((round, roundIndex) => {
      groups.push({
        key: emit.length > 1 ? `${key}#r${roundIndex + 1}` : key,
        runId,
        unitIdent,
        mode,
        dim: dimKey,
        // Stable corpus order within the group.
        ids: round.map((m) => m.id),
        size: round.length,
        agentIndexed: Boolean(rounds),
      });
    });
  }

  const groupableItems = groups.reduce((s, g) => s + g.size, 0);
  const sizeHistogram = {};
  const sizeHistogramByMode = {};
  let singletonItems = 0;
  let qualifyingGroups = 0;
  let qualifyingItems = 0;
  for (const g of groups) {
    sizeHistogram[g.size] = (sizeHistogram[g.size] || 0) + 1;
    if (!sizeHistogramByMode[g.mode]) sizeHistogramByMode[g.mode] = {};
    sizeHistogramByMode[g.mode][g.size] = (sizeHistogramByMode[g.mode][g.size] || 0) + 1;
    if (g.size === 1) singletonItems += g.size;
    if (g.size >= minGroupSize) {
      qualifyingGroups += 1;
      qualifyingItems += g.size;
    }
  }

  return {
    minGroupSize,
    minQualifyingGroups,
    minQualifyingItems,
    totalItems: items.length,
    constructedExcluded,
    nonGatingExcluded,
    unrecoverableUnitExcluded,
    groupableItems,
    groups,
    groupCount: groups.length,
    sizeHistogram,
    sizeHistogramByMode,
    singletonItems,
    qualifyingGroups,
    qualifyingItems,
    roundSplits,
    agentIndexedGroups,
    meetsMinimum: qualifyingGroups >= minQualifyingGroups && qualifyingItems >= minQualifyingItems,
  };
}

function histogramLine(hist) {
  const sizes = Object.keys(hist)
    .map(Number)
    .sort((a, b) => a - b);
  return sizes.length ? sizes.map((s) => `${s}:${hist[s]}`).join(', ') : '(none)';
}

/**
 * Render the batch-power analysis, ending in an explicit
 * `POWER: SUFFICIENT | INSUFFICIENT` verdict line. This output is the
 * PRE-REGISTERED step 1 of the phase: it is pasted verbatim into
 * `docs/refuter-batching.md` § Corpus power BEFORE any dispatch.
 *
 * @param {object} summary - a `groupCorpusForBatching` result
 * @returns {string}
 */
export function formatBatchPower(summary) {
  const out = [];
  out.push('Batch-size distribution the corpus can actually form (UNIT-SCOPED key: runId|unitIdent|mode|dim)');
  out.push('');
  out.push(`  corpus items                 ${summary.totalItems}`);
  out.push(`- constructed (no run/unit)    ${summary.constructedExcluded}`);
  out.push(`- non-gating (${HISTORICAL_ONLY_SEVERITIES.join(', ')})       ${summary.nonGatingExcluded}`);
  out.push(`- unrecoverable unit identity  ${summary.unrecoverableUnitExcluded}`);
  out.push(`= groupable items              ${summary.groupableItems}  in ${summary.groupCount} group(s)`);
  out.push('');
  out.push(`Size histogram (size:groups)   ${histogramLine(summary.sizeHistogram)}`);
  for (const mode of MODES) {
    const hist = summary.sizeHistogramByMode[mode];
    out.push(`  ${mode.padEnd(4)}                        ${hist ? histogramLine(hist) : '(none)'}`);
  }
  out.push('');
  out.push(`Minimum group size for the anchoring measurement: ${summary.minGroupSize}`);
  out.push(`Size-1 groups are EXCLUDED from it (${summary.singletonItems} item(s) in singleton groups).`);
  out.push(
    `Qualifying population: ${summary.qualifyingGroups} group(s) / ${summary.qualifyingItems} item(s) ` +
      `against floors of ${summary.minQualifyingGroups} group(s) / ${summary.minQualifyingItems} item(s).`
  );
  if (summary.agentIndexedGroups === 0) {
    out.push(
      'No group carries provenance.agentIndex, so every size below is an UPPER BOUND: a REWORK re-review is a ' +
        'second dispatch this key cannot yet split apart.'
    );
  } else {
    out.push(`Round splitting applied to ${summary.agentIndexedGroups} group(s); ${summary.roundSplits} split(s) made.`);
  }
  out.push('');
  out.push('POWER: ' + (summary.meetsMinimum ? 'SUFFICIENT' : 'INSUFFICIENT'));
  if (!summary.meetsMinimum) {
    out.push(
      'A batched arm built from this population is byte-for-byte a per-finding arm across most of its items, ' +
        'so the anchoring effect would be unobservable. This is a NO-MEASUREMENT outcome, not a passing gate.'
    );
  }
  return out.join('\n');
}

// --- The batched experiment shape -----------------------------------------

/**
 * The batched refuter prompt — a MINIMAL DELTA from `refutePrompt`.
 *
 * Same READ-ONLY stance, same "NOT a real issue unless the code/plan proves
 * otherwise" sentence. The only change is that the dimension's whole gating
 * finding set is rendered AS ONE UNIT — a JSON array with an explicit
 * `refute_id` per entry — and the closing instruction asks for one verdict per
 * `refute_id`. Any wording change beyond that would be a confound: the A/B
 * varies SHAPE, and shape only.
 *
 * This lives here, in the experiment, and NOT in
 * `.claude/workflows/lib/review.mjs`: the lane stays byte-unchanged until the
 * pre-registered decision rule passes.
 *
 * @param {string} mode
 * @param {{ key: string }} dim
 * @param {object[]} findings - each carrying a `refute_id`
 * @param {{ target?: string }} [context]
 * @returns {string}
 */
export function buildBatchPrompt(mode, dim, findings, context) {
  const target = (context && context.target) || '(the target described in your working directory)';
  return [
    'You are a READ-ONLY refuter. Do not edit any files.',
    'A prior reviewer raised these ' + dim.key + ' findings against ' + target + ':',
    JSON.stringify(findings, null, 2),
    'Start from the stance: this is NOT a real issue unless the ' +
      (mode === 'code' ? 'code' : 'plan') +
      ' proves otherwise. Read the actual cited location and its surrounding context before deciding.',
    'Return JSON matching the BATCH_VERDICT schema: verdicts, an array with ONE entry per refute_id above, ' +
      'each { id (the refute_id), refuted (boolean — true if that finding does not hold up), ' +
      'confidence (0-100 in your verdict), rationale }.',
  ].join('\n');
}

/**
 * Build the batched trial plan from a `groupCorpusForBatching` summary.
 *
 * Batched trials are built ONLY from unit-scoped groups at or above the minimum
 * size, so every batched dispatch is a shape production can actually produce.
 *
 * THROWS on an underpowered population. This is the MECHANICAL form of the
 * phase's own prohibition — "a batched arm dominated by size-1 batches is not
 * evidence and may not be reported as a passing gate" — rather than advisory
 * prose. `allowUnderpowered: true` builds it anyway, but stamps
 * `noMeasurement: true`, which forces the report's NO MEASUREMENT banner and
 * suppresses any decision line.
 *
 * @param {object} summary
 * @param {{ tiers: string[], replicates?: number, allowUnderpowered?: boolean }} opts
 */
export function buildBatchTrials(summary, opts = {}) {
  if (!isPlainObject(summary) || !Array.isArray(summary.groups)) {
    throw new Error('buildBatchTrials requires a groupCorpusForBatching summary (with .groups and .meetsMinimum)');
  }
  const tiers = Array.isArray(opts.tiers) ? opts.tiers : [];
  const replicates = opts.replicates === undefined ? 2 : opts.replicates;
  if (tiers.length === 0) throw new Error('buildBatchTrials requires at least one tier');
  if (!Number.isInteger(replicates) || replicates < 1) {
    throw new Error(`replicates must be a positive integer, got ${JSON.stringify(opts.replicates)}`);
  }
  if (!summary.meetsMinimum && !opts.allowUnderpowered) {
    throw new Error(
      'buildBatchTrials refuses to build an UNDERPOWERED batched arm: ' +
        `${summary.qualifyingGroups} qualifying group(s) / ${summary.qualifyingItems} item(s) at ` +
        `minGroupSize ${summary.minGroupSize}, against floors of ${summary.minQualifyingGroups} / ` +
        `${summary.minQualifyingItems}. A batched arm dominated by size-1 batches is not evidence and may not ` +
        'be reported as a passing gate — mine more adjudicated findings, or report a no-measurement outcome. ' +
        'Pass allowUnderpowered: true to build it anyway; the result is stamped noMeasurement and can never ' +
        'carry a decision.'
    );
  }

  const qualifying = summary.groups.filter((g) => g.size >= summary.minGroupSize);
  const trials = [];
  for (const g of qualifying) {
    for (const tier of tiers) {
      for (let r = 1; r <= replicates; r++) {
        const trialId = `${g.key}|${tier}|${r}`;
        trials.push({
          trialId,
          dispatchId: trialId,
          arm: 'batched',
          groupKey: g.key,
          unitIdent: g.unitIdent,
          mode: g.mode,
          dim: g.dim,
          tier,
          replicate: r,
          corpusIds: g.ids.slice(),
          dispatchSize: g.size,
        });
      }
    }
  }
  return {
    trials,
    groups: qualifying,
    corpusIds: [...new Set(qualifying.flatMap((g) => g.ids))],
    underpowered: !summary.meetsMinimum,
    noMeasurement: !summary.meetsMinimum,
  };
}

/**
 * Flatten completed BATCHED dispatches into per-finding scoring rows.
 *
 * Three resilience rules, mirroring the ones a shipped pipeline would have to
 * honor:
 *
 *  - a verdict for an id the dispatch did NOT contain is DROPPED and recorded
 *    under `unknownVerdictIds` — it never reaches any finding;
 *  - an id the response OMITS keeps `verdict: null` (ungraded) — never coerced
 *    to `refuted: false`, which would silently inflate the false-positive rate;
 *  - a crashed/malformed dispatch leaves every one of its ids ungraded.
 *
 * COST ATTRIBUTION: the dispatch's FULL usage/toolCalls is attributed to its
 * FIRST row and zeros to the rest, so `cost.totalTokens` stays exact while
 * `cost.dispatches` (counted by unique `dispatchId`) is not diluted by the
 * expansion.
 *
 * @param {Array<object>} batchResults
 * @returns {{ rows: object[], unknownVerdictIds: string[], omittedIds: string[] }}
 */
export function expandBatchResults(batchResults) {
  const rows = [];
  const unknownVerdictIds = [];
  const omittedIds = [];
  for (const res of Array.isArray(batchResults) ? batchResults : []) {
    const corpusIds = Array.isArray(res.corpusIds) ? res.corpusIds : [];
    const expected = new Set(corpusIds.map(String));
    const byId = new Map();
    const verdicts = res && res.verdicts;
    if (Array.isArray(verdicts)) {
      for (const v of verdicts) {
        if (!isPlainObject(v)) continue;
        const id = String(v.id);
        if (!expected.has(id)) {
          unknownVerdictIds.push(`${res.dispatchId}:${id}`);
          continue;
        }
        byId.set(id, {
          refuted: typeof v.refuted === 'boolean' ? v.refuted : null,
          confidence: typeof v.confidence === 'number' ? v.confidence : null,
          rationale: typeof v.rationale === 'string' ? v.rationale : null,
        });
      }
    }
    corpusIds.forEach((corpusId, index) => {
      const verdict = byId.get(String(corpusId)) || null;
      if (verdict === null) omittedIds.push(`${res.dispatchId}:${corpusId}`);
      rows.push({
        trialId: `${res.dispatchId}|${corpusId}`,
        corpusId,
        tier: res.tier,
        replicate: res.replicate,
        arm: 'batched',
        dispatchId: res.dispatchId,
        groupKey: res.groupKey,
        dispatchSize: corpusIds.length,
        positionInBatch: index + 1,
        verdict: verdict && typeof verdict.refuted === 'boolean' ? verdict : null,
        error: (res && res.error) || null,
        // The dispatch's whole cost lands on its FIRST row.
        usage: index === 0 ? res.usage || {} : {},
        toolCalls: index === 0 ? res.toolCalls || 0 : 0,
      });
    });
  }
  return { rows, unknownVerdictIds, omittedIds };
}

// --- Scoring --------------------------------------------------------------
//
// THE TWO RATES, DEFINED PRECISELY. They are not interchangeable and this
// module never averages them:
//
//   FALSE NEGATIVE  groundTruth.defect === true  AND  refuted === true
//                   A real defect was wrongly refuted. THIS SHIPS A BUG.
//                   Denominator: defect-truth trials only.
//
//   FALSE POSITIVE  groundTruth.defect === false AND  refuted === false
//                   A non-defect was kept. THIS COSTS A REWORK ROUND.
//                   Denominator: non-defect-truth trials only.
//
// The two denominators are structurally different, so a pooled "accuracy" is
// not merely discouraged here — it is unrepresentable. No combined field is
// emitted anywhere in the report, and `scripts/verify-refuter-agreement.sh`
// asserts that recursively.
//
// A trial whose response could not be graded (`verdict === null`, e.g. a
// missing or non-boolean `refuted`) is UNGRADED: it belongs to neither rate and
// is reported as its own count. Coercing it to `false` would silently inflate
// the false-positive rate.

function rate(part, whole) {
  return whole ? Math.round((part / whole) * 1000) / 10 : null;
}

function emptyRateSet() {
  return {
    trials: 0,
    ungraded: 0,
    defectTrials: 0,
    nonDefectTrials: 0,
    falseNegatives: 0,
    falseNegativeRate: null,
    correctRefutations: 0,
    falsePositives: 0,
    falsePositiveRate: null,
    correctKeeps: 0,
  };
}

function tallyInto(set, defect, refuted) {
  set.trials += 1;
  if (typeof refuted !== 'boolean') {
    set.ungraded += 1;
    return;
  }
  if (defect) {
    set.defectTrials += 1;
    if (refuted) set.falseNegatives += 1;
    else set.correctKeeps += 1;
  } else {
    set.nonDefectTrials += 1;
    if (refuted) set.correctRefutations += 1;
    else set.falsePositives += 1;
  }
}

function finalizeRateSet(set) {
  set.falseNegativeRate = rate(set.falseNegatives, set.defectTrials);
  set.falsePositiveRate = rate(set.falsePositives, set.nonDefectTrials);
  return set;
}

function emptyUsage() {
  const u = { toolCalls: 0 };
  for (const c of TOKEN_CLASSES) u[c] = 0;
  return u;
}

function sumTokens(u) {
  return TOKEN_CLASSES.reduce((s, c) => s + (u[c] || 0), 0);
}

function mean(total, n) {
  return n ? Math.round((total / n) * 10) / 10 : null;
}

/**
 * The report bucket a trial belongs to.
 *
 * A trial with no `arm` buckets under its bare tier, exactly as before — the
 * per-finding arm's report shape is unchanged. A trial carrying an `arm`
 * buckets under `tier|arm`, so the two SHAPES are scored side by side over the
 * same corpus without either denominator contaminating the other.
 *
 * @param {{ tier: string, arm?: string }} trial
 * @returns {string}
 */
export function bucketKeyFor(trial) {
  if (!trial) return '';
  return trial.arm ? `${trial.tier}|${trial.arm}` : String(trial.tier);
}

/**
 * Score a completed trial set against the corpus.
 *
 * Returns, per tier:
 *   - THREE parallel rate sets: `authoritativeOnly` (the DECISION-GRADE figure),
 *     `judgementCallOnly`, and `all`.
 *   - a per-class breakdown, because the corpus is deliberately weighted toward
 *     the divergence class and the aggregate is therefore NOT a population
 *     estimate of production finding mix.
 *   - self-consistency: for each (item, tier) pair with >= 2 graded replicates,
 *     whether every replicate returned the same boolean. A tier can post a good
 *     FN rate purely by coin-flip; `flipRate` is what makes that visible.
 *   - the four token classes plus tool-call counts, so agreement and cost are
 *     read together, never apart.
 *
 * Trials whose severity is in `HISTORICAL_ONLY_SEVERITIES` are excluded from
 * every headline rate (phase 6 stopped spawning refuters for them, so they
 * cannot be affected by any tiering decision) and reported as their own count.
 *
 * @param {object[]} corpus
 * @param {Array<object>} trials - each `{ corpusId, tier, replicate, verdict }`
 *   where `verdict` is `{ refuted, confidence?, rationale? }` or null, plus an
 *   optional `usage` and `toolCalls`.
 * @param {{ baselineTier?: string }} [opts]
 */
export function scoreTrials(corpus, trials, opts = {}) {
  const byId = new Map(corpus.map((i) => [i.id, i]));
  const unknownIds = [];
  const tierOrder = [];
  const perTier = new Map();

  let historicalOnlyTrials = 0;

  function tierBucket(bucket, trial) {
    let b = perTier.get(bucket);
    if (!b) {
      b = {
        bucket,
        tier: trial.tier,
        arm: trial.arm || null,
        authoritativeOnly: emptyRateSet(),
        judgementCallOnly: emptyRateSet(),
        all: emptyRateSet(),
        byClass: new Map(),
        usage: emptyUsage(),
        gradedTrials: 0,
        gradedFindings: 0,
        dispatchIds: new Set(),
        replicatePairs: 0,
        replicateFlips: 0,
        flipRate: null,
        historicalOnlyTrials: 0,
      };
      perTier.set(bucket, b);
      tierOrder.push(bucket);
    }
    return b;
  }

  // Per (item, tier) verdict lists, for self-consistency.
  const replicateVerdicts = new Map();

  for (const t of trials) {
    const item = byId.get(t.corpusId);
    if (!item) {
      unknownIds.push(t.corpusId);
      continue;
    }
    const b = tierBucket(bucketKeyFor(t), t);

    // Cost is recorded for EVERY dispatched trial, including historical-only
    // and ungraded ones — the tokens were really spent.
    const u = t.usage || {};
    for (const c of TOKEN_CLASSES) b.usage[c] += Number(u[c] || 0);
    b.usage.toolCalls += Number(t.toolCalls || 0);
    b.gradedTrials += 1;
    // Unique DISPATCHES, so an expanded batched row set does not read as N
    // separate dispatches and dilute the per-dispatch token figure.
    b.dispatchIds.add(t.dispatchId === undefined || t.dispatchId === null ? t.trialId : t.dispatchId);

    if (HISTORICAL_ONLY_SEVERITIES.indexOf(item.finding.severity) !== -1) {
      b.historicalOnlyTrials += 1;
      historicalOnlyTrials += 1;
      continue;
    }
    b.gradedFindings += 1;

    const refuted = t.verdict && typeof t.verdict.refuted === 'boolean' ? t.verdict.refuted : null;
    const defect = item.groundTruth.defect;

    tallyInto(b.all, defect, refuted);
    tallyInto(item.groundTruth.authority === 'authoritative' ? b.authoritativeOnly : b.judgementCallOnly, defect, refuted);

    let cls = b.byClass.get(item.groundTruth.class);
    if (!cls) {
      cls = emptyRateSet();
      b.byClass.set(item.groundTruth.class, cls);
    }
    tallyInto(cls, defect, refuted);

    // Keyed on the BUCKET, not the bare tier: with two arms in one report,
    // `opus` and `opus|batched` must keep independent self-consistency figures.
    const key = `${t.corpusId} ${b.bucket}`;
    let entry = replicateVerdicts.get(key);
    if (!entry) {
      entry = { bucket: b.bucket, list: [] };
      replicateVerdicts.set(key, entry);
    }
    entry.list.push(refuted);
  }

  for (const entry of replicateVerdicts.values()) {
    const graded = entry.list.filter((v) => typeof v === 'boolean');
    if (graded.length < 2) continue;
    const b = perTier.get(entry.bucket);
    b.replicatePairs += 1;
    if (!graded.every((v) => v === graded[0])) b.replicateFlips += 1;
  }

  const tiers = tierOrder.map((bucketKey) => {
    const b = perTier.get(bucketKey);
    finalizeRateSet(b.authoritativeOnly);
    finalizeRateSet(b.judgementCallOnly);
    finalizeRateSet(b.all);
    b.flipRate = rate(b.replicateFlips, b.replicatePairs);
    const byClass = [...b.byClass.entries()]
      .sort((a, c) => (a[0] < c[0] ? -1 : a[0] > c[0] ? 1 : 0))
      .map(([className, set]) => ({ class: className, ...finalizeRateSet(set) }));
    const totalTokens = sumTokens(b.usage);
    const dispatches = b.dispatchIds.size;
    return {
      bucket: b.bucket,
      tier: b.tier,
      arm: b.arm,
      authoritativeOnly: b.authoritativeOnly,
      judgementCallOnly: b.judgementCallOnly,
      all: b.all,
      byClass,
      selfConsistency: { replicatePairs: b.replicatePairs, replicateFlips: b.replicateFlips, flipRate: b.flipRate },
      historicalOnlyTrials: b.historicalOnlyTrials,
      cost: {
        dispatchedTrials: b.gradedTrials,
        // DISPATCHES vs GRADED FINDINGS: identical for the per-finding arm, and
        // deliberately different for the batched one — that ratio IS the token
        // argument this phase is testing.
        dispatches,
        gradedFindings: b.gradedFindings,
        ...Object.fromEntries(TOKEN_CLASSES.map((c) => [c, b.usage[c]])),
        totalTokens,
        meanTokensPerTrial: mean(totalTokens, b.gradedTrials),
        meanTokensPerDispatch: mean(totalTokens, dispatches),
        meanTokensPerGradedFinding: mean(totalTokens, b.gradedFindings),
        toolCalls: b.usage.toolCalls,
        meanToolCallsPerTrial: mean(b.usage.toolCalls, b.gradedTrials),
      },
    };
  });

  const baselineTier = opts.baselineTier || (tiers.length ? tiers[0].bucket : null);
  const baseline = tiers.find((t) => t.bucket === baselineTier) || tiers.find((t) => t.tier === baselineTier) || null;
  for (const t of tiers) {
    if (
      !baseline ||
      t.bucket === baseline.bucket ||
      baseline.cost.meanTokensPerTrial === null ||
      t.cost.meanTokensPerTrial === null
    ) {
      t.tokenDelta = null;
      continue;
    }
    const delta = t.cost.meanTokensPerTrial - baseline.cost.meanTokensPerTrial;
    t.tokenDelta = {
      baselineTier,
      meanTokensPerTrial: t.cost.meanTokensPerTrial,
      baselineMeanTokensPerTrial: baseline.cost.meanTokensPerTrial,
      delta: Math.round(delta * 10) / 10,
      percent: baseline.cost.meanTokensPerTrial ? Math.round((delta / baseline.cost.meanTokensPerTrial) * 1000) / 10 : null,
    };
  }

  const report = {
    corpus: summarizeCorpus(corpus),
    baselineTier,
    tiers,
    historicalOnlyTrials,
    unknownCorpusIds: [...new Set(unknownIds)].sort(),
    caveats: CAVEATS,
  };
  // A batched arm built from an underpowered population can never carry a
  // decision — `formatReport` prints a NO MEASUREMENT banner and no decision
  // line whenever this is set.
  if (opts.noMeasurement) report.noMeasurement = true;
  if (opts.anchoring) report.anchoring = opts.anchoring;
  if (opts.decision) report.decision = opts.decision;
  if (opts.batchPower) report.batchPower = opts.batchPower;
  return report;
}

/**
 * The ANCHORING measurement: does batching CORRELATE verdicts?
 *
 * Reported over QUALIFYING groups only (`size >= minGroupSize`) — a size-1
 * "batch" is byte-for-byte a per-finding dispatch and can exhibit no anchoring,
 * so including it would dilute the effect this function exists to detect.
 *
 * Two signals, both per arm over the SAME group set:
 *
 *  - `allSameVerdictShare` — the share of (group × tier × replicate) dispatches
 *    whose graded verdicts were all identical. A batched refuter that anchors on
 *    its first verdict posts a HIGHER share than the per-finding arm.
 *  - `refutationRateByPosition` — the refutation rate at position 1 versus
 *    positions 2..n in the group's stable id order. A rise after position 1 is
 *    the anchoring signature.
 *
 * FN and FP are NOT combined here — this block reports refutation rates and
 * agreement shares, never an accuracy.
 *
 * @param {Array<object>} groups - qualifying groups from `groupCorpusForBatching`
 * @param {{ [arm: string]: Array<object> }} rowsByArm - scoring rows per arm
 * @param {{ minGroupSize?: number }} [opts]
 */
export function scoreAnchoring(groups, rowsByArm, opts = {}) {
  const minGroupSize = opts.minGroupSize === undefined ? MIN_BATCH_GROUP_SIZE : opts.minGroupSize;
  const qualifying = (Array.isArray(groups) ? groups : []).filter((g) => g.size >= minGroupSize);
  const groupOf = new Map();
  const positionOf = new Map();
  for (const g of qualifying) {
    g.ids.forEach((id, i) => {
      groupOf.set(id, g.key);
      positionOf.set(id, i + 1);
    });
  }

  const arms = Object.keys(rowsByArm)
    .sort()
    .map((arm) => {
      const dispatches = new Map();
      const byPosition = new Map();
      for (const row of rowsByArm[arm] || []) {
        const groupKey = groupOf.get(row.corpusId);
        if (groupKey === undefined) continue;
        const refuted = row.verdict && typeof row.verdict.refuted === 'boolean' ? row.verdict.refuted : null;
        const dkey = `${groupKey} ${row.tier} ${row.replicate}`;
        let d = dispatches.get(dkey);
        if (!d) {
          d = [];
          dispatches.set(dkey, d);
        }
        d.push(refuted);

        const pos = positionOf.get(row.corpusId);
        let p = byPosition.get(pos);
        if (!p) {
          p = { position: pos, graded: 0, refuted: 0, refutedRate: null };
          byPosition.set(pos, p);
        }
        if (typeof refuted === 'boolean') {
          p.graded += 1;
          if (refuted) p.refuted += 1;
        }
      }

      let considered = 0;
      let allSame = 0;
      for (const list of dispatches.values()) {
        const graded = list.filter((v) => typeof v === 'boolean');
        if (graded.length < 2) continue;
        considered += 1;
        if (graded.every((v) => v === graded[0])) allSame += 1;
      }

      const positions = [...byPosition.values()].sort((a, b) => a.position - b.position);
      for (const p of positions) p.refutedRate = rate(p.refuted, p.graded);
      const first = positions.find((p) => p.position === 1) || { graded: 0, refuted: 0 };
      const later = positions
        .filter((p) => p.position > 1)
        .reduce((acc, p) => ({ graded: acc.graded + p.graded, refuted: acc.refuted + p.refuted }), { graded: 0, refuted: 0 });
      const laterRates = positions.filter((p) => p.position > 1 && p.graded > 0).map((p) => p.refutedRate);
      const risesAfterFirst =
        laterRates.length > 0 &&
        first.graded > 0 &&
        laterRates.every((r, i) => (i === 0 ? r > rate(first.refuted, first.graded) : r >= laterRates[i - 1])) &&
        laterRates[laterRates.length - 1] > rate(first.refuted, first.graded);

      return {
        arm,
        dispatchesConsidered: considered,
        allSameVerdict: allSame,
        allSameVerdictShare: rate(allSame, considered),
        refutationRateByPosition: {
          firstPosition: { graded: first.graded, refuted: first.refuted, refutedRate: rate(first.refuted, first.graded) },
          laterPositions: { graded: later.graded, refuted: later.refuted, refutedRate: rate(later.refuted, later.graded) },
          byPosition: positions,
          risesAfterFirst,
        },
      };
    });

  return {
    minGroupSize,
    qualifyingGroups: qualifying.length,
    qualifyingItems: qualifying.reduce((s, g) => s + g.size, 0),
    arms,
    note:
      'A HIGHER all-same-verdict share for the batched arm plus a refutation rate that RISES after position 1 is ' +
      'the anchoring signature this phase exists to detect. Reported over qualifying groups only — a size-1 ' +
      'group is byte-for-byte a per-finding dispatch and can exhibit no anchoring.',
  };
}

/** Standing caveats. Rendered verbatim by `formatReport`; carried in the JSON too. */
export const CAVEATS = [
  'The corpus is DELIBERATELY WEIGHTED toward ' + DIVERGENCE_CLASS + ' (the class where the tiers diverged). ' +
    'The aggregate is therefore NOT a population estimate of production finding mix — quote the per-class rates.',
  'False negatives and false positives have asymmetric cost and structurally different denominators. ' +
    'They are reported separately and are never averaged into an accuracy number.',
  'Token figures here are VOLUME, not price. Re-tiering changes price-per-token, not token volume ' +
    '(the initial A/B measured Sonnet at 52.9k vs Opus 48.4k per refuter). Any cost conclusion is a ' +
    'price-per-token argument and must be labelled as such.',
  'Historical `suggestion`-severity items are recorded but excluded from every headline rate: phase 6 ' +
    'landed NON_GATING_SEVERITIES = [suggestion], so no refuter is spawned for one again.',
  'Ground truth is adjudicated against the PINNED TREE recorded in each item\'s ' +
    'groundTruth.adjudicatedAgainstCommit — the tree a replay run actually reads — not against the tree ' +
    'the historical refuter read. A finding that was real then and is fixed now is `stale-fact`/defect:false, ' +
    'because that is the correct answer for a refuter reading this tree.',
];

/** Composition summary — sizes, class mix, authority split, provenance split. */
export function summarizeCorpus(corpus) {
  const byClass = {};
  const byAuthority = {};
  const byProvenance = {};
  const bySeverity = {};
  const byMode = {};
  for (const item of corpus) {
    byClass[item.groundTruth.class] = (byClass[item.groundTruth.class] || 0) + 1;
    byAuthority[item.groundTruth.authority] = (byAuthority[item.groundTruth.authority] || 0) + 1;
    byProvenance[item.provenance.kind] = (byProvenance[item.provenance.kind] || 0) + 1;
    bySeverity[item.finding.severity] = (bySeverity[item.finding.severity] || 0) + 1;
    byMode[item.mode] = (byMode[item.mode] || 0) + 1;
  }
  const size = corpus.length;
  const share = (n) => (size ? Math.round(((n || 0) / size) * 1000) / 10 : 0);
  return {
    size,
    byClass,
    byAuthority,
    byProvenance,
    bySeverity,
    byMode,
    divergenceClassShare: share(byClass[DIVERGENCE_CLASS]),
    authoritativeShare: share(byAuthority.authoritative),
    minedShare: share(byProvenance.mined),
    defectItems: corpus.filter((i) => i.groundTruth.defect).length,
    nonDefectItems: corpus.filter((i) => !i.groundTruth.defect).length,
    driftedItems: corpus.filter((i) => i.promptDrift).length,
  };
}

// --- Rendering ------------------------------------------------------------

function pctStr(v) {
  return v === null ? 'n/a' : v.toFixed(1) + '%';
}

function num(n) {
  return typeof n === 'number' ? n.toLocaleString('en-US') : String(n);
}

/**
 * Render the report.
 *
 * The text form deliberately renders FALSE NEGATIVES and FALSE POSITIVES as two
 * visually separate labelled blocks with their asymmetric consequences spelled
 * out inline, so a skimmer cannot mentally average them — and puts each tier's
 * token and tool-call columns on the SAME row as its FN/FP figures, so
 * agreement and cost are never read apart.
 *
 * @param {object} report
 * @param {'text'|'json'} format
 */
export function formatReport(report, format = 'text') {
  if (format === 'json') return JSON.stringify(report, null, 2);
  const out = [];
  const c = report.corpus;
  if (report.noMeasurement) {
    out.push('NO MEASUREMENT — batched arm was underpowered');
    out.push(
      'The qualifying (size >= ' +
        MIN_BATCH_GROUP_SIZE +
        ') unit-scoped batch population did not clear the pre-registered floor, so the figures below describe ' +
        'a batched arm dominated by size-1 batches. That is not evidence and carries NO decision.'
    );
    out.push('');
  }
  out.push('Refuter agreement by model tier — ' + c.size + ' corpus item(s), baseline tier "' + report.baselineTier + '"');
  out.push(
    'Composition: ' + Object.entries(c.byClass).map(([k, v]) => k + ' ' + v).join(', ') +
      ' | authority: ' + Object.entries(c.byAuthority).map(([k, v]) => k + ' ' + v).join(', ') +
      ' | provenance: ' + Object.entries(c.byProvenance).map(([k, v]) => k + ' ' + v).join(', ')
  );
  out.push('');

  out.push('## FALSE NEGATIVES — a real defect wrongly refuted -> SHIPS A DEFECT');
  out.push('Denominator: defect-truth trials only. Authoritative-only is the DECISION-GRADE figure.');
  out.push('| tier | authoritative FN | judgement-call FN | all FN | output | uncached input | cache write | cache read | mean tokens/trial | mean tool calls/trial |');
  out.push('|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|');
  for (const t of report.tiers) {
    out.push(
      '| ' +
        [
          t.bucket,
          t.authoritativeOnly.falseNegatives + '/' + t.authoritativeOnly.defectTrials + ' (' + pctStr(t.authoritativeOnly.falseNegativeRate) + ')',
          t.judgementCallOnly.falseNegatives + '/' + t.judgementCallOnly.defectTrials + ' (' + pctStr(t.judgementCallOnly.falseNegativeRate) + ')',
          t.all.falseNegatives + '/' + t.all.defectTrials + ' (' + pctStr(t.all.falseNegativeRate) + ')',
          num(t.cost.output),
          num(t.cost.uncachedInput),
          num(t.cost.cacheWrite),
          num(t.cost.cacheRead),
          num(t.cost.meanTokensPerTrial),
          num(t.cost.meanToolCallsPerTrial),
        ].join(' | ') +
        ' |'
    );
  }
  out.push('');

  out.push('## FALSE POSITIVES — a non-defect kept -> COSTS A REWORK ROUND');
  out.push('Denominator: non-defect-truth trials only. A DIFFERENT denominator from the block above.');
  out.push('| tier | authoritative FP | judgement-call FP | all FP | ungraded | mean tokens/trial | mean tool calls/trial |');
  out.push('|---|---:|---:|---:|---:|---:|---:|');
  for (const t of report.tiers) {
    out.push(
      '| ' +
        [
          t.bucket,
          t.authoritativeOnly.falsePositives + '/' + t.authoritativeOnly.nonDefectTrials + ' (' + pctStr(t.authoritativeOnly.falsePositiveRate) + ')',
          t.judgementCallOnly.falsePositives + '/' + t.judgementCallOnly.nonDefectTrials + ' (' + pctStr(t.judgementCallOnly.falsePositiveRate) + ')',
          t.all.falsePositives + '/' + t.all.nonDefectTrials + ' (' + pctStr(t.all.falsePositiveRate) + ')',
          t.all.ungraded,
          num(t.cost.meanTokensPerTrial),
          num(t.cost.meanToolCallsPerTrial),
        ].join(' | ') +
        ' |'
    );
  }
  out.push('');

  out.push('## SELF-CONSISTENCY — same item, same tier, replicate disagreement');
  out.push('| tier | replicate pairs | flips | flip rate |');
  out.push('|---|---:|---:|---:|');
  for (const t of report.tiers) {
    out.push('| ' + [t.bucket, t.selfConsistency.replicatePairs, t.selfConsistency.replicateFlips, pctStr(t.selfConsistency.flipRate)].join(' | ') + ' |');
  }
  out.push('A tier with a low FN rate but a high flip rate is not safer — it is lucky.');
  out.push('');

  out.push('## PER-CLASS (the aggregate above is over a deliberately weighted corpus)');
  for (const t of report.tiers) {
    out.push('### ' + t.bucket);
    out.push('| class | FN (defect-truth trials) | FP (non-defect-truth trials) | ungraded |');
    out.push('|---|---:|---:|---:|');
    for (const row of t.byClass) {
      out.push(
        '| ' +
          [
            row.class,
            row.falseNegatives + '/' + row.defectTrials + ' (' + pctStr(row.falseNegativeRate) + ')',
            row.falsePositives + '/' + row.nonDefectTrials + ' (' + pctStr(row.falsePositiveRate) + ')',
            row.ungraded,
          ].join(' | ') +
          ' |'
      );
    }
    out.push('');
  }

  out.push('## TOKEN VOLUME vs BASELINE');
  for (const t of report.tiers) {
    if (!t.tokenDelta) {
      out.push('- ' + t.bucket + ': baseline (' + num(t.cost.meanTokensPerTrial) + ' mean tokens/trial, ' + num(t.cost.meanToolCallsPerTrial) + ' mean tool calls/trial).');
      continue;
    }
    const d = t.tokenDelta;
    out.push(
      '- ' + t.bucket + ': ' + num(d.meanTokensPerTrial) + ' mean tokens/trial vs ' + num(d.baselineMeanTokensPerTrial) +
        ' for ' + d.baselineTier + ' — a delta of ' + num(d.delta) + ' (' + pctStr(d.percent) + ').'
    );
  }
  out.push('Re-tiering changes PRICE-PER-TOKEN, not token VOLUME. These are volume figures only.');
  out.push('');

  if (report.tiers.some((t) => t.arm)) {
    out.push('## TOKENS PER GRADED FINDING (the shape comparison — dispatches vs findings graded)');
    out.push('| bucket | dispatches | graded findings | total tokens | mean tokens/dispatch | mean tokens/graded finding |');
    out.push('|---|---:|---:|---:|---:|---:|');
    for (const t of report.tiers) {
      out.push(
        '| ' +
          [
            t.bucket,
            t.cost.dispatches,
            t.cost.gradedFindings,
            num(t.cost.totalTokens),
            num(t.cost.meanTokensPerDispatch),
            num(t.cost.meanTokensPerGradedFinding),
          ].join(' | ') +
          ' |'
      );
    }
    out.push('');
  }

  if (report.anchoring) {
    const a = report.anchoring;
    out.push('## ANCHORING — does batching CORRELATE verdicts?');
    out.push(
      'Over qualifying groups only (size >= ' + a.minGroupSize + '): ' + a.qualifyingGroups + ' group(s) / ' +
        a.qualifyingItems + ' item(s).'
    );
    out.push('| arm | dispatches considered | all-same verdict | all-same share | refuted @pos1 | refuted @pos2..n | rises after pos1 |');
    out.push('|---|---:|---:|---:|---:|---:|---|');
    for (const arm of a.arms) {
      const p = arm.refutationRateByPosition;
      out.push(
        '| ' +
          [
            arm.arm,
            arm.dispatchesConsidered,
            arm.allSameVerdict,
            pctStr(arm.allSameVerdictShare),
            p.firstPosition.refuted + '/' + p.firstPosition.graded + ' (' + pctStr(p.firstPosition.refutedRate) + ')',
            p.laterPositions.refuted + '/' + p.laterPositions.graded + ' (' + pctStr(p.laterPositions.refutedRate) + ')',
            p.risesAfterFirst ? 'YES' : 'no',
          ].join(' | ') +
          ' |'
      );
    }
    out.push(a.note);
    out.push('');
  }

  if (report.decision && !report.noMeasurement) {
    out.push('DECISION: ' + report.decision);
    out.push('');
  }

  if (report.historicalOnlyTrials) {
    out.push('Excluded from every rate above: ' + report.historicalOnlyTrials + ' trial(s) on historical-only severities (' +
      HISTORICAL_ONLY_SEVERITIES.join(', ') + ').');
    out.push('');
  }
  if (report.unknownCorpusIds.length) {
    out.push('WARNING: ' + report.unknownCorpusIds.length + ' trial corpus id(s) are not in the corpus: ' + report.unknownCorpusIds.join(', '));
    out.push('');
  }

  out.push('## CAVEATS');
  for (const line of report.caveats) out.push('- ' + line);
  return out.join('\n');
}

// --- docs/token-baseline.json § refuterModelTiering audit -----------------

/**
 * Corpus-FREE arithmetic audit of a `refuterModelTiering` section: do the doc's
 * own numbers agree with each other? Catches a hand-edited or stale figure on
 * any machine without reading a single sidecar or dispatching a single agent.
 *
 * @param {object} section
 * @returns {string[]} problems, empty when consistent
 */
export function auditTieringSection(section) {
  const problems = [];
  if (!isPlainObject(section)) return ['refuterModelTiering section is not an object'];

  const corpus = section.corpus || {};
  if (!Number.isInteger(corpus.size) || corpus.size < MIN_CORPUS_SIZE) {
    problems.push(`corpus.size must be an integer >= ${MIN_CORPUS_SIZE}, got ${JSON.stringify(corpus.size)}`);
  }
  for (const [field, map] of [['byClass', corpus.byClass], ['byAuthority', corpus.byAuthority], ['byProvenance', corpus.byProvenance]]) {
    if (!isPlainObject(map)) {
      problems.push(`corpus.${field} is missing`);
      continue;
    }
    const sum = Object.values(map).reduce((s, n) => s + (Number(n) || 0), 0);
    if (sum !== corpus.size) problems.push(`corpus.${field} counts sum to ${sum}, corpus.size says ${corpus.size}`);
  }
  if (isPlainObject(corpus.byClass)) {
    const derived = corpus.size ? Math.round(((corpus.byClass[DIVERGENCE_CLASS] || 0) / corpus.size) * 1000) / 10 : 0;
    if (corpus.divergenceClassShare !== derived) {
      problems.push(`corpus.divergenceClassShare: doc ${corpus.divergenceClassShare}, derived ${derived}`);
    }
    if (derived < MIN_DIVERGENCE_CLASS_SHARE * 100) {
      problems.push(`divergence-class share ${derived}% is below the ${MIN_DIVERGENCE_CLASS_SHARE * 100}% floor`);
    }
  }
  if (isPlainObject(corpus.byAuthority)) {
    const derived = corpus.size ? Math.round(((corpus.byAuthority.authoritative || 0) / corpus.size) * 1000) / 10 : 0;
    if (corpus.authoritativeShare !== derived) problems.push(`corpus.authoritativeShare: doc ${corpus.authoritativeShare}, derived ${derived}`);
    if (derived < MIN_AUTHORITATIVE_SHARE * 100) {
      problems.push(`authoritative share ${derived}% is below the ${MIN_AUTHORITATIVE_SHARE * 100}% floor`);
    }
  }
  if (isPlainObject(corpus.byProvenance)) {
    const derived = corpus.size ? Math.round(((corpus.byProvenance.mined || 0) / corpus.size) * 1000) / 10 : 0;
    if (corpus.minedShare !== derived) problems.push(`corpus.minedShare: doc ${corpus.minedShare}, derived ${derived}`);
    if (derived < MIN_MINED_SHARE * 100) problems.push(`mined share ${derived}% is below the ${MIN_MINED_SHARE * 100}% floor`);
  }

  const tiers = Array.isArray(section.tiers) ? section.tiers : [];
  if (tiers.length < 2) problems.push('tiers must record at least two model tiers');
  for (const t of tiers) {
    for (const setName of ['authoritativeOnly', 'judgementCallOnly', 'all']) {
      const s = t[setName];
      if (!isPlainObject(s)) {
        problems.push(`${t.tier}.${setName} is missing`);
        continue;
      }
      if ((s.falseNegatives || 0) > (s.defectTrials || 0)) {
        problems.push(`${t.tier}.${setName}: falseNegatives ${s.falseNegatives} exceeds defectTrials ${s.defectTrials}`);
      }
      if ((s.falsePositives || 0) > (s.nonDefectTrials || 0)) {
        problems.push(`${t.tier}.${setName}: falsePositives ${s.falsePositives} exceeds nonDefectTrials ${s.nonDefectTrials}`);
      }
      const fn = rate(s.falseNegatives || 0, s.defectTrials || 0);
      if (s.falseNegativeRate !== fn) problems.push(`${t.tier}.${setName}.falseNegativeRate: doc ${s.falseNegativeRate}, derived ${fn}`);
      const fp = rate(s.falsePositives || 0, s.nonDefectTrials || 0);
      if (s.falsePositiveRate !== fp) problems.push(`${t.tier}.${setName}.falsePositiveRate: doc ${s.falsePositiveRate}, derived ${fp}`);
      if ((s.defectTrials || 0) + (s.nonDefectTrials || 0) + (s.ungraded || 0) !== (s.trials || 0)) {
        problems.push(
          `${t.tier}.${setName}: defectTrials + nonDefectTrials + ungraded ` +
            `(${(s.defectTrials || 0) + (s.nonDefectTrials || 0) + (s.ungraded || 0)}) != trials ${s.trials}`
        );
      }
    }
    // authoritativeOnly + judgementCallOnly must partition `all`.
    const a = t.authoritativeOnly || {};
    const j = t.judgementCallOnly || {};
    const all = t.all || {};
    for (const f of ['trials', 'ungraded', 'defectTrials', 'nonDefectTrials', 'falseNegatives', 'falsePositives']) {
      if ((a[f] || 0) + (j[f] || 0) !== (all[f] || 0)) {
        problems.push(`${t.tier}.all.${f} ${all[f]} != authoritativeOnly ${a[f]} + judgementCallOnly ${j[f]}`);
      }
    }
    const cost = t.cost || {};
    const summed = TOKEN_CLASSES.reduce((s, cl) => s + (Number(cost[cl]) || 0), 0);
    if (cost.totalTokens !== summed) problems.push(`${t.tier}.cost.totalTokens: doc ${cost.totalTokens}, four classes sum to ${summed}`);
    const meanTokens = mean(summed, cost.dispatchedTrials || 0);
    if (cost.meanTokensPerTrial !== meanTokens) {
      problems.push(`${t.tier}.cost.meanTokensPerTrial: doc ${cost.meanTokensPerTrial}, derived ${meanTokens}`);
    }
    const meanCalls = mean(cost.toolCalls || 0, cost.dispatchedTrials || 0);
    if (cost.meanToolCallsPerTrial !== meanCalls) {
      problems.push(`${t.tier}.cost.meanToolCallsPerTrial: doc ${cost.meanToolCallsPerTrial}, derived ${meanCalls}`);
    }
    const sc = t.selfConsistency || {};
    const fr = rate(sc.replicateFlips || 0, sc.replicatePairs || 0);
    if (sc.flipRate !== fr) problems.push(`${t.tier}.selfConsistency.flipRate: doc ${sc.flipRate}, derived ${fr}`);
  }

  // tokenDelta rows must agree with the rows they compare.
  const baselineTier = section.baselineTier;
  const baseline = tiers.find((t) => t.tier === baselineTier);
  if (!baseline) problems.push(`baselineTier "${baselineTier}" is not among the recorded tiers`);
  for (const t of tiers) {
    if (t.tier === baselineTier) {
      if (t.tokenDelta !== null && t.tokenDelta !== undefined) problems.push(`${t.tier} is the baseline and must carry tokenDelta: null`);
      continue;
    }
    if (!isPlainObject(t.tokenDelta)) {
      problems.push(`${t.tier}.tokenDelta is missing`);
      continue;
    }
    if (!baseline) continue;
    const expected = Math.round((t.cost.meanTokensPerTrial - baseline.cost.meanTokensPerTrial) * 10) / 10;
    if (t.tokenDelta.delta !== expected) problems.push(`${t.tier}.tokenDelta.delta: doc ${t.tokenDelta.delta}, derived ${expected}`);
  }

  if (!nonEmptyString(section.decision)) problems.push('decision must be a non-empty string');
  if (!nonEmptyString(section.doc)) problems.push('doc pointer must be a non-empty string');
  if (!isPlainObject(section.measurementWindow) || !nonEmptyString(section.measurementWindow.until)) {
    problems.push('measurementWindow.until must be recorded');
  }

  return problems;
}

// --- docs/token-baseline.json § refuterBatching audit ---------------------

/** The closed decision vocabulary for the batching question. */
export const BATCHING_DECISIONS = ['ship-batched', 'no-ship-worse-fn', 'no-ship-anchoring', 'no-measurement'];

/**
 * Corpus-FREE arithmetic audit of a `refuterBatching` section.
 *
 * Re-derives every rate, asserts the authoritative+judgement partition, checks
 * the token sums and `meanTokensPerGradedFinding`, and — the part specific to
 * this phase — re-derives the corpus-power counts: all three exclusions plus
 * the groupable items must sum to the corpus size, the qualifying population
 * must agree with the size histogram, and it must never count a size-1 group.
 *
 * @param {object} section
 * @returns {string[]} problems, empty when consistent
 */
export function auditBatchingSection(section) {
  const problems = [];
  if (!isPlainObject(section)) return ['refuterBatching section is not an object'];

  if (BATCHING_DECISIONS.indexOf(section.decision) === -1) {
    problems.push(
      `decision must be one of ${BATCHING_DECISIONS.join('|')}, got ${JSON.stringify(section.decision)}`
    );
  }
  if (!nonEmptyString(section.doc)) problems.push('doc pointer must be a non-empty string');
  if (!isPlainObject(section.measurementWindow) || !nonEmptyString(section.measurementWindow.until)) {
    problems.push('measurementWindow.until must be recorded');
  }

  const power = section.corpusPower;
  if (!isPlainObject(power)) {
    problems.push('corpusPower is missing');
  } else {
    const accounted =
      (Number(power.constructedExcluded) || 0) +
      (Number(power.nonGatingExcluded) || 0) +
      (Number(power.unrecoverableUnitExcluded) || 0) +
      (Number(power.groupableItems) || 0);
    if (accounted !== Number(power.totalItems)) {
      problems.push(
        `corpusPower: constructed + nonGating + unrecoverableUnit + groupable (${accounted}) != totalItems ${power.totalItems}`
      );
    }
    const hist = isPlainObject(power.sizeHistogram) ? power.sizeHistogram : {};
    let groupsFromHist = 0;
    let itemsFromHist = 0;
    let qualifyingGroups = 0;
    let qualifyingItems = 0;
    const minGroupSize = Number(power.minGroupSize);
    if (!Number.isInteger(minGroupSize) || minGroupSize < MIN_BATCH_GROUP_SIZE) {
      problems.push(`corpusPower.minGroupSize must be an integer >= ${MIN_BATCH_GROUP_SIZE}, got ${JSON.stringify(power.minGroupSize)}`);
    }
    for (const [sizeKey, count] of Object.entries(hist)) {
      const size = Number(sizeKey);
      const n = Number(count) || 0;
      groupsFromHist += n;
      itemsFromHist += size * n;
      if (size >= minGroupSize) {
        qualifyingGroups += n;
        qualifyingItems += size * n;
      }
      if (size === 1 && size >= minGroupSize) problems.push('corpusPower: a size-1 group may never qualify');
    }
    if (groupsFromHist !== Number(power.groupCount)) {
      problems.push(`corpusPower.groupCount: doc ${power.groupCount}, histogram sums to ${groupsFromHist}`);
    }
    if (itemsFromHist !== Number(power.groupableItems)) {
      problems.push(`corpusPower.groupableItems: doc ${power.groupableItems}, histogram sums to ${itemsFromHist}`);
    }
    if (qualifyingGroups !== Number(power.qualifyingGroups)) {
      problems.push(`corpusPower.qualifyingGroups: doc ${power.qualifyingGroups}, derived ${qualifyingGroups}`);
    }
    if (qualifyingItems !== Number(power.qualifyingItems)) {
      problems.push(`corpusPower.qualifyingItems: doc ${power.qualifyingItems}, derived ${qualifyingItems}`);
    }
    const meets =
      qualifyingGroups >= Number(power.minQualifyingGroups) && qualifyingItems >= Number(power.minQualifyingItems);
    if (power.meetsMinimum !== meets) {
      problems.push(`corpusPower.meetsMinimum: doc ${power.meetsMinimum}, derived ${meets}`);
    }
    if (!meets && section.decision !== 'no-measurement') {
      problems.push(
        `corpusPower says the batched arm is underpowered, but decision is "${section.decision}" — an ` +
          'underpowered arm can only carry decision "no-measurement"'
      );
    }
    // Per-mode histograms must partition the overall one.
    if (isPlainObject(power.sizeHistogramByMode)) {
      const merged = {};
      for (const modeHist of Object.values(power.sizeHistogramByMode)) {
        for (const [size, n] of Object.entries(modeHist || {})) merged[size] = (merged[size] || 0) + Number(n || 0);
      }
      for (const size of new Set([...Object.keys(hist), ...Object.keys(merged)])) {
        if ((Number(hist[size]) || 0) !== (merged[size] || 0)) {
          problems.push(`corpusPower.sizeHistogramByMode does not partition sizeHistogram at size ${size}`);
        }
      }
    }
  }

  const arms = Array.isArray(section.arms) ? section.arms : [];
  if (section.decision !== 'no-measurement' && arms.length < 2) {
    problems.push('a decision other than "no-measurement" requires both arms to be recorded');
  }
  for (const a of arms) {
    if (!nonEmptyString(a.arm)) problems.push('an arm row carries no arm label');
    for (const setName of ['authoritativeOnly', 'judgementCallOnly', 'all']) {
      const s = a[setName];
      if (!isPlainObject(s)) {
        problems.push(`${a.arm}.${setName} is missing`);
        continue;
      }
      const fn = rate(s.falseNegatives || 0, s.defectTrials || 0);
      if (s.falseNegativeRate !== fn) problems.push(`${a.arm}.${setName}.falseNegativeRate: doc ${s.falseNegativeRate}, derived ${fn}`);
      const fp = rate(s.falsePositives || 0, s.nonDefectTrials || 0);
      if (s.falsePositiveRate !== fp) problems.push(`${a.arm}.${setName}.falsePositiveRate: doc ${s.falsePositiveRate}, derived ${fp}`);
      if ((s.defectTrials || 0) + (s.nonDefectTrials || 0) + (s.ungraded || 0) !== (s.trials || 0)) {
        problems.push(`${a.arm}.${setName}: defectTrials + nonDefectTrials + ungraded != trials ${s.trials}`);
      }
    }
    const au = a.authoritativeOnly || {};
    const ju = a.judgementCallOnly || {};
    const all = a.all || {};
    for (const f of ['trials', 'ungraded', 'defectTrials', 'nonDefectTrials', 'falseNegatives', 'falsePositives']) {
      if ((au[f] || 0) + (ju[f] || 0) !== (all[f] || 0)) {
        problems.push(`${a.arm}.all.${f} ${all[f]} != authoritativeOnly ${au[f]} + judgementCallOnly ${ju[f]}`);
      }
    }
    const cost = a.cost || {};
    const summed = TOKEN_CLASSES.reduce((s, cl) => s + (Number(cost[cl]) || 0), 0);
    if (cost.totalTokens !== summed) problems.push(`${a.arm}.cost.totalTokens: doc ${cost.totalTokens}, four classes sum to ${summed}`);
    const perFinding = mean(summed, cost.gradedFindings || 0);
    if (cost.meanTokensPerGradedFinding !== perFinding) {
      problems.push(`${a.arm}.cost.meanTokensPerGradedFinding: doc ${cost.meanTokensPerGradedFinding}, derived ${perFinding}`);
    }
    const perDispatch = mean(summed, cost.dispatches || 0);
    if (cost.meanTokensPerDispatch !== perDispatch) {
      problems.push(`${a.arm}.cost.meanTokensPerDispatch: doc ${cost.meanTokensPerDispatch}, derived ${perDispatch}`);
    }
    const sc = a.selfConsistency || {};
    const fr = rate(sc.replicateFlips || 0, sc.replicatePairs || 0);
    if (sc.flipRate !== fr) problems.push(`${a.arm}.selfConsistency.flipRate: doc ${sc.flipRate}, derived ${fr}`);
  }

  const anchoring = section.anchoring;
  if (isPlainObject(anchoring)) {
    if (Number(anchoring.minGroupSize) < MIN_BATCH_GROUP_SIZE) {
      problems.push(`anchoring.minGroupSize ${anchoring.minGroupSize} is below the ${MIN_BATCH_GROUP_SIZE} floor`);
    }
    for (const a of Array.isArray(anchoring.arms) ? anchoring.arms : []) {
      const derived = rate(a.allSameVerdict || 0, a.dispatchesConsidered || 0);
      if (a.allSameVerdictShare !== derived) {
        problems.push(`anchoring.${a.arm}.allSameVerdictShare: doc ${a.allSameVerdictShare}, derived ${derived}`);
      }
    }
  }

  return problems;
}

/**
 * Recursively assert the report carries no blended-accuracy field. This is the
 * mechanical form of "FN and FP are never averaged together" — a future
 * contributor adding a convenience `accuracy` key trips it.
 *
 * @param {unknown} value
 * @returns {string[]} offending key paths
 */
export function findBlendedAccuracyKeys(value, pathPrefix = '$') {
  const bad = [];
  const RE = /accuracy|overallCorrect|combinedRate/i;
  if (Array.isArray(value)) {
    value.forEach((v, i) => bad.push(...findBlendedAccuracyKeys(v, `${pathPrefix}[${i}]`)));
  } else if (isPlainObject(value)) {
    for (const [k, v] of Object.entries(value)) {
      if (RE.test(k)) bad.push(`${pathPrefix}.${k}`);
      bad.push(...findBlendedAccuracyKeys(v, `${pathPrefix}.${k}`));
    }
  }
  return bad;
}
