#!/usr/bin/env node
// mine-refuter-corpus.mjs — recover real historical refuter findings, verbatim,
// from the full-fidelity agent transcripts.
//
// WHY THIS EXISTS
// ------------------------------------------------------------------------
// The refuter-agreement harness needs real production findings, not invented
// ones. They already exist on disk: every refuter the lane ever dispatched has
// a `subagents/workflows/<runId>/agent-<agentId>.jsonl` transcript whose
// INITIATING user turn carries the COMPLETE `refutePrompt` it was given, and
// whose assistant turns carry the COMPLETE `StructuredOutput` verdict it
// returned. Historical findings are therefore replayable verbatim.
//
// The 401-character hard truncation people remember applies ONLY to the
// `promptPreview` / `resultPreview` fields of the `wf_*.json` sidecars. It does
// not apply to the transcripts, and this miner deliberately reads the
// transcripts. `scripts/verify-refuter-agreement.sh` asserts every mined prompt
// exceeds 401 characters precisely to prove that.
//
// THE HISTORICAL VERDICT IS NOT GROUND TRUTH
// ------------------------------------------------------------------------
// Every run on disk was refuted by an opus-class model. Scoring opus against
// its own past verdicts would be CIRCULAR and would manufacture ~100 %
// agreement for the baseline tier. This miner therefore emits
// `groundTruth: null` on every record and records the historical verdict only
// under `provenance.historicalVerdict`, as a CANDIDATE SIGNAL FOR ADJUDICATION.
// Assigning ground truth is a separate, human, evidence-citing pass.
//
// Usage:
//   node scripts/mine-refuter-corpus.mjs [options] > candidates.jsonl
//
// Options:
//   --root <dir>          Session-sidecar root (default: ~/.claude/projects).
//   --project-slug <s>    Repeatable/comma-separated project-slug prefix filter.
//                         Defaults to this repo's own slugs (including the
//                         `--worktrees-` variants) so an unrelated project's
//                         source text is never checked into the rdm repo.
//   --until <iso-date>    Ignore runs starting after this instant.
//   --severity <s>        Repeatable/comma-separated finding-severity filter.
//   --limit <n>           Stop after n recovered records.
//   --out <path>          Write JSONL here instead of stdout.
//   --format jsonl|json   Output format (default: jsonl).
//   --help                Print this help and exit.
//
// Determinism: no clock, no RNG, no network.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  defaultProjectsRoot,
  locateSessionDirs,
  findWorkflowRunFiles,
  buildRecords,
  transcriptPathFor,
} from './lib/token-report.mjs';
// Re-use the PROVEN brace-matched, sentinel-anchored extractor rather than
// re-inventing it: `refutePrompt` interpolates the target INLINE and the
// `--implementation-plan` plan-review target is itself pretty-printed JSON, so
// a naive `indexOf('{')` finds the TARGET, not the finding. Importing the two
// symbols keeps this instrument and measure-refuter-severity.mjs from drifting.
import { matchBrace, extractFinding } from './measure-refuter-severity.mjs';
import {
  sha256,
  reconstructRefuteInputs,
  CORPUS_SCHEMA_VERSION,
  groupCorpusForBatching,
  batchGroupKeyFor,
  unitIdentOf,
} from './lib/refuter-agreement.mjs';

// Re-exported so the harness can drive the same unit-scoped grouping the
// --min-group-size filter uses, without importing two modules.
export { batchGroupKeyFor, unitIdentOf };

// Silence the unused-import lint concern: matchBrace is re-exported so a
// consumer (and the harness) can reach the shared implementation through this
// module too, proving the single-source claim above.
export { matchBrace, extractFinding };

// The same six-lane run-set filter docs/token-baseline.json's `runSet` uses.
const LANE_WORKFLOWS = ['autopilot', 'dispatch-phase', 'plan-review', 'backlog', 'estimate', 'document'];

