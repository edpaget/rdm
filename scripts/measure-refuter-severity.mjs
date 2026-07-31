#!/usr/bin/env node
// measure-refuter-severity.mjs — what did the lane spend refuting findings that
// could never have changed an outcome?
//
// WHY THIS EXISTS
// ------------------------------------------------------------------------
// `docs/token-baseline.json` measures the refuter class as ONE bucket: 886
// agents, 19.6 % of all measured lane tokens. That is enough to know refutation
// is expensive and not enough to know which part of it is wasted, because the
// bucket does not know the SEVERITY of the finding each refuter graded — and
// severity is the only thing that decides whether a verdict can change anything
// (see `hasBlocking` in .claude/workflows/lib/review.mjs: the blocker set is
// ['blocking'], widened to ['blocking','concern'] at the `large` tier, so
// `suggestion` gates nothing at any tier).
//
// This script recovers that missing dimension from the corpus that already
// exists. Each refuter's transcript opens with the prompt it was given, and
// `refutePrompt` embeds the WHOLE finding as pretty-printed JSON — severity
// included. Joining that back to the sidecar's per-agent usage yields agent
// count and all four token classes per severity, which is what turns "skip
// non-gating refutation" from an assertion into a measured figure.
//
// WHAT IT DOES NOT CLAIM
// ------------------------------------------------------------------------
// No post-change lane corpus exists — every run on disk executed pre-change
// code. The projected drop is therefore measured over the HISTORICAL corpus
// (the refuters this change would not have spawned, priced at their own real
// recorded usage), not claimed from a fresh run. It is an exact accounting of
// what was actually spent on non-gating refutation, not a prediction about
// future traffic mix.
//
// Usage:
//   node scripts/measure-refuter-severity.mjs [options]
//
// Options:
//   --root <dir>       Session-sidecar root (default: ~/.claude/projects).
//                      Point it at a fixture in a hermetic harness.
//   --until <iso-date> Ignore runs starting after this instant. Pins the
//                      measurement window so a later lane run cannot silently
//                      re-baseline a committed figure.
//   --format text|json Output format (default: text).
//   --check <doc>      Recompute over the corpus and assert the figures match
//                      <doc>'s `nonGatingRefutationSkip` section exactly.
//                      Applies the window recorded in that section unless
//                      --until overrides it.
//   --audit <doc>      Corpus-FREE arithmetic audit of <doc>'s own
//                      `nonGatingRefutationSkip` numbers (rows vs totals,
//                      projected drop vs the non-gating row, percentages).
//                      Reads no sidecars, so it gates the committed figures on
//                      any machine.
//   --help             Print this help and exit.
//
// Determinism: no Date.now(), no Math.random(), no network. Same corpus and
// window in, same numbers out.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  defaultProjectsRoot,
  locateSessionDirs,
  findWorkflowRunFiles,
  buildRecords,
  aggregate,
  transcriptPathFor,
  percentile,
} from './lib/token-report.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(HERE, '..');

// The same six-lane run-set filter docs/token-baseline.json's `runSet` uses, so
// this measurement is over the same corpus the baseline describes.
const LANE_WORKFLOWS = ['autopilot', 'dispatch-phase', 'plan-review', 'backlog', 'estimate', 'document'];

// The severities a finder may emit, in report order. Anything else a transcript
// yields is reported under its own literal key after these, so a new severity
// can never be silently folded into an existing row.
const SEVERITY_ORDER = ['blocking', 'concern', 'suggestion'];

// The severity set this change stops refuting. Kept in sync with
// NON_GATING_SEVERITIES in .claude/workflows/lib/review.mjs by
// scripts/verify-token-report.sh, which greps the lib rather than trusting this
// copy — the two live in different runtimes and cannot import each other.
const NON_GATING_SEVERITIES = ['suggestion'];

// Buckets for refuters whose severity could not be recovered. Reported
// separately and NEVER counted toward the projected drop: an unrecoverable
// severity is unknown, not non-gating.
const NO_TRANSCRIPT = 'unrecoverable:no-transcript';
const UNPARSEABLE = 'unrecoverable:unparseable';

const TOKEN_CLASSES = ['output', 'uncachedInput', 'cacheWrite', 'cacheRead'];

// --- Finding-severity recovery ------------------------------------------

/**
 * Forward brace-match from `start` (which must index a `{`), honouring JSON
 * string literals and their escapes, and return the index of the matching `}`
 * — or -1 if the text ends first.
 *
 * @param {string} text
 * @param {number} start
 */
export function matchBrace(text, start) {
  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let i = start; i < text.length; i++) {
    const ch = text[i];
    if (inString) {
      if (escaped) escaped = false;
      else if (ch === '\\') escaped = true;
      else if (ch === '"') inString = false;
      continue;
    }
    if (ch === '"') inString = true;
    else if (ch === '{') depth++;
    else if (ch === '}') {
      depth--;
      if (depth === 0) return i;
    }
  }
  return -1;
}

