# Specs and Standards: A Durable Intent Layer Above Roadmaps

## Summary

rdm tracks **changes** — a roadmap and its phases are a proposal plus the plan to deliver it — but
nothing records the **current truth** those changes move. This document proposes two kinds of
durable document that sit above roadmaps, both living in the **source repo** next to the code they
describe:

- **Specs** ("what to build"): one living document per capability describing the behavior the
  system has now, with stable requirement IDs that phases cite.
- **Standards** ("how to build"): several documents describing architecture constraints and
  process practices (TDD, mutation testing, commit conventions), each declaring which lifecycle
  stages and paths it governs.

rdm does not own either kind. The plan repo keeps holding changes; roadmaps, phases, and reviews
*reference* specs and standards at a source-repo revision, and the dispatch lane loads, checks,
and reconciles them at the stages they declare.

The work is split into two roadmaps that share one prerequisite (listing source-repo files at a
revision): **specs** and **standards**.

Research behind this proposal (tool survey, pre-LLM precedent, academic evidence) is summarised in
[Research basis](#research-basis) below.

## Problem

- **Intent lives only in change documents.** A roadmap's `## Intent` section
  ([`authoring-grammar.md`](authoring-grammar.md)) states a goal, non-goals, and "Done looks
  like" for *that change*. Once the roadmap lands, nothing states what the system now does; the
  next roadmap re-derives it from code.
- **Tasks have no intent source.** Plan review's `intent-alignment` dimension reads the parent
  roadmap's `## Intent`; a task has no parent roadmap, so it gets no alignment check.
- **"How to build" guidance is one hardcoded file.** Review prompts in
  `.claude/workflows/lib/review.mjs` read "`docs/principles.md` if present, otherwise `CLAUDE.md` /
  `AGENTS.md`". There is no way to split standards across documents, scope a standard to part of
  the tree, bind a practice to a lifecycle stage, or tell which version of a standard a plan was
  reviewed against.
- **Practices are not bound to a stage.** "Use TDD" (principles §4) is prose every agent pays for
  and a reviewer may or may not apply; nothing tells the planner it applies at planning time or
  scopes it to the paths it governs.

## Design principles

These follow from the research and from rdm's own principles.

1. **Current truth and changes are different documents.** Every durable spec process pairs a
   frozen change document with a living reference (PEP + Language Reference, Rust RFC + Reference,
   TC39 proposal + ecma262, OpenSpec `changes/` + `specs/`) and joins them with a gate at
   completion. rdm already has the change half; this adds the current-truth half.
2. **Durable docs live with the code.** Spec and standard changes must be atomic with the code
   that realises them, reviewable in the same diff, and discarded with an abandoned branch.
   OpenSpec, Kiro, Spec Kit, the Rust Reference, and WHATWG all do this.
3. **Reconcile at land, mechanically.** Specs stay current only when something in the release path
   forces it. Discipline alone produces spec "exhaust" nobody reads.
4. **Load narrowly.** Standing context files measurably add cost without reliably improving agent
   success. Agents receive the requirements a phase cites and the standards that apply to the
   current stage and paths — never whole documents by default.
5. **Enforcement and guidance stay separate.** Executed checks remain project config
   (`dispatch.verify`, [`verify-gate.md`](verify-gate.md)); standards are prose guidance and never
   carry commands. This keeps the decision recorded by the `dispatch-dev-discipline` roadmap.
6. **Optional and small.** No roadmap or task is required to cite a spec. Templates are short.

## Specs: what to build

### Location and shape

Specs live in the source repo under a configured directory (default `specs/`), one file per
capability — not per feature or per change:

```markdown
---
title: Plan review gate
status: active            # draft | active | deprecated | superseded
superseded_by: null
---

## Intent
Why this capability exists; the problem it solves.

## Requirements

### R1 — Stamp new items
WHEN plan_review resolves to true
THEN roadmap/phase/task create SHALL add the needs-plan-review tag.

### R2 — …

## Non-goals
- …

## Open questions
- …
```

- **Requirement IDs** (`R1`, `R2`, …) are stable and never reused. WHEN/THEN scenarios are
  recommended; full EARS is not required.
- **Behavior only.** Design and implementation detail stay in roadmaps, plans, and code.
- **Status describes the document, not implementation progress.** Coverage (planned, in progress,
  implemented) is derived from the roadmaps and phases that cite the spec, never stored — the KEP
  split between `status` and `stage`.

### Linking

- A roadmap declares the specs it changes in frontmatter: `specs: [specs/plan-review.md]`, and
  rdm records the source-repo SHA of each at link time.
- Phases and acceptance criteria cite individual requirements with the existing link grammar's
  source form, extended with a requirement anchor: `rdm:src/specs/plan-review.md#R1`.
- rdm derives, rather than stores, the reverse direction: which roadmaps cite a spec, which
  requirements no phase cites, which phases cite no requirement.
- **Suspect links.** When a cited spec changes on the default branch after the SHA a roadmap
  recorded, rdm flags the link — the DOORS pattern. The agent can diff the spec between the two
  revisions to see whether intent moved under its plan.

### Change tracking

Because specs live in the source repo, a roadmap's worktree branch carries its spec edits:

- The branch diff of `specs/` **is** the change document. No separate delta format or merge engine
  is needed.
- An abandoned (`wont-fix`) roadmap discards its spec edits with its branch.
- Two roadmaps editing the same spec conflict at rebase in `rdm-land`, surfaced like any other
  conflict, instead of interleaving shipped and unshipped behavior on `main`.
- **Spec-first convention.** A roadmap that changes behavior makes its spec edit the first commit on
  its branch (or its first phase), so plan review can review the proposed behavior change before
  implementation.
- **Land gate (hard block).** When a roadmap that declares `specs:` lands, `rdm-land` refuses unless
  the branch diff touches each declared spec *or* the roadmap records an explicit "no spec change"
  acknowledgement. Landing is where the spec is reconciled with what was actually built.

### What specs feed

- **Plan review.** `intent-alignment` reads the cited requirements alongside the roadmap's
  `## Intent` (which keeps describing *why this change*). Tasks that cite a spec gain an intent
  source.
- **Spec authoring review.** A plan-review dimension for specs themselves checks for ambiguity and
  proposes clarifying questions instead of letting agents guess.
- **Dispatch.** The planner and implementer receive only the requirements the phase cites.
- **Backlog grooming.** Signals: active specs no roadmap cites, suspect links, specs untouched while
  roadmaps citing them landed.

## Standards: how to build

### Two kinds

| | Architecture standards | Practice standards |
|---|---|---|
| Examples | "Core is the source of truth", "no derived index", error-handling layering | "Use TDD", "mutation testing on core", "conventional commits", "ACs must be artifact-satisfiable" |
| Governs | The code that results | How the work is done |
| Applies at | Plan review and code review | Specific lifecycle stages |
| Conformance | Reviewer judgment against the text | Reviewer judgment against the text and artifacts; mechanical enforcement, if any, lives in `dispatch.verify` |

### Location and shape

Standards are several documents in the source repo, discovered through a project config key
(default `docs/standards/*.md`, falling back to today's `docs/principles.md` / `CLAUDE.md` /
`AGENTS.md` chain so existing projects keep working). Each carries light frontmatter:

```markdown
---
title: Test-driven development
kind: practice             # architecture | practice
stages: [plan, review]     # plan | implement | verify | review | land
scope: [rdm-core/**]       # optional path globs; omitted = whole repo
---

## TDD-1 — Failing test first
A plan SHALL list the failing test before the implementation step that makes it pass.
```

- **Stable anchors** (`TDD-1`, or today's `§4`) let review findings cite the rule they apply.
- **`stages`** tells the lane where the standard is loaded: a `plan` standard is given to the planner
  and plan review; an `implement` standard to the implementer; a `review` standard becomes review
  context.
- **`scope`** narrows by path, so a standard only loads for phases touching those paths.
- **Standards are prose only.** A practice such as mutation testing that should be *enforced* is
  enforced by the project adding the command to `dispatch.verify`; the standard explains the
  practice and reviewers judge conformance.
- **No double loading.** Standards must not also be auto-loaded through `CLAUDE.md` imports or
  `.claude/rules/`, or every agent pays for them twice — the reason `dispatch-dev-discipline`
  phase 3 was closed `wont-fix`. Stage- and path-scoped loading is the capability that phase found
  missing from harness auto-load, so its value should be measured, not assumed.

### Change tracking

- **History:** standards change by decision, not by feature. Each change lands in the source repo
  alongside a short decision record (ADR-style: context, decision, consequences) so the *why* is
  kept, not just the diff.
- **Version pinning:** phases, tasks, and reviews record the standards revision (source SHA) and
  the list of standard paths they loaded, like `review_sha`. Pinning is record-only: a read-time
  flag for in-flight items whose pinned standards have since changed is deferred until drift proves
  to be a real problem. Recording the loaded paths is what makes that flag decidable later.

### Migrating rdm's own principles

`docs/principles.md` splits into standards documents, each section keeping its text and gaining
kind, stages, and scope:

| Sections | Kind | Stages | Scope |
|---|---|---|---|
| §1 core source of truth, §2 thin adapters | architecture | plan, review | unscoped |
| §3 Store, §5 errors, §6 frontmatter, §7 status enums, §8 API docs, §9 re-exports | architecture | plan, review | `rdm-core/**` |
| §10 unsafe, §12 workspace dependencies | architecture | review | unscoped |
| §11 content negotiation | architecture | plan, review | `rdm-server/**` |
| §4 testing, §13 commits, §14 no guards, §15 artifact-satisfiable ACs, §16 nothing built for rdm alone | practice | plan, implement, review | unscoped |

The fallback chain keeps the single-file form valid for projects that don't split.

### How the lane uses standards, and how that is verified

- **Workflows (deterministic).** The review workflows (code review and plan review) build a
  structured `standardsCommand` per review unit from `{stage, paths}`, alongside the `itemCommand`
  and `roadmapCommand` they already hand units, and each agent runs it. This is tested in the
  mocked-agent harness (`rdm-cli/tests/workflow_review/`): the real driver runs with a scripted fake
  `agent()`, and the command each agent was handed is executed against a fixture source repo,
  asserting on the returned standards set, never on prompt text.
- **Skills (prose).** The dispatch planner and implementer, `rdm-do`, the review and plan-review
  skills, and `rdm-roadmap` are directed to run `rdm standards list` for their stage and paths.
  These directions ship in the templates and plugin tree and are verified by code review of the
  template diff. There is no grep test over skill text, and no live dispatch serves as a test or an
  acceptance criterion until a prompt-testing library is integrated.
- **`rdm agent-config --principles-file`** feeds the fallback chain instead of rendering a separate
  "read this file" note, so standards are never delivered twice.

## Sweeping for existing deviations

Standards also make pre-existing debt discoverable. The research (2026-09-27 survey, summarised
under [Research basis](#research-basis)) points to three channels, in order of payoff:

1. **Diff-time, touched-file advisories.** Code review already receives scoped standards.
   Deviations on changed lines are ordinary findings; pre-existing deviations elsewhere in touched
   files are advisory, never gating. Facebook's Infer saw a ~70% fix rate for diff-time findings
   against ~0% for the same findings delivered in batch.
2. **Agent-authored deterministic detectors.** Where a rule can be mechanised (banned APIs, crate
   dependency direction), an agent writes the detector — a clippy `disallowed-*` config, a dylint
   lint, a dependency check — and it runs as a project check in `dispatch.verify`, keeping
   enforcement in config. Grep scripts are not detectors, and there is no shrink-only baseline file
   (a guard of the kind principles §14 rules out): a newly mechanised rule's existing violations
   are fixed with it, or the rule starts advisory.
3. **A periodic judgment sweep** reusing the review pipeline, for rules that need judgment:
   units are (rule × module), hotspots first (churn × size, TODO/FIXME density); cheap deterministic
   candidate discovery precedes LLM judgment; every finding quotes its rule anchor and names a
   runnable witness the refuter executes (unwitnessed findings stay advisory); findings are
   clustered by rule and root cause, matched against the backlog, and emitted as propose-only task
   commands; wont-fix dismissals suppress re-raising, and per-rule acceptance rates demote noisy
   rules.

Practice rules such as TDD have no reliable detector and stay advisory at diff time.

## What rdm stores vs reads

| Lives in | Holds |
|---|---|
| Source repo | Specs, standards, decision records, code |
| Plan repo | Roadmaps (with `specs:` and pinned SHAs), phases (with requirement citations and standards SHA), tasks, reviews, plans |
| Project config | Spec directory, standards glob |

Core reads specs and standards from the source repo **at a revision** (default branch head, or a
pinned SHA). This is a new read path: `Store` abstracts plan-repo documents only
([principles §1](principles.md)), so source-repo reads need their own seam in an I/O crate,
following how `rdm:src/` links and `change/` reviews already resolve against the source repo.

## Non-goals

- A spec or standards entity stored in the plan repo.
- A separate change-proposal entity or delta-merge format (the roadmap is the proposal; the branch
  diff is the delta).
- Traceability matrices maintained by hand.
- Generating code from specs (spec-as-source).
- Tool-specific prompt dialects inside specs or standards.
- Commands inside standards; enforcement stays in `dispatch.verify`.

## Open questions

- **Multi-repo projects.** A project whose capability spans several source repos has no single home
  for its spec. Out of scope until a real case appears.
- **Standards review severity.** When a reviewer judges a practice standard violated, is that a
  gating finding or advisory? Today's review severity contract decides; revisit after dogfooding.

## Research basis

A survey conducted 2026-09-26 by parallel research agents; the full write-up with citations is the
"A Spec Layer for rdm" document shared from that session.

- **Tools.** OpenSpec's split of `specs/` (current truth) from `changes/` (deltas), Spec Kit's later
  `converge` and "living spec" guidance, and Kiro's requirement-cited tasks and EARS-derived
  property tests all converge on living, capability-organised specs with a reconciliation step.
  Practitioner reports (Scott Logic; Alfredo Perez's "238 specs, never read one again") show
  per-feature specs become unread exhaust and multiply review cost; Agent OS v3 dropped spec
  scaffolding but kept standards injection.
- **Precedent.** PEP 1 declares resolved PEPs historical and keeps behavior in the Language
  Reference; Rust stabilization requires a Reference PR; TC39 Stage 4 requires the ecma262 merge;
  KEPs separate document status from implementation stage; DOORS flags downstream links suspect
  when a requirement changes; ADRs are append-only decision records.
- **Evidence.** Agents rarely detect underspecified tasks and gain substantially from clarification
  (Ambig-SWE, ICLR 2026; ClarifyGPT, FSE 2024); making the contract explicit first improves agent
  test generation (Google, 2026 preprint); trace links improve maintenance speed and correctness
  (Mäder & Egyed, EMSE 2015; Rempel & Mäder, TSE 2017); repository context files add inference cost
  without reliable gains (ETH, 2026 preprint). No controlled study yet tests a persistent spec layer
  above plans — rdm's own use should be treated as the experiment.
- **Sweeping for deviations.** Diff-time delivery vastly outperforms batch reports (Distefano et
  al., CACM 2019; Rosik et al., SPE 2011). Tools must keep the effective false-positive rate under
  ~10% or be ignored (Sadowski et al., ICSE 2015 / CACM 2018). Raw LLM findings run 60–80% false
  positive (SmellBench 2026 preprint; Du et al., ICSE 2026 SEIP), and LLMs do not self-correct
  without external feedback (Huang et al., ICLR 2024), so verification needs a mechanical witness.
  Narrow, tool-grounded detection beats open-ended prompting (Liu et al., ASE journal 2025; iSMELL,
  ASE 2024). Prose rules can be compiled to checks (Dicto, WICSA 2015; ContextCov, 2026 preprint).
  Per-rule thresholds and suppression made Google's AutoCommenter useful (arXiv 2405.13565).
  Aggressive automated refactoring degrades quality (SmellBench; RefactorBench, ICLR 2025), so
  sweeps propose rather than apply. Recall of whole-repo agent audits is essentially unmeasured.