// This repo's own project slugs. `~/.claude/projects` holds unrelated projects
// (bowling-app and its worktrees) whose findings quote foreign source text;
// mining defaults to these prefixes so that text is never checked in here.
export const DEFAULT_PROJECT_SLUG_PREFIXES = ['-Users-edward-Projects-rdm'];

/**
 * Read one refuter transcript and recover BOTH halves of the record: the
 * verbatim prompt from its initiating user turn, and the verdict from the last
 * `StructuredOutput` tool call in its assistant turns.
 *
 * @param {string} filePath
 * @returns {{ ok: boolean, reason?: string, prompt?: string, verdict?: object|null }}
 */
export function readRefuterRecord(filePath) {
  let raw;
  try {
    raw = fs.readFileSync(filePath, 'utf8');
  } catch (err) {
    return { ok: false, reason: 'no-transcript', detail: err.message };
  }
  let prompt = null;
  let verdict = null;
  for (const line of raw.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    let entry;
    try {
      entry = JSON.parse(trimmed);
    } catch {
      continue;
    }
    if (prompt === null && entry.type === 'user') {
      const content = entry.message && entry.message.content;
      // The INITIATING turn carries the prompt as a bare string; later user
      // turns are tool_result arrays, which this type check skips.
      if (typeof content !== 'string') continue;
      prompt = content;
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
          if (typeof input.refuted === 'boolean') {
            verdict = {
              refuted: input.refuted,
              confidence: typeof input.confidence === 'number' ? input.confidence : null,
              rationale: typeof input.rationale === 'string' ? input.rationale : null,
            };
          }
        }
      }
    }
  }
  if (prompt === null) return { ok: false, reason: 'no-prompt' };
  return { ok: true, prompt, verdict };
}

/**
 * Turn one recovered refuter into a candidate corpus record, or explain why it
 * could not be turned into one. Never assigns ground truth.
 *
 * @param {object} record - an agent record from `buildRecords`
 * @param {string} prompt
 * @param {object|null} verdict
 * @param {number} [agentIndex] - the agent's position in the run sidecar's agent
 *   array. Recorded so a REWORK re-review (a SECOND find→refute chain against
 *   the same unit and dimension) can be split off from a first-round batch
 *   instead of silently inflating its apparent size — see
 *   `groupCorpusForBatching`'s round splitting.
 */
export function buildCandidate(record, prompt, verdict, agentIndex) {
  const inputs = reconstructRefuteInputs(prompt, { extractFinding, matchBrace });
  if (!inputs.finding) return { ok: false, reason: 'unparseable-finding' };
  if (!inputs.mode) return { ok: false, reason: 'unrecoverable-mode' };
  if (!inputs.dimKey) return { ok: false, reason: 'unrecoverable-dim' };
  if (verdict === null) return { ok: false, reason: 'no-verdict' };
  return {
    ok: true,
    item: {
      id: `mined-${record.runId}-${record.agentId}`,
      schemaVersion: CORPUS_SCHEMA_VERSION,
      mode: inputs.mode,
      dim: { key: inputs.dimKey },
      target: inputs.target === null ? '' : inputs.target,
      finding: inputs.finding,
      promptSha256: sha256(prompt),
      promptDrift: false,
      provenance: {
        kind: 'mined',
        projectSlug: record.projectSlug,
        sessionId: record.sessionId,
        runId: record.runId,
        agentId: record.agentId,
        workflow: record.workflowName,
        // CANDIDATE SIGNAL FOR ADJUDICATION, NOT GROUND TRUTH. Every run on
        // disk was refuted by an opus-class model; scoring opus against its own
        // past verdicts is circular.
        historicalVerdict: verdict,
        historicalModel: record.model,
        ...(Number.isInteger(agentIndex) ? { agentIndex } : {}),
      },
      // The miner NEVER assigns ground truth. Adjudication is a separate,
      // human, evidence-citing pass against the repo at the recorded commit.
      groundTruth: null,
      promptLength: prompt.length,
    },
  };
}

