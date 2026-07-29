#!/usr/bin/env node
// run-refuter-agreement.mjs — dispatch the checked-in finding corpus through the
// REAL refuter prompt on two or more model tiers and score the result.
//
// WHY THIS EXISTS
// ------------------------------------------------------------------------
// Refuters run on the most expensive tier everywhere. Deciding whether they
// must needs evidence, not an anecdote: a corpus with recorded ground truth, a
// byte-identical replay of the real prompt, replicates so a coin-flip cannot
// masquerade as agreement, and FALSE-NEGATIVE / FALSE-POSITIVE rates reported
// separately because a false negative ships a defect and a false positive costs
// a rework round.
//
// COST WARNING
// ------------------------------------------------------------------------
// A real run DISPATCHES PAID AGENTS — one per corpus item per tier per
// replicate. Use --dry-run to see the plan and spend nothing, --dispatch-stub
// to drive the whole path hermetically, and --score-only to re-score a saved
// results file. `scripts/verify-refuter-agreement.sh` never dispatches.
//
// ON-DEMAND ONLY
// ------------------------------------------------------------------------
// Nothing under `.claude/workflows/` imports this. It imports FROM
// `.claude/workflows/lib/review.mjs` and never the other way round, so it can
// never be wired into the lane's hot path.
//
// Usage:
//   node scripts/run-refuter-agreement.mjs --tiers opus,sonnet --replicates 2
//
// Options:
//   --corpus <path>        Corpus JSONL (default: tests/fixtures/refuter-agreement/corpus.jsonl).
//   --tiers <a,b>          Repeatable/comma-separated model ids or aliases. First is the baseline.
//   --replicates <n>       Replicates per (item, tier). Default 2.
//   --filter-class <c>     Repeatable/comma-separated groundTruth.class filter.
//   --filter-severity <s>  Repeatable/comma-separated finding.severity filter.
//   --only <id,...>        Run exactly these corpus ids (errors on an unknown id).
//   --limit <n>            Cap the corpus to the first n items after filtering.
//   --label <s>            Run label. Defaults to the corpus sha — NEVER the clock.
//   --out <path>           Write the results JSON here.
//   --format text|json     Report format (default: text).
//   --dry-run              Build and print the trial plan; dispatch nothing.
//   --dispatch-stub <mod>  Inject a dispatcher module (default export or `dispatch`) instead of `claude`.
//   --score-only <path>    Score a saved results JSON; dispatch nothing.
//   --audit <doc>          Corpus-free arithmetic audit of a doc's refuterModelTiering section.
//   --help                 Print this help and exit.
//
// Determinism: no clock, no RNG, and no network beyond the dispatcher the
// operator explicitly asks for. The trial plan and the report are a pure
// function of the corpus, the tiers, and the replicate count.

import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';
import {
  loadCorpus,
  buildTrials,
  scoreTrials,
  formatReport,
  regeneratePrompt,
  checkPromptFidelity,
  sha256,
  GROUND_TRUTH_CLASSES,
  TOKEN_CLASSES,
  auditTieringSection,
} from './lib/refuter-agreement.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(HERE, '..');
const DEFAULT_CORPUS = 'tests/fixtures/refuter-agreement/corpus.jsonl';

// The tier aliases `agent()` accepts, plus a passthrough for a concrete model
// id. An unrecognised tier must fail AT PLAN TIME, before a single dispatch —
// otherwise a whole expensive run comes back as null verdicts that look like
// refuter crashes rather than a typo'd flag.
const KNOWN_TIER_ALIASES = ['opus', 'sonnet', 'haiku'];

export function isLegalTier(tier) {
  return KNOWN_TIER_ALIASES.indexOf(tier) !== -1 || /^claude-[a-z0-9.\-[\]]+$/i.test(tier);
}

// --- Dispatch --------------------------------------------------------------

