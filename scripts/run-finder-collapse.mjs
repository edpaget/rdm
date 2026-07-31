#!/usr/bin/env node
// run-finder-collapse.mjs — drive the collapsed-plan-finder A/B.
//
// ARM A is the CURRENT shape: three separate always-on plan finders
// (coherence, architectural-fit, restraint), each built by the REAL exported
// `findPrompt` from `.claude/workflows/lib/review.mjs` over the REAL always-on
// `DIMENSIONS.plan` entries. This runner NEVER carries a copy of the production
// prompt — a drifted copy would measure a strawman.
//
// ARM B is the CANDIDATE shape: one finder holding all three lenses, built by
// `buildCollapsedPlanPrompt`, which lives in the INSTRUMENT during the
// experiment so a no-ship decision leaves the lane byte-unchanged.
//
// COST WARNING: `--dispatch` spends real tokens
// (units x (lenses + 1) x replicates paid agents). `--dry-run` prints the plan
// and spends nothing; `--dispatch-stub` drives the real parsing path with an
// injected dispatcher; `--score` replays a saved run; `--audit` needs no corpus
// at all.
//
// Nothing under `.claude/workflows/` imports this. It imports FROM
// `.claude/workflows/lib/review.mjs` and never the other way round.
//
// Determinism: no clock, no RNG, and no network beyond the dispatcher.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import {
  ALWAYS_ON_PLAN_LENSES,
  POWER_FLOORS,
  DECISION_RULE,
  loadCorpus,
  loadAdjudication,
  assessPower,
  selectRunUnits,
  buildCollapseTrials,
  buildCollapsedPlanPrompt,
  dispatchTrial,
  runCollapseTrials,
  scoreCollapse,
  formatReport,
  auditCollapseDoc,
} from './lib/finder-collapse.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(HERE, '..');
const DEFAULT_CORPUS = 'tests/fixtures/finder-collapse/corpus.jsonl';
const DEFAULT_ADJUDICATION = 'tests/fixtures/finder-collapse/adjudication.jsonl';
const REVIEW_LIB = '.claude/workflows/lib/review.mjs';

/**
 * Load the REAL production prompt builder and the REAL always-on plan
 * dimensions. Fails loudly if the always-on set is not what the instrument was
 * pre-registered against — a silently-changed dimension list would make arm A a
 * different experiment from the one the committed figures describe.
 */
export async function loadRealPlanFindSurface(repoRoot = REPO_ROOT) {
  const mod = await import(pathToFileURL(path.join(repoRoot, REVIEW_LIB)).href);
  const alwaysOn = mod.DIMENSIONS.plan.filter((d) => !d.when);
  const keys = alwaysOn.map((d) => d.key);
  const merged = alwaysOn.filter((d) => Array.isArray(d.lenses));
  return {
    findPrompt: mod.findPrompt,
    planDimensions: alwaysOn,
    alwaysOnKeys: keys,
    // Present only once the merge has SHIPPED; the instrument then renders arm B
    // from review.mjs rather than from its own builder, and the harness asserts
    // the two renders are byte-identical.
    mergedDimension: merged.length ? merged[0] : null,
    calibration: mod.PLAN_SEVERITY_CALIBRATION || null,
  };
}