/**
 * Mine candidate corpus records from a sidecar tree.
 *
 * @param {{ root?: string, projectSlugPrefixes?: string[], until?: string,
 *   severities?: string[], limit?: number, minGroupSize?: number,
 *   excludeIds?: Set<string>|string[] }} [options]
 */
export function mine(options = {}) {
  const projectsRoot = options.root || defaultProjectsRoot();
  const prefixes = options.projectSlugPrefixes && options.projectSlugPrefixes.length
    ? options.projectSlugPrefixes
    : DEFAULT_PROJECT_SLUG_PREFIXES;

  const sessionDirs = locateSessionDirs(projectsRoot).filter((s) =>
    prefixes.some((p) => s.projectSlug.startsWith(p))
  );

  let runFiles = findWorkflowRunFiles(sessionDirs, { workflowNames: LANE_WORKFLOWS });
  let untilMs;
  if (options.until) {
    untilMs = Date.parse(options.until);
    if (Number.isNaN(untilMs)) throw new Error(`--until value is not a parseable date: "${options.until}"`);
    runFiles = runFiles.filter((rf) => rf.run.startTimeMs !== undefined && rf.run.startTimeMs <= untilMs);
  }

  const sessionDirOf = new Map();
  // The agent's ordinal position within its run's own agent array. A REWORK
  // re-review is a SECOND dispatch against the same (run, unit, mode, dim), and
  // without an ordering field the four-part batch key silently merges the two.
  const agentIndexOf = new Map();
  for (const rf of runFiles) {
    sessionDirOf.set(`${rf.projectSlug}|${rf.sessionId}|${rf.run.runId}`, rf.sessionDir);
    (rf.run.agents || []).forEach((agent, index) => {
      if (agent && agent.agentId) {
        agentIndexOf.set(`${rf.projectSlug}|${rf.sessionId}|${rf.run.runId}|${agent.agentId}`, index);
      }
    });
  }

  const records = buildRecords(runFiles).filter((r) => r.agentClass === 'refute');
  // Stable order: a corpus mined twice from the same tree is byte-identical.
  records.sort((a, b) => {
    const ka = `${a.projectSlug}|${a.sessionId}|${a.runId}|${a.agentId}`;
    const kb = `${b.projectSlug}|${b.sessionId}|${b.runId}|${b.agentId}`;
    return ka < kb ? -1 : ka > kb ? 1 : 0;
  });

  const items = [];
  // Skips are BUCKETED WITH COUNTS, never silently dropped — an unrecoverable
  // refuter is unknown, and an unknown must never contribute to any rate.
  const skips = {};
  const bump = (reason) => {
    skips[reason] = (skips[reason] || 0) + 1;
  };

  const excludeIds =
    options.excludeIds instanceof Set ? options.excludeIds : new Set(options.excludeIds || []);

  for (const r of records) {
    if (options.limit !== undefined && items.length >= options.limit) break;
    const sessionDir = sessionDirOf.get(`${r.projectSlug}|${r.sessionId}|${r.runId}`);
    if (!sessionDir || !r.agentId) {
      bump('no-transcript');
      continue;
    }
    const transcriptPath = transcriptPathFor(sessionDir, r.runId, r.agentId);
    const read = readRefuterRecord(transcriptPath);
    if (!read.ok) {
      bump(read.reason);
      continue;
    }
    const built = buildCandidate(
      r,
      read.prompt,
      read.verdict,
      agentIndexOf.get(`${r.projectSlug}|${r.sessionId}|${r.runId}|${r.agentId}`)
    );
    if (!built.ok) {
      bump(built.reason);
      continue;
    }
    if (options.severities && options.severities.length) {
      const sev = built.item.finding.severity;
      if (options.severities.indexOf(sev) === -1) {
        bump('severity-filtered');
        continue;
      }
    }
    items.push(built.item);
  }

  // --min-group-size: emit only candidates whose UNIT-SCOPED batch group has at
  // least n members, so a costly hand-adjudication pass buys only
  // power-ADDING candidates. Candidates whose unit identity is unrecoverable are
  // never grouped — they go to the same counted skip bucket everything else does.
  //
  // ORDER MATTERS: grouping runs over EVERY recovered candidate, INCLUDING ones
  // already adjudicated into the checked-in corpus. An already-adjudicated item
  // still occupies its slot in the real dispatch, so dropping it first would
  // shrink partially-adjudicated groups below the floor and hide exactly the
  // cheapest candidates to complete.
  let batchGrouping = null;
  if (options.minGroupSize !== undefined) {
    const summary = groupCorpusForBatching(items, { minGroupSize: options.minGroupSize });
    const keep = new Set();
    for (const g of summary.groups) {
      if (g.size >= options.minGroupSize) for (const id of g.ids) keep.add(id);
    }
    const before = items.length;
    const kept = items.filter((i) => keep.has(i.id));
    items.length = 0;
    items.push(...kept);
    skips['below-min-group-size'] = (skips['below-min-group-size'] || 0) + (before - kept.length);
    batchGrouping = {
      minGroupSize: options.minGroupSize,
      groupCount: summary.groupCount,
      sizeHistogram: summary.sizeHistogram,
      sizeHistogramByMode: summary.sizeHistogramByMode,
      qualifyingGroups: summary.qualifyingGroups,
      qualifyingItems: summary.qualifyingItems,
      unrecoverableUnitExcluded: summary.unrecoverableUnitExcluded,
      nonGatingExcluded: summary.nonGatingExcluded,
    };
  }

  // --exclude-corpus, applied LAST: an already-adjudicated item counted toward
  // its group's size above, but must not be emitted again as a candidate.
  if (excludeIds.size) {
    const before = items.length;
    const kept = items.filter((i) => !excludeIds.has(i.id));
    items.length = 0;
    items.push(...kept);
    if (before - kept.length) skips['already-adjudicated'] = (skips['already-adjudicated'] || 0) + (before - kept.length);
  }

  return {
    corpusRoot: projectsRoot,
    projectSlugPrefixes: prefixes,
    until: options.until || null,
    refuterRecordCount: records.length,
    recovered: items.length,
    skips,
    batchGrouping,
    items,
  };
}

