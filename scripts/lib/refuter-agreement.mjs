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

  function tierBucket(tier) {
    let b = perTier.get(tier);
    if (!b) {
      b = {
        tier,
        authoritativeOnly: emptyRateSet(),
        judgementCallOnly: emptyRateSet(),
        all: emptyRateSet(),
        byClass: new Map(),
        usage: emptyUsage(),
        gradedTrials: 0,
        replicatePairs: 0,
        replicateFlips: 0,
        flipRate: null,
        historicalOnlyTrials: 0,
      };
      perTier.set(tier, b);
      tierOrder.push(tier);
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
    const b = tierBucket(t.tier);

    // Cost is recorded for EVERY dispatched trial, including historical-only
    // and ungraded ones — the tokens were really spent.
    const u = t.usage || {};
    for (const c of TOKEN_CLASSES) b.usage[c] += Number(u[c] || 0);
    b.usage.toolCalls += Number(t.toolCalls || 0);
    b.gradedTrials += 1;

    if (HISTORICAL_ONLY_SEVERITIES.indexOf(item.finding.severity) !== -1) {
      b.historicalOnlyTrials += 1;
      historicalOnlyTrials += 1;
      continue;
    }

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

    const key = `${t.corpusId}|${t.tier}`;
    let list = replicateVerdicts.get(key);
    if (!list) {
      list = [];
      replicateVerdicts.set(key, list);
    }
    list.push(refuted);
  }

  for (const [key, list] of replicateVerdicts) {
    const tier = key.slice(key.lastIndexOf('|') + 1);
    const graded = list.filter((v) => typeof v === 'boolean');
    if (graded.length < 2) continue;
    const b = perTier.get(tier);
    b.replicatePairs += 1;
    if (!graded.every((v) => v === graded[0])) b.replicateFlips += 1;
  }

  const tiers = tierOrder.map((tier) => {
    const b = perTier.get(tier);
    finalizeRateSet(b.authoritativeOnly);
    finalizeRateSet(b.judgementCallOnly);
    finalizeRateSet(b.all);
    b.flipRate = rate(b.replicateFlips, b.replicatePairs);
    const byClass = [...b.byClass.entries()]
      .sort((a, c) => (a[0] < c[0] ? -1 : a[0] > c[0] ? 1 : 0))
      .map(([className, set]) => ({ class: className, ...finalizeRateSet(set) }));
    const totalTokens = sumTokens(b.usage);
    return {
      tier,
      authoritativeOnly: b.authoritativeOnly,
      judgementCallOnly: b.judgementCallOnly,
      all: b.all,
      byClass,
      selfConsistency: { replicatePairs: b.replicatePairs, replicateFlips: b.replicateFlips, flipRate: b.flipRate },
      historicalOnlyTrials: b.historicalOnlyTrials,
      cost: {
        dispatchedTrials: b.gradedTrials,
        ...Object.fromEntries(TOKEN_CLASSES.map((c) => [c, b.usage[c]])),
        totalTokens,
        meanTokensPerTrial: mean(totalTokens, b.gradedTrials),
        toolCalls: b.usage.toolCalls,
        meanToolCallsPerTrial: mean(b.usage.toolCalls, b.gradedTrials),
      },
    };
  });

  const baselineTier = opts.baselineTier || (tiers.length ? tiers[0].tier : null);
  const baseline = tiers.find((t) => t.tier === baselineTier) || null;
  for (const t of tiers) {
    if (!baseline || t.tier === baselineTier || baseline.cost.meanTokensPerTrial === null || t.cost.meanTokensPerTrial === null) {
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

  return {
    corpus: summarizeCorpus(corpus),
    baselineTier,
    tiers,
    historicalOnlyTrials,
    unknownCorpusIds: [...new Set(unknownIds)].sort(),
    caveats: CAVEATS,
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
          t.tier,
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
          t.tier,
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
    out.push('| ' + [t.tier, t.selfConsistency.replicatePairs, t.selfConsistency.replicateFlips, pctStr(t.selfConsistency.flipRate)].join(' | ') + ' |');
  }
  out.push('A tier with a low FN rate but a high flip rate is not safer — it is lucky.');
  out.push('');

  out.push('## PER-CLASS (the aggregate above is over a deliberately weighted corpus)');
  for (const t of report.tiers) {
    out.push('### ' + t.tier);
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
      out.push('- ' + t.tier + ': baseline (' + num(t.cost.meanTokensPerTrial) + ' mean tokens/trial, ' + num(t.cost.meanToolCallsPerTrial) + ' mean tool calls/trial).');
      continue;
    }
    const d = t.tokenDelta;
    out.push(
      '- ' + t.tier + ': ' + num(d.meanTokensPerTrial) + ' mean tokens/trial vs ' + num(d.baselineMeanTokensPerTrial) +
        ' for ' + d.baselineTier + ' — a delta of ' + num(d.delta) + ' (' + pctStr(d.percent) + ').'
    );
  }
  out.push('Re-tiering changes PRICE-PER-TOKEN, not token VOLUME. These are volume figures only.');
  out.push('');

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