/**
 * Locate the finding object embedded in a refuter prompt, without parsing it.
 *
 * `refutePrompt` builds exactly:
 *
 *   You are a READ-ONLY refuter. Do not edit any files.
 *   A prior reviewer raised this <dim> finding against <target>:
 *   <JSON.stringify(finding, null, 2)>
 *   Start from the stance: ...
 *
 * `<target>` is interpolated INLINE on the header line and can itself be a
 * pretty-printed JSON document (the `--implementation-plan` plan-review target
 * is exactly that), so "the first `{`" is the wrong anchor — it finds the
 * target. The finding is instead the brace-matched object that begins at the
 * start of a line and whose closing brace sits immediately before the
 * `Start from the stance:` sentinel. That pins it unambiguously.
 *
 * Exported (rather than folded into `extractFinding`) so `extractRefuterContext`
 * can reuse the same span to read the HEADER text that precedes the finding —
 * the dimension and target this refuter was told about — without re-deriving
 * it from a second scan.
 *
 * @param {string} prompt
 * @returns {{ start: number, end: number }|null} indices of the finding's
 *   opening and closing braces (inclusive), or null if it cannot be located.
 */
export function locateFindingSpan(prompt) {
  if (typeof prompt !== 'string') return null;
  const sentinel = '\nStart from the stance:';
  const sentinelAt = prompt.indexOf(sentinel);
  if (sentinelAt === -1) return null;
  const closeAt = sentinelAt - 1;
  if (prompt[closeAt] !== '}') return null;
  // Candidate openers: a `{` at the very start of the prompt or immediately
  // after a newline. An inline `{` (the target's own) is excluded by
  // construction, and any nested `{` is indented, so this stays a short list.
  for (let i = 0; i <= closeAt; i++) {
    if (prompt[i] !== '{') continue;
    if (i !== 0 && prompt[i - 1] !== '\n') continue;
    if (matchBrace(prompt, i) !== closeAt) continue;
    try {
      const parsed = JSON.parse(prompt.slice(i, closeAt + 1));
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) return { start: i, end: closeAt };
    } catch {
      // Not the finding after all — keep scanning.
    }
  }
  return null;
}

/**
 * Pull the finding object out of a refuter prompt. Thin wrapper over
 * `locateFindingSpan` — kept as its own export so existing callers/tests are
 * unaffected by the span/parse split.
 *
 * @param {string} prompt
 * @returns {object|null} the parsed finding, or null if it cannot be recovered
 */
export function extractFinding(prompt) {
  const span = locateFindingSpan(prompt);
  if (!span) return null;
  try {
    return JSON.parse(prompt.slice(span.start, span.end + 1));
  } catch {
    return null;
  }
}

// --- Review-unit recovery -------------------------------------------------

// The fixed header marker `refutePrompt` always writes, and the separator
// between the dimension key and the target it names. Both are literal
// substrings of the prompt built in .claude/workflows/lib/review.mjs's
// `refutePrompt` — see that function for the exact template.
const HEADER_MARKER = 'A prior reviewer raised this ';
const FINDING_AGAINST = ' finding against ';

// A pathologically long single-line plan document with no early newline
// should not be captured as a fake unit identity — cap the candidate length.
const MAX_UNIT_IDENT_LENGTH = 200;

/**
 * Reject a candidate unit identity that is empty, contains raw JSON structure
 * (a `{` or `"` — the `--implementation-plan` shape, where the target itself
 * is pretty-printed JSON and the "first line" heuristic below lands on its
 * opening brace), or is implausibly long (a single-line plan doc with no
 * early newline).
 *
 * @param {string} s
 * @returns {boolean}
 */
export function isPlausibleUnitIdent(s) {
  if (typeof s !== 'string' || s.length === 0) return false;
  if (s.indexOf('{') !== -1 || s.indexOf('"') !== -1) return false;
  if (s.length > MAX_UNIT_IDENT_LENGTH) return false;
  return true;
}

/**
 * Recover the review-unit identity and dimension a refuter's finding was
 * raised against, from the SAME header line `readRefuterTranscript` already
 * scans for severity — one file read, one pass.
 *
 * The unit identity comes from `context.target`, embedded inline in the
 * header (`refutePrompt` in .claude/workflows/lib/review.mjs):
 *
 *   - **plan mode**: `target` is `'phase ' + roadmap + '/' + stem + '\n\n' +
 *     body` (or the roadmap/implementation-plan equivalents) — MULTI-line, so
 *     its first line (before the `\n\n`) already IS the identity, well
 *     before the trailing `:` that trails the entire body many lines later.
 *   - **code mode**: `target` is a single-line bare `task/<slug>` or
 *     `<roadmap>/<stem>` — the trailing `:` sits on the SAME line, so it is
 *     stripped instead.
 *
 * A target that is itself pretty-printed JSON (the `--implementation-plan`
 * shape) or an implausibly long single-line target is rejected by
 * `isPlausibleUnitIdent` rather than captured as a fake identity.
 *
 * @param {string} prompt
 * @returns {{ dimKey: string|null, unitIdent: string|null }}
 */
export function extractRefuterContext(prompt) {
  const span = locateFindingSpan(prompt);
  if (!span) return { dimKey: null, unitIdent: null };
  let header = prompt.slice(0, span.start);
  if (header.endsWith('\n')) header = header.slice(0, -1);
  const markerAt = header.indexOf(HEADER_MARKER);
  if (markerAt === -1) return { dimKey: null, unitIdent: null };
  const afterMarker = header.slice(markerAt + HEADER_MARKER.length);
  const sepAt = afterMarker.indexOf(FINDING_AGAINST);
  if (sepAt === -1) return { dimKey: null, unitIdent: null };
  const dimKey = afterMarker.slice(0, sepAt) || null;
  const targetPlusColon = afterMarker.slice(sepAt + FINDING_AGAINST.length);
  const newlineAt = targetPlusColon.indexOf('\n');
  const candidate =
    newlineAt !== -1
      ? targetPlusColon.slice(0, newlineAt)
      : targetPlusColon.endsWith(':')
        ? targetPlusColon.slice(0, -1)
        : targetPlusColon;
  return { dimKey, unitIdent: isPlausibleUnitIdent(candidate) ? candidate : null };
}

