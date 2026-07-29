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
 * Pull the finding object out of a refuter prompt.
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
 * @param {string} prompt
 * @returns {object|null} the parsed finding, or null if it cannot be recovered
 */
export function extractFinding(prompt) {
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
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) return parsed;
    } catch {
      // Not the finding after all — keep scanning.
    }
  }
  return null;
}

/**
 * Read one refuter transcript and recover BOTH halves of what it did: the
 * severity of the finding it was handed (from its initiating user turn) and the
 * verdict it returned (from its forced `StructuredOutput` tool call).
 *
 * The verdict half is what settles which severities are worth refuting at all —
 * a severity whose refuters mostly say "refuted" is earning its cost.
 *
 * @param {string} filePath
 * @returns {{ severity: string|null, refuted: boolean|null }}
 */
export function readRefuterTranscript(filePath) {
  let raw;
  try {
    raw = fs.readFileSync(filePath, 'utf8');
  } catch {
    return { severity: null, refuted: null };
  }
  let severity = null;
  let severitySeen = false;
  let refuted = null;
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
  return { severity, refuted };
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

  for (const r of refuters) {
    r.refuted = null;
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
  return { docPath, section };
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
    const { section } = readDoc(args.audit);
    const problems = auditDoc(section);
    if (problems.length) {
      console.error('measure-refuter-severity --audit FAILED against ' + args.audit + ':');
      for (const m of problems) console.error('  ' + m);
      process.exit(1);
    }
    console.log('measure-refuter-severity --audit OK: ' + args.audit + "'s figures are internally consistent");
    process.exit(0);
  }

  if (args.check) {
    const { section } = readDoc(args.check);
    // The doc's own recorded window is what makes a committed figure stable as
    // the corpus grows; --until overrides it for an ad hoc re-measurement.
    const until = args.until || (section.measurementWindow && section.measurementWindow.until) || undefined;
    const report = measure({ root: args.root, until });
    const missing = checkDoc(report, section);
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
