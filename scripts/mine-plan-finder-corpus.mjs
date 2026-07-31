#!/usr/bin/env node
// mine-plan-finder-corpus.mjs — recover real PLAN REVIEW UNITS, verbatim, from
// the full-fidelity agent transcripts, for the collapsed-plan-finder A/B.
//
// WHY THIS EXISTS
// ------------------------------------------------------------------------
// `bound-review-fan-out` phase 5 asks whether `plan` mode's THREE always-on
// finders (coherence, architectural-fit, restraint) can be collapsed into ONE
// agent holding three lenses without losing findings per lens. Answering it
// needs real plan documents and the real three-finder (arm A) output over them.
// Both already exist on disk: every plan finder the lane ever dispatched has a
// `subagents/workflows/<runId>/agent-<agentId>.jsonl` transcript whose
// INITIATING user turn carries the COMPLETE `findPrompt` it was given — the
// plan document included, because `findPrompt` interpolates the target INLINE —
// and whose assistant turns carry the COMPLETE `StructuredOutput` findings it
// returned.
//
// The 401-character hard truncation applies ONLY to the `promptPreview` /
// `resultPreview` fields of the `wf_*.json` sidecars, never to the transcripts.
// This miner reads the transcripts.
//
// THE RECORDED FINDINGS ARE NOT AN ADJUDICATION
// ------------------------------------------------------------------------
// The recorded arm-A findings are exactly that: what the three-finder shape
// produced on that day, on that model. They are NOT ground truth about the plan
// document, and this miner never assigns any. Scoring an arm against a verdict
// the same shape produced would be circular. Adjudication is a separate, human,
// evidence-citing pass (`tests/fixtures/finder-collapse/adjudication.jsonl`).
// The recorded findings are used for exactly two things: (1) the corpus power
// analysis (which lenses actually fired on which units), and (2) a validity
// cross-check against the LIVE arm-A distribution.
//
// THE REVIEW-UNIT BOUNDARY
// ------------------------------------------------------------------------
// `buildReviewPipeline` is invoked ONCE PER REVIEW UNIT, never once per run, so
// a unit is `(runId, unitIdent)` — the identical boundary
// `docs/token-baseline.json` § `refuterFanout` documents and
// `scripts/lib/refuter-agreement.mjs` § `unitIdentOf` implements. In plan mode
// `findPrompt`'s target is `<kind> <identity>` followed by a blank line and the
// plan document body, so the FIRST LINE is the identity and the rest is the
// document.
//
// Usage:
//   node scripts/mine-plan-finder-corpus.mjs [options] > corpus.jsonl
//
// Options:
//   --root <dir>          Session-sidecar root (default: ~/.claude/projects).
//   --project-slug <s>    Repeatable/comma-separated project-slug prefix filter.
//   --until <iso-date>    Ignore runs starting after this instant (the PINNED
//                         selection window; recorded in the header record).
//   --limit <n>           Stop after n recovered review units.
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
import {
  PLAN_FINDER_CORPUS_SCHEMA_VERSION,
  ALWAYS_ON_PLAN_LENSES,
  MIN_PLAN_DOC_CHARS,
  MAX_UNIT_IDENT_LENGTH,
  PLAN_TARGET_TYPES,
  unitIdentOf,
  targetTypeOf,
  planDocOf,
} from './lib/finder-collapse.mjs';

// The same six-lane run-set filter docs/token-baseline.json's `runSet` uses.
const LANE_WORKFLOWS = ['autopilot', 'dispatch-phase', 'plan-review', 'backlog', 'estimate', 'document'];

// This repo's own project slugs. `~/.claude/projects` holds unrelated projects
// whose plan text must never be checked in here.
export const DEFAULT_PROJECT_SLUG_PREFIXES = ['-Users-edward-Projects-rdm'];

// `find:plan:<dim>`, tolerating the runtime's ` (retry N)` label suffix. A retry
// is the SAME dispatch re-attempted, so it collapses onto its dimension rather
// than becoming a second lens observation.
const PLAN_FINDER_LABEL = /^find:plan:([a-z][a-z0-9-]*)(?: \(retry \d+\))?$/;