/**
 * Read one refuter transcript and recover BOTH halves of what it did: the
 * severity of the finding it was handed (from its initiating user turn) and the
 * verdict it returned (from its forced `StructuredOutput` tool call).
 *
 * The verdict half is what settles which severities are worth refuting at all —
 * a severity whose refuters mostly say "refuted" is earning its cost.
 *
 * `dimKey`/`unitIdent` are pulled from the SAME initiating turn's prompt text
 * via `extractRefuterContext` — one read, one scan, no second transcript pass.
 *
 * @param {string} filePath
 * @returns {{ severity: string|null, refuted: boolean|null, dimKey: string|null, unitIdent: string|null }}
 */
export function readRefuterTranscript(filePath) {
  let raw;
  try {
    raw = fs.readFileSync(filePath, 'utf8');
  } catch {
    return { severity: null, refuted: null, dimKey: null, unitIdent: null };
  }
  let severity = null;
  let severitySeen = false;
  let refuted = null;
  let dimKey = null;
  let unitIdent = null;
  for (const line of raw.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    let entry;
    try {
      entry = JSON.parse(trimmed);
    } catch {
      continue;
    }
    if (!severitySeen && entry.type === 'user') {
      const content = entry.message && entry.message.content;
      // The INITIATING turn carries the prompt as a bare string; later user
      // turns are tool_result arrays, which this type check skips.
      if (typeof content !== 'string') continue;
      severitySeen = true;
      const finding = extractFinding(content);
      severity = finding && typeof finding.severity === 'string' ? finding.severity : null;
      const ctx = extractRefuterContext(content);
      dimKey = ctx.dimKey;
      unitIdent = ctx.unitIdent;
      continue;
    }
    if (entry.type === 'assistant') {
      const content = (entry.message && entry.message.content) || [];
      if (!Array.isArray(content)) continue;
      for (const block of content) {
        if (block && block.type === 'tool_use' && block.name === 'StructuredOutput') {
          const input = block.input || {};
          // Last StructuredOutput wins, matching the transcript's own
          // last-write-wins streaming shape.
          if (typeof input.refuted === 'boolean') refuted = input.refuted;
        }
      }
    }
  }
  return { severity, refuted, dimKey, unitIdent };
}

/**
 * Read one finder transcript and recover how many findings it reported, from
 * its own forced `StructuredOutput` call — never inferred from how many
 * refuters were later dispatched against it.
 *
 * `findingsCount` is `null` ONLY when no `StructuredOutput` tool call was ever
 * seen (an unreadable transcript, or an agent that never returned one) — a
 * finder that legitimately reports zero narrative findings (e.g. the `ac`
 * dimension in code mode, which returns only a structured AC table) still
 * yields `0`, not `null`, so it is never conflated with an unreadable
 * transcript.
 *
 * @param {string} filePath
 * @returns {{ findingsCount: number|null }}
 */
export function readFinderTranscript(filePath) {
  let raw;
  try {
    raw = fs.readFileSync(filePath, 'utf8');
  } catch {
    return { findingsCount: null };
  }
  let sawStructuredOutput = false;
  let findingsCount = null;
  for (const line of raw.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    let entry;
    try {
      entry = JSON.parse(trimmed);
    } catch {
      continue;
    }
    if (entry.type !== 'assistant') continue;
    const content = (entry.message && entry.message.content) || [];
    if (!Array.isArray(content)) continue;
    for (const block of content) {
      if (block && block.type === 'tool_use' && block.name === 'StructuredOutput') {
        // Last StructuredOutput wins, matching readRefuterTranscript's own
        // last-write-wins convention.
        sawStructuredOutput = true;
        const input = block.input || {};
        findingsCount = Array.isArray(input.findings) ? input.findings.length : 0;
      }
    }
  }
  return { findingsCount: sawStructuredOutput ? findingsCount : null };
}

/**
 * Summarize a numeric array as n/min/p50/p90/max, reusing `percentile` from
 * `token-report.mjs`. Callers must never call this on an empty array — a
 * finder/unit group with zero eligible values is omitted from its output
 * entirely (mirroring `floorByAgentClass`'s "omit empty classes" convention)
 * rather than reported with a synthetic `n: 0` row.
 *
 * @param {number[]} values - non-empty.
 * @returns {{ n: number, min: number, p50: number, p90: number, max: number }}
 */
export function summarizeCounts(values) {
  const sorted = [...values].sort((a, b) => a - b);
  return {
    n: sorted.length,
    min: sorted[0],
    p50: percentile(sorted, 0.5),
    p90: percentile(sorted, 0.9),
    max: sorted[sorted.length - 1],
  };
}

// --- Measurement ---------------------------------------------------------

/**
 * Locate every in-scope run, build its agent records, and classify each
 * `refute:*` record by the severity of the finding it graded.
 *
 * @param {{ root?: string, until?: string }} [options]
 */
