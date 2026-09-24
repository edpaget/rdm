# Run accounting: `rdm cost`

`rdm cost` reports the tokens a Claude Code session spent, per model and per token class, with a
breakdown by spend source. It reads Claude Code's own transcript files, needs no plan repo, never
touches the network, and reports tokens only (no dollars).

```bash
rdm cost --session <uuid>              # one session
rdm cost --workflow-run <wf_id>        # one Workflow run in a session (the `wf_` prefix is optional)
rdm cost                               # the session named by CLAUDE_CODE_SESSION_ID
rdm cost --session <uuid> --format json
rdm cost report --roadmap <slug> --project <p>   # a roadmap's runs, joined to spend (needs the plan repo)
```

The rules live in `rdm-core` (`rdm_core::transcript`, over the pure token accounting in
`rdm_core::usage`). The filesystem reader is the `rdm-transcript` crate. [Run records](#run-records)
say what ran when, and [`rdm cost report`](#cost-report-joining-runs-to-spend)
(`rdm_core::cost_report`) joins them to spend by time window.

## Where the data is

The projects directory is `$CLAUDE_CONFIG_DIR/projects` when `CLAUDE_CONFIG_DIR` is set and
non-empty, and `$HOME/.claude/projects` otherwise. With neither set, `rdm cost` fails and says
so. With no `--session` or `--workflow-run`, it reports the session in `CLAUDE_CODE_SESSION_ID`
(the raw value Claude Code sets), and fails with an error naming all three ways to pick a session
when that is unset.

The on-disk layout, as verified on 2026-09-24:

```text
<projects>/<project-slug>/<session>.jsonl                         main transcript
<projects>/<project-slug>/<session>/subagents/agent-<id>.jsonl    Agent subagent transcript
<projects>/<project-slug>/<session>/subagents/agent-<id>.meta.json
<projects>/<project-slug>/<session>/workflows/<runId>.json        Workflow sidecar (runId = wf_…)
<projects>/<project-slug>/<session>/subagents/workflows/<runId>/agent-<id>.jsonl
```

- A project slug is the working directory with separators replaced by `-`. A git worktree gets a
  slug of its own (for example `-Users-me-src-proj--worktrees-roadmap-x`). So a session is found
  by walking **every** slug directory for `<uuid>.jsonl` or `<uuid>/`, never by deriving a slug
  from the current directory. If a uuid appears under several slugs, the first in name order is
  used and a warning names the others. A uuid found nowhere fails with an error naming the uuid,
  the projects root and every slug directory searched.
- `--workflow-run` finds the session whose `workflows/` holds `<runId>.json`.
- `agent-<id>.meta.json` carries `agentType`, `description`, `toolUseId`, `spawnDepth` and, for a
  nested agent, `parentAgentId`. Nested `Agent` subagents live flat in the same `subagents/`
  directory as their parents.
- A Workflow sidecar carries `runId`, `workflowName`, `totalTokens` and `workflowProgress[]`. Each
  `workflow_agent` entry has `agentId`, `label`, `model`, `state` (`done`, `error`, …), `cached`
  and `tokens`.

**The sidecar and transcript formats are undocumented Claude Code internals and may change
without notice.** Nothing in rdm's test suite can detect such a change. The checked-in fixture
(`tests/fixtures/cost-session/`) freezes the shape observed on 2026-09-24. Dogfooding is the
only check against the live format.

## Spend sources and anchors

A session has three kinds of spend source. Each is anchored to a timestamp from the main
transcript, so a later phase can split a session by time window.

| source | transcript | anchor |
| --- | --- | --- |
| `main` | `<session>.jsonl` | its own first timestamped line; each request keeps its own timestamp |
| `agent` | `subagents/agent-<id>.jsonl` | the main-transcript line holding the `tool_use` block whose `id` is the meta's `toolUseId` |
| `workflow_agent` | `subagents/workflows/<runId>/agent-<id>.jsonl` | the main-transcript line whose `toolUseResult.runId` names the run |

- **Nested `Agent` subagents.** A nested agent's `toolUseId` names a `tool_use` in its parent's
  transcript, not the main one. The reaper therefore follows `parentAgentId` through the sibling
  `.meta.json` files until it reaches an ancestor whose `toolUseId` is in the main transcript, and
  uses that line's timestamp. A cycle or a missing parent ends the walk unanchored.
- **Labels.** An `agent` source is labelled with its `description`, and its `agentType` is shown
  alongside. A `workflow_agent` is labelled `<workflowName> / <workflowProgress[].label>`.
- **Unanchored is not dropped.** A source whose anchor cannot be found (a `toolUseId` that is not
  in the main transcript, or a Workflow run that no `tool_result` names) is still counted. It is
  flagged `anchored: false` and listed in `unanchored` with its kind, id and label. An
  unanchored Workflow run is listed once, as a `workflow_run` entry, and its agents carry
  `anchored: false`. A session directory with no main transcript is reported too: every source
  in it is unanchored, and a warning says so.

## What is counted

- **Per model and per token class.** The classes are uncached `input`, `output`,
  `cache_write_5m`, `cache_write_1h` and `cache_read`. The model is the raw `message.model`
  string.
- **Dedupe within a transcript.** A request streams as several lines sharing one `requestId`. The
  last line's usage wins, and the request keeps its first line's position and timestamp.
- **Dedupe across sources.** Sources are processed in a fixed order: main, then `Agent` subagents
  by id, then Workflow runs by run id and each run's agents by id. The first source to count a
  `requestId` owns it. A later copy is dropped, counted in that source's `duplicates_dropped`,
  and reported in a warning.
- **All-zero requests are excluded.** A request whose final usage is zero in every class is not a
  measurement. It is left out and reported in a warning that names its source.
- **Cached Workflow agents cost zero.** A `cached: true` agent reused an earlier result, so no
  transcript is expected for it.
- **Errored Workflow agents are flagged.** An agent whose `state` is `"error"` gets
  `errored: true`. Its transcript is still counted.
- **Sidecar figures are never counted.** The run's `totalTokens` is shown as
  `sidecar_total_tokens`, labelled "not the cost basis", for reference only. A per-agent `tokens`
  figure is never used, not even when the agent's transcript is missing. That agent is reported
  as zero, with a warning.
- **A transcript with no progress entry is still counted**, labelled by its agent id. A run
  directory under `subagents/workflows/` with no readable sidecar is still counted as a run, with
  a warning.

## Degradation and warnings

`rdm cost` reports what it can and says what it could not:

- A `workflows/wf_*.json` that is not JSON, or has no string `runId`, is skipped with a warning
  naming its full path. The rest of the session is still reported. Asking for that run with
  `--workflow-run` fails, and the error points at `--session` for the warning.
- An unreadable subagent transcript or meta file yields a warning. The source is then counted as
  zero or left unanchored.
- **`workflow-nested agents unverified`.** An agent spawned *below* a Workflow agent has never been
  observed on disk, so the reaper does not look for one. Any session that contains a Workflow run
  carries this warning, and so does every `--workflow-run` report.

In human output, warnings go to stderr as `warning: …`. With `--format json`, they appear only
in the report's `warnings[]` array, so stdout stays clean JSON. `--format table` and
`--format markdown` print the human rendering.

### Warnings in a `--workflow-run` report

A `--workflow-run` report narrows every section to that run. `sources`, `unanchored`,
`workflow_runs`, `totals` and the per-model `models` all cover only the run's agents. Dedupe
has already run over the whole session, so the run's figures match its rows in the full session
report. `warnings` is filtered the same way. It keeps the warnings about the session as a whole
(location warnings, a missing main transcript, `workflow-nested agents unverified`) and the ones
about this run or its agents. It drops the ones about the main transcript, `Agent` subagents,
other runs and unreadable sidecars.

## Run records

`rdm cost` answers "what did this session spend"; a **run record** answers "what ran in it, and
when". `rdm run record` mints a `projects/<project>/runs/<id>.md` file naming the driver, the
roadmap or task, and the raw `CLAUDE_CODE_SESSION_ID`; `rdm run unit-start` / `rdm run unit-end`
bracket each dispatched unit with timestamps and an outcome; `rdm run close` ends the run. The
format, lifecycle and session-capture rule are in
[`file-formats.md` § Run Files](file-formats.md#run-files).

`rdm cost report` joins a run to its session's spend by **unit time window** (next section). That
is why unit windows never overlap and why timestamps keep sub-second precision. A run with no
`session_uuid` is reported as unjoinable, and one still `open` is joined but labelled incomplete.

## Cost report: joining runs to spend

`rdm cost report --roadmap <slug>` answers what a roadmap cost and which phases dominated. It
loads every run record of the project, keeps the runs whose target is the roadmap, locates and
reaps each distinct session they name **once**, and attributes every spend source of those
sessions by time. Unlike bare `rdm cost` it reads the plan repo, so it resolves the project like
any other command (`--project` > `RDM_PROJECT` > `default_project`). It exits 0 even when some
sessions are missing, and prints a one-line report when no run targets the roadmap.

The join lives in `rdm_core::cost_report`, which reads transcripts only through the
`TranscriptSource` port. The CLI loads the runs, builds the filesystem source and renders.

### Identifier spaces

| identifier | names | joins |
| --- | --- | --- |
| run id (`2026-09-24-1530-a1b2`) | one run record | nothing; it only labels the run |
| session uuid (`session_uuid`) | one Claude Code session | a run to the transcripts it spent through |
| unit stem + `attempt` | one unit entry of a run | a phase row; every attempt of a stem sums into it |
| source kind + id | one spend source (`main` + session uuid, `agent` + agent id, `workflow_agent` + agent id) | a source to one bucket |
| Workflow `runId` (`wf_…`) | one Workflow run in a session | its agents to their anchor (the `tool_result` naming it) |
| `requestId` | one API request | dedupe within and across a session's sources (see [What is counted](#what-is-counted)) |

The join itself uses only the session uuid and timestamps. No label, ordinal or `requestId` crosses
a session boundary.

### What is placed, by which timestamp

- **`agent` and `workflow_agent` sources are placed whole, by their anchor**: the main-transcript
  line that launched the source (the `Agent` `tool_use`) or returned it (the Workflow
  `tool_result`). The orchestrator dispatches units one at a time and waits for each, so that
  line falls inside the window of the unit that spent it. A subagent's own line timestamps are not
  main-transcript time and are not used. Keeping a subagent whole also makes it one labelled line
  in the output.
- **The `main` source is split per request, by each request's own timestamp.** Main spans the whole
  session and carries the orchestrator's own turns, the backbone of every phase in the prose lane.
  Placing it by its anchor (the session's first line) would put all of that spend in one bucket,
  and per-phase figures would leave out the orchestrator. With the split, the orchestrator's turns
  inside a unit window count toward that unit, and the turns between units count as overhead.

### Windows

Every window is half-open, `start <= ts < end`.

- **Run window**: `[started, ended)`. A run with no `ended` (an `open` run) ends at the `started` of
  the next run, of **any** target, recorded in the same session, and is unbounded if there is none.
- **Unit window**: `[started, ended)`. A unit entry with no `ended` is **incomplete**; its window
  ends at the next unit entry's `started` in the same run, or else at the run window's end.
- **Ownership**: an item belongs to the run with the latest `started <= ts` among the session's
  runs of every target, and counts only if `ts` is also inside that run's window. The same rule
  picks among a run's unit entries. Each item has at most one owner, even when two runs share a
  session or bad data makes windows overlap, so nothing is counted twice.

### Buckets

| bucket | contents |
| --- | --- |
| **per-phase** | inside a complete unit entry's window, and not an errored Workflow agent; summed per phase stem across entries and runs, so a rework pair lands on one stem |
| **run overhead** | inside the run window but outside every unit window, and not errored: the estimate pre-pass, `rdm next`, advance and park writes, and the orchestrator's turns between units |
| **unattributed (runs)**, itemised | inside the run window and either an errored Workflow agent (cause `errored`, taking precedence over the two buckets above) or inside an incomplete unit entry (cause `unit-incomplete`, naming the stem and attempt). Main requests here are aggregated into one `main session` item per run and unit entry |
| **unattributed (sessions)**, itemised | an unanchored `agent` or Workflow agent (cause `unanchored`), and main requests with no timestamp (cause `untimestamped`). They cannot be placed in any window and two runs may share the session, so they are listed once per session, never inside a run |
| **excluded** | placed, but owned by no run of this roadmap: before or after every run, between runs, or inside another target's run. Reported as a count of sources, a count of main requests and their tokens, and never part of any total |

Every figure carries the five token classes plus requests and their total, per model where a
model breakdown is shown. Three identities hold per token class:

- per run: `Σ per-unit + overhead + unattributed == run total`;
- per joined session: `Σ this roadmap's run totals + session-unattributed + excluded == session
  total`, so nothing is dropped;
- per roadmap: `total == Σ joined run totals + Σ session-unattributed`, each session counted once.

### Per-phase rows

One row per phase stem that has a unit entry in a joined run, ordered by total tokens descending
and then by stem. Each row gives the tokens by class, `attempts` (unit entries for the stem,
incomplete ones included) and wall clock (the sum of `ended − started` over complete entries).
A row with an incomplete entry is flagged: its wall clock is partial and that entry's spend is in
the unattributed bucket. JSON durations are `wall_clock_ms`.

### Missing, unjoinable, open and copied

- **Missing**: the run names a session that cannot be located or read, typically because it was
  pruned from disk. The run is listed with the error as its reason; its units add no tokens,
  attempts or wall clock; the other runs are still reported.
- **Unjoinable**: the run has no `session_uuid`. It is listed and contributes nothing.
- **Open**: joined normally with the window above, labelled `open (incomplete)`.
- **Cross-session copies**: a resumed or forked session that copies earlier lines keeps their
  original timestamps, so they fall outside the new session's run windows and are excluded there.
  No cross-session `requestId` dedupe is done.

### Output

The human view prints a header with run counts by join state, tokens by model, the bucket table
(its `run overhead` row is the overhead line), the per-phase table, itemised run and session
unattributed items, the excluded line, the runs table and the reason for each missing session.
Reap warnings go to stderr as `warning: <session>: …`. `--format json` prints the same data on
stdout with the warnings inside `warnings[]`, and stderr stays empty. `--format table` and
`--format markdown` print the human view. The checked-in fixture is
`tests/fixtures/cost-report/`, whose plan repo is joined to the `cost-session` transcripts.

## Historical provenance

The following figures were measured manually, once, before this command existed. No test checks
them. They come from the Workflow-lane autopilot run `wf_a0402a3d-697`, as recorded in the
`autopilot-run-accounting` roadmap:

- 109 agents, 818 deduped requests, all on `claude-opus-4-8`, over 68 minutes.
- Output 76,097; uncached input 6,925; cache write 4,331,529 (all with the 5-minute TTL);
  cache read 36,739,533. About 96% of the total is cache write plus cache read.
- The sidecar's `totalTokens` was 5,003,952. It did not reconcile with the deduped classes and
  left out cache reads entirely.

That run is why costing is per model **and** per token class, and why `totalTokens` is never a
cost basis. `docs/token-baseline.md` records the wider multi-run survey.