// The two sentinels `findPrompt(mode, dim, ctx)` puts around the target in plan
// mode. They are matched as literals rather than reconstructed, because the
// target is interpolated INLINE and is routinely multi-line.
const TARGET_PREFIX = 'Review target: ';
const TARGET_SUFFIX = '\nInspect the plan document text.';

/**
 * Recover the verbatim `target` string a plan finder was given.
 *
 * Returns null when the prompt is not a plan-mode `findPrompt` render (a
 * code-mode prompt, or a shape this miner does not recognize) — never a
 * best-effort partial, which would silently truncate the plan document.
 *
 * @param {string} prompt
 * @returns {string|null}
 */
export function extractPlanTarget(prompt) {
  if (typeof prompt !== 'string') return null;
  const start = prompt.indexOf(TARGET_PREFIX);
  if (start === -1) return null;
  const end = prompt.indexOf(TARGET_SUFFIX, start);
  if (end === -1 || end <= start) return null;
  const raw = prompt.slice(start + TARGET_PREFIX.length, end);
  // findPrompt appends a literal '.' after the interpolated target.
  return raw.endsWith('.') ? raw.slice(0, -1) : raw;
}

/**
 * Read one plan-finder transcript and recover BOTH halves: the verbatim prompt
 * from its initiating user turn, and the findings array from the last
 * `StructuredOutput` tool call in its assistant turns.
 *
 * A finder that returned no `StructuredOutput` at all yields `findings: null`
 * (ungraded), never `[]` — an empty array would read as "this lens found the
 * document clean", which is a materially different claim from "this lens did
 * not report".
 *
 * @param {string} filePath
 * @returns {{ ok: boolean, reason?: string, prompt?: string, findings?: object[]|null }}
 */
export function readFinderRecord(filePath) {
  let raw;
  try {
    raw = fs.readFileSync(filePath, 'utf8');
  } catch (err) {
    return { ok: false, reason: 'no-transcript', detail: err.message };
  }
  let prompt = null;
  let findings = null;
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
          if (Array.isArray(input.findings)) findings = input.findings;
        }
      }
    }
  }
  if (prompt === null) return { ok: false, reason: 'no-prompt' };
  return { ok: true, prompt, findings };
}

/** Deterministic, filesystem-safe slug for a unit identity. */
export function slugifyIdent(ident) {
  return String(ident)
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 80);
}

/**
 * Mine plan REVIEW UNITS from a sidecar tree.
 *
 * Every exclusion is BUCKETED WITH A COUNT, never silently dropped — an
 * unrecoverable unit is unknown, and an unknown must never contribute to any
 * rate or to the power analysis.
 *
 * @param {{ root?: string, projectSlugPrefixes?: string[], until?: string,
 *   limit?: number }} [options]
 */