export function measure(options = {}) {
  const projectsRoot = options.root || defaultProjectsRoot();
  const sessionDirs = locateSessionDirs(projectsRoot);
  let runFiles = findWorkflowRunFiles(sessionDirs, { workflowNames: LANE_WORKFLOWS });

  let untilMs;
  if (options.until) {
    untilMs = Date.parse(options.until);
    if (Number.isNaN(untilMs)) throw new Error(`--until value is not a parseable date: "${options.until}"`);
    runFiles = runFiles.filter((rf) => rf.run.startTimeMs !== undefined && rf.run.startTimeMs <= untilMs);
  }

  // buildRecords does not carry sessionDir through, so keep the run→sessionDir
  // map here: it is what locates each agent's transcript.
  const sessionDirOf = new Map();
  for (const rf of runFiles) {
    sessionDirOf.set(`${rf.projectSlug}|${rf.sessionId}|${rf.run.runId}`, rf.sessionDir);
  }

  const records = buildRecords(runFiles);
  const refuters = records.filter((r) => r.agentClass === 'refute');
  const finders = records.filter((r) => r.agentClass === 'find');

  for (const r of refuters) {
    r.refuted = null;
    r.dimKey = null;
    r.unitIdent = null;
    const sessionDir = sessionDirOf.get(`${r.projectSlug}|${r.sessionId}|${r.runId}`);
    if (!sessionDir || !r.agentId) {
      r.severity = NO_TRANSCRIPT;
      continue;
    }
    const transcriptPath = transcriptPathFor(sessionDir, r.runId, r.agentId);
    if (!fs.existsSync(transcriptPath)) {
      r.severity = NO_TRANSCRIPT;
      continue;
    }
    const recovered = readRefuterTranscript(transcriptPath);
    r.severity = recovered.severity === null ? UNPARSEABLE : recovered.severity;
    r.refuted = recovered.refuted;
    // No transcript ⇒ no context either. A severity that could not be
    // recovered means the header line could not be trusted, so neither can
    // its dimension or unit identity.
    if (r.severity !== UNPARSEABLE) {
      r.dimKey = recovered.dimKey;
      r.unitIdent = recovered.unitIdent;
    }
  }

  const bySeverity = withVerdictRates(aggregate(refuters, (r) => r.severity), refuters);
  const laneTotals = aggregate(records, () => 'all')[0] || emptyBucket('all');
  const refuteTotals = aggregate(refuters, () => 'refute')[0] || emptyBucket('refute');

  return {
    corpus: {
      projectsRoot,
      until: options.until || null,
      laneWorkflows: LANE_WORKFLOWS,
      runCount: runFiles.length,
      agentRecordCount: records.length,
    },
    refuteBySeverity: sortRows(bySeverity),
    refuteTotals: stripKey(refuteTotals),
    laneTotals: stripKey(laneTotals),
    projected: projectDrop(bySeverity, refuteTotals, laneTotals),
    refuterFanout: buildRefuterFanout(finders, refuters, sessionDirOf),
  };
}

/**
 * Build the two descriptive distributions this phase adds: findings-per-finder
 * (split by mode + dimension, sourced from each finder's OWN transcript
 * output) and per-review-unit refuter counts (keyed by the unit identity
 * embedded in each refuter's own prompt).
 *
 * NEITHER distribution keys anything on `phaseTitle`/`phaseIndex`. Those are
 * the workflow's own declared pipeline stages, and in a `plan-review` run they
 * are IDENTICAL across every rdm review unit dispatched in that run — measured
 * directly on run `wf_55af7324-87c` (`--roadmap project-agnostic-lane`): 152
 * agents, all 96 refuters sitting at the SAME `phaseIndex 3` across 9 distinct
 * review units. Grouping by that key would silently yield "one unit with 96
 * refuters" instead of nine, inflating exactly the tail a future cap would
 * size against — with no error, just a plausible-looking wrong number.
 * `phaseTitle` IS a valid grouping key elsewhere (autopilot's own nested
 * `▸ dispatch-phase #N` markers, per `autopilot-run-accounting`), but that is
 * a different, narrower case this script does not use. The unit key here
 * comes exclusively from `extractRefuterContext`'s parse of the refuter's own
 * initiating prompt.
 *
 * @param {ReturnType<typeof buildRecords>} finders
 * @param {ReturnType<typeof buildRecords>} refuters - already annotated with
 *   `severity`/`refuted`/`dimKey`/`unitIdent` by the caller's loop.
 * @param {Map<string, string>} sessionDirOf
 */