/**
 * Dispatch one trial through `claude -p`, returning the verdict and the usage
 * broken into the same four token classes `scripts/lib/token-report.mjs` uses,
 * plus the tool-call count (13-20 calls for Opus vs 7-9 for Sonnet is what
 * explained the initial A/B's divergence, so it is a first-class column).
 *
 * Never throws for a per-trial failure: a long, expensive run must not abort on
 * one bad response. An unparseable or non-boolean `refuted` records
 * `verdict: null` and is bucketed as `ungraded` — coercing it to `false` would
 * silently inflate the false-positive rate.
 */
export async function claudeDispatch(trial, prompt, opts = {}) {
  // ASYNC on purpose. A synchronous spawn would block Node's single thread and
  // silently defeat --concurrency, turning a bounded-parallel run back into a
  // serial one with no error to show for it.
  const res = await runProcess('claude', ['-p', '--model', trial.tier, '--output-format', 'json'], {
    input: prompt,
    cwd: opts.cwd || REPO_ROOT,
  });
  if (res.error && res.error.code === 'ENOENT') {
    throw new Error(
      'the `claude` binary was not found on PATH. The refuter-agreement runner dispatches real agents ' +
        'through `claude -p`; install/authenticate the CLI, or use --dry-run / --dispatch-stub / --score-only.'
    );
  }
  if (res.error) {
    return { verdict: null, error: `claude failed to start: ${res.error.message}`, usage: {}, toolCalls: 0 };
  }
  if (res.status !== 0) {
    return {
      verdict: null,
      error: `claude exited ${res.status}: ${String(res.stderr || '').trim().slice(0, 400)}`,
      usage: {},
      toolCalls: 0,
    };
  }
  let body;
  try {
    body = JSON.parse(res.stdout);
  } catch (err) {
    return { verdict: null, error: `claude returned a non-JSON body: ${err.message}`, usage: {}, toolCalls: 0 };
  }
  const parsed = parseClaudeResult(body);
  // `claude -p --output-format json` reports usage but not a tool-call count.
  // Recover it from the session transcript the run just wrote — the same
  // full-fidelity source scripts/lib/token-report.mjs reads. Degrades to the
  // turn count rather than throwing, so a missing transcript costs one column
  // and not the trial.
  if (parsed.toolCalls === 0 && typeof body.session_id === 'string') {
    const counted = countSessionToolUses(body.session_id, opts.cwd || REPO_ROOT);
    if (counted !== null) parsed.toolCalls = counted;
  }
  return parsed;
}

/** Promise wrapper over `spawn`, collecting stdout/stderr. Never rejects. */
function runProcess(cmd, argv, opts = {}) {
  return new Promise((resolve) => {
    let child;
    try {
      child = spawn(cmd, argv, { cwd: opts.cwd, stdio: ['pipe', 'pipe', 'pipe'] });
    } catch (error) {
      resolve({ error, status: null, stdout: '', stderr: '' });
      return;
    }
    let stdout = '';
    let stderr = '';
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', (d) => {
      stdout += d;
    });
    child.stderr.on('data', (d) => {
      stderr += d;
    });
    child.on('error', (error) => resolve({ error, status: null, stdout, stderr }));
    child.on('close', (status) => resolve({ error: null, status, stdout, stderr }));
    child.stdin.on('error', () => {});
    child.stdin.end(opts.input ?? '');
  });
}

/** Project-slug directory name Claude Code uses for a working directory. */
export function projectSlugFor(cwd) {
  return String(cwd).replace(/[^A-Za-z0-9]/g, '-');
}

/**
 * Count `tool_use` blocks in a finished session's transcript.
 * Returns null when the transcript cannot be located or read.
 */
