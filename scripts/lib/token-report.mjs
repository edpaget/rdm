// token-report.mjs — pure, stdlib-only Node module for measuring token usage
// across Claude Code Workflow runs recorded in `~/.claude/projects/**`.
//
// This module is the read side of the token-reduction measurement harness: it
// locates workflow-run sidecar files (`workflows/wf_*.json`) and their
// per-agent transcripts (`subagents/workflows/<runId>/agent-*.jsonl`), joins
// them into flat per-agent records broken out by token class, and aggregates
// those records by agent class / full label / model / workflow — plus a
// per-agent-class "first transcript request" floor (median/min/p10/mean of
// `firstRequestTokens`, matching `docs/token-baseline.json`'s
// `agentContextFloor.measuredFloor` definition verbatim). No network, no
// third-party packages — `node:fs`, `node:path`, `node:os` only.
//
// Every fact this module relies on about the on-disk shape was verified by
// direct inspection of real sidecar/transcript files (not assumed from the
// phase body alone):
//
//   - A `wf_*.json` file's `runId` field equals its own filename stem
//     (`wf_<x>.json` -> `runId: "wf_<x>"`), and the matching transcript
//     directory is `subagents/workflows/<runId>/` using that SAME full runId
//     (including the `wf_` prefix) — not a stripped version of it.
//   - `workflow_agent` entries that were served from cache (`cached: true`)
//     carry no `tokens`, `toolCalls`, or `durationMs` field at all — they are
//     genuinely zero-cost, not "unmeasured".
//   - A `wf_*.json`'s top-level `startTime` is a numeric epoch-ms value (its
//     sibling `timestamp` field is the ISO string); this module compares
//     `--since` against `startTime` as a millisecond epoch.
//   - Inside an `agent-*.jsonl` transcript, the SAME `requestId` appears on
//     several consecutive `type: "assistant"` lines while a request streams:
//     `input_tokens` / `cache_creation_input_tokens` / `cache_read_input_tokens`
//     stay constant across those lines while `output_tokens` climbs to its
//     final value only on the LAST line for that `requestId`. Last-write-wins
//     dedupe by `requestId` is therefore required — summing every line, or
//     keeping only the first, both overcount or undercount output tokens.
//   - `type: "user"` transcript lines (prompts, tool results, attachments)
//     carry no `requestId` and no `message.usage` and must be skipped.

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

/** Default session-sidecar root: `~/.claude/projects`. */
export function defaultProjectsRoot() {
  return path.join(os.homedir(), '.claude', 'projects');
}

/**
 * Find every session directory under `projectsRoot` that carries a
 * `workflows/` subdirectory, searching across ALL project-slug directories
 * (including `--worktrees-`-named ones — a worktree gets its own project-slug
 * directory, so the session directory is not derivable from `cwd` and must be
 * located by this kind of directory walk).
 *
 * @param {string} projectsRoot
 * @returns {Array<{ projectSlug: string, sessionId: string, sessionDir: string }>}
 */
export function locateSessionDirs(projectsRoot) {
  let projectEntries;
  try {
    projectEntries = fs.readdirSync(projectsRoot, { withFileTypes: true });
  } catch (err) {
    throw new Error(`cannot read projects root "${projectsRoot}": ${err.message}`);
  }

  const sessions = [];
  for (const projEnt of projectEntries) {
    if (!projEnt.isDirectory()) continue;
    const projectSlug = projEnt.name;
    const projectDir = path.join(projectsRoot, projectSlug);

    let sessionEntries;
    try {
      sessionEntries = fs.readdirSync(projectDir, { withFileTypes: true });
    } catch {
      continue;
    }

    for (const sessEnt of sessionEntries) {
      if (!sessEnt.isDirectory()) continue;
      const sessionId = sessEnt.name;
      const sessionDir = path.join(projectDir, sessionId);
      const workflowsDir = path.join(sessionDir, 'workflows');
      let isDir = false;
      try {
        isDir = fs.statSync(workflowsDir).isDirectory();
      } catch {
        isDir = false;
      }
      if (isDir) {
        sessions.push({ projectSlug, sessionId, sessionDir });
      }
    }
  }
  return sessions;
}