function buildRefuterFanout(finders, refuters, sessionDirOf) {
  // --- findings-per-finder, split by mode + dimension ---------------------
  let unreadableFinderCount = 0;
  let unresolvedLabelCount = 0;
  const finderGroups = new Map();

  for (const f of finders) {
    const parts = typeof f.label === 'string' ? f.label.split(':') : [];
    if (parts.length !== 3 || parts[0] !== 'find') {
      unresolvedLabelCount += 1;
      continue;
    }
    const [, mode, dim] = parts;

    const sessionDir = sessionDirOf.get(`${f.projectSlug}|${f.sessionId}|${f.runId}`);
    let findingsCount = null;
    if (sessionDir && f.agentId) {
      const transcriptPath = transcriptPathFor(sessionDir, f.runId, f.agentId);
      if (fs.existsSync(transcriptPath)) {
        findingsCount = readFinderTranscript(transcriptPath).findingsCount;
      }
    }
    if (findingsCount === null) {
      unreadableFinderCount += 1;
      continue;
    }

    const key = mode + ':' + dim;
    let group = finderGroups.get(key);
    if (!group) {
      group = { mode, dim, key, values: [] };
      finderGroups.set(key, group);
    }
    group.values.push(findingsCount);
  }

  const findingsPerFinderRows = [...finderGroups.values()]
    .map((g) => ({ mode: g.mode, dim: g.dim, key: g.key, ...summarizeCounts(g.values), refutersDispatched: 0 }))
    .sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
  const rowByKey = new Map(findingsPerFinderRows.map((r) => [r.key, r]));

  // A refuter's dimension is resolved from its OWN prompt (`r.dimKey`), never
  // from its label — `refute:mode:(f.id|dim.key:idx)` displaces the dimension
  // whenever the finder supplied `f.id` (agent-authored free text, the common
  // case). Mode alone is trusted from the label: it is not user-influenced.
  for (const r of refuters) {
    if (!r.dimKey) continue;
    const labelParts = typeof r.label === 'string' ? r.label.split(':') : [];
    const mode = labelParts[1];
    if (!mode) continue;
    const row = rowByKey.get(mode + ':' + r.dimKey);
    // A refuter whose resolved dimension has no matching finder row in this
    // corpus window is counted in refuterCountsByUnit below but intentionally
    // NOT represented in any findingsPerFinder.refutersDispatched figure.
    if (row) row.refutersDispatched += 1;
  }

  // --- per-review-unit refuter counts --------------------------------------
  let recoveredRefuters = 0;
  let unrecoverableRefuterCount = 0;
  const unitCounts = new Map();
  for (const r of refuters) {
    if (r.unitIdent) {
      recoveredRefuters += 1;
      const unitKey = `${r.projectSlug}|${r.sessionId}|${r.runId}|${r.unitIdent}`;
      unitCounts.set(unitKey, (unitCounts.get(unitKey) || 0) + 1);
    } else {
      unrecoverableRefuterCount += 1;
    }
  }
  const totalRefuters = refuters.length;
  const unitCountValues = [...unitCounts.values()];
  const unitSummary =
    unitCountValues.length > 0 ? summarizeCounts(unitCountValues) : { n: 0, min: 0, p50: 0, p90: 0, max: 0 };

  return {
    findingsPerFinder: {
      rows: findingsPerFinderRows,
      unreadableFinderCount,
      unresolvedLabelCount,
    },
    refuterCountsByUnit: {
      ...unitSummary,
      totalRefuters,
      recoveredRefuters,
      unrecoverableRefuterCount,
      recoveryRatePercent: pct(recoveredRefuters, totalRefuters),
    },
  };
}

/**
 * Fold each severity row's VERDICT tally in beside its token columns: how many
 * of that severity's refuters returned a readable verdict (`graded`), how many
 * of those said `refuted: true`, and the resulting rate.
 *
 * This is the half of the measurement that decides which severities are worth
 * refuting: a severity whose refuters mostly overturn the finding is buying
 * something with its tokens; one that gates nothing is not, whatever its rate.
 */
export function withVerdictRates(rows, refuters) {
  const tally = new Map();
  for (const r of refuters) {
    let t = tally.get(r.severity);
    if (!t) {
      t = { graded: 0, refuted: 0 };
      tally.set(r.severity, t);
    }
    if (typeof r.refuted === 'boolean') {
      t.graded += 1;
      if (r.refuted) t.refuted += 1;
    }
  }
  return rows.map((row) => {
    const t = tally.get(row.key) || { graded: 0, refuted: 0 };
    return { ...row, graded: t.graded, refuted: t.refuted, refutedRate: pct(t.refuted, t.graded) };
  });
}

function emptyBucket(key) {
  return { key, agentCount: 0, dedupedRequestCount: 0, output: 0, uncachedInput: 0, cacheWrite: 0, cacheRead: 0 };
}

function stripKey(bucket) {
  const { key, ...rest } = bucket;
  return rest;
}