export function countSessionToolUses(sessionId, cwd, projectsRoot) {
  const root = projectsRoot || path.join(process.env.HOME || '', '.claude', 'projects');
  const file = path.join(root, projectSlugFor(cwd), `${sessionId}.jsonl`);
  let raw;
  try {
    raw = fs.readFileSync(file, 'utf8');
  } catch {
    return null;
  }
  let n = 0;
  for (const line of raw.split('\n')) {
    const t = line.trim();
    if (!t) continue;
    let entry;
    try {
      entry = JSON.parse(t);
    } catch {
      continue;
    }
    if (entry.type !== 'assistant') continue;
    const content = (entry.message && entry.message.content) || [];
    if (!Array.isArray(content)) continue;
    for (const block of content) if (block && block.type === 'tool_use') n += 1;
  }
  return n;
}

/**
 * Pull `{ verdict, usage, toolCalls }` out of a `claude -p --output-format json`
 * body. Exported so the harness can drive it without spawning anything.
 */
export function parseClaudeResult(body) {
  const usage = {};
  for (const c of TOKEN_CLASSES) usage[c] = 0;
  const u = (body && body.usage) || {};
  usage.output = Number(u.output_tokens || 0);
  usage.uncachedInput = Number(u.input_tokens || 0);
  usage.cacheWrite = Number(u.cache_creation_input_tokens || 0);
  usage.cacheRead = Number(u.cache_read_input_tokens || 0);

  let toolCalls = 0;
  let verdict = null;
  const messages = Array.isArray(body && body.messages) ? body.messages : [];
  for (const m of messages) {
    const content = m && m.message && Array.isArray(m.message.content) ? m.message.content : [];
    for (const block of content) {
      if (block && block.type === 'tool_use') {
        toolCalls += 1;
        if (block.name === 'StructuredOutput' && block.input && typeof block.input.refuted === 'boolean') {
          verdict = {
            refuted: block.input.refuted,
            confidence: typeof block.input.confidence === 'number' ? block.input.confidence : null,
            rationale: typeof block.input.rationale === 'string' ? block.input.rationale : null,
          };
        }
      }
    }
  }
  if (verdict === null) {
    // The `claude -p --output-format json` shape carries the answer as a
    // top-level `result` string, not as a StructuredOutput tool_use. A model
    // that wraps it in prose or a fence still parses; one that returns no
    // boolean `refuted` at all stays null and is bucketed `ungraded` — NEVER
    // coerced to false, which would silently inflate the false-positive rate.
    const parsed = tryParseJson(body.result) || tryParseEmbeddedJson(body.result);
    if (parsed && typeof parsed.refuted === 'boolean') {
      verdict = {
        refuted: parsed.refuted,
        confidence: typeof parsed.confidence === 'number' ? parsed.confidence : null,
        rationale: typeof parsed.rationale === 'string' ? parsed.rationale : null,
      };
    }
  }
  if (typeof body.num_tool_uses === 'number' && toolCalls === 0) toolCalls = body.num_tool_uses;
  return { verdict, usage, toolCalls, error: verdict === null ? 'no boolean `refuted` in the response' : null };
}

function tryParseJson(text) {
  if (typeof text !== 'string') return null;
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

/** Last resort: the first balanced `{...}` span inside a prose/fenced answer. */
function tryParseEmbeddedJson(text) {
  if (typeof text !== 'string') return null;
  const start = text.indexOf('{');
  if (start === -1) return null;
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
    else if (ch === '}' && --depth === 0) return tryParseJson(text.slice(start, i + 1));
  }
  return null;
}

// --- Run -------------------------------------------------------------------

/**
 * Execute the trial plan. Dependency-injected so the harness can drive the
 * whole path with no subprocess and no spend.
 */
