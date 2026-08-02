#!/usr/bin/env node
// measure-hoist-delta.mjs — the post-change half of this roadmap's phase-3
// measurement, expressed against docs/token-baseline.json.
//
// WHY THIS EXISTS, AND WHY IT IS NOT `measure-lane-tokens.mjs`
// ------------------------------------------------------------------------
// scripts/measure-lane-tokens.mjs measures what the lane ACTUALLY SPENT, by
// reading Claude Code's session sidecars. It can only ever describe runs that
// already happened, and every run on disk executed PRE-change code — including
// the dispatch that implemented this phase, whose workflow file was loaded
// before the edits landed. Subtracting two `measure-lane-tokens.mjs` snapshots
// therefore measures CORPUS GROWTH, not this change.
//
// This script measures the other half, and measures it EMPIRICALLY rather than
// by asserting an elimination rule on paper: it EXECUTES the real, post-change
// workflow driver under a recording fake `agent`, twice — once with the args a
// PRE-change caller passed, once with the args the POST-change shim passes —
// and counts the mechanical subagents each run actually spawns. That is a
// direct observation of the shipped code, not a replay rule applied by hand.
//
// It then PRICES those counted agents using docs/token-baseline.json's own
// measured figures (`byAgentClass[].agentCount` and its four token columns give
// a mean tokens-per-agent per class). So the reported token delta is
// (measured post-change agent counts) x (measured pre-change per-agent cost),
// with both factors empirical and neither hand-transcribed.
//
// WHAT IT DOES NOT CLAIM
// ------------------------------------------------------------------------
// This is not a substitute for a real post-change lane run. A fake agent
// returns canned values instantly, so it measures AGENT COUNT exactly and
// per-agent TOKEN COST not at all — the cost factor is borrowed from the
// baseline. The residual assumption is that eliminating a mechanical agent
// removes a cost drawn from the same distribution the baseline measured for
// its class. Confirm on the next real lane run with:
//
//   node scripts/measure-lane-tokens.mjs --format json --workflow dispatch-phase
//
// Usage:
//   node scripts/measure-hoist-delta.mjs [--format text|json] [--check <doc>]
//
//   --format text|json   Output format (default: text).
//   --check <doc>        Assert the figures this script computes appear
//                        verbatim in <doc> (default:
//                        docs/mechanical-agent-inventory.md) and exit non-zero
//                        if they do not. Keeps the doc from rotting silently.
//
// Determinism: no Date.now(), no Math.random(), no network, no sidecar reads.
// Same checkout in, same numbers out.

import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(HERE, '..');

// --- Fixtures -----------------------------------------------------------
// Deliberately the same shapes scripts/verify-workflow-dispatch.sh section 6
// uses, so a fixture drift in one shows up as a failure in the other.

const MODELS = {
  plan: 'm-plan',
  implement: 'm-impl',
  review_find: 'm-find',
  review_verify: 'm-verify',
  mechanical: 'm-mech',
};
const PHASE_META = {
  roadmap: 'rm',
  phase: '1',
  stem: 'phase-1-x',
  model: 'medium',
  body: 'PHASE BODY TEXT',
  models: MODELS,
};
const TASK_META = { task: 'my-task', body: 'TASK BODY TEXT', models: MODELS };
const PLAN_DOC = {
  steps_per_ac: [{ ac: 'AC1', steps: ['do it'] }],
  file_map: [{ path: 'a.rs', change: 'edit' }],
  tests_per_ac: [{ ac: 'AC1', test: 't' }],
  edge_cases: [],
  cross_phase_deps: [],
  summary: 'plan',
};
// What the post-change implementer returns once its prompt asks for the diff.
const ABSORBED = { changedFiles: ['rdm-core/src/lib.rs'], diffText: 'diff --git a b' };

// --- The Workflow runtime's ambient globals, reference implementations ----

async function refParallel(thunks) {
  return Promise.all(thunks.map((t) => Promise.resolve().then(t).catch(() => null)));
}
async function refPipeline(items, ...stages) {
  return Promise.all(
    items.map(async (item, i) => {
      let acc = item;
      for (const stage of stages) {
        try {
          acc = await stage(acc, item, i);
        } catch {
          return null;
        }
      }
      return acc;
    })
  );
}