/**
 * Parse one `wf_*.json` workflow-run sidecar file.
 *
 * @param {string} filePath
 * @returns {{ ok: true, filePath: string, runId: string, workflowName: string,
 *   status: string, agentCount: number, totalTokens: number,
 *   totalToolCalls: number, durationMs: number, defaultModel: string,
 *   startTimeMs: number|undefined, agents: object[] }
 *   | { ok: false, filePath: string, error: string }}
 */
export function parseWorkflowRun(filePath) {
  let raw;
  try {
    raw = fs.readFileSync(filePath, 'utf8');
  } catch (err) {
    return { ok: false, filePath, error: `read failed: ${err.message}` };
  }

  let data;
  try {
    data = JSON.parse(raw);
  } catch (err) {
    // A wf_*.json can be corrupted by a partial write mid-session. Skip it
    // rather than aborting the whole report — one bad run must not blank out
    // an entire measurement pass.
    return { ok: false, filePath, error: `JSON parse failed: ${err.message}` };
  }

  const startTimeMs =
    typeof data.startTime === 'number' ? data.startTime : Date.parse(data.startTime ?? data.timestamp ?? '');

  const agents = (Array.isArray(data.workflowProgress) ? data.workflowProgress : []).filter(
    (entry) => entry && entry.type === 'workflow_agent',
  );

  return {
    ok: true,
    filePath,
    runId: data.runId,
    workflowName: data.workflowName,
    status: data.status,
    agentCount: data.agentCount,
    totalTokens: typeof data.totalTokens === 'number' ? data.totalTokens : 0,
    totalToolCalls: data.totalToolCalls,
    durationMs: data.durationMs,
    defaultModel: data.defaultModel,
    startTimeMs: Number.isFinite(startTimeMs) ? startTimeMs : undefined,
    agents,
  };
}

/**
 * Glob every `workflows/wf_*.json` under the given session directories, parse
 * each one, and apply the `--since`/`--workflow` filters.
 *
 * `workflowNames` is OR'd (a run matches if its `workflowName` is any one of
 * the given names) — unlike rdm's `--tag` convention, which ANDs repeated
 * flags. Callers surfacing this to a human (the CLI `--help` text) should say
 * so explicitly to avoid surprising a user familiar with rdm's convention.
 *
 * @param {Array<{ projectSlug: string, sessionId: string, sessionDir: string }>} sessionDirs
 * @param {{ since?: string, workflowNames?: string[] }} [filters]
 * @param {{ warn?: (msg: string) => void }} [opts]
 * @returns {Array<{ projectSlug: string, sessionId: string, sessionDir: string,
 *   filePath: string, run: ReturnType<typeof parseWorkflowRun> }>}
 */
export function findWorkflowRunFiles(sessionDirs, filters = {}, opts = {}) {
  const { since, workflowNames } = filters;
  const warn = opts.warn ?? (() => {});

  let sinceMs;
  if (since !== undefined && since !== null && since !== '') {
    sinceMs = Date.parse(since);
    if (Number.isNaN(sinceMs)) {
      throw new Error(`--since value is not a parseable date: "${since}"`);
    }
  }
  const wfNameSet = workflowNames && workflowNames.length ? new Set(workflowNames) : null;

  const results = [];
  for (const { projectSlug, sessionId, sessionDir } of sessionDirs) {
    const workflowsDir = path.join(sessionDir, 'workflows');
    let files;
    try {
      files = fs.readdirSync(workflowsDir);
    } catch {
      continue;
    }
    for (const fname of files) {
      if (!fname.startsWith('wf_') || !fname.endsWith('.json')) continue;
      const filePath = path.join(workflowsDir, fname);
      const run = parseWorkflowRun(filePath);
      if (!run.ok) {
        warn(`skipping unparsable workflow run ${filePath}: ${run.error}`);
        continue;
      }
      if (wfNameSet && !wfNameSet.has(run.workflowName)) continue;
      if (sinceMs !== undefined && (run.startTimeMs === undefined || run.startTimeMs < sinceMs)) continue;
      results.push({ projectSlug, sessionId, sessionDir, filePath, run });
    }
  }
  return results;
}