// Report order: the three real severities first, then any unexpected key, then
// the two explicit unrecoverable buckets last.
function sortRows(rows) {
  const rank = (k) => {
    const i = SEVERITY_ORDER.indexOf(k);
    if (i !== -1) return i;
    if (k === NO_TRANSCRIPT) return 100;
    if (k === UNPARSEABLE) return 101;
    return 50;
  };
  return rows.slice().sort((a, b) => rank(a.key) - rank(b.key) || (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
}

export function allTokens(b) {
  return TOKEN_CLASSES.reduce((s, c) => s + (b[c] || 0), 0);
}

// "Fresh" tokens exclude cache reads — the cheapest token there is, and the one
// that dominates raw totals. Both are reported; neither alone.
export function freshTokens(b) {
  return (b.output || 0) + (b.uncachedInput || 0) + (b.cacheWrite || 0);
}

function pct(part, whole) {
  return whole ? Math.round((part / whole) * 1000) / 10 : 0;
}

/**
 * The drop this change would have produced over the measured corpus: exactly
 * the rows whose severity is in NON_GATING_SEVERITIES. Unrecoverable rows are
 * never included — an unknown severity is not evidence of a non-gating one.
 */
export function projectDrop(bySeverity, refuteTotals, laneTotals) {
  const dropped = bySeverity.filter((r) => NON_GATING_SEVERITIES.indexOf(r.key) !== -1);
  const sum = emptyBucket('dropped');
  for (const r of dropped) {
    sum.agentCount += r.agentCount;
    sum.dedupedRequestCount += r.dedupedRequestCount;
    for (const c of TOKEN_CLASSES) sum[c] += r[c];
  }
  return {
    severities: NON_GATING_SEVERITIES.slice(),
    agentsNotSpawned: sum.agentCount,
    ...Object.fromEntries(TOKEN_CLASSES.map((c) => [c, sum[c]])),
    allTokens: allTokens(sum),
    freshTokens: freshTokens(sum),
    percentOfRefuteAgents: pct(sum.agentCount, refuteTotals.agentCount),
    percentOfRefuteTokens: pct(allTokens(sum), allTokens(refuteTotals)),
    percentOfLaneTokens: pct(allTokens(sum), allTokens(laneTotals)),
  };
}

// --- Rendering -----------------------------------------------------------

function renderText(report) {
  const out = [];
  out.push('Refuter spend by graded finding severity — ' + report.corpus.runCount + ' run(s), ' +
    report.corpus.agentRecordCount + ' agent record(s)');
  out.push('Corpus: ' + report.corpus.projectsRoot + (report.corpus.until ? ' (until ' + report.corpus.until + ')' : ''));
  out.push('');
  out.push('| severity | agents | graded | refuted | refuted rate | output | uncached input | cache write | cache read | all tokens | fresh tokens |');
  out.push('|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|');
  for (const r of report.refuteBySeverity) {
    out.push(
      '| ' +
        [
          r.key,
          r.agentCount,
          r.graded,
          r.refuted,
          r.refutedRate + '%',
          r.output.toLocaleString('en-US'),
          r.uncachedInput.toLocaleString('en-US'),
          r.cacheWrite.toLocaleString('en-US'),
          r.cacheRead.toLocaleString('en-US'),
          allTokens(r).toLocaleString('en-US'),
          freshTokens(r).toLocaleString('en-US'),
        ].join(' | ') +
        ' |'
    );
  }
  const t = report.refuteTotals;
  out.push(
    '| **refute total** | **' + t.agentCount + '** | | | | ' +
      [t.output, t.uncachedInput, t.cacheWrite, t.cacheRead].map((n) => n.toLocaleString('en-US')).join(' | ') +
      ' | **' + allTokens(t).toLocaleString('en-US') + '** | **' + freshTokens(t).toLocaleString('en-US') + '** |'
  );
  out.push('');
  out.push('`graded` counts refuters whose returned verdict was recoverable from the transcript; the');
  out.push('rate is over those, not over every dispatched refuter.');
  out.push('');
  const p = report.projected;
  out.push(
    'Projected drop (severities ' + p.severities.join(', ') + ', measured over the historical corpus): ' +
      p.agentsNotSpawned + ' refuter(s) not spawned — ' + p.percentOfRefuteAgents + '% of all refuters, ' +
      p.percentOfRefuteTokens + '% of refuter tokens, ' + p.percentOfLaneTokens + '% of all lane tokens.'
  );
  out.push(
    '  ' + p.allTokens.toLocaleString('en-US') + ' tokens (' + p.freshTokens.toLocaleString('en-US') +
      ' excluding cache reads).'
  );
  out.push('');
  out.push('No post-change lane corpus exists: every run above executed pre-change code. This is an');
  out.push('exact accounting of what non-gating refutation actually cost, not a forecast.');
  out.push('');

  const fp = report.refuterFanout.findingsPerFinder;
  out.push('Findings per finder, by mode and dimension:');
  out.push('');
  out.push('| mode | dim | n | min | p50 | p90 | max | refuters dispatched |');
  out.push('|---|---|---:|---:|---:|---:|---:|---:|');
  for (const row of fp.rows) {
    out.push(
      '| ' + [row.mode, row.dim, row.n, row.min, row.p50, row.p90, row.max, row.refutersDispatched].join(' | ') + ' |'
    );
  }
  out.push('');
  out.push(
    'Unreadable finder transcripts: ' + fp.unreadableFinderCount + '. Finders with an unresolved label: ' +
      fp.unresolvedLabelCount + '. Neither contributes to any row above.'
  );
  out.push('');

  const u = report.refuterCountsByUnit;
  out.push('Refuters dispatched per review unit (unit identity from each refuter\'s own prompt):');
  out.push('');
  out.push('| units | min | p50 | p90 | max |');
  out.push('|---:|---:|---:|---:|---:|');
  out.push('| ' + [u.n, u.min, u.p50, u.p90, u.max].join(' | ') + ' |');
  out.push('');
  out.push(
    'Unit recovered for ' + u.recoveredRefuters + '/' + u.totalRefuters + ' refuters (' + u.recoveryRatePercent +
      '%); ' + u.unrecoverableRefuterCount + ' unrecoverable (never bucketed into any unit).'
  );
  return out.join('\n');
}

// --- --check / --audit ---------------------------------------------------

function readDoc(docArg) {
  const docPath = path.resolve(REPO_ROOT, docArg);
  const raw = fs.readFileSync(docPath, 'utf8');
  let parsed;
  try {
    parsed = JSON.parse(raw);
  } catch (err) {
    throw new Error(`${docArg} is not parseable JSON (--check/--audit expect docs/token-baseline.json): ${err.message}`);
  }
  const section = parsed.nonGatingRefutationSkip;
  if (!section) throw new Error(`${docArg} has no "nonGatingRefutationSkip" section`);
  const fanoutSection = parsed.refuterFanout;
  if (!fanoutSection) throw new Error(`${docArg} has no "refuterFanout" section`);
  return { docPath, section, fanoutSection };
}

/** Compare a computed report against the doc's recorded figures. */
export function checkDoc(report, section) {
  const missing = [];
  const expectRows = Array.isArray(section.refuteBySeverity) ? section.refuteBySeverity : [];
  const gotByKey = new Map(report.refuteBySeverity.map((r) => [r.key, r]));
  if (expectRows.length !== report.refuteBySeverity.length) {
    missing.push(
      `row count: doc has ${expectRows.length}, corpus yields ${report.refuteBySeverity.length}`
    );
  }
  for (const row of expectRows) {
    const got = gotByKey.get(row.key);
    if (!got) {
      missing.push(`row "${row.key}" is in the doc but not in the corpus`);
      continue;
    }
    for (const field of ['agentCount', 'graded', 'refuted', 'refutedRate', ...TOKEN_CLASSES]) {
      if (row[field] !== got[field]) {
        missing.push(`${row.key}.${field}: doc ${row[field]} vs corpus ${got[field]}`);
      }
    }
  }
  const p = section.projected || {};
  for (const field of ['agentsNotSpawned', 'allTokens', 'freshTokens', 'percentOfRefuteAgents', 'percentOfRefuteTokens', 'percentOfLaneTokens']) {
    if (p[field] !== report.projected[field]) {
      missing.push(`projected.${field}: doc ${p[field]} vs corpus ${report.projected[field]}`);
    }
  }
  return missing;
}

/**
 * Corpus-free audit: are the doc's own numbers internally consistent? Catches a
 * hand-edited or stale figure on any machine, without reading a single sidecar.
 */
export function auditDoc(section) {
  const problems = [];
  const rows = Array.isArray(section.refuteBySeverity) ? section.refuteBySeverity : [];
  if (rows.length === 0) problems.push('refuteBySeverity is empty');
  const totals = section.refuteTotals || {};
  const sum = emptyBucket('sum');
  for (const r of rows) {
    sum.agentCount += r.agentCount || 0;
    for (const c of TOKEN_CLASSES) sum[c] += r[c] || 0;
    // A row cannot have graded more refuters than it dispatched, nor refuted
    // more than it graded, and its rate must be the quotient of its own tally.
    if ((r.graded || 0) > (r.agentCount || 0)) {
      problems.push(`${r.key}: graded ${r.graded} exceeds agentCount ${r.agentCount}`);
    }
    if ((r.refuted || 0) > (r.graded || 0)) {
      problems.push(`${r.key}: refuted ${r.refuted} exceeds graded ${r.graded}`);
    }
    const rate = pct(r.refuted || 0, r.graded || 0);
    if (r.refutedRate !== rate) {
      problems.push(`${r.key}.refutedRate: doc ${r.refutedRate}, derived ${rate}`);
    }
  }
  if (sum.agentCount !== totals.agentCount) {
    problems.push(`refuteBySeverity agent counts sum to ${sum.agentCount}, refuteTotals says ${totals.agentCount}`);
  }
  for (const c of TOKEN_CLASSES) {
    if (sum[c] !== totals[c]) problems.push(`refuteBySeverity ${c} sums to ${sum[c]}, refuteTotals says ${totals[c]}`);
  }
  const p = section.projected || {};
  const expected = projectDrop(rows, totals, section.laneTotals || emptyBucket('lane'));
  for (const field of ['agentsNotSpawned', ...TOKEN_CLASSES, 'allTokens', 'freshTokens', 'percentOfRefuteAgents', 'percentOfRefuteTokens', 'percentOfLaneTokens']) {
    if (p[field] !== expected[field]) {
      problems.push(`projected.${field}: doc ${p[field]}, derived from the doc's own rows ${expected[field]}`);
    }
  }
  return problems;
}

/**
 * Compare a computed report's `refuterFanout` against the doc's recorded
 * figures — the same shape of comparison `checkDoc` runs for
 * `nonGatingRefutationSkip`, over the two new distributions instead.
 */
export function checkFanoutDoc(report, fanoutSection) {
  const missing = [];
  const got = report.refuterFanout;
  const expectFP = (fanoutSection && fanoutSection.findingsPerFinder) || {};
  const expectRows = Array.isArray(expectFP.rows) ? expectFP.rows : [];
  const gotRowByKey = new Map(got.findingsPerFinder.rows.map((r) => [r.key, r]));
  if (expectRows.length !== got.findingsPerFinder.rows.length) {
    missing.push(
      `findingsPerFinder row count: doc has ${expectRows.length}, corpus yields ${got.findingsPerFinder.rows.length}`
    );
  }
  for (const row of expectRows) {
    const gotRow = gotRowByKey.get(row.key);
    if (!gotRow) {
      missing.push(`findingsPerFinder row "${row.key}" is in the doc but not in the corpus`);
      continue;
    }
    for (const field of ['n', 'min', 'p50', 'p90', 'max', 'refutersDispatched']) {
      if (row[field] !== gotRow[field]) {
        missing.push(`findingsPerFinder.${row.key}.${field}: doc ${row[field]} vs corpus ${gotRow[field]}`);
      }
    }
  }
  for (const field of ['unreadableFinderCount', 'unresolvedLabelCount']) {
    if (expectFP[field] !== got.findingsPerFinder[field]) {
      missing.push(`findingsPerFinder.${field}: doc ${expectFP[field]} vs corpus ${got.findingsPerFinder[field]}`);
    }
  }

  const expectUnit = (fanoutSection && fanoutSection.refuterCountsByUnit) || {};
  const gotUnit = got.refuterCountsByUnit;
  for (const field of [
    'n',
    'min',
    'p50',
    'p90',
    'max',
    'totalRefuters',
    'recoveredRefuters',
    'unrecoverableRefuterCount',
    'recoveryRatePercent',
  ]) {
    if (expectUnit[field] !== gotUnit[field]) {
      missing.push(`refuterCountsByUnit.${field}: doc ${expectUnit[field]} vs corpus ${gotUnit[field]}`);
    }
  }
  return missing;
}

/**
 * Corpus-free audit of a doc's `refuterFanout` section: are its own numbers
 * internally consistent? Mirrors `auditDoc`'s arithmetic-only checks.
 */
export function auditFanoutDoc(fanoutSection) {
  const problems = [];
  const fp = (fanoutSection && fanoutSection.findingsPerFinder) || {};
  const rows = Array.isArray(fp.rows) ? fp.rows : [];
  for (const r of rows) {
    if (!(r.min <= r.p50 && r.p50 <= r.p90 && r.p90 <= r.max)) {
      problems.push(`findingsPerFinder.${r.key}: min/p50/p90/max not monotonic (${r.min}/${r.p50}/${r.p90}/${r.max})`);
    }
  }

  const u = (fanoutSection && fanoutSection.refuterCountsByUnit) || {};
  if ((u.n || 0) > 0 && !(u.min <= u.p50 && u.p50 <= u.p90 && u.p90 <= u.max)) {
    problems.push(`refuterCountsByUnit: min/p50/p90/max not monotonic (${u.min}/${u.p50}/${u.p90}/${u.max})`);
  }
  const recSum = (u.recoveredRefuters || 0) + (u.unrecoverableRefuterCount || 0);
  if (recSum !== u.totalRefuters) {
    problems.push(
      `refuterCountsByUnit: recoveredRefuters + unrecoverableRefuterCount (${recSum}) !== totalRefuters (${u.totalRefuters})`
    );
  }
  const derivedRate = pct(u.recoveredRefuters || 0, u.totalRefuters || 0);
  if (u.recoveryRatePercent !== derivedRate) {
    problems.push(`refuterCountsByUnit.recoveryRatePercent: doc ${u.recoveryRatePercent}, derived ${derivedRate}`);
  }
  return problems;
}

// --- CLI ------------------------------------------------------------------

export function parseArgs(argv) {
  const args = { root: undefined, until: undefined, format: 'text', check: null, audit: null, help: false };
  let i;
  function takeValue(flag) {
    const next = argv[i + 1];
    if (next === undefined || next.startsWith('--')) throw new Error(`${flag} requires a value`);
    i += 1;
    return next;
  }
  for (i = 0; i < argv.length; i++) {
    switch (argv[i]) {
      case '--root':
        args.root = takeValue('--root');
        break;
      case '--until':
        args.until = takeValue('--until');
        break;
      case '--format':
        args.format = takeValue('--format');
        break;
      case '--check':
        args.check = takeValue('--check');
        break;
      case '--audit':
        args.audit = takeValue('--audit');
        break;
      case '--help':
      case '-h':
        args.help = true;
        break;
      default:
        throw new Error(`unrecognized argument: "${argv[i]}"`);
    }
  }
  if (args.format !== 'text' && args.format !== 'json') {
    throw new Error(`--format must be "text" or "json", got "${args.format}"`);
  }
  return args;
}

const HELP = `Usage: node scripts/measure-refuter-severity.mjs [options]

Options:
  --root <dir>       Session-sidecar root (default: ~/.claude/projects).
  --until <iso-date> Ignore runs starting after this instant.
  --format text|json Output format (default: text).
  --check <doc>      Recompute over the corpus and assert <doc>'s
                     nonGatingRefutationSkip figures match exactly.
  --audit <doc>      Corpus-free arithmetic audit of <doc>'s own figures.
  --help             Print this help and exit.
`;

// `import.meta.main` is not available on the pinned node, so gate on argv[1].
const invokedDirectly = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invokedDirectly) {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    console.log(HELP);
    process.exit(0);
  }

  if (args.audit) {
    const { section, fanoutSection } = readDoc(args.audit);
    const problems = auditDoc(section).concat(auditFanoutDoc(fanoutSection));
    if (problems.length) {
      console.error('measure-refuter-severity --audit FAILED against ' + args.audit + ':');
      for (const m of problems) console.error('  ' + m);
      process.exit(1);
    }
    console.log('measure-refuter-severity --audit OK: ' + args.audit + "'s figures are internally consistent");
    process.exit(0);
  }

  if (args.check) {
    const { section, fanoutSection } = readDoc(args.check);
    // The doc's own recorded window is what makes a committed figure stable as
    // the corpus grows; --until overrides it for an ad hoc re-measurement.
    const until = args.until || (section.measurementWindow && section.measurementWindow.until) || undefined;
    const report = measure({ root: args.root, until });
    const missing = checkDoc(report, section).concat(checkFanoutDoc(report, fanoutSection));
    if (missing.length) {
      console.error('measure-refuter-severity --check FAILED against ' + args.check + ':');
      for (const m of missing) console.error('  ' + m);
      console.error('\nRe-run `node scripts/measure-refuter-severity.mjs --format json` and update the doc.');
      process.exit(1);
    }
    console.log('measure-refuter-severity --check OK: ' + args.check + ' matches the measured corpus');
    process.exit(0);
  }

  const report = measure({ root: args.root, until: args.until });
  if (args.format === 'json') {
    console.log(JSON.stringify({ instrument: 'scripts/measure-refuter-severity.mjs', ...report }, null, 2));
  } else {
    console.log(renderText(report));
  }
}