/**
 * A recording fake `agent`. Answers every label the dispatch-phase driver can
 * emit and records the label of each call, so the count IS the measurement.
 *
 * @param {{ absorbDiff?: boolean }} [o]
 */
function makeAgent(o) {
  const opts = o || {};
  const calls = [];
  const agent = async (prompt, agentOpts) => {
    const label = (agentOpts && agentOpts.label) || '';
    calls.push(label);
    if (label === 'fetch:phase-meta') return PHASE_META;
    if (label === 'fetch:task-meta') return TASK_META;
    if (label === 'stamp:in-progress') return { ok: true };
    if (label === 'plan:author' || label === 'plan:revise') return PLAN_DOC;
    if (label === 'act:code') return { handled: [] };
    if (label === 'diff:signals') return { changedFiles: ['rdm-core/src/lib.rs'], diffText: '' };
    if (label === 'implement:worktree' || label === 'implement:rework') {
      // Pre-change, the implementer had no output schema and returned nothing
      // usable; post-change it returns the diff the absorbed command produced.
      return opts.absorbDiff ? ABSORBED : undefined;
    }
    const parts = label.split(':');
    if (parts[0] === 'find') {
      if (parts[2] === 'ac') return { ac: [], findings: [] };
      return { findings: [] };
    }
    if (parts[0] === 'refute') return { refuted: false, confidence: 95 };
    throw new Error('unexpected agent label: ' + label);
  };
  return { agent, calls };
}

/** Load a workflow script and expose its top-level body as a callable driver. */
async function loadWorkflow(relPath) {
  const src = fs.readFileSync(path.join(REPO_ROOT, relPath), 'utf8').replace(/^export /m, '');
  const wrapped = path.join(os.tmpdir(), 'measure-hoist-delta-' + path.basename(relPath, '.js') + '.mjs');
  fs.writeFileSync(wrapped, 'export default async function(args, agent, pipeline, parallel, log) {\n' + src + '\n}\n');
  const mod = await import('file://' + wrapped + '?t=' + process.pid);
  return mod.default;
}

/** Agent class = the label's prefix, matching token-report.mjs's own grouping. */
function agentClass(label) {
  const i = label.indexOf(':');
  return i === -1 ? label : label.slice(0, i);
}

// The mechanical classes this phase acts on. `find`/`refute`/`plan`/`implement`
// /`act` are judgment agents and are explicitly out of scope.
const MECHANICAL_CLASSES = ['fetch', 'stamp', 'diff', 'model'];

function tally(labels) {
  const byLabel = new Map();
  for (const l of labels) {
    if (!MECHANICAL_CLASSES.includes(agentClass(l))) continue;
    byLabel.set(l, (byLabel.get(l) || 0) + 1);
  }
  return byLabel;
}

// --- The measurement ------------------------------------------------------

/**
 * Drive one dispatch twice over the SAME seed: once with the args a pre-change
 * caller supplied, once with the args the post-change shim supplies. The only
 * difference between the two runs is the args and the implementer's return
 * shape — the workflow file is identical, and it is the shipped one.
 */
async function measureDispatch(run, mode) {
  const isTask = mode === 'task';
  // `rdmBin` is REQUIRED by the workflow's fail-closed environment contract (no
  // ambient PATH fallback), so BOTH runs carry the explicit `'rdm'` sentinel.
  // It is identical across the two runs, so it cannot influence the delta.
  const baseArgs = isTask ? { task: 'my-task', rdmBin: 'rdm' } : { roadmap: 'rm', phase: '1', rdmBin: 'rdm' };

  const before = makeAgent({ absorbDiff: false });
  const outBefore = await run(baseArgs, before.agent, refPipeline, refParallel, () => {});

  const hoistedArgs = isTask
    ? { task: 'my-task', rdmBin: 'rdm', taskMeta: TASK_META, alreadyInProgress: true }
    : { roadmap: 'rm', phase: '1', rdmBin: 'rdm', phaseMeta: PHASE_META, alreadyInProgress: true };
  const after = makeAgent({ absorbDiff: true });
  const outAfter = await run(hoistedArgs, after.agent, refPipeline, refParallel, () => {});

  // The whole point of the change is that behaviour is untouched. If the two
  // runs disagree the measurement is meaningless, so refuse to report one.
  assert.deepEqual(
    outAfter,
    outBefore,
    'dispatch-phase (' + mode + '): the hoisted run returned a DIFFERENT OUTCOME than the unhoisted run — ' +
      'the hoists are supposed to be behaviour-neutral, so this measurement is void'
  );

  return { mode, before: tally(before.calls), after: tally(after.calls) };
}