export async function runTrials(items, trials, prompts, dispatch, opts = {}) {
  const log = opts.log || (() => {});
  const concurrency = Math.max(1, opts.concurrency || 1);
  // Results are written into an INDEXED slot, never pushed, so the output order
  // is the trial-plan order regardless of completion order — concurrency must
  // not make a run non-reproducible.
  const results = new Array(trials.length);
  let next = 0;

  async function worker() {
    for (;;) {
      const i = next++;
      if (i >= trials.length) return;
      const t = trials[i];
      const prompt = prompts.get(t.corpusId);
      // A THROWN dispatcher is fatal (a missing binary, a bad tier) — that is a
      // setup error, not a per-trial failure, and must not be swallowed.
      const outcome = await dispatch(t, prompt);
      results[i] = {
        trialId: t.trialId,
        corpusId: t.corpusId,
        tier: t.tier,
        replicate: t.replicate,
        verdict: outcome && outcome.verdict ? outcome.verdict : null,
        error: (outcome && outcome.error) || null,
        usage: (outcome && outcome.usage) || {},
        toolCalls: (outcome && outcome.toolCalls) || 0,
      };
      log(`trial ${t.trialId}: ${outcome && outcome.verdict ? 'refuted=' + outcome.verdict.refuted : 'ungraded'}`);
    }
  }

  await Promise.all(Array.from({ length: Math.min(concurrency, trials.length) }, worker));
  return results;
}

// --- CLI -------------------------------------------------------------------