/**
 * Derive the agent-class rollup key from a `workflowProgress[].label`, e.g.
 * `refute:plan:coherence-1` and `refute:plan:af-2` both roll up to `refute`.
 * A label with no colon at all degrades to returning the whole string rather
 * than throwing on an out-of-range split.
 *
 * @param {string} label
 * @returns {string}
 */
export function agentClassFromLabel(label) {
  if (typeof label !== 'string' || label.length === 0) return String(label ?? '');
  const idx = label.indexOf(':');
  return idx === -1 ? label : label.slice(0, idx);
}

/**
 * Absolute path to the transcript file for one agent within one run.
 *
 * @param {string} sessionDir
 * @param {string} runId - the FULL runId, including its `wf_` prefix.
 * @param {string} agentId
 * @returns {string}
 */
export function transcriptPathFor(sessionDir, runId, agentId) {
  return path.join(sessionDir, 'subagents', 'workflows', runId, `agent-${agentId}.jsonl`);
}

/**
 * Parse one `agent-*.jsonl` transcript, deduping usage by `requestId` with
 * last-write-wins semantics (see the module header for why).
 *
 * Lines that fail to `JSON.parse`, are not `type: "assistant"`, or lack a
 * `requestId`/`message.usage` are skipped rather than throwing or polluting
 * the dedupe map with an undefined key.
 *
 * @param {string} filePath
 * @returns {{ ok: true, filePath: string,
 *   perRequest: Map<string, { model: string, usage: { outputTokens: number,
 *     inputTokens: number, cacheCreationInputTokens: number,
 *     cacheReadInputTokens: number } }>, dedupedRequestCount: number }
 *   | { ok: false, filePath: string, error: string, perRequest: Map, dedupedRequestCount: 0 }}
 */
export function parseAgentTranscript(filePath) {
  let raw;
  try {
    raw = fs.readFileSync(filePath, 'utf8');
  } catch (err) {
    return { ok: false, filePath, error: err.message, perRequest: new Map(), dedupedRequestCount: 0 };
  }

  const perRequest = new Map();
  for (const line of raw.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed) continue;

    let entry;
    try {
      entry = JSON.parse(trimmed);
    } catch {
      continue; // malformed line — skip rather than abort the whole transcript
    }

    if (entry.type !== 'assistant') continue;
    const requestId = entry.requestId;
    const usage = entry.message && entry.message.usage;
    if (!requestId || !usage) continue;

    // Last-write-wins: later lines for the same requestId overwrite earlier
    // ones, matching the observed streaming pattern where output_tokens only
    // reaches its final value on the last line for a given requestId.
    perRequest.set(requestId, {
      model: entry.message.model,
      usage: {
        outputTokens: usage.output_tokens ?? 0,
        inputTokens: usage.input_tokens ?? 0,
        cacheCreationInputTokens: usage.cache_creation_input_tokens ?? 0,
        cacheReadInputTokens: usage.cache_read_input_tokens ?? 0,
      },
    });
  }

  return { ok: true, filePath, perRequest, dedupedRequestCount: perRequest.size };
}