export function mine(options = {}) {
  const projectsRoot = options.root || defaultProjectsRoot();
  const prefixes =
    options.projectSlugPrefixes && options.projectSlugPrefixes.length
      ? options.projectSlugPrefixes
      : DEFAULT_PROJECT_SLUG_PREFIXES;

  const sessionDirs = locateSessionDirs(projectsRoot).filter((s) => prefixes.some((p) => s.projectSlug.startsWith(p)));

  let runFiles = findWorkflowRunFiles(sessionDirs, { workflowNames: LANE_WORKFLOWS });
  if (options.until) {
    const untilMs = Date.parse(options.until);
    if (Number.isNaN(untilMs)) throw new Error(`--until value is not a parseable date: "${options.until}"`);
    runFiles = runFiles.filter((rf) => rf.run.startTimeMs !== undefined && rf.run.startTimeMs <= untilMs);
  }

  const sessionDirOf = new Map();
  for (const rf of runFiles) sessionDirOf.set(`${rf.projectSlug}|${rf.sessionId}|${rf.run.runId}`, rf.sessionDir);

  const records = buildRecords(runFiles).filter((r) => PLAN_FINDER_LABEL.test(r.label));
  // Stable order: a corpus mined twice from the same tree is byte-identical.
  records.sort((a, b) => {
    const ka = `${a.projectSlug}|${a.sessionId}|${a.runId}|${a.agentId}`;
    const kb = `${b.projectSlug}|${b.sessionId}|${b.runId}|${b.agentId}`;
    return ka < kb ? -1 : ka > kb ? 1 : 0;
  });

  const skips = {};
  const bump = (reason) => {
    skips[reason] = (skips[reason] || 0) + 1;
  };

  // Pass 1: fold every recovered finder onto its review unit.
  const byUnit = new Map();
  for (const r of records) {
    const sessionDir = sessionDirOf.get(`${r.projectSlug}|${r.sessionId}|${r.runId}`);
    if (!sessionDir || !r.agentId) {
      bump('no-transcript');
      continue;
    }
    const read = readFinderRecord(transcriptPathFor(sessionDir, r.runId, r.agentId));
    if (!read.ok) {
      bump(read.reason);
      continue;
    }
    const target = extractPlanTarget(read.prompt);
    if (target === null) {
      bump('unrecoverable-target');
      continue;
    }
    const ident = unitIdentOf(target);
    if (ident === null) {
      bump('unrecoverable-unit-identity');
      continue;
    }
    if (read.findings === null) {
      bump('no-findings-output');
      continue;
    }
    const lens = PLAN_FINDER_LABEL.exec(r.label)[1];
    const key = `${r.runId}|${ident}`;
    if (!byUnit.has(key)) {
      byUnit.set(key, {
        runId: r.runId,
        ident,
        target,
        byLens: {},
        usageByLens: {},
        agentIds: {},
        projectSlug: r.projectSlug,
        sessionId: r.sessionId,
        sessionDir,
      });
    }
    const u = byUnit.get(key);
    // A retry collapses onto its lens; last recovered wins, and the ordering
    // above makes "last" deterministic.
    u.byLens[lens] = read.findings;
    u.usageByLens[lens] = {
      output: r.output,
      uncachedInput: r.uncachedInput,
      cacheWrite: r.cacheWrite,
      cacheRead: r.cacheRead,
    };
    u.agentIds[lens] = r.agentId;
  }

  // Pass 2: qualify. Each rejection is counted; nothing is silently dropped.
  const units = [];
  for (const [, u] of [...byUnit.entries()].sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0))) {
    const targetType = targetTypeOf(u.ident);
    if (targetType === null) {
      bump('unrecognized-target-type');
      continue;
    }
    const planDoc = planDocOf(u.target);
    if (planDoc.length < MIN_PLAN_DOC_CHARS) {
      // A target whose body is a fetch-status line ("Successfully fetched
      // roadmap X ...") is NOT a plan document. Reviewing it measures nothing
      // about lens dilution, so it is excluded and counted rather than padding
      // the corpus. The floor is pre-registered in scripts/lib/finder-collapse.mjs.
      bump('plan-doc-below-floor');
      continue;
    }
    const missing = ALWAYS_ON_PLAN_LENSES.filter((l) => !Array.isArray(u.byLens[l]));
    if (missing.length) {
      bump('incomplete-always-on-lenses');
      continue;
    }
    if (options.limit !== undefined && units.length >= options.limit) break;
    const byLens = {};
    const armAUsage = {};
    for (const lens of ALWAYS_ON_PLAN_LENSES) {
      byLens[lens] = u.byLens[lens];
      armAUsage[lens] = u.usageByLens[lens];
    }
    units.push({
      id: `unit-${u.runId}-${slugifyIdent(u.ident)}`,
      schemaVersion: PLAN_FINDER_CORPUS_SCHEMA_VERSION,
      targetType,
      targetId: u.ident,
      target: u.target,
      planDoc,
      armA: { byLens },
      armAUsage,
      provenance: {
        runId: u.runId,
        agentIds: u.agentIds,
        projectSlug: u.projectSlug,
        sessionId: u.sessionId,
        sessionPath: path.relative(projectsRoot, u.sessionDir),
      },
    });
  }

  return {
    corpusRoot: projectsRoot,
    projectSlugPrefixes: prefixes,
    until: options.until || null,
    finderRecordCount: records.length,
    recovered: units.length,
    skips,
    units,
  };
}