/** Mean tokens per agent, per class, from the committed baseline. */
function baselineUnitCost() {
  const baseline = JSON.parse(fs.readFileSync(path.join(REPO_ROOT, 'docs/token-baseline.json'), 'utf8'));
  const byClass = new Map();
  for (const row of baseline.byAgentClass || []) {
    const total = (row.output || 0) + (row.uncachedInput || 0) + (row.cacheWrite || 0) + (row.cacheRead || 0);
    // Cache reads dominate every class's raw total and are the cheapest token
    // there is, so a total-only figure overstates what elimination saves.
    // Report the fresh (non-cache-read) figure alongside it and let the doc
    // quote both — never one without the other.
    const fresh = (row.output || 0) + (row.uncachedInput || 0) + (row.cacheWrite || 0);
    const n = row.agentCount || 0;
    if (n > 0) {
      byClass.set(row.key, {
        agentCount: n,
        totalTokens: total,
        meanTokens: Math.round(total / n),
        freshTokens: fresh,
        meanFreshTokens: Math.round(fresh / n),
      });
    }
  }
  return byClass;
}

function buildReport(results, unitCost) {
  const rows = [];
  const labels = new Set();
  for (const r of results) {
    for (const k of r.before.keys()) labels.add(r.mode + ' ' + k);
    for (const k of r.after.keys()) labels.add(r.mode + ' ' + k);
  }
  for (const key of [...labels].sort()) {
    const [mode, label] = key.split(' ');
    const r = results.find((x) => x.mode === mode);
    const before = r.before.get(label) || 0;
    const after = r.after.get(label) || 0;
    const cls = agentClass(label);
    const unit = unitCost.get(cls);
    rows.push({
      mode,
      label,
      class: cls,
      agentsBefore: before,
      agentsAfter: after,
      agentsEliminated: before - after,
      baselineMeanTokensPerAgent: unit ? unit.meanTokens : null,
      baselineMeanFreshTokensPerAgent: unit ? unit.meanFreshTokens : null,
      tokensEliminated: unit ? (before - after) * unit.meanTokens : null,
      freshTokensEliminated: unit ? (before - after) * unit.meanFreshTokens : null,
    });
  }
  const totals = {
    agentsBefore: rows.reduce((s, r) => s + r.agentsBefore, 0),
    agentsAfter: rows.reduce((s, r) => s + r.agentsAfter, 0),
    agentsEliminated: rows.reduce((s, r) => s + r.agentsEliminated, 0),
    tokensEliminated: rows.reduce((s, r) => s + (r.tokensEliminated || 0), 0),
    freshTokensEliminated: rows.reduce((s, r) => s + (r.freshTokensEliminated || 0), 0),
  };
  totals.percentEliminated = totals.agentsBefore
    ? Math.round((totals.agentsEliminated / totals.agentsBefore) * 1000) / 10
    : 0;
  return { rows, totals };
}