/**
 * Join every `workflow_agent` sidecar entry across the given runs with its
 * transcript-derived (deduped) usage, producing one flat record per agent.
 *
 * Three cases per agent, in priority order:
 *   1. `cached: true` — zero-cost, no transcript to read. `firstRequestTokens`
 *      is `null` — a cached agent made no request of its own, so there is
 *      nothing to measure a floor from (NOT `0` — `0` would silently drag a
 *      floor's median down rather than honestly excluding the record).
 *   2. A transcript file exists and yields at least one deduped request — use
 *      its per-class breakdown. `firstRequestTokens` is set to the FIRST
 *      entry of `transcript.perRequest` (a Map, so insertion-ordered; the
 *      dedupe in `parseAgentTranscript` only overwrites the value at a
 *      repeated key's original position, it never reorders) — its
 *      `inputTokens + cacheCreationInputTokens + cacheReadInputTokens`,
 *      deliberately excluding `outputTokens`. This matches
 *      `docs/token-baseline.json`'s `agentContextFloor.measuredFloor`
 *      description verbatim ("median of each agent's first transcript
 *      request only (uncachedInput + cacheWrite + cacheRead, before any tool
 *      use)").
 *   3. Otherwise (no `agentId`, no transcript file, or an empty transcript) —
 *      degrade to a sidecar-only fallback: the entry's own `tokens` scalar
 *      (defaulting to 0) is attributed entirely to the `output` class, since
 *      that is the only number available and no per-class split can be
 *      recovered from it. `sidecarOnly: true` flags this so callers can tell
 *      measured records from the fallback. `firstRequestTokens` is `null`
 *      here too — no per-class split (and therefore no first-request figure)
 *      is recoverable from a single undifferentiated sidecar scalar.
 *
 * `firstRequestTokens` is consumed by `floorByAgentClass` below, which
 * filters to records where it is a `number` before computing any statistic —
 * a `null` never contributes to a class's n, min, p10, median, or mean.
 *
 * @param {ReturnType<typeof findWorkflowRunFiles>} runFiles
 * @param {{ warn?: (msg: string) => void }} [opts]
 */
export function buildRecords(runFiles, opts = {}) {
  const warn = opts.warn ?? (() => {});
  const records = [];

  for (const { projectSlug, sessionId, sessionDir, run } of runFiles) {
    for (const agent of run.agents) {
      const label = typeof agent.label === 'string' ? agent.label : String(agent.label ?? 'unknown');
      const base = {
        projectSlug,
        sessionId,
        runId: run.runId,
        agentId: agent.agentId,
        label,
        agentClass: agentClassFromLabel(label),
        model: agent.model ?? 'unknown',
        workflowName: run.workflowName ?? 'unknown',
        agentCount: 1,
      };

      if (agent.cached) {
        records.push({
          ...base,
          output: 0,
          uncachedInput: 0,
          cacheWrite: 0,
          cacheRead: 0,
          dedupedRequestCount: 0,
          sidecarOnly: false,
          cached: true,
          firstRequestTokens: null,
        });
        continue;
      }

      const agentId = agent.agentId;
      const transcriptPath = agentId ? transcriptPathFor(sessionDir, run.runId, agentId) : null;
      let transcript = null;
      if (transcriptPath && fs.existsSync(transcriptPath)) {
        transcript = parseAgentTranscript(transcriptPath);
      }

      if (transcript && transcript.ok && transcript.perRequest.size > 0) {
        // The FIRST entry inserted into the perRequest Map is the agent's
        // first transcript request — a Map iterates in first-insertion
        // order per key, and parseAgentTranscript's last-write-wins dedupe
        // only overwrites the VALUE at a repeated requestId's original
        // position, it never reorders. This must be read before the
        // summing loop below consumes the same iterator's values.
        const firstPerRequestEntry = transcript.perRequest.values().next().value;
        const firstRequestTokens =
          firstPerRequestEntry.usage.inputTokens +
          firstPerRequestEntry.usage.cacheCreationInputTokens +
          firstPerRequestEntry.usage.cacheReadInputTokens;

        let output = 0;
        let uncachedInput = 0;
        let cacheWrite = 0;
        let cacheRead = 0;
        for (const { usage } of transcript.perRequest.values()) {
          output += usage.outputTokens;
          uncachedInput += usage.inputTokens;
          cacheWrite += usage.cacheCreationInputTokens;
          cacheRead += usage.cacheReadInputTokens;
        }
        records.push({
          ...base,
          output,
          uncachedInput,
          cacheWrite,
          cacheRead,
          dedupedRequestCount: transcript.perRequest.size,
          sidecarOnly: false,
          cached: false,
          firstRequestTokens,
        });
      } else {
        if (!agentId) {
          warn(`agent at label "${label}" in run ${run.runId} has no agentId; cannot locate a transcript`);
        } else if (!transcriptPath || !fs.existsSync(transcriptPath)) {
          warn(`no transcript found for agent ${agentId} in run ${run.runId}; falling back to sidecar tokens`);
        } else if (transcript && !transcript.ok) {
          warn(
            `transcript for agent ${agentId} in run ${run.runId} failed to read: ${transcript.error}; falling back to sidecar tokens`,
          );
        } else if (transcript && transcript.ok && transcript.perRequest.size === 0) {
          warn(
            `transcript for agent ${agentId} in run ${run.runId} contained no usable assistant/usage lines (empty transcript); falling back to sidecar tokens`,
          );
        }
        const tokens = typeof agent.tokens === 'number' ? agent.tokens : 0;
        records.push({
          ...base,
          output: tokens,
          uncachedInput: 0,
          cacheWrite: 0,
          cacheRead: 0,
          dedupedRequestCount: 0,
          sidecarOnly: true,
          cached: false,
          firstRequestTokens: null,
        });
      }
    }
  }
  return records;
}