/**
 * Read the ids already present in a checked-in corpus, so `--exclude-corpus`
 * can make a re-mine APPEND rather than duplicate. A missing file is an
 * actionable error, never a silent empty set.
 *
 * @param {string} filePath
 * @returns {Set<string>}
 */
export function readCorpusIds(filePath) {
  let raw;
  try {
    raw = fs.readFileSync(filePath, 'utf8');
  } catch (err) {
    throw new Error(`--exclude-corpus could not read "${filePath}": ${err.message}`);
  }
  const ids = new Set();
  for (const line of raw.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    try {
      const item = JSON.parse(trimmed);
      if (item && typeof item.id === 'string') ids.add(item.id);
    } catch {
      // A corpus line that does not parse cannot contribute an id to exclude;
      // loadCorpus is the gate that reports it.
    }
  }
  return ids;
}

// --- CLI ------------------------------------------------------------------

export function parseArgs(argv) {
  const args = {
    root: undefined,
    projectSlugPrefixes: [],
    until: undefined,
    severities: [],
    limit: undefined,
    minGroupSize: undefined,
    excludeCorpus: null,
    out: null,
    format: 'jsonl',
    help: false,
  };
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
      case '--project-slug':
        args.projectSlugPrefixes.push(...takeValue('--project-slug').split(',').map((s) => s.trim()).filter(Boolean));
        break;
      case '--until':
        args.until = takeValue('--until');
        break;
      case '--severity':
        args.severities.push(...takeValue('--severity').split(',').map((s) => s.trim()).filter(Boolean));
        break;
      case '--limit': {
        const raw = takeValue('--limit');
        const n = Number(raw);
        if (!Number.isInteger(n) || n < 1) throw new Error(`--limit must be a positive integer, got "${raw}"`);
        args.limit = n;
        break;
      }
      case '--min-group-size': {
        const raw = takeValue('--min-group-size');
        const n = Number(raw);
        if (!Number.isInteger(n) || n < 2) throw new Error(`--min-group-size must be an integer >= 2, got "${raw}"`);
        args.minGroupSize = n;
        break;
      }
      case '--exclude-corpus':
        args.excludeCorpus = takeValue('--exclude-corpus');
        break;
      case '--out':
        args.out = takeValue('--out');
        break;
      case '--format':
        args.format = takeValue('--format');
        break;
      case '--help':
      case '-h':
        args.help = true;
        break;
      default:
        throw new Error(`unrecognized argument: "${argv[i]}"`);
    }
  }
  if (args.format !== 'jsonl' && args.format !== 'json') {
    throw new Error(`--format must be "jsonl" or "json", got "${args.format}"`);
  }
  return args;
}

