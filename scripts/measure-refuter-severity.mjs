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
//                      <doc>'s `nonGatingRefutationSkip`, `refuterFanout` and
//                      `determiningFindingRank` sections exactly. Applies the
//                      window recorded in the first section unless --until
//                      overrides it.
//   --audit <doc>      Corpus-FREE arithmetic audit of <doc>'s own
//                      `nonGatingRefutationSkip` / `refuterFanout` /
//                      `determiningFindingRank` numbers (rows vs totals,
//                      projected drop vs the non-gating row, percentages, the
//                      unit-status partition, and the re-derived cap verdict).
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
// THE RANKING AND GATING RULE IS IMPORTED, NEVER REIMPLEMENTED.
//
// This phase's whole question is "where in the ranking does the finding that
// determined the outcome sit?", and its answer is only worth anything if the
// ranking it replays is the SAME ranking the live pipeline applies. A copy of
// `rankFindings`/`survives`/`hasBlocking` — even a faithful one — could drift
// from `.claude/workflows/lib/review.mjs` silently, and the measurement would
// then be predicting the behavior of code that no longer exists. So the three
// decision functions, the dimension table and the non-gating severity set are
// all taken from the canonical review source, read-only. Nothing under
// `.claude/workflows/` is modified by this instrument; it only imports.
//
// This import is safe from Node: review.mjs's top level is pure consts and
// function declarations plus a Node-only `export {}` block, and the Workflow
// globals (`agent()`/`pipeline()`/`parallel()`) are only touched INSIDE
// `buildReviewPipeline`'s body, which this file never calls.
import {
  survives,
  rankFindings,
  hasBlocking,
  acTableHasGap,
  DIMENSIONS,
  NON_GATING_SEVERITIES as LIB_NON_GATING_SEVERITIES,
} from '../.claude/workflows/lib/review.mjs';

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

// ...and now that the canonical source is importable from here, pin the copy to
// it at module load as well, so the duplicate can never drift even between
// harness runs. (The grep-based pin in scripts/verify-token-report.sh stays: it
// catches the drift without executing anything.)
if (JSON.stringify(NON_GATING_SEVERITIES) !== JSON.stringify(LIB_NON_GATING_SEVERITIES)) {
  throw new Error(
    'NON_GATING_SEVERITIES drifted from .claude/workflows/lib/review.mjs: ' +
      JSON.stringify(NON_GATING_SEVERITIES) +
      ' vs ' +
      JSON.stringify(LIB_NON_GATING_SEVERITIES)
  );
}

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

// The fixed markers `findPrompt` always writes, and the two literal `diffHint`
// lines that terminate the target it interpolates. All three are verbatim
// substrings of the prompt built in .claude/workflows/lib/review.mjs's
// `findPrompt` — see that function for the exact template.
const FIND_TARGET_MARKER = 'Review target: ';
const FIND_DIFF_HINTS = [
  '\nInspect the implementation diff (use git log / git diff in the worktree).',
  '\nInspect the plan document text.',
];
const FIND_DIMENSION_MARKER = 'Your single dimension is ';
const FIND_DIMENSION_KEY = /^[^\n]*?\(([^()\n]+)\)/;

// The Workflow runtime suffixes a retried dispatch's label with " (retry N)".
// That suffix names the ATTEMPT, not the dimension (see buildRefuterFanout).
const RETRY_LABEL_SUFFIX = / \(retry \d+\)$/;

/**
 * Recover the review-unit identity and dimension a FINDER was pointed at, from
 * its own initiating prompt.
 *
 * This is a SECOND READER OF THE SAME KEY `extractRefuterContext` reads, not a
 * new key. `findPrompt` writes `'Review target: ' + context.target + '.'` from
 * the identical `context.target` that `refutePrompt` interpolates into its
 * `... finding against <target>:` header, so both recover the same identity for
 * the same unit — which is exactly what lets a finder's findings and a
 * refuter's verdict be joined into one review unit.
 *
 * Nothing here (and nothing anywhere in the determining-rank measurement) reads
 * `phaseTitle`/`phaseIndex`. Those are the workflow's own pipeline stages and
 * are identical across every review unit of a plan-review run — see
 * `buildRefuterFanout` for the measured evidence.
 *
 * The trailing-punctuation asymmetry mirrors `extractRefuterContext` exactly:
 *
 *   - **plan mode**: `target` is MULTI-line (`'phase <roadmap>/<stem>\n\n<body>'`),
 *     so its first line already IS the identity and the `.` `findPrompt`
 *     appends sits many lines later, on the body's last line. Never strip it.
 *   - **code mode**: `target` is a single-line bare `task/<slug>` or
 *     `<roadmap>/<stem>`, so the appended `.` sits on the SAME line and is
 *     stripped instead.
 *
 * The target's extent is bounded by the `diffHint` line `findPrompt` always
 * writes immediately after it — the finder-side equivalent of the finding-JSON
 * span that bounds the refuter-side header.
 *
 * @param {string} prompt
 * @returns {{ dimKey: string|null, unitIdent: string|null }}
 */
export function extractFinderContext(prompt) {
  if (typeof prompt !== 'string') return { dimKey: null, unitIdent: null };
  const markerAt = prompt.indexOf(FIND_TARGET_MARKER);
  if (markerAt === -1) return { dimKey: null, unitIdent: null };
  const rest = prompt.slice(markerAt + FIND_TARGET_MARKER.length);
  let end = -1;
  for (const hint of FIND_DIFF_HINTS) {
    const at = rest.indexOf(hint);
    if (at !== -1 && (end === -1 || at < end)) end = at;
  }
  if (end === -1) return { dimKey: null, unitIdent: null };
  const targetPlusDot = rest.slice(0, end);
  const newlineAt = targetPlusDot.indexOf('\n');
  const candidate =
    newlineAt !== -1
      ? targetPlusDot.slice(0, newlineAt)
      : targetPlusDot.endsWith('.')
        ? targetPlusDot.slice(0, -1)
        : targetPlusDot;

  // The dimension key is the parenthesised `dim.key` on the "Your single
  // dimension is <title> (<key>)." line. Read from the prompt rather than the
  // label so both sides of the finder↔refuter join use a prompt-derived
  // dimension (a refuter's label displaces its dimension whenever the finder
  // supplied `f.id`), and so a retry-suffixed label can never leak in.
  let dimKey = null;
  const dimAt = prompt.indexOf(FIND_DIMENSION_MARKER, markerAt);
  if (dimAt !== -1) {
    const m = FIND_DIMENSION_KEY.exec(prompt.slice(dimAt + FIND_DIMENSION_MARKER.length));
    if (m) dimKey = m[1];
  }

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
 * `finding` is the WHOLE parsed finding object (not just its severity) — the
 * determining-rank measurement joins each refuter back to the candidate finding
 * it graded, and needs the finding's `id` (or, when that is missing or
 * duplicated, its full structure) to do so.
 *
 * @param {string} filePath
 * @returns {{ severity: string|null, refuted: boolean|null, dimKey: string|null, unitIdent: string|null, finding: object|null }}
 */
