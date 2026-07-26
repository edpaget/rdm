#!/usr/bin/env node
// measure-lane-tokens.mjs — CLI over scripts/lib/token-report.mjs.
//
// Reports token usage across Claude Code Workflow runs, broken out by token
// class (output / uncached input / cache write / cache read) and grouped by
// agent class, full label, model, and workflow — with the sidecar-vs-deduped
// totalTokens discrepancy always surfaced as its own named line.
//
// Usage:
//   node scripts/measure-lane-tokens.mjs [options]
//
// Options:
//   --since <iso-date>     Only include runs starting on/after this date.
//   --workflow <name>      Only include runs with this workflowName. Repeatable;
//                          repeats are OR'd (any match), NOT AND'd — this is the
//                          opposite of rdm's --tag convention, which ANDs
//                          repeated flags. Passing --workflow twice widens the
//                          result set, it does not narrow it.
//   --format text|json     Output format (default: text).
//   --out <path>           Write output to this file instead of stdout. The
//                          parent directory must already exist.
//   --root <dir>           Session-sidecar root to search (default:
//                          ~/.claude/projects). Override this in tests/hermetic
//                          harnesses to avoid touching real data.
//   --help                 Print this help and exit.

import fs from 'node:fs';
import path from 'node:path';
import { buildReport, defaultProjectsRoot } from './lib/token-report.mjs';

const HELP = `Usage: node scripts/measure-lane-tokens.mjs [options]

Options:
  --since <iso-date>   Only include runs starting on/after this date.
  --workflow <name>    Only include runs with this workflowName. Repeatable;
                        repeats are OR'd (any match), unlike rdm's --tag
                        convention which ANDs repeated flags.
  --format text|json   Output format (default: text).
  --out <path>         Write output to this file instead of stdout. The
                        parent directory must already exist.
  --root <dir>         Session-sidecar root to search (default:
                        ~/.claude/projects).
  --help                Print this help and exit.
`;

/**
 * @param {string[]} argv
 */
export function parseArgs(argv) {
  const args = { since: undefined, workflowNames: [], format: 'text', out: undefined, root: undefined, help: false };

  // Consume the next argv element as this flag's value. Throws an actionable
  // error rather than silently accepting `undefined` (or another flag's own
  // `--xyz` token) when the value was omitted — see cf-2: an omitted value
  // used to flow all the way into a Set/string comparison and silently filter
  // every run out, with no error and no warning.
  function takeValue(flag) {
    const next = argv[i + 1];
    if (next === undefined || next.startsWith('--')) {
      throw new Error(`${flag} requires a value`);
    }
    i += 1;
    return next;
  }

  let i;
  for (i = 0; i < argv.length; i++) {
    const a = argv[i];
    switch (a) {
      case '--since':
        args.since = takeValue('--since');
        break;
      case '--workflow':
        args.workflowNames.push(takeValue('--workflow'));
        break;
      case '--format':
        args.format = takeValue('--format');
        break;
      case '--out':
        args.out = takeValue('--out');
        break;
      case '--root':
        args.root = takeValue('--root');
        break;
      case '--help':
      case '-h':
        args.help = true;
        break;
      default:
        throw new Error(`unrecognized argument: "${a}"`);
    }
  }
  if (args.format !== 'text' && args.format !== 'json') {
    throw new Error(`--format must be "text" or "json", got "${args.format}"`);
  }
  return args;
}

function fmtGroup(title, rows) {
  const lines = [`-- ${title} --`];
  if (rows.length === 0) {
    lines.push('  (none)');
    return lines.join('\n');
  }
  for (const r of rows) {
    lines.push(
      `  ${r.key}  agents=${r.agentCount} requests=${r.dedupedRequestCount} ` +
        `output=${r.output} uncachedInput=${r.uncachedInput} cacheWrite=${r.cacheWrite} cacheRead=${r.cacheRead}`,
    );
  }
  return lines.join('\n');
}

/**
 * @param {ReturnType<typeof buildReport>} report
 * @param {string[]} warnings
 */
export function formatText(report, warnings) {
  const d = report.totalsDiscrepancy;
  const lines = [
    `Runs considered: ${report.runsConsidered}  Agent records: ${report.recordCount}`,
    '',
    `Sidecar totalTokens vs deduped-sum discrepancy: ${d.delta} ` +
      `(sidecar=${d.sidecarTotalTokens}, deduped=${d.dedupedTotalTokens})`,
    '',
    fmtGroup('By agent class', report.byAgentClass),
    '',
    fmtGroup('By label', report.byLabel),
    '',
    fmtGroup('By model', report.byModel),
    '',
    fmtGroup('By workflow', report.byWorkflow),
  ];
  if (warnings.length > 0) {
    lines.push('', '-- Warnings --');
    for (const w of warnings) lines.push(`  ${w}`);
  }
  return lines.join('\n');
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    process.stdout.write(HELP);
    return;
  }

  const warnings = [];
  const report = buildReport({
    root: args.root ?? defaultProjectsRoot(),
    since: args.since,
    workflowNames: args.workflowNames,
    warn: (msg) => warnings.push(msg),
  });

  const output =
    args.format === 'json' ? JSON.stringify({ ...report, warnings }, null, 2) : formatText(report, warnings);

  if (args.out) {
    const dir = path.dirname(args.out);
    if (!fs.existsSync(dir)) {
      throw new Error(`--out parent directory does not exist: "${dir}"`);
    }
    fs.writeFileSync(args.out, `${output}\n`);
  } else {
    console.log(output);
  }
}

const isMain = process.argv[1] && path.resolve(process.argv[1]) === path.resolve(new URL(import.meta.url).pathname);
if (isMain) {
  try {
    main();
  } catch (err) {
    console.error(`error: ${err.message}`);
    process.exit(1);
  }
}