/**
 * The header record every mined corpus carries on its FIRST line. It pins the
 * exact selection window, so a later mining pass cannot silently re-baseline
 * the population the committed figures were measured over.
 *
 * @param {object} result - a `mine()` result
 * @returns {object}
 */
export function buildHeader(result) {
  const byType = {};
  for (const t of PLAN_TARGET_TYPES) byType[t] = result.units.filter((u) => u.targetType === t).length;
  return {
    kind: 'header',
    schemaVersion: PLAN_FINDER_CORPUS_SCHEMA_VERSION,
    instrument: 'scripts/mine-plan-finder-corpus.mjs',
    window: {
      until: result.until,
      projectSlugPrefixes: result.projectSlugPrefixes,
      laneWorkflows: LANE_WORKFLOWS,
      minPlanDocChars: MIN_PLAN_DOC_CHARS,
      maxUnitIdentLength: MAX_UNIT_IDENT_LENGTH,
      alwaysOnLenses: ALWAYS_ON_PLAN_LENSES,
    },
    finderRecordCount: result.finderRecordCount,
    unitCount: result.units.length,
    unitsByTargetType: byType,
    skips: result.skips,
  };
}

// --- CLI ------------------------------------------------------------------

export function parseArgs(argv) {
  const args = {
    root: undefined,
    projectSlugPrefixes: [],
    until: undefined,
    limit: undefined,
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
        args.projectSlugPrefixes.push(
          ...takeValue('--project-slug')
            .split(',')
            .map((s) => s.trim())
            .filter(Boolean)
        );
        break;
      case '--until':
        args.until = takeValue('--until');
        break;
      case '--limit': {
        const raw = takeValue('--limit');
        const n = Number(raw);
        if (!Number.isInteger(n) || n < 1) throw new Error(`--limit must be a positive integer, got "${raw}"`);
        args.limit = n;
        break;
      }
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

const HELP = `Usage: node scripts/mine-plan-finder-corpus.mjs [options] > corpus.jsonl

Mines real PLAN REVIEW UNITS verbatim from the full-fidelity
subagents/workflows/<runId>/agent-*.jsonl transcripts, for the
collapsed-plan-finder A/B (docs/finder-collapse.md). A unit is
(runId, unit identity) — the same boundary docs/token-baseline.json
§ refuterFanout documents — and carries the plan document text plus the
recorded three-finder (arm A) findings per lens.

The recorded findings are NOT ground truth and this miner never adjudicates:
scoring an arm against a verdict the same shape produced would be circular.

Options:
  --root <dir>          Session-sidecar root (default: ~/.claude/projects).
  --project-slug <s>    Repeatable/comma-separated project-slug prefix filter
                        (default: this repo's slugs, incl. --worktrees- variants).
  --until <iso-date>    Ignore runs starting after this instant (the PINNED window).
  --limit <n>           Stop after n recovered review units.
  --out <path>          Write JSONL here instead of stdout.
  --format jsonl|json   Output format (default: jsonl).
  --help                Print this help and exit.
`;

/**
 * CLI entry point. Returns the process exit code; every failure surfaces as a
 * thrown `Error` whose message is actionable.
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
    limit: args.limit,
  });
  const header = buildHeader(result);

  const body =
    args.format === 'json'
      ? JSON.stringify({ instrument: 'scripts/mine-plan-finder-corpus.mjs', header, ...result }, null, 2)
      : [header, ...result.units].map((i) => JSON.stringify(i)).join('\n') + '\n';

  if (args.out) fs.writeFileSync(path.resolve(args.out), body);
  else process.stdout.write(body);

  const skipSummary = Object.entries(result.skips)
    .sort((a, b) => (a[0] < b[0] ? -1 : 1))
    .map(([k, v]) => `${k}=${v}`)
    .join(' ');
  console.error(
    `mine-plan-finder-corpus: ${result.recovered} review unit(s) from ${result.finderRecordCount} plan-finder record(s)` +
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