/**
 * Generic aggregation: bucket `records` by `keyFn(record)`, summing counts and
 * every token class. Every grouping (by agent class, by label, by model, by
 * workflow) is a thin wrapper over this one function so they can never drift
 * from each other.
 *
 * @param {ReturnType<typeof buildRecords>} records
 * @param {(record: object) => string} keyFn
 * @returns {Array<{ key: string, agentCount: number, dedupedRequestCount: number,
 *   output: number, uncachedInput: number, cacheWrite: number, cacheRead: number }>}
 *   sorted by key for deterministic output.
 */
export function aggregate(records, keyFn) {
  const map = new Map();
  for (const r of records) {
    const key = keyFn(r);
    let bucket = map.get(key);
    if (!bucket) {
      bucket = { key, agentCount: 0, dedupedRequestCount: 0, output: 0, uncachedInput: 0, cacheWrite: 0, cacheRead: 0 };
      map.set(key, bucket);
    }
    bucket.agentCount += r.agentCount;
    bucket.dedupedRequestCount += r.dedupedRequestCount;
    bucket.output += r.output;
    bucket.uncachedInput += r.uncachedInput;
    bucket.cacheWrite += r.cacheWrite;
    bucket.cacheRead += r.cacheRead;
  }
  return [...map.values()].sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
}

/** Aggregate by agent class (the first colon segment of `label`). */
export function aggregateByAgentClass(records) {
  return aggregate(records, (r) => r.agentClass);
}

/** Aggregate by the full, un-stripped `label`. */
export function aggregateByLabel(records) {
  return aggregate(records, (r) => r.label);
}

/** Aggregate by model. */
export function aggregateByModel(records) {
  return aggregate(records, (r) => r.model);
}

/** Aggregate by workflow name. */
export function aggregateByWorkflow(records) {
  return aggregate(records, (r) => r.workflowName);
}

/**
 * Percentile of a numeric array using linear interpolation between the two
 * nearest ranks (the same method as NumPy's default `'linear'` interpolation
 * / Excel's `PERCENTILE.INC`). At `n === 1` every percentile collapses to
 * the single value regardless of interpolation method — this choice is only
 * observable once a class has 2+ eligible records in the real corpus, which
 * is why the fixture (every class n=1) cannot distinguish between
 * interpolation methods and this is documented here instead.
 *
 * @param {number[]} values - need not be pre-sorted.
 * @param {number} p - in `[0, 1]`.
 * @returns {number}
 */
function percentile(values, p) {
  const sorted = [...values].sort((a, b) => a - b);
  const n = sorted.length;
  if (n === 1) return sorted[0];
  const idx = (n - 1) * p;
  const lower = Math.floor(idx);
  const upper = Math.ceil(idx);
  if (lower === upper) return sorted[lower];
  const weight = idx - lower;
  return sorted[lower] + (sorted[upper] - sorted[lower]) * weight;
}

/**
 * Summarize a numeric array as n/min/p10/median/mean, mirroring the shape of
 * `docs/token-baseline.json`'s `agentContextFloor.measuredFloor`.
 *
 * @param {number[]} values - non-empty.
 * @returns {{ n: number, minTokens: number, p10Tokens: number, medianTokens: number, meanTokens: number }}
 */
