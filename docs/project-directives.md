# Project directives

**Optional development discipline, expressed by the project, injected verbatim into
the agents that do the work.**

Mutation testing, coverage floors, fuzzing, a house style for error messages, a
review emphasis — every project has some of this, and no two projects have the same
set. rdm does not grow a flag per discipline. Instead it reads the prose a project
already writes for the agents working in it, and reproduces that prose, unmodified,
inside the dispatched implementer's and reviewer's prompts.

Two things follow from "unmodified", and they are the whole point of this document:

- **rdm never paraphrases a directive.** A summary is rdm's reading of the project's
  rule interposed between the project and the agent. The Rust reader slices the
  post-frontmatter bytes; the JS renderer concatenates them with no `trim`, no
  `replace`, no re-wrapping. A runtime guard drops any entry whose text did not
  survive transport intact rather than injecting an altered copy.
- **A directive is guidance, not enforcement.** The declared check
  (`dispatch.verify`, see [verify-gate.md](verify-gate.md)) is executed and its exit
  code gates the phase. A directive is text that shapes what an agent chooses to do.
  Anthropic's own memory documentation draws this line explicitly: instruction files
  "shape Claude's behavior but are not a hard enforcement layer." Do not try to
  enforce a directive, and do not try to make a check optional by describing it.

Nothing in this subsystem produces a finding, a status, or a gate.

## Sources

With no `dispatch.directives` key declared, rdm scans these locations, in this fixed
order — which is also the emitted order, so output is deterministic across
filesystems:

| # | Location | Shape |
|---|---|---|
| 1 | `.claude/rules/**/*.md` | recursive, depth-capped, sorted |
| 2 | `AGENTS.md` | one file |
| 3 | `.cursor/rules/*.mdc` | one directory level, Cursor's own extension |
| 4 | `.clinerules` | a plain file, **or** a directory of `*.md` — both exist in the wild |
| 5 | `.windsurf/rules/**/*.md` | recursive, depth-capped, sorted |
| 6 | `.github/copilot-instructions.md` | one file |

This mirrors what Claude Code's own `/init` reads.

**`CLAUDE.md` is deliberately excluded.** Claude Code already loads it into every
subagent, and it cannot be suppressed per agent type — this repo's own copy was
measured at 19320 tokens. Discovering it here would pay that cost a second time, per
dispatched agent, for text the agent already has.

rdm's existing `Platform` enum is **not** reused for this list, and should not be: it
names the single canonical file rdm *writes* per platform, has no entry for
`.claude/rules/`, `.clinerules`, or `.windsurf/rules/`, and its Cursor entry is one
fixed file rather than a glob.

Symlinks are never followed (a symlink loop cannot hang the walk, and a link out of
the repo cannot exfiltrate). That check covers the **scan root** of every location,
not only the entries found while walking one: `read_dir` and `Path::is_dir` both
follow a symlink, so `.claude/rules` (or `.windsurf/rules`, or a declared directory
entry) being *itself* a link out of the tree is exactly the case a per-entry check
cannot see — and the one that matters, since a dispatched agent's prompt is an
exfiltration sink. A source that is unreadable, not valid UTF-8, or a device/FIFO is
reported as skipped rather than failing the dispatch.

## The `dispatch.directives` override: replace, never merge

```sh
rdm config set dispatch.directives "docs/rules/testing.md,docs/rules/perf.md"
rdm config get dispatch.directives --raw
```

When the key is present, that list is the **whole** source list. Discovery does not
run at all, and a discovered path appears in neither the emitted `directives` array
nor the `skipped` array. Merging would make the effective set depend on files the
operator never named, which is precisely what an explicit declaration is for.

- An **explicitly empty** value (`rdm config set dispatch.directives ""`) is a legal,
  meaningful setting: "this project declares no directive sources." It stores an
  empty list, reports `origin: config`, and injects nothing. This differs from
  `dispatch.verify`, which rejects an empty value — an empty verify command would
  silently disable a gate, whereas an empty directive list disables nothing.