function renderText(report, unitCost) {
  const out = [];
  out.push('Measured mechanical-agent delta — dispatch-phase, post-change code, one dispatch per mode');
  out.push('');
  out.push('| mode | label | class | before | after | eliminated | baseline mean tok/agent | tokens eliminated | fresh tokens eliminated |');
  out.push('|---|---|---|---|---|---|---|---|---|');
  for (const r of report.rows) {
    out.push(
      '| ' +
        [
          r.mode,
          '`' + r.label + '`',
          r.class,
          r.agentsBefore,
          r.agentsAfter,
          r.agentsEliminated,
          r.baselineMeanTokensPerAgent === null ? '—' : r.baselineMeanTokensPerAgent.toLocaleString('en-US'),
          r.tokensEliminated === null ? '—' : r.tokensEliminated.toLocaleString('en-US'),
          r.freshTokensEliminated === null ? '—' : r.freshTokensEliminated.toLocaleString('en-US'),
        ].join(' | ') +
        ' |'
    );
  }
  out.push(
    '| **total** | | | **' +
      report.totals.agentsBefore +
      '** | **' +
      report.totals.agentsAfter +
      '** | **' +
      report.totals.agentsEliminated +
      '** | | **' +
      report.totals.tokensEliminated.toLocaleString('en-US') +
      '** | **' +
      report.totals.freshTokensEliminated.toLocaleString('en-US') +
      '** |'
  );
  out.push('');
  out.push(
    'Measured: ' +
      report.totals.agentsEliminated +
      ' of ' +
      report.totals.agentsBefore +
      ' mechanical subagents (' +
      report.totals.percentEliminated +
      '%) are no longer spawned per dispatch pair.'
  );
  out.push('');
  out.push('Baseline unit costs used (docs/token-baseline.json), per agent:');
  out.push('  class     all tokens   fresh (ex cache-read)   over N agents');
  for (const cls of MECHANICAL_CLASSES) {
    const u = unitCost.get(cls);
    if (u) {
      out.push(
        '  ' +
          cls.padEnd(8) +
          String(u.meanTokens).padStart(10) +
          String(u.meanFreshTokens).padStart(23) +
          String(u.agentCount).padStart(16)
      );
    }
  }
  out.push('');
  out.push('Cache reads dominate the raw totals and are the cheapest token there is, so the');
  out.push('"fresh" column is the decision-relevant one. Both are reported; neither alone.');
  return out.join('\n');
}

// --- CLI -----------------------------------------------------------------

function parseArgs(argv) {
  const args = { format: 'text', check: null };
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === '--format') {
      args.format = argv[++i];
    } else if (argv[i] === '--check') {
      const next = argv[i + 1];
      args.check = next && !next.startsWith('--') ? (i++, next) : 'docs/mechanical-agent-inventory.md';
    } else if (argv[i] === '--help' || argv[i] === '-h') {
      args.help = true;
    } else {
      throw new Error('unknown argument: ' + argv[i]);
    }
  }
  if (args.format !== 'text' && args.format !== 'json') {
    throw new Error('--format must be text or json');
  }
  return args;
}

const args = parseArgs(process.argv.slice(2));
if (args.help) {
  console.log('Usage: node scripts/measure-hoist-delta.mjs [--format text|json] [--check <doc>]');
  process.exit(0);
}

const run = await loadWorkflow('.claude/workflows/dispatch-phase.js');
const results = [await measureDispatch(run, 'phase'), await measureDispatch(run, 'task')];
const unitCost = baselineUnitCost();
const report = buildReport(results, unitCost);

if (args.check) {
  // The doc must carry the numbers this script computes, so it cannot drift
  // into being a stale hand-transcription — which is exactly the failure mode
  // the prior pass's "replay delta" prose had.
  // `resolve`, not `join`: a relative path still resolves against the repo root
  // (so the documented `--check docs/...` invocation works from any cwd), but an
  // ABSOLUTE path is honoured as given — which is what lets a harness point the
  // checker at a mutated copy outside the working tree for a self-test.
  const docPath = path.resolve(REPO_ROOT, args.check);
  const doc = fs.readFileSync(docPath, 'utf8');
  const missing = [];
  const need = [
    ['measured per-dispatch total before', String(report.totals.agentsBefore)],
    ['measured per-dispatch total after', String(report.totals.agentsAfter)],
    ['measured eliminated', String(report.totals.agentsEliminated)],
    ['measured tokens eliminated', report.totals.tokensEliminated.toLocaleString('en-US')],
    ['measured fresh tokens eliminated', report.totals.freshTokensEliminated.toLocaleString('en-US')],
  ];
  for (const [what, literal] of need) {
    if (!doc.includes(literal)) missing.push(what + ' (' + literal + ')');
  }
  for (const r of report.rows) {
    if (!doc.includes('`' + r.label + '`')) missing.push('a row for label ' + r.label);
  }
  if (missing.length) {
    console.error('measure-hoist-delta --check FAILED against ' + args.check + ':');
    for (const m of missing) console.error('  missing: ' + m);
    console.error('\nRe-run `node scripts/measure-hoist-delta.mjs` and update the doc.');
    process.exit(1);
  }
  console.log('measure-hoist-delta --check OK: ' + args.check + ' carries the measured figures');
  process.exit(0);
}

if (args.format === 'json') {
  console.log(JSON.stringify({ instrument: 'scripts/measure-hoist-delta.mjs', ...report }, null, 2));
} else {
  console.log(renderText(report, unitCost));
}