function summarizeFloor(values) {
  const sorted = [...values].sort((a, b) => a - b);
  const n = sorted.length;
  const sum = sorted.reduce((s, v) => s + v, 0);
  return {
    n,
    minTokens: sorted[0],
    p10Tokens: percentile(sorted, 0.1),
    medianTokens: percentile(sorted, 0.5),
    meanTokens: sum / n,
  };
}

/**
 * Per-agent-class first-request floor: the median (plus n, min, p10, mean)
 * of `firstRequestTokens` across every record in that class that HAS one —
 * i.e. every `buildRecords` record for which `firstRequestTokens` is a
 * `number`, not `null`. `cached: true` and `sidecarOnly: true` records carry
 * `firstRequestTokens: null` (see `buildRecords`) and are filtered out
 * before grouping; they contribute to NO class's n/min/p10/median/mean.
 *
 * A class whose every record is cached/sidecarOnly is OMITTED from the
 * returned array entirely — no `n: 0` / `medianTokens: null` bucket is
 * emitted for it. Callers must not assume every `byAgentClass` key has a
 * matching entry here.
 *
 * This is a sibling aggregation to `aggregateByAgentClass`, not a
 * replacement — `byAgentClass` still reports whole-agent totals across
 * every record (including cached/sidecarOnly ones); this reports a
 * first-request-only floor across the measured subset.
 *
 * @param {ReturnType<typeof buildRecords>} records
 * @returns {Array<{ key: string, n: number, minTokens: number, p10Tokens: number,
 *   medianTokens: number, meanTokens: number }>} sorted by key for
 *   deterministic output.
 */
export function floorByAgentClass(records) {
  const byClass = new Map();
  for (const r of records) {
    if (typeof r.firstRequestTokens !== 'number') continue;
    let values = byClass.get(r.agentClass);
    if (!values) {
      values = [];
      byClass.set(r.agentClass, values);
    }
    values.push(r.firstRequestTokens);
  }

  const result = [];
  for (const [key, values] of byClass) {
    result.push({ key, ...summarizeFloor(values) });
  }
  return result.sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
}

/**
 * Assemble the full report: every grouping, the per-agent-class first-request
 * floor (`floorByAgentClass`), plus the `totalsDiscrepancy` line.
 *
 * `totalsDiscrepancy` compares each run's own `totalTokens` field (summed
 * across runs in scope) against the independently deduped per-class sum
 * (summed across every record, including sidecar-only-fallback
 * contributions). This is NEVER reconciled into a single number — both sides
 * and the delta are reported so a caller can see the two measures disagree
 * rather than have that disagreement silently hidden. `totalTokens` /
 * `workflowProgress[].tokens` are not a cost basis (see the roadmap body and
 * `autopilot-run-accounting`); this discrepancy line operationalizes that
 * finding rather than trying to explain it away.
 *
 * @param {{ root?: string, since?: string, workflowNames?: string[], warn?: (msg: string) => void }} [options]
 */
export function buildReport(options = {}) {
  const projectsRoot = options.root ?? defaultProjectsRoot();
  const warn = options.warn ?? (() => {});

  const sessionDirs = locateSessionDirs(projectsRoot);
  const runFiles = findWorkflowRunFiles(sessionDirs, { since: options.since, workflowNames: options.workflowNames }, { warn });
  const records = buildRecords(runFiles, { warn });

  const sidecarTotalTokens = runFiles.reduce((sum, r) => sum + (r.run.totalTokens ?? 0), 0);
  const dedupedTotalTokens = records.reduce(
    (sum, r) => sum + r.output + r.uncachedInput + r.cacheWrite + r.cacheRead,
    0,
  );

  return {
    projectsRoot,
    runsConsidered: runFiles.length,
    recordCount: records.length,
    byAgentClass: aggregateByAgentClass(records),
    byLabel: aggregateByLabel(records),
    byModel: aggregateByModel(records),
    byWorkflow: aggregateByWorkflow(records),
    floorByAgentClass: floorByAgentClass(records),
    totalsDiscrepancy: {
      sidecarTotalTokens,
      dedupedTotalTokens,
      delta: sidecarTotalTokens - dedupedTotalTokens,
    },
  };
}