const HELP = `Usage: node scripts/mine-refuter-corpus.mjs [options] > candidates.jsonl

Mines real historical refuter findings verbatim from the full-fidelity
subagents/workflows/<runId>/agent-*.jsonl transcripts. Emits groundTruth: null
on every record — the historical verdict is a candidate signal for adjudication,
not ground truth (scoring opus against its own past verdicts is circular).

Options:
  --root <dir>          Session-sidecar root (default: ~/.claude/projects).
  --project-slug <s>    Repeatable/comma-separated project-slug prefix filter
                        (default: this repo's slugs, incl. --worktrees- variants).
  --until <iso-date>    Ignore runs starting after this instant.
  --severity <s>        Repeatable/comma-separated finding-severity filter.
  --limit <n>           Stop after n recovered records.
  --min-group-size <n>  Emit only candidates whose UNIT-SCOPED batch group
                        (runId, unit identity, mode, dimension) has >= n members,
                        so a hand-adjudication pass buys only power-adding items.
  --exclude-corpus <p>  Skip candidates whose id is already in this checked-in
                        corpus JSONL, so a re-mine appends rather than duplicates.
  --out <path>          Write JSONL here instead of stdout.
  --format jsonl|json   Output format (default: jsonl).
  --help                Print this help and exit.
`;

/**
 * CLI entry point. Returns the process exit code; every failure surfaces as a
 * thrown `Error` whose message is actionable, so the caller can render it
 * without a stack trace (matching `run-refuter-agreement.mjs`).
 *
 * @param {string[]} argv
 * @returns {number}
 */
export function main(argv) {
  const args = parseArgs(argv);
  if (args.help) {
    console.log(HELP);
    return 0;
  }
  const result = mine({
    root: args.root,
    projectSlugPrefixes: args.projectSlugPrefixes,
    until: args.until,
    severities: args.severities,
    limit: args.limit,
    minGroupSize: args.minGroupSize,
    excludeIds: args.excludeCorpus ? readCorpusIds(path.resolve(args.excludeCorpus)) : undefined,
  });

  const body =
    args.format === 'json'
      ? JSON.stringify({ instrument: 'scripts/mine-refuter-corpus.mjs', ...result }, null, 2)
      : result.items.map((i) => JSON.stringify(i)).join('\n') + (result.items.length ? '\n' : '');

  if (args.out) fs.writeFileSync(path.resolve(args.out), body);
  else process.stdout.write(body);

  const skipSummary = Object.entries(result.skips)
    .sort((a, b) => (a[0] < b[0] ? -1 : 1))
    .map(([k, v]) => `${k}=${v}`)
    .join(' ');
  console.error(
    `mine-refuter-corpus: ${result.recovered} recovered of ${result.refuterRecordCount} refuter record(s)` +
      (skipSummary ? ` — skipped: ${skipSummary}` : '')
  );
  return 0;
}

// `import.meta.main` is not available on the pinned node, so gate on argv[1].
const invokedDirectly = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invokedDirectly) {
  try {
    process.exit(main(process.argv.slice(2)));
  } catch (err) {
    console.error(String(err && err.message ? err.message : err));
    process.exit(1);
  }
}