export function readRefuterTranscript(filePath) {
  let raw;
  try {
    raw = fs.readFileSync(filePath, 'utf8');
  } catch {
    return { severity: null, refuted: null, dimKey: null, unitIdent: null, finding: null };
  }
  let severity = null;
  let severitySeen = false;
  let refuted = null;
  let dimKey = null;
  let unitIdent = null;
  let finding = null;
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
      finding = extractFinding(content);
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
  return { severity, refuted, dimKey, unitIdent, finding };
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
 * The determining-rank measurement needs three more things from the same single
 * read, so they ride alongside without disturbing `findingsCount`'s semantics
 * (phase 1's `refuterFanout` figures depend on those being unchanged):
 *
 *   - `findings` — the WHOLE array, not just its length: it is this unit's
 *     candidate list, the thing a refutation budget would truncate.
 *   - `acTable`  — the `ac` dimension's structured code-mode table, for the
 *     `acTableHasGap` side-channel diagnostic.
 *   - `dimKey`/`unitIdent` — from the initiating prompt, via
 *     `extractFinderContext`.
 *
 * `findings` is `null` under exactly the same condition as `findingsCount`
 * (no StructuredOutput ever seen), so "unknown findings" and "zero findings"
 * stay distinguishable on both.
 *
 * @param {string} filePath
 * @returns {{ findingsCount: number|null, findings: object[]|null, acTable: object[]|null, dimKey: string|null, unitIdent: string|null }}
 */
export function readFinderTranscript(filePath) {
  let raw;
  try {
    raw = fs.readFileSync(filePath, 'utf8');
  } catch {
    return { findingsCount: null, findings: null, acTable: null, dimKey: null, unitIdent: null };
  }
  let sawStructuredOutput = false;
  let findingsCount = null;
  let findings = null;
  let acTable = null;
  let promptSeen = false;
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
    if (!promptSeen && entry.type === 'user') {
      const content = entry.message && entry.message.content;
      // The INITIATING turn carries the prompt as a bare string; later user
      // turns are tool_result arrays, which this type check skips.
      if (typeof content !== 'string') continue;
      promptSeen = true;
      const ctx = extractFinderContext(content);
      dimKey = ctx.dimKey;
      unitIdent = ctx.unitIdent;
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
        findings = Array.isArray(input.findings) ? input.findings : [];
        findingsCount = findings.length;
        acTable = Array.isArray(input.ac) ? input.ac : null;
      }
    }
  }
  return {
    findingsCount: sawStructuredOutput ? findingsCount : null,
    findings: sawStructuredOutput ? findings : null,
    acTable,
    dimKey,
    unitIdent,
  };
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
    r.finding = null;
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
      r.finding = recovered.finding;
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
    // Built from the SAME `records`/`sessionDirOf` — and therefore the same
    // already-filtered `runFiles` — so `--until` applies identically here and
    // a committed rank figure cannot silently re-baseline while the phase-1
    // figures stay pinned.
    determiningFindingRank: buildDeterminingFindingRank(finders, refuters, sessionDirOf),
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
    const [, mode, rawDim] = parts;
    // The Workflow runtime suffixes a retried dispatch's label with
    // " (retry N)" (e.g. `find:code:ac (retry 1)`). That suffix names the
    // ATTEMPT, not the dimension, so it must be stripped before grouping —
    // otherwise a single logical dimension fragments into one row per retry
    // count, each with its own n/min/p50/p90/max, and `refutersDispatched`
    // permanently reads 0 on every such row because a refuter's own
    // `dimKey` (parsed from ITS prompt) is never retry-suffixed and so can
    // never match a retry-suffixed key.
    const dim = rawDim.replace(/ \(retry \d+\)$/, '');

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

// --- Determining-finding rank ---------------------------------------------
//
// THE QUESTION: where in a ranked list does the finding that actually
// determined the outcome sit? A refutation budget that grades only the top N
// candidates is free iff that finding is almost always near the top, and sheds
// real signal iff the rank is spread out.
//
// RANKED OVER THE CANDIDATE LIST, NOT THE SURVIVOR LIST. Ranking among
// SURVIVORS is degenerate: `SEVERITY_RANK` orders blocking(0) < concern(1) <
// suggestion(2), so the top-ranked survivor IS by construction the one that
// makes `hasBlocking` true, and the answer would be a constant 1 for every
// determining unit — a tautology, not a measurement. A refutation budget
// truncates the CANDIDATE list (what the finders emitted) before any of it is
// graded, so the candidate list is the ranking a cap would actually apply, and
// it is the one measured here.

// The closed reason vocabulary for a unit whose rank cannot be reconstructed,
// in fixed report order. Every reason is decidable from the unit's OWN records.
// There is deliberately NO run-wide reason (e.g. "an orphan agent shared this
// unit's run") — see `buildUnitCandidates` for why.
const RANK_UNRECOVERABLE_REASONS = [
  'unknown-disposition-above-determining',
  'unreadable-finder-transcript',
  'dimension-coverage-gap',
  'multi-round-unit',
  'ambiguous-finding-join',
];

// The dimensions whose ABSENCE from a unit's own finder set is positive local
// evidence that its candidate list is incomplete.
//
// This is deliberately a CORPUS-STABLE SUBSET of review.mjs's always-on
// dimensions, not the whole set: `restraint` is always-on in `plan` mode TODAY
// but was added part-way through the measured window (§ refuterFanout:
// plan:restraint n=26 against plan:coherence n=118), so requiring it would
// mark every pre-restraint plan unit incomplete for a reason that is an artefact
// of when the dimension shipped rather than of the corpus. The four listed here
// have been always-on for the whole window. `assertCoverageDimensions` below
// pins them to review.mjs so this can only ever be a subset of the real
// always-on set, never a divergent list.
const COVERAGE_REQUIRED_DIMENSIONS = {
  code: ['ac', 'correctness'],
  plan: ['coherence', 'architectural-fit'],
};

function assertCoverageDimensions() {
  for (const [mode, keys] of Object.entries(COVERAGE_REQUIRED_DIMENSIONS)) {
    const alwaysOn = (DIMENSIONS[mode] || []).filter((d) => !d.when).map((d) => d.key);
    for (const k of keys) {
      if (alwaysOn.indexOf(k) === -1) {
        throw new Error(
          `COVERAGE_REQUIRED_DIMENSIONS.${mode} names "${k}", which is not an always-on dimension in ` +
            '.claude/workflows/lib/review.mjs — the coverage check must stay a subset of the real always-on set'
        );
      }
    }
  }
}
assertCoverageDimensions();

/** Stable structural key for a finding, used when an `id` join is unusable. */
function structuralKey(value) {
  if (value === null || typeof value !== 'object') return JSON.stringify(value === undefined ? null : value);
  if (Array.isArray(value)) return '[' + value.map(structuralKey).join(',') + ']';
  const keys = Object.keys(value).sort();
  return '{' + keys.map((k) => JSON.stringify(k) + ':' + structuralKey(value[k])).join(',') + '}';
}

function runKeyOf(r) {
  return `${r.projectSlug}|${r.sessionId}|${r.runId}`;
}

/**
 * Group every finder and refuter into review units keyed by phase 1's
 * prompt-derived key (`projectSlug|sessionId|runId|unitIdent`), assemble each
 * unit's candidate finding list from its finders' own StructuredOutput output,
 * attach each candidate's refuter disposition, and record any TIER-INDEPENDENT
 * structural reason the unit cannot be reconstructed.
 *
 * ORPHAN CONTAMINATION IS PER UNIT, NEVER RUN-WIDE. An agent whose `unitIdent`
 * does not resolve — chiefly the `--implementation-plan` target, which is itself
 * pretty-printed JSON and is rejected by `isPlausibleUnitIdent` by construction
 * — cannot be attributed to ANY unit, so it marks no unit unrecoverable. It is
 * counted in the `orphanAgents` diagnostic and reported. This is exactly phase
 * 1's own precedent (an unresolved refuter goes to `unrecoverableRefuterCount`
 * and is never bucketed into a unit, while the units that DID resolve in that
 * run stay valid). A run-wide rule would not be conservative but destructive:
 * real runs mix many named units with an occasional orphan (a 9-unit run is
 * cited in `buildRefuterFanout`), so it could zero out the recoverable share on
 * one bad target and manufacture a verdict out of a corpus artefact. The
 * missing-candidate hazard such a rule reaches for is caught locally instead, by
 * `dimension-coverage-gap`.
 *
 * @param {ReturnType<typeof buildRecords>} finders
 * @param {ReturnType<typeof buildRecords>} refuters - already annotated with
 *   `dimKey`/`unitIdent`/`refuted`/`finding` by measure()'s loop.
 * @param {Map<string, string>} sessionDirOf
 */
export function buildUnitCandidates(finders, refuters, sessionDirOf) {
  const units = new Map();
  const orphan = { finders: 0, refuters: 0, runs: new Set() };

  function unitFor(rec, unitIdent, mode) {
    const key = `${runKeyOf(rec)}|${unitIdent}`;
    let u = units.get(key);
    if (!u) {
      u = { key, mode: mode || null, finders: [], refuters: [] };
      units.set(key, u);
    }
    if (!u.mode && mode) u.mode = mode;
    return u;
  }

  for (const f of finders) {
    const parts = typeof f.label === 'string' ? f.label.split(':') : [];
    const mode = parts.length === 3 && parts[0] === 'find' ? parts[1] : null;
    const labelDim = parts.length === 3 ? parts[2].replace(RETRY_LABEL_SUFFIX, '') : null;
    const isRetry = parts.length === 3 && RETRY_LABEL_SUFFIX.test(parts[2]);

    const sessionDir = sessionDirOf.get(runKeyOf(f));
    let t = null;
    if (sessionDir && f.agentId) {
      const transcriptPath = transcriptPathFor(sessionDir, f.runId, f.agentId);
      if (fs.existsSync(transcriptPath)) t = readFinderTranscript(transcriptPath);
    }
    // No transcript at all, or a transcript whose prompt does not yield a
    // plausible unit identity: not attributable to any unit, so it invalidates
    // none. Counted and reported, never imputed.
    if (!t || !t.unitIdent) {
      orphan.finders += 1;
      orphan.runs.add(runKeyOf(f));
      continue;
    }
    const u = unitFor(f, t.unitIdent, mode);
    u.finders.push({
      dim: t.dimKey || labelDim,
      isRetry,
      findings: t.findings,
      acTable: t.acTable,
    });
  }

  for (const r of refuters) {
    if (!r.unitIdent) {
      orphan.refuters += 1;
      orphan.runs.add(runKeyOf(r));
      continue;
    }
    const parts = typeof r.label === 'string' ? r.label.split(':') : [];
    const mode = parts.length >= 2 && parts[0] === 'refute' ? parts[1] : null;
    const u = unitFor(r, r.unitIdent, mode);
    u.refuters.push({ dim: r.dimKey, finding: r.finding, refuted: r.refuted });
  }

  const prepared = [...units.values()].sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
  for (const u of prepared) prepareUnit(u);
  return { units: prepared, orphan };
}

/**
 * Resolve one unit's candidate list, dispositions and structural
 * recoverability. Everything decided here is TIER-INDEPENDENT, so both the
 * default-blocker-set and `largeTier` walks reuse it.
 *
 * Exported so the harness can drive the closed reason vocabulary and its
 * PRECEDENCE directly, without having to seed a whole sidecar run per reason:
 * three of the five reasons (`multi-round-unit`, `ambiguous-finding-join`,
 * `unreadable-finder-transcript`) are corpus-rare, and the order they resolve
 * in is load-bearing — see the precedence comment at the foot of this function.
 *
 * @param {{ mode: string|null, finders: Array<{dim: string|null, isRetry: boolean, findings: object[]|null, acTable?: unknown}>, refuters: Array<{dim: string|null, finding: object|null, refuted: boolean|null}> }} u
 */
export function prepareUnit(u) {
  // Retry supersession: a ` (retry N)` dispatch replaces the previous attempt at
  // that dimension rather than adding a second candidate set. Two NON-retry
  // finders at the same dimension are a different thing entirely — a second
  // review round over the same target (`codeReviewRounds`), which collapses into
  // one `unitIdent` and would silently inflate the candidate list. That unit is
  // marked unrecoverable rather than guessed at.
  const byDim = new Map();
  for (const f of u.finders) {
    const k = f.dim || '';
    if (!byDim.has(k)) byDim.set(k, []);
    byDim.get(k).push(f);
  }
  let multiRound = false;
  const selected = [];
  for (const [, group] of byDim) {
    if (group.filter((f) => !f.isRetry).length > 1) multiRound = true;
    selected.push(group[group.length - 1]);
  }

  u.candidates = [];
  let unreadable = false;
  for (const f of selected) {
    // `findings === null` means no StructuredOutput was ever seen. A findings
    // array carrying a non-object entry is agent-authored garbage that
    // `rankFindings` would throw on; treat it the same way — the finder's
    // output could not be read, so this unit's candidate list is incomplete —
    // rather than crashing the whole measurement on one bad transcript.
    if (f.findings === null || f.findings.some((x) => !x || typeof x !== 'object' || Array.isArray(x))) {
      unreadable = true;
      continue;
    }
    for (const finding of f.findings) {
      u.candidates.push({ finding, dim: f.dim, verdict: null, disposition: 'unknown' });
    }
  }

  const ambiguous = attachDispositions(u);

  // dimension-coverage-gap: the unit's OWN evidence shows its candidate list is
  // incomplete — either its finder set does not cover the always-on dimensions
  // for its mode, or it has a refuter for a dimension it has no finder for.
  const finderDims = new Set(selected.map((f) => f.dim));
  let coverageGap = (COVERAGE_REQUIRED_DIMENSIONS[u.mode] || []).some((d) => !finderDims.has(d));
  if (!coverageGap) {
    for (const r of u.refuters) {
      if (r.dim && !finderDims.has(r.dim)) {
        coverageGap = true;
        break;
      }
    }
  }

  u.acTableGap = u.finders.some((f) => acTableHasGap(f.acTable));

  // Precedence: the reasons that invalidate the candidate list itself come
  // first, because a walk over a wrong list can produce a plausible wrong rank.
  u.structuralReason = multiRound
    ? 'multi-round-unit'
    : ambiguous
      ? 'ambiguous-finding-join'
      : unreadable
        ? 'unreadable-finder-transcript'
        : coverageGap
          ? 'dimension-coverage-gap'
          : null;
}

/**
 * Join each candidate finding to the refuter that graded it and record the
 * resulting disposition. Returns true if any join was ambiguous.
 *
 * A candidate with NO matching refuter is a legitimate post-phase-6 pass-through
 * when its severity is non-gating (`unrefuted: true`, verdict stays null); with
 * a gating severity it is `unknown`, because the corpus cannot tell a refuter
 * that was never dispatched from one whose transcript is missing. A refuter
 * whose own verdict could not be read is `unknown` for the same reason. Nothing
 * is imputed either way.
 */
function attachDispositions(u) {
  const byDim = new Map();
  for (const r of u.refuters) {
    const k = r.dim || '';
    if (!byDim.has(k)) byDim.set(k, []);
    byDim.get(k).push(r);
  }
  let ambiguous = false;
  for (const c of u.candidates) {
    const pool = byDim.get(c.dim || '') || [];
    const id = c.finding && c.finding.id;
    let match = null;
    const byId = id == null ? [] : pool.filter((r) => r.finding && r.finding.id === id);
    if (byId.length === 1) {
      match = byId[0];
    } else {
      // Missing or duplicated `id`: fall back to a structural deep-equal of the
      // finding JSON the refuter was actually handed.
      const wanted = structuralKey(c.finding);
      const deep = pool.filter((r) => r.finding && structuralKey(r.finding) === wanted);
      if (deep.length === 1) match = deep[0];
      else if (deep.length > 1 || byId.length > 1) ambiguous = true;
    }
    if (match) {
      if (typeof match.refuted === 'boolean') {
        c.verdict = { refuted: match.refuted };
        c.disposition = 'resolved';
      } else {
        c.verdict = null;
        c.disposition = 'unknown';
      }
      continue;
    }
    const severity = c.finding && c.finding.severity;
    if (NON_GATING_SEVERITIES.indexOf(severity) !== -1) {
      c.verdict = null;
      c.disposition = 'resolved';
    } else {
      c.verdict = null;
      c.disposition = 'unknown';
    }
  }
  return ambiguous;
}

/**
 * Reconstruct the rank of the outcome-determining finding for ONE unit.
 *
 * UNIT STATUSES ARE DECIDED PER UNIT, FROM EVIDENCE LOCAL TO THAT UNIT — never
 * run-wide. No condition observed on a sibling unit, and no agent that could not
 * be attributed to any unit, may change this unit's status.
 *
 * The walk stops at the FIRST candidate whose disposition is `unknown`: an
 * ungraded finding ranked ABOVE the determining one could itself have been the
 * determining finding, at a better rank. An unknown ranked strictly BELOW the
 * determining finding cannot change the answer and is therefore harmless —
 * that asymmetry is the recoverability rule.
 *
 * @returns {{ status: 'determining', rank: number } | { status: 'non-determining' } | { status: 'unrecoverable', reason: string }}
 */
export function determineRankForUnit(unit, tier) {
  if (unit.structuralReason) return { status: 'unrecoverable', reason: unit.structuralReason };
  const byFinding = new Map();
  for (const c of unit.candidates) if (!byFinding.has(c.finding)) byFinding.set(c.finding, c);
  const ranked = rankFindings(unit.candidates.map((c) => c.finding));
  for (let i = 0; i < ranked.length; i++) {
    const c = byFinding.get(ranked[i]);
    if (!c || c.disposition === 'unknown') {
      return { status: 'unrecoverable', reason: 'unknown-disposition-above-determining' };
    }
    if (survives(c.finding, c.verdict) && hasBlocking([c.finding], tier)) {
      return { status: 'determining', rank: i + 1 };
    }
  }
  return { status: 'non-determining' };
}

/** The N values phase 4 will choose between, in fixed report order. */
const WITHIN_TOP_N = [3, 5];

function summarizeTier(units, tier) {
  const reasons = new Map();
  const ranks = [];
  let determining = 0;
  let nonDetermining = 0;
  let unrecoverable = 0;
  const candidateSizes = [];

  for (const u of units) {
    const outcome = determineRankForUnit(u, tier);
    if (outcome.status === 'determining') {
      determining += 1;
      ranks.push(outcome.rank);
      candidateSizes.push(u.candidates.length);
    } else if (outcome.status === 'non-determining') {
      nonDetermining += 1;
      candidateSizes.push(u.candidates.length);
    } else {
      unrecoverable += 1;
      reasons.set(outcome.reason, (reasons.get(outcome.reason) || 0) + 1);
    }
  }

  const total = units.length;
  const recoverable = determining + nonDetermining;
  const histogram = new Map();
  for (const r of ranks) histogram.set(r, (histogram.get(r) || 0) + 1);

  return {
    units: {
      total,
      determining,
      nonDetermining,
      unrecoverable,
      recoverable,
      recoverableSharePercent: pct(recoverable, total),
    },
    unrecoverableByReason: RANK_UNRECOVERABLE_REASONS.filter((r) => reasons.has(r)).map((reason) => ({
      reason,
      count: reasons.get(reason),
    })),
    rankHistogram: [...histogram.keys()].sort((a, b) => a - b).map((rank) => ({ rank, count: histogram.get(rank) })),
    rankSummary: ranks.length > 0 ? summarizeCounts(ranks) : null,
    withinTop: WITHIN_TOP_N.map((n) => {
      const count = ranks.filter((r) => r <= n).length;
      return { n, count, percentOfDetermining: pct(count, determining), percentOfRecoverable: pct(count, recoverable) };
    }),
    candidateSetSize: candidateSizes.length > 0 ? summarizeCounts(candidateSizes) : null,
  };
}

/**
 * The thresholds that turn the measured distribution into a supports/kills
 * answer. Named and exported so the conclusion is DERIVED from the figures and
 * re-derivable by `auditRankDoc`, never a hand-written sentence that can drift
 * from the data it claims to read.
 */
export const CAP_VERDICT_RULE = {
  supportsCapAtOrAbovePercent: 95,
  killsCapBelowPercent: 80,
  minRecoverableSharePercent: 50,
  minDeterminingUnits: 20,
  basis:
    'withinTop n=5, as a percentage of DETERMINING units; a cap is only supported when enough units were ' +
    'recoverable to speak to it at all.',
};

/**
 * `supports-cap` | `kills-cap` | `inconclusive` — a non-concentrating
 * distribution, or too few recoverable units to speak to one, is a first-class
 * recordable answer, not a measurement failure.
 */
export function deriveCapVerdict(inputs) {
  const determining = inputs.determining || 0;
  const share = inputs.recoverableSharePercent || 0;
  const top5 = inputs.withinTop5PercentOfDetermining || 0;
  if (determining < CAP_VERDICT_RULE.minDeterminingUnits) return 'inconclusive';
  if (top5 < CAP_VERDICT_RULE.killsCapBelowPercent) return 'kills-cap';
  if (top5 >= CAP_VERDICT_RULE.supportsCapAtOrAbovePercent && share >= CAP_VERDICT_RULE.minRecoverableSharePercent) {
    return 'supports-cap';
  }
  return 'inconclusive';
}

function capVerdictInputs(base) {
  const top5 = base.withinTop.find((w) => w.n === 5);
  return {
    determining: base.units.determining,
    recoverable: base.units.recoverable,
    total: base.units.total,
    recoverableSharePercent: base.units.recoverableSharePercent,
    withinTop5PercentOfDetermining: top5 ? top5.percentOfDetermining : 0,
  };
}

/**
 * Build the `determiningFindingRank` report block: the headline distribution at
 * the default blocker set, a `largeTier` sensitivity variant, the orphan-agent
 * diagnostic, the `ac`-table side-channel diagnostic, and the derived cap
 * verdict.
 */
export function buildDeterminingFindingRank(finders, refuters, sessionDirOf) {
  const { units, orphan } = buildUnitCandidates(finders, refuters, sessionDirOf);
  const base = summarizeTier(units, undefined);
  // The tier is threaded through `context` at runtime and is embedded in NEITHER
  // prompt, so it is not recoverable per unit. The headline uses the default
  // blocker set (['blocking']); this variant widens it to ['blocking','concern']
  // so a reader can see how much the answer moves rather than guessing a tier.
  const large = summarizeTier(units, 'large');
  const inputs = capVerdictInputs(base);

  return {
    units: base.units,
    unrecoverableByReason: base.unrecoverableByReason,
    orphanAgents: { finders: orphan.finders, refuters: orphan.refuters, runsAffected: orphan.runs.size },
    rankHistogram: base.rankHistogram,
    ...(base.rankSummary ? { rankSummary: base.rankSummary } : {}),
    withinTop: base.withinTop,
    ...(base.candidateSetSize ? { candidateSetSize: base.candidateSetSize } : {}),
    largeTier: {
      units: large.units,
      rankHistogram: large.rankHistogram,
      ...(large.rankSummary ? { rankSummary: large.rankSummary } : {}),
      withinTop: large.withinTop,
    },
    // The `ac` dimension is a SECOND outcome channel: in code mode
    // `classifyOutcome` routes a FAIL/PARTIAL AC table through `acTableHasGap`
    // directly, never through finding severity, so a unit can have been
    // `rework` with no gating finding at all. Reported as its own diagnostic —
    // this phase's question is scoped to `hasBlocking`, and such a unit is NOT
    // folded into `non-determining`.
    acTableGapUnits: units.filter((u) => u.acTableGap).length,
    capVerdict: { verdict: deriveCapVerdict(inputs), rule: CAP_VERDICT_RULE, inputs },
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

  const u = report.refuterFanout.refuterCountsByUnit;
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
  out.push('');

  const d = report.determiningFindingRank;
  out.push('Rank of the outcome-determining finding, over each unit\'s full CANDIDATE list');
  out.push('(ranking among survivors is degenerate — severity sorts first, so it is always 1):');
  out.push('');
  out.push('| rank | units |');
  out.push('|---:|---:|');
  for (const row of d.rankHistogram) out.push('| ' + row.rank + ' | ' + row.count + ' |');
  if (d.rankHistogram.length === 0) out.push('| _(no determining units)_ | 0 |');
  out.push('');
  out.push('| units | determining | non-determining | unrecoverable | recoverable | recoverable share |');
  out.push('|---:|---:|---:|---:|---:|---:|');
  out.push(
    '| ' +
      [
        d.units.total,
        d.units.determining,
        d.units.nonDetermining,
        d.units.unrecoverable,
        d.units.recoverable,
        d.units.recoverableSharePercent + '%',
      ].join(' | ') +
      ' |'
  );
  out.push('');
  for (const w of d.withinTop) {
    out.push(
      'Determining finding within top ' + w.n + ': ' + w.count + ' unit(s) — ' + w.percentOfDetermining +
        '% of determining units, ' + w.percentOfRecoverable + '% of recoverable units, over a recoverable share of ' +
        d.units.recoverableSharePercent + '% (' + d.units.recoverable + '/' + d.units.total + ' units).'
    );
  }
  out.push('');
  if (d.unrecoverableByReason.length > 0) {
    out.push('| unrecoverable reason | units |');
    out.push('|---|---:|');
    for (const row of d.unrecoverableByReason) out.push('| ' + row.reason + ' | ' + row.count + ' |');
    out.push('');
  }
  out.push(
    'Orphan agents (unit identity unresolvable, attributable to no unit and therefore invalidating none): ' +
      d.orphanAgents.finders + ' finder(s), ' + d.orphanAgents.refuters + ' refuter(s), across ' +
      d.orphanAgents.runsAffected + ' run(s).'
  );
  out.push(
    'Units whose `ac` table carried a FAIL/PARTIAL (a second outcome channel this measurement does not score): ' +
      d.acTableGapUnits + '.'
  );
  out.push(
    'Tier is not recoverable from either prompt; at the `large` blocker set (blocking+concern) the same corpus ' +
      'yields ' + d.largeTier.units.determining + ' determining unit(s), ' +
      d.largeTier.withinTop.map((w) => w.count + ' within top ' + w.n).join(', ') + '.'
  );
  out.push('Cap verdict (derived, not asserted): ' + d.capVerdict.verdict + '.');
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
  const rankSection = parsed.determiningFindingRank;
  if (!rankSection) throw new Error(`${docArg} has no "determiningFindingRank" section`);
  return { docPath, section, fanoutSection, rankSection };
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

const RANK_SUMMARY_FIELDS = ['n', 'min', 'p50', 'p90', 'max'];

/**
 * Compare a computed report's `determiningFindingRank` against the doc's
 * recorded figures, field by field.
 */
export function checkRankDoc(report, rankSection) {
  const missing = [];
  const got = report.determiningFindingRank;
  const exp = rankSection || {};

  function cmpUnits(gotU, expU, prefix) {
    for (const f of ['total', 'determining', 'nonDetermining', 'unrecoverable', 'recoverable', 'recoverableSharePercent']) {
      if ((expU || {})[f] !== gotU[f]) missing.push(`${prefix}.${f}: doc ${(expU || {})[f]} vs corpus ${gotU[f]}`);
    }
  }
  function cmpHistogram(gotH, expH, prefix) {
    const e = Array.isArray(expH) ? expH : [];
    if (e.length !== gotH.length) {
      missing.push(`${prefix} row count: doc has ${e.length}, corpus yields ${gotH.length}`);
    }
    const byRank = new Map(gotH.map((r) => [r.rank, r.count]));
    for (const row of e) {
      if (byRank.get(row.rank) !== row.count) {
        missing.push(`${prefix}[rank ${row.rank}]: doc ${row.count} vs corpus ${byRank.get(row.rank)}`);
      }
    }
  }
  function cmpWithinTop(gotW, expW, prefix) {
    const e = Array.isArray(expW) ? expW : [];
    const byN = new Map(gotW.map((w) => [w.n, w]));
    for (const w of e) {
      const g = byN.get(w.n);
      if (!g) {
        missing.push(`${prefix}[n=${w.n}] is in the doc but not in the corpus`);
        continue;
      }
      for (const f of ['count', 'percentOfDetermining', 'percentOfRecoverable']) {
        if (w[f] !== g[f]) missing.push(`${prefix}[n=${w.n}].${f}: doc ${w[f]} vs corpus ${g[f]}`);
      }
    }
  }
  function cmpSummary(gotS, expS, prefix) {
    if (!gotS && !expS) return;
    if (!gotS || !expS) {
      missing.push(`${prefix}: doc ${expS ? 'has' : 'omits'} it, corpus ${gotS ? 'has' : 'omits'} it`);
      return;
    }
    for (const f of RANK_SUMMARY_FIELDS) {
      if (expS[f] !== gotS[f]) missing.push(`${prefix}.${f}: doc ${expS[f]} vs corpus ${gotS[f]}`);
    }
  }

  cmpUnits(got.units, exp.units, 'units');
  const expReasons = Array.isArray(exp.unrecoverableByReason) ? exp.unrecoverableByReason : [];
  if (expReasons.length !== got.unrecoverableByReason.length) {
    missing.push(
      `unrecoverableByReason row count: doc has ${expReasons.length}, corpus yields ${got.unrecoverableByReason.length}`
    );
  }
  const gotReasons = new Map(got.unrecoverableByReason.map((r) => [r.reason, r.count]));
  for (const row of expReasons) {
    if (gotReasons.get(row.reason) !== row.count) {
      missing.push(`unrecoverableByReason[${row.reason}]: doc ${row.count} vs corpus ${gotReasons.get(row.reason)}`);
    }
  }
  for (const f of ['finders', 'refuters', 'runsAffected']) {
    if ((exp.orphanAgents || {})[f] !== got.orphanAgents[f]) {
      missing.push(`orphanAgents.${f}: doc ${(exp.orphanAgents || {})[f]} vs corpus ${got.orphanAgents[f]}`);
    }
  }
  cmpHistogram(got.rankHistogram, exp.rankHistogram, 'rankHistogram');
  cmpSummary(got.rankSummary, exp.rankSummary, 'rankSummary');
  cmpWithinTop(got.withinTop, exp.withinTop, 'withinTop');
  cmpSummary(got.candidateSetSize, exp.candidateSetSize, 'candidateSetSize');
  if (exp.acTableGapUnits !== got.acTableGapUnits) {
    missing.push(`acTableGapUnits: doc ${exp.acTableGapUnits} vs corpus ${got.acTableGapUnits}`);
  }
  const expLarge = exp.largeTier || {};
  cmpUnits(got.largeTier.units, expLarge.units, 'largeTier.units');
  cmpHistogram(got.largeTier.rankHistogram, expLarge.rankHistogram, 'largeTier.rankHistogram');
  cmpSummary(got.largeTier.rankSummary, expLarge.rankSummary, 'largeTier.rankSummary');
  cmpWithinTop(got.largeTier.withinTop, expLarge.withinTop, 'largeTier.withinTop');
  if ((exp.capVerdict || {}).verdict !== got.capVerdict.verdict) {
    missing.push(`capVerdict.verdict: doc ${(exp.capVerdict || {}).verdict} vs corpus ${got.capVerdict.verdict}`);
  }
  return missing;
}

/**
 * Corpus-free audit of a doc's `determiningFindingRank`: is the unit partition
 * exact, is every unrecoverable reason in the closed vocabulary, do the
 * histogram and top-N counts reconcile against `determining`, do all the
 * percentages re-derive from the doc's own counts, and does the recorded cap
 * verdict re-derive from `deriveCapVerdict`?
 *
 * The verdict re-derivation is what makes the supports/kills claim machine-
 * gated rather than asserted: a hand-written conclusion cannot drift from the
 * numbers it reads without this failing.
 */
export function auditRankDoc(rankSection) {
  const problems = [];
  const s = rankSection || {};

  function auditBlock(block, prefix, opts) {
    const u = (block && block.units) || {};
    const total = u.total || 0;
    const determining = u.determining || 0;
    const nonDetermining = u.nonDetermining || 0;
    const unrecoverable = u.unrecoverable || 0;
    const recoverable = u.recoverable || 0;
    if (determining + nonDetermining + unrecoverable !== total) {
      problems.push(
        `${prefix}.units: determining + nonDetermining + unrecoverable (${determining + nonDetermining + unrecoverable}) !== total (${total})`
      );
    }
    if (determining + nonDetermining !== recoverable) {
      problems.push(`${prefix}.units.recoverable: doc ${recoverable}, derived ${determining + nonDetermining}`);
    }
    const derivedShare = pct(recoverable, total);
    if (u.recoverableSharePercent !== derivedShare) {
      problems.push(`${prefix}.units.recoverableSharePercent: doc ${u.recoverableSharePercent}, derived ${derivedShare}`);
    }

    const hist = Array.isArray(block && block.rankHistogram) ? block.rankHistogram : [];
    const histSum = hist.reduce((n, r) => n + (r.count || 0), 0);
    if (histSum !== determining) {
      problems.push(`${prefix}.rankHistogram counts sum to ${histSum}, units.determining says ${determining}`);
    }
    for (let i = 1; i < hist.length; i++) {
      if (!(hist[i - 1].rank < hist[i].rank)) {
        problems.push(`${prefix}.rankHistogram is not in strictly ascending rank order`);
        break;
      }
    }
    const summary = block && block.rankSummary;
    if (determining === 0 && summary) problems.push(`${prefix}.rankSummary is present with zero determining units`);
    if (determining > 0) {
      if (!summary) problems.push(`${prefix}.rankSummary is missing with ${determining} determining units`);
      else {
        if (summary.n !== determining) problems.push(`${prefix}.rankSummary.n: doc ${summary.n}, determining ${determining}`);
        if (!(summary.min <= summary.p50 && summary.p50 <= summary.p90 && summary.p90 <= summary.max)) {
          problems.push(
            `${prefix}.rankSummary: min/p50/p90/max not monotonic (${summary.min}/${summary.p50}/${summary.p90}/${summary.max})`
          );
        }
      }
    }

    const within = Array.isArray(block && block.withinTop) ? block.withinTop : [];
    if (within.map((w) => w.n).join(',') !== WITHIN_TOP_N.join(',')) {
      problems.push(`${prefix}.withinTop must carry exactly n=${WITHIN_TOP_N.join(' and n=')}, in that order`);
    }
    let previous = 0;
    for (const w of within) {
      const count = w.count || 0;
      if (count < previous) problems.push(`${prefix}.withinTop[n=${w.n}].count (${count}) is below a smaller n's count`);
      previous = count;
      if (count > determining) {
        problems.push(`${prefix}.withinTop[n=${w.n}].count (${count}) exceeds units.determining (${determining})`);
      }
      const dPct = pct(count, determining);
      const rPct = pct(count, recoverable);
      if (w.percentOfDetermining !== dPct) {
        problems.push(`${prefix}.withinTop[n=${w.n}].percentOfDetermining: doc ${w.percentOfDetermining}, derived ${dPct}`);
      }
      if (w.percentOfRecoverable !== rPct) {
        problems.push(`${prefix}.withinTop[n=${w.n}].percentOfRecoverable: doc ${w.percentOfRecoverable}, derived ${rPct}`);
      }
    }

    if (opts && opts.withReasons) {
      const reasons = Array.isArray(block && block.unrecoverableByReason) ? block.unrecoverableByReason : [];
      const reasonSum = reasons.reduce((n, r) => n + (r.count || 0), 0);
      if (reasonSum !== unrecoverable) {
        problems.push(`${prefix}.unrecoverableByReason counts sum to ${reasonSum}, units.unrecoverable says ${unrecoverable}`);
      }
      let lastIdx = -1;
      for (const r of reasons) {
        const idx = RANK_UNRECOVERABLE_REASONS.indexOf(r.reason);
        if (idx === -1) {
          problems.push(
            `${prefix}.unrecoverableByReason: "${r.reason}" is not in the closed vocabulary ` +
              `[${RANK_UNRECOVERABLE_REASONS.join(', ')}] — unit statuses are decided PER UNIT, so there is no run-wide reason`
          );
        } else if (idx <= lastIdx) {
          problems.push(`${prefix}.unrecoverableByReason is not in the fixed report order`);
        } else {
          lastIdx = idx;
        }
      }
    }
  }

  auditBlock(s, 'determiningFindingRank', { withReasons: true });
  auditBlock(s.largeTier || {}, 'determiningFindingRank.largeTier', { withReasons: false });

  // The largeTier blocker set is a strict superset of the default one, and the
  // walk order is identical, so a unit that gated at the default set gates no
  // later at `large`: the determining set can only grow and every within-top
  // count can only rise.
  const baseWithin = new Map((Array.isArray(s.withinTop) ? s.withinTop : []).map((w) => [w.n, w.count || 0]));
  for (const w of Array.isArray((s.largeTier || {}).withinTop) ? s.largeTier.withinTop : []) {
    if ((w.count || 0) < (baseWithin.get(w.n) || 0)) {
      problems.push(
        `largeTier.withinTop[n=${w.n}].count (${w.count}) is below the default tier's (${baseWithin.get(w.n)}) — ` +
          'widening the blocker set can only move a determining finding earlier, never later'
      );
    }
  }
  if (((s.largeTier || {}).units || {}).determining < (s.units || {}).determining) {
    problems.push('largeTier.units.determining is below the default tier\'s — widening the blocker set cannot un-determine a unit');
  }

  const cap = s.capVerdict || {};
  const inputs = cap.inputs || {};
  for (const [field, expected] of [
    ['determining', (s.units || {}).determining],
    ['recoverable', (s.units || {}).recoverable],
    ['total', (s.units || {}).total],
    ['recoverableSharePercent', (s.units || {}).recoverableSharePercent],
  ]) {
    if (inputs[field] !== expected) {
      problems.push(`capVerdict.inputs.${field}: doc ${inputs[field]}, units block says ${expected}`);
    }
  }
  const top5 = (Array.isArray(s.withinTop) ? s.withinTop : []).find((w) => w.n === 5);
  const top5Pct = top5 ? top5.percentOfDetermining : 0;
  if (inputs.withinTop5PercentOfDetermining !== top5Pct) {
    problems.push(
      `capVerdict.inputs.withinTop5PercentOfDetermining: doc ${inputs.withinTop5PercentOfDetermining}, withinTop says ${top5Pct}`
    );
  }
  const derivedVerdict = deriveCapVerdict(inputs);
  if (cap.verdict !== derivedVerdict) {
    problems.push(
      `capVerdict.verdict: doc "${cap.verdict}", re-derived from the doc's own figures "${derivedVerdict}" ` +
        '— the supports/kills conclusion must follow from the numbers, not be asserted beside them'
    );
  }
  for (const key of Object.keys(CAP_VERDICT_RULE)) {
    if ((cap.rule || {})[key] !== CAP_VERDICT_RULE[key]) {
      problems.push(`capVerdict.rule.${key}: doc ${(cap.rule || {})[key]}, instrument ${CAP_VERDICT_RULE[key]}`);
    }
  }

  const orphans = s.orphanAgents || {};
  for (const f of ['finders', 'refuters', 'runsAffected']) {
    if (typeof orphans[f] !== 'number' || orphans[f] < 0) {
      problems.push(`orphanAgents.${f} must be a non-negative number, got ${orphans[f]}`);
    }
  }
  if (typeof s.acTableGapUnits !== 'number' || s.acTableGapUnits < 0) {
    problems.push(`acTableGapUnits must be a non-negative number, got ${s.acTableGapUnits}`);
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
                     nonGatingRefutationSkip / refuterFanout /
                     determiningFindingRank figures match exactly.
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
    const { section, fanoutSection, rankSection } = readDoc(args.audit);
    const problems = auditDoc(section).concat(auditFanoutDoc(fanoutSection), auditRankDoc(rankSection));
    if (problems.length) {
      console.error('measure-refuter-severity --audit FAILED against ' + args.audit + ':');
      for (const m of problems) console.error('  ' + m);
      process.exit(1);
    }
    console.log('measure-refuter-severity --audit OK: ' + args.audit + "'s figures are internally consistent");
    process.exit(0);
  }

  if (args.check) {
    const { section, fanoutSection, rankSection } = readDoc(args.check);
    // The doc's own recorded window is what makes a committed figure stable as
    // the corpus grows; --until overrides it for an ad hoc re-measurement.
    const until = args.until || (section.measurementWindow && section.measurementWindow.until) || undefined;
    const report = measure({ root: args.root, until });
    const missing = checkDoc(report, section).concat(
      checkFanoutDoc(report, fanoutSection),
      checkRankDoc(report, rankSection)
    );
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