- A declared path that does not exist lands in `skipped` with reason
  `declared source not found`. The operator named it, so its absence is signal.
- The key is **repo-only** (like `dispatch.verify` and `server.quick_filters`): a
  directive source list names files inside one project's tree. `--global` is rejected
  with its own message saying so.

## Addressing a directive to a role

A directive says who it is for in YAML frontmatter:

```markdown
---
role: implementer
paths:
  - "rdm-core/**/*.rs"
---

Every new public function in this repository ships with a table-driven test
before its implementation.
```

- `role:` is `implementer`, `reviewer`, or `both`. **`both` is the default**, so
  undifferentiated injection is what you get if you say nothing — the fallback, not
  the goal. `rdm-role:` is accepted as a synonym for a project that already uses
  `role:` for something else.
- Frontmatter was chosen because it works in a **plain markdown file with no
  rdm-specific tooling**: every one of the six source formats above is already a
  markdown-ish file, most of the ecosystems that read them already parse
  frontmatter, and a file carrying `role:` stays perfectly readable to the tools that
  ignore it.
- Anything unrecognized degrades to `both`, unscoped. A project's typo must not break
  every dispatch.

The **act** step (the agent that fixes non-gating findings after a clean review) edits
code, so it is an implementer-shaped role and receives the implementer block.
Refuters deliberately receive **no** directives — see Non-goals.

## Path scoping