export function parseArgs(argv) {
  const args = {
    corpus: DEFAULT_CORPUS,
    tiers: [],
    replicates: 2,
    filterClass: [],
    filterSeverity: [],
    only: [],
    limit: undefined,
    label: null,
    out: null,
    format: 'text',
    dryRun: false,
    dispatchStub: null,
    concurrency: 1,
    scoreOnly: null,
    audit: null,
    help: false,
  };
  let i;
  function takeValue(flag) {
    const next = argv[i + 1];
    if (next === undefined || next.startsWith('--')) throw new Error(`${flag} requires a value`);
    i += 1;
    return next;
  }
  const split = (v) => v.split(',').map((s) => s.trim()).filter(Boolean);
  for (i = 0; i < argv.length; i++) {
    switch (argv[i]) {
      case '--corpus':
        args.corpus = takeValue('--corpus');
        break;
      case '--tiers':
        args.tiers.push(...split(takeValue('--tiers')));
        break;
      case '--replicates': {
        const raw = takeValue('--replicates');
        const n = Number(raw);
        if (!Number.isInteger(n) || n < 1) throw new Error(`--replicates must be a positive integer, got "${raw}"`);
        args.replicates = n;
        break;
      }
      case '--filter-class':
        args.filterClass.push(...split(takeValue('--filter-class')));
        break;
      case '--filter-severity':
        args.filterSeverity.push(...split(takeValue('--filter-severity')));
        break;
      case '--only':
        args.only.push(...split(takeValue('--only')));
        break;
      case '--limit': {
        const raw = takeValue('--limit');
        const n = Number(raw);
        if (!Number.isInteger(n) || n < 1) throw new Error(`--limit must be a positive integer, got "${raw}"`);
        args.limit = n;
        break;
      }
      case '--label':
        args.label = takeValue('--label');
        break;
      case '--out':
        args.out = takeValue('--out');
        break;
      case '--format':
        args.format = takeValue('--format');
        break;
      case '--dry-run':
        args.dryRun = true;
        break;
      case '--dispatch-stub':
        args.dispatchStub = takeValue('--dispatch-stub');
        break;
      case '--concurrency': {
        const raw = takeValue('--concurrency');
        const n = Number(raw);
        if (!Number.isInteger(n) || n < 1) throw new Error(`--concurrency must be a positive integer, got "${raw}"`);
        args.concurrency = n;
        break;
      }
      case '--score-only':
        args.scoreOnly = takeValue('--score-only');
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
  for (const c of args.filterClass) {
    if (GROUND_TRUTH_CLASSES.indexOf(c) === -1) {
      throw new Error(`--filter-class "${c}" is not a known ground-truth class (${GROUND_TRUTH_CLASSES.join(', ')})`);
    }
  }
  // Fail loudly AT PLAN TIME on an unusable tier, never after N dispatches.
  for (const t of args.tiers) {
    if (!isLegalTier(t)) {
      throw new Error(
        `--tiers value "${t}" is not a tier alias (${KNOWN_TIER_ALIASES.join('|')}) or a claude-* model id. ` +
          'An unknown model id makes every dispatch return null, which looks like a refuter crash rather than a typo.'
      );
    }
  }
  return args;
}

const HELP = `Usage: node scripts/run-refuter-agreement.mjs [options]

Dispatches the checked-in finding corpus through the REAL refutePrompt on two or
more model tiers, with replicates, and reports FALSE-NEGATIVE and FALSE-POSITIVE
rates SEPARATELY per tier alongside per-tier token cost and tool-call counts.

COST WARNING: a real run dispatches paid agents (items x tiers x replicates).

Options:
  --corpus <path>        Corpus JSONL (default: ${DEFAULT_CORPUS}).
  --tiers <a,b>          Repeatable/comma-separated tiers. First is the baseline.
  --replicates <n>       Replicates per (item, tier). Default 2.
  --filter-class <c>     Repeatable/comma-separated groundTruth.class filter.
  --filter-severity <s>  Repeatable/comma-separated finding.severity filter.
  --only <id,...>        Run exactly these corpus ids (errors on an unknown id).
  --limit <n>            Cap the corpus after filtering.
  --label <s>            Run label. Defaults to the corpus sha, never the clock.
  --out <path>           Write the results JSON here.
  --format text|json     Report format (default: text).
  --dry-run              Print the trial plan; dispatch NOTHING.
  --dispatch-stub <mod>  Inject a dispatcher module instead of \`claude\`.
  --concurrency <n>      Dispatch n trials at a time (default 1). Output order is unaffected.
  --score-only <path>    Score a saved results JSON; dispatch nothing.
  --audit <doc>          Corpus-free arithmetic audit of refuterModelTiering.
  --help                 Print this help and exit.
`;

function resolveRepoPath(p) {
  return path.isAbsolute(p) ? p : path.resolve(REPO_ROOT, p);
}

async function main(argv) {
  const args = parseArgs(argv);
  if (args.help) {
    console.log(HELP);
    return 0;
  }

  if (args.audit) {
    const docPath = resolveRepoPath(args.audit);
    let parsed;
    try {
      parsed = JSON.parse(fs.readFileSync(docPath, 'utf8'));
    } catch (err) {
      console.error(`${args.audit} is not parseable JSON (--audit expects docs/token-baseline.json): ${err.message}`);
      return 1;
    }
    const section = parsed.refuterModelTiering;
    if (!section) {
      console.error(`${args.audit} has no "refuterModelTiering" section`);
      return 1;
    }
    const problems = auditTieringSection(section);
    if (problems.length) {
      console.error('run-refuter-agreement --audit FAILED against ' + args.audit + ':');
      for (const p of problems) console.error('  ' + p);
      return 1;
    }
    console.log("run-refuter-agreement --audit OK: " + args.audit + "'s refuterModelTiering figures are internally consistent");
    return 0;
  }

  const corpusPath = resolveRepoPath(args.corpus);
  const corpusText = fs.readFileSync(corpusPath, 'utf8');
  const { items: allItems, errors } = loadCorpus(corpusText);
  if (errors.length) {
    console.error(`corpus ${args.corpus} has ${errors.length} validation error(s):`);
    for (const e of errors.slice(0, 25)) console.error('  ' + e);
    return 1;
  }

  let items = allItems;
  if (args.filterClass.length) items = items.filter((i) => args.filterClass.indexOf(i.groundTruth.class) !== -1);
  if (args.filterSeverity.length) items = items.filter((i) => args.filterSeverity.indexOf(i.finding.severity) !== -1);
  if (args.only.length) {
    const wanted = new Set(args.only);
    const found = new Set();
    items = items.filter((i) => {
      if (!wanted.has(i.id)) return false;
      found.add(i.id);
      return true;
    });
    const missingIds = args.only.filter((id) => !found.has(id));
    // A typo'd --only id must fail LOUDLY, never silently shrink an expensive run.
    if (missingIds.length) throw new Error('--only names ' + missingIds.length + ' id(s) not in the corpus: ' + missingIds.join(', '));
  }
  if (args.limit !== undefined) items = items.slice(0, args.limit);

  if (args.scoreOnly) {
    const saved = JSON.parse(fs.readFileSync(resolveRepoPath(args.scoreOnly), 'utf8'));
    const report = scoreTrials(items, saved.trials || [], { baselineTier: saved.baselineTier });
    console.log(formatReport(report, args.format));
    return 0;
  }

  // Regenerate every prompt through the REAL refutePrompt and sha-compare it
  // against the recorded original. A mismatch is REPORTED (promptDrift), never
  // silently accepted: it means refutePrompt changed after the corpus was mined
  // and replay is no longer byte-identical to history.
  const { refutePrompt } = await import(pathToFileURL(path.join(REPO_ROOT, '.claude/workflows/lib/review.mjs')).href);
  const prompts = new Map();
  const drifted = [];
  for (const item of items) {
    prompts.set(item.id, regeneratePrompt(item, refutePrompt));
    if (checkPromptFidelity(item, refutePrompt).drifted) drifted.push(item.id);
  }
  if (drifted.length) {
    console.error(
      `WARNING: ${drifted.length} corpus item(s) no longer regenerate byte-identically through refutePrompt ` +
        `(promptDrift): ${drifted.slice(0, 8).join(', ')}${drifted.length > 8 ? ', …' : ''}`
    );
  }

  if (args.tiers.length === 0) throw new Error('--tiers is required (e.g. --tiers opus,sonnet)');
  const trials = buildTrials(items, { tiers: args.tiers, replicates: args.replicates });
  // The run label is operator-supplied or derived from the corpus — NEVER
  // generated from the clock, so two runs over the same corpus are comparable.
  const label = args.label || sha256(corpusText).slice(0, 12);

  if (args.dryRun) {
    console.log(`DRY RUN — label ${label}: ${items.length} item(s) x ${args.tiers.length} tier(s) x ${args.replicates} replicate(s) = ${trials.length} trial(s). Nothing dispatched.`);
    for (const t of trials) console.log(`  ${t.trialId}  (${prompts.get(t.corpusId).length} prompt chars)`);
    return 0;
  }

  let dispatch = claudeDispatch;
  if (args.dispatchStub) {
    const mod = await import(pathToFileURL(resolveRepoPath(args.dispatchStub)).href);
    dispatch = mod.dispatch || mod.default;
    if (typeof dispatch !== 'function') {
      throw new Error(`--dispatch-stub module "${args.dispatchStub}" exports neither \`dispatch\` nor a default function`);
    }
  }

  const results = await runTrials(items, trials, prompts, dispatch, {
    log: (m) => console.error(m),
    concurrency: args.concurrency,
  });
  const report = scoreTrials(items, results, { baselineTier: args.tiers[0] });
  const payload = {
    instrument: 'scripts/run-refuter-agreement.mjs',
    label,
    corpusPath: args.corpus,
    corpusSha256: sha256(corpusText),
    tiers: args.tiers,
    replicates: args.replicates,
    baselineTier: args.tiers[0],
    promptDrift: drifted,
    trials: results,
    report,
  };
  if (args.out) fs.writeFileSync(resolveRepoPath(args.out), JSON.stringify(payload, null, 2) + '\n');
  console.log(formatReport(report, args.format));
  return 0;
}

// `import.meta.main` is not available on the pinned node, so gate on argv[1].
const invokedDirectly = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invokedDirectly) {
  main(process.argv.slice(2)).then(
    (code) => process.exit(code),
    (err) => {
      console.error(String(err && err.message ? err.message : err));
      process.exit(1);
    }
  );
}