export function parseArgs(argv) {
  const args = {
    corpus: DEFAULT_CORPUS,
    adjudication: DEFAULT_ADJUDICATION,
    replicates: POWER_FLOORS.minReplicates,
    runUnits: POWER_FLOORS.minUnits,
    model: 'opus',
    concurrency: 4,
    dryRun: false,
    dispatch: false,
    dispatchStub: null,
    score: null,
    audit: null,
    out: null,
    allowUnderpowered: false,
    format: 'text',
    help: false,
  };
  let i;
  function takeValue(flag) {
    const next = argv[i + 1];
    if (next === undefined || next.startsWith('--')) throw new Error(`${flag} requires a value`);
    i += 1;
    return next;
  }
  function takePositiveInt(flag) {
    const raw = takeValue(flag);
    const n = Number(raw);
    if (!Number.isInteger(n) || n < 1) throw new Error(`${flag} must be a positive integer, got "${raw}"`);
    return n;
  }
  for (i = 0; i < argv.length; i++) {
    switch (argv[i]) {
      case '--corpus':
        args.corpus = takeValue('--corpus');
        break;
      case '--adjudication':
        args.adjudication = takeValue('--adjudication');
        break;
      case '--replicates':
        args.replicates = takePositiveInt('--replicates');
        break;
      case '--run-units':
        args.runUnits = takePositiveInt('--run-units');
        break;
      case '--concurrency':
        args.concurrency = takePositiveInt('--concurrency');
        break;
      case '--model':
        args.model = takeValue('--model');
        break;
      case '--dry-run':
        args.dryRun = true;
        break;
      case '--dispatch':
        args.dispatch = true;
        break;
      case '--dispatch-stub':
        args.dispatchStub = takeValue('--dispatch-stub');
        break;
      case '--score':
        args.score = takeValue('--score');
        break;
      case '--audit':
        args.audit = takeValue('--audit');
        break;
      case '--out':
        args.out = takeValue('--out');
        break;
      case '--allow-underpowered':
        args.allowUnderpowered = true;
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
  if (args.format !== 'text' && args.format !== 'json') {
    throw new Error(`--format must be "text" or "json", got "${args.format}"`);
  }
  return args;
}

const HELP = `Usage: node scripts/run-finder-collapse.mjs [options]

A/B the COLLAPSED plan finder (one agent, three lenses) against the CURRENT
shape (three always-on finder agents), over real mined plan documents.
See docs/finder-collapse.md for the pre-registered decision rule.

COST WARNING: --dispatch spends real tokens
(units x (${ALWAYS_ON_PLAN_LENSES.length} + 1) x replicates paid agents).

Modes (pick one; --dry-run is the default):
  --dry-run              Build and print the trial plan; dispatch NOTHING.
  --dispatch             Dispatch both arms through \`claude -p\`. PAID.
  --dispatch-stub <mod>  Inject a dispatcher module (default export or
                         \`dispatch\`) instead of \`claude\`; drives the real
                         parsing/scoring path with no spend.
  --score <trials.json>  Score a saved run; dispatch nothing.
  --audit <doc.json>     Corpus-free arithmetic audit of the committed
                         planFinderCollapse figures; reads nothing else.

Options:
  --corpus <path>        Mined review-unit corpus (default: ${DEFAULT_CORPUS}).
  --adjudication <path>  Hand adjudication JSONL (default: ${DEFAULT_ADJUDICATION}).
  --replicates <n>       Replicates per arm per unit (default: ${POWER_FLOORS.minReplicates}).
  --run-units <n>        Review units to run (default: ${POWER_FLOORS.minUnits}).
  --concurrency <n>      Parallel dispatches (default: 4).
  --model <id>           Model for BOTH arms (default: opus).
  --out <path>           Write the trials/report JSON here.
  --allow-underpowered   Build the plan anyway; stamps NO MEASUREMENT and
                         suppresses the decision line.
  --format text|json     Report format (default: text).
  --help                 Print this help and exit.
`;

function readFileOrThrow(p, what) {
  try {
    return fs.readFileSync(p, 'utf8');
  } catch (err) {
    throw new Error(`could not read the ${what} at "${p}": ${err.message}`);
  }
}

/**
 * CLI entry point. Returns the process exit code; every failure surfaces as a
 * thrown `Error` whose message is actionable.
 *
 * @param {string[]} argv
 * @returns {Promise<number>}
 */
export async function main(argv) {
  const args = parseArgs(argv);
  if (args.help) {
    console.log(HELP);
    return 0;
  }

  // --audit needs NO corpus, no trials and no adjudication: the committed
  // figures must be gateable in CI with none of the measurement inputs present.
  if (args.audit) {
    const doc = JSON.parse(readFileOrThrow(path.resolve(args.audit), 'baseline document'));
    const result = auditCollapseDoc(doc.planFinderCollapse);
    if (args.format === 'json') console.log(JSON.stringify(result, null, 2));
    else {
      for (const c of result.checks) console.log(`  ok   ${c}`);
      for (const e of result.errors) console.log(`  FAIL ${e}`);
      console.log(result.ok ? 'audit: planFinderCollapse figures are internally consistent' : 'audit: FAILED');
    }
    return result.ok ? 0 : 1;
  }

  const adjudicationText = fs.existsSync(path.resolve(args.adjudication))
    ? fs.readFileSync(path.resolve(args.adjudication), 'utf8')
    : '';
  const { rows: adjudication, errors: adjErrors } = loadAdjudication(adjudicationText);
  if (adjErrors.length) throw new Error(`adjudication is invalid:\n  ${adjErrors.join('\n  ')}`);

  // --score replays a saved run. It still needs the corpus only for the power
  // record, which the saved run already carries, so it reads nothing else.
  if (args.score) {
    const run = JSON.parse(readFileOrThrow(path.resolve(args.score), 'saved trials file'));
    const report = scoreCollapse(run, adjudication, {
      noMeasurement: run.noMeasurement === true,
      power: run.power,
    });
    const text = formatReport(report, args.format);
    if (args.out) fs.writeFileSync(path.resolve(args.out), JSON.stringify(report, null, 2));
    console.log(text);
    return 0;
  }

  const corpusText = readFileOrThrow(path.resolve(args.corpus), 'corpus');
  const corpus = loadCorpus(corpusText);
  if (corpus.errors.length) throw new Error(`corpus is invalid:\n  ${corpus.errors.join('\n  ')}`);

  const surface = await loadRealPlanFindSurface();
  const plan = buildCollapseTrials(corpus, {
    findPrompt: surface.findPrompt,
    planDimensions: surface.mergedDimension ? surface.mergedDimension.lenses : surface.planDimensions,
    calibration: surface.calibration,
    replicates: args.replicates,
    runUnits: args.runUnits,
    model: args.model,
    allowUnderpowered: args.allowUnderpowered,
  });

  if (!args.dispatch && !args.dispatchStub) {
    // --dry-run (the default): print the plan, spend nothing.
    console.log(`POWER: ${plan.power.power}`);
    for (const r of plan.power.reasons) console.log(`  - ${r}`);
    console.log(
      `Corpus ${corpus.units.length} unit(s); run population ${plan.runUnits.length} unit(s) ` +
        `by target type ${JSON.stringify(plan.power.byTargetType)}`
    );
    console.log(
      `${plan.trials.length} trial(s) = ${plan.runUnits.length} unit(s) x ` +
        `(${ALWAYS_ON_PLAN_LENSES.length} arm-A lens finder(s) + 1 arm-B collapsed finder) x ${args.replicates} replicate(s). ` +
        'Nothing dispatched.'
    );
    for (const t of plan.trials) {
      console.log(`  ${t.trialId}  (${plan.prompts.get(t.trialId).length} prompt chars)`);
    }
    return 0;
  }

  let dispatch = dispatchTrial;
  if (args.dispatchStub) {
    const mod = await import(pathToFileURL(path.resolve(args.dispatchStub)).href);
    dispatch = mod.dispatch || mod.default;
    if (typeof dispatch !== 'function') {
      throw new Error(`--dispatch-stub module "${args.dispatchStub}" exports neither \`dispatch\` nor a default function`);
    }
  }

  const results = await runCollapseTrials(plan.trials, plan.prompts, dispatch, {
    concurrency: args.concurrency,
    log: (m) => console.error(m),
  });

  const run = {
    instrument: 'scripts/run-finder-collapse.mjs',
    corpusWindow: corpus.header ? corpus.header.window : null,
    model: args.model,
    replicates: args.replicates,
    runUnitIds: plan.runUnits.map((u) => u.id),
    power: plan.power,
    noMeasurement: plan.noMeasurement,
    rule: DECISION_RULE,
    trials: results,
  };
  if (args.out) fs.writeFileSync(path.resolve(args.out), JSON.stringify(run, null, 2) + '\n');

  const report = scoreCollapse(run, adjudication, { noMeasurement: plan.noMeasurement, power: plan.power });
  console.log(formatReport(report, args.format));
  return 0;
}

// Re-exported so the harness can drive the pure surface without a subprocess.
export { assessPower, selectRunUnits, buildCollapsedPlanPrompt, scoreCollapse, formatReport, auditCollapseDoc };

// `import.meta.main` is not available on the pinned node, so gate on argv[1].
const invokedDirectly = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invokedDirectly) {
  main(process.argv.slice(2))
    .then((code) => process.exit(code))
    .catch((err) => {
      console.error(String(err && err.message ? err.message : err));
      process.exit(1);
    });
}