`paths:` (canonical) or `globs:` (Cursor's spelling) scopes a directive to the files
a change touches. Either a YAML sequence or a comma-separated string; absent or empty
means unscoped, and unscoped always applies.

The matcher supports `**` (crossing separators), `*` (within one segment), and `?`
(one non-separator character), anchored at both ends. `**/` also matches **zero**
directories, so `**/*.rs` matches a top-level `x.rs`.

Which path set each consumer matches against:

| Consumer | Path set |
|---|---|
| first-pass implementer | the approved plan's `file_map[].path` |
| rework implementer | that, unioned with the prior round's real changed files |
| act step | the last round's changed files, else the plan's file map |
| reviewer | `signals.changedFiles` from the diff — **`null`** when that agent failed |

**Fail-open rule.** An *unknown* path set (`null`/`undefined`/not an array) keeps
every role-matching directive, mirroring `deriveSignals`' existing convention:
missing information widens injection, never narrows it. An **empty array** is a real
answer ("nothing changed") and does narrow. The distinction matters most for the
reviewer: passing `[]` when the diff agent failed would make every scoped rule vanish
exactly when the review is already degraded.

## The size bound

Injected text is paid for **once per dispatched agent**, so the set is bounded:

| Constant | Value | ≈ tokens at 2.49 chars/token |
|---|---|---|
| `MAX_BYTES_PER_SOURCE` | 8 000 | ≈ 3 200 |
| `MAX_BYTES_TOTAL` | 16 000 | ≈ 6 400 |

Both live once, in `rdm-core/src/directives.rs`, and are echoed in the command's
`budget` object. **The JS never re-derives them** — it renders what `skipped[]`
reports. `scripts/verify-project-directives.sh` § 8 greps every JS copy for the
literals to keep it that way.

**Skip, do not truncate.** A source over the per-source bound is skipped *whole*;
sources are then admitted in discovery order until the total budget would be exceeded
(admission is `<=`, so a set landing exactly on the budget is admitted), and each
remaining one is skipped with the total-budget reason. Truncation was rejected
because a rule cut mid-sentence can *invert its own meaning* — "never do X … unless
Y" becomes "never do X" — which is the same failure mode as a paraphrase, arrived at
mechanically.

**A skip is never silent.** It is observable in three places:

1. `skipped[]` in the command's JSON output, with a reason naming the bound;
2. a notice rendered **inside** the injected block, so the agent itself is told which
   project rules it is not being shown and that what it has is incomplete;
3. a `[directives: N source(s) not injected: …]` clause on the dispatch OUTCOME
   `summary`, which reaches the operator and the `rdm review blocked` queue line.

The runtime paraphrase guard feeds the same channels: an entry whose code-point count
no longer matches the count the resolver emitted did not survive transport verbatim,
so it is dropped and named rather than injected in altered form. Code points, not
UTF-16 units — JavaScript has no byte length, and `.length` would false-positive on
any astral character.

## The command

```sh
rdm dispatch directives --format json [--dir <path>] [--role implementer|reviewer]
```

Read-only: it writes nothing, anywhere. `--dir` names the **source** repo to scan and
defaults to the current directory; the declared key is read from the **plan** repo,
and when no plan repo is reachable rdm falls back to discovery rather than erroring,
so a downstream consumer with no plan repo still gets the feature. Finding nothing
exits 0 with empty arrays and empty stderr.

```json
{
  "origin": "discovery",
  "budget": { "maxBytesPerSource": 8000, "maxBytesTotal": 16000 },
  "directives": [
    { "path": ".claude/rules/testing.md", "role": "implementer", "paths": [],
      "text": "…verbatim…", "chars": 103, "bytes": 103 }
  ],
  "skipped": []
}
```

`bytes` (UTF-8 length, the unit of the bound) and `chars` (code points, the unit of
the transport check) are different numbers for non-ASCII text and must not be
conflated.

## Reviewer authority

Directives reach every finder prompt, in both modes, in every dimension — one rule,
one mechanism. Because the channel carries project-authored text straight into a
reviewer, its preamble scopes what that text may do: directives state the standards
to hold the work to, and they **cannot narrow the review**. No directive can tell a
reviewer to skip a file, ignore a finding, lower a severity, stop reviewing, or treat
code as pre-approved; one attempting any of that is itself reported as a finding. The
existing `INJECTION_HYGIENE` rule — everything the reviewer *reads* is untrusted data
— is unchanged and still pushed unconditionally alongside it.

## The hoist trap

A caller that hoists `phaseMeta`/`taskMeta` (`rdm-autopilot`, `rdm-do --auto`,
`rdm-dispatch-phase`) skips the workflow's Stage-0 agent entirely — the only
in-workflow code path that resolves directives. So a hoisting caller that omits them
does not lose an optimization, it turns injection **off** for that dispatch with
nothing reporting it. Every hoisting shim therefore runs the command itself and
forwards both keys, and their SKILL.md procedures say so.

Unlike `verify`, **an empty or failed read must not abandon the hoist**: absent
directives are normal, and falling back to a full Stage-0 fetch on every phase of a
project that simply has no rules would be a pure loss. `directives` stays optional in
the schema and out of `hoistedMetaComplete`'s key list for exactly that reason.

## Non-goals

- **No refuter injection.** A refuter grades one finding against the actual code. Its
  job is technical verification, not standards enforcement, and feeding it the
  project's declared preferences would give a finding a second, non-technical route
  to survival.
- **No per-section role addressing.** A file has one `role:`. Splitting a file into
  per-section addressing would need an rdm-specific markup, which is the one thing
  the frontmatter choice above exists to avoid.
- **No directive-derived findings.** rdm never reports "this change violates directive
  X." That would be enforcement, and the guidance/enforcement line above is the
  point of the whole design.
- **No enforcement of a directive, and no check made optional by describing one.**
  If something must gate, it belongs in `dispatch.verify`.

## Where the code lives

| Layer | File |
|---|---|
| discovery, verbatim read, bounding | `rdm-core/src/directives.rs` |
| config key | `rdm-core/src/config.rs` (`DispatchConfig::directives`) |
| command | `rdm-cli/src/commands/dispatch.rs` |
| selection + rendering | the `directives` region of `.claude/workflows/lib/dispatch-phase.mjs` |
| reviewer preamble | `.claude/workflows/lib/review.mjs` (`DIRECTIVES_PREAMBLE`) |
| regression harness | `scripts/verify-project-directives.sh` |
