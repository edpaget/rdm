# Plugin Distribution: Naming, Layout, and Runtime Arguments

This document records the naming and layout decisions for rdm's Claude Code plugin distribution channel. It resolves the questions left open in the `plugin-distribution` roadmap's central constraint, establishing how skills, workflows (engines), and runtime arguments are handled when rdm is distributed as a Claude Code plugin rather than as raw emitted skills.

## Context

rdm ships two independent distribution surfaces:

1. **Raw skills** (`rdm agent-config claude --skills --out <dir>`) — emits 11 skills and 2 distributed workflow scripts (`.js` files) to a target directory, landing in a flat `.claude/skills/` namespace with no collision protection.
2. **Plugin** (to be implemented in phases 2–3) — packages the same skills and workflows as an installable Claude Code plugin named `rdm`, providing automatic namespace prefixing (`rdm:<name>`) and plugin-root-relative workflow paths.

When a plugin is named `rdm`, the hand-rolled `rdm-` prefixes in skill names and the `rdm-wf-` prefixes in engine names become redundant *at the naming level* — the plugin runtime supplies the disambiguation. This document settles whether to drop these prefixes in plugin mode, and if so, which prefix to drop and why.

## Central Constraint — RESOLVED

**Workflows are a first-class plugin component.** The mechanism is proven by official Anthropic plugins:

- **claude-security** plugin (1 workflow): demonstrates both invocation forms in production.
  - Mechanism source: [Distribute a workflow in a plugin](https://code.claude.com/docs/en/workflows)
  - Official registry: [claude-plugins-official](https://github.com/anthropics/claude-plugins-official) (verified: 1 workflow: scan.js)
  - Example invocation: `Workflow({ name: "claude-security:scan", args: {…} })`
  - Scripts directory: `workflows/` at the plugin root, sibling to `skills/`
  - Manifest: `.claude-plugin/plugin.json`, no `workflows` field (convention-discovered directory)

- **code-modernization** plugin (6 workflows): demonstrates the scriptPath invocation form.
  - Mechanism source: [Plugins Reference](https://code.claude.com/docs/en/plugins-reference)
  - Official registry: [claude-plugins-official](https://github.com/anthropics/claude-plugins-official) (verified: 6 workflows including portfolio-assess.js and others)
  - Example invocation: `Workflow({ scriptPath: "${CLAUDE_PLUGIN_ROOT}/workflows/portfolio-assess.js" })`
  - Layout: workflows/ directory at plugin root, manifest under `.claude-plugin/`

**Key facts from the official sources:**

- Plugin workflows are namespaced by plugin name at invocation time — `meta.name` in the script stays bare (e.g., `scan`, not `claude-security:scan`), and the runtime prepends the plugin namespace.
- The `workflows/` directory is **convention-discovered** — neither plugin declares a `workflows` key in `plugin.json`.
- A `workflows` manifest field (where it exists) **replaces** the default directory location rather than adding to it.

### Caveat: `claude plugin details` Display Gap

The `claude plugin details` command shows Skills / Agents / Hooks / MCP servers / LSP servers, but **has no Workflows category**. This was verified against `claude-security`, which ships 1 workflow (scan.js) and shows none in the details output. This is a display gap in the CLI tool, **not** evidence that the mechanism is unsupported. Later phases must check `.claude-plugin/plugin.json` or the file tree directly; they must **not** use `plugin details` as a workflow-presence signal.

## The Disjointness Invariant

**At most one of the two prefixes may be dropped in plugin mode.** This is a joint constraint, not a pair of independent decisions.

### Why This Constraint Exists

rdm's raw-skills distribution includes both skills and workflow engines:

- **Emitted raw-distribution skills (11 total):** rdm-autopilot, rdm-backlog, rdm-dispatch-phase, rdm-do, rdm-document, rdm-estimate, rdm-land, rdm-plan-review, rdm-review, rdm-revise, rdm-roadmap
- **Emitted workflow engines (2 total):** rdm-wf-dispatch-phase, rdm-wf-review-refute-fix

Note: `rdm-dispatch-phase` appears in **both** lists — it is a thin skill shim that invokes the `rdm-wf-dispatch-phase` workflow engine.

### The Collision Scenario

If both prefixes are dropped in plugin mode:

- Skill `rdm-dispatch-phase` → `rdm:dispatch-phase` (dropping `rdm-`)
- Engine `rdm-wf-dispatch-phase` → `rdm:dispatch-phase` (dropping both `rdm-` and `wf-`)

Both the skill shim and the engine map to the same listing entry: `/rdm:dispatch-phase`. The skill shim would invoke a workflow via `Workflow({ name: "rdm:dispatch-phase" })`, creating **self-reference** — it would invoke itself instead of the engine.

### The Surviving Disambiguator

**The recommended default is to drop `rdm-` from skills and keep `rdm-wf-` on engines:**

- **Skills in plugin mode:** `rdm:roadmap`, `rdm:dispatch-phase`, `rdm:estimate`, etc. (drop `rdm-`)
- **Engines in plugin mode:** `/rdm:rdm-wf-dispatch-phase`, `/rdm:rdm-wf-review-refute-fix` (keep `rdm-wf-`)

This choice:
- Eliminates the collision (engines have the `wf` disambiguator; skills do not).
- Preserves all existing gates and validation in the source tree (see "Coexistence with Source-Tree Gates" below).
- Leaves `.claude/workflows/*.js` and every `meta.name` unchanged.
- Is the cheapest consistent pair absent a stronger argument.

## Decision 1: Skill Naming in Plugin Mode

**Decision:** Drop the `rdm-` prefix from skill names when emitted in plugin mode.

**Plugin-mode names (11 skills):**

- `rdm:autopilot`, `rdm:backlog`, `rdm:dispatch-phase`, `rdm:do`, `rdm:document`, `rdm:estimate`, `rdm:land`, `rdm:plan-review`, `rdm:review`, `rdm:revise`, `rdm:roadmap`

**Rationale:**

- The plugin namespace (`rdm:`) provides automatic disambiguation against global skills and other projects' skills; the hand-rolled `rdm-` prefix becomes redundant.
- Dropping it yields shorter, more readable names in the Claude Code UI and in slash-command references.
- It aligns with the thin-shim naming convention used in `.claude/skills/` — each skill is a wrapper around rdm CLI commands or workflow invocations, not a monolithic implementation. The prefix is a stylistic convention, not a functional requirement.
- It reduces visual clutter: `rdm:roadmap` is clearer than `rdm:rdm-roadmap`.

## Decision 2: Engine Naming in Plugin Mode

**Decision:** Keep the `rdm-wf-` prefix on workflow engine names when emitted in plugin mode.

**Plugin-mode names (2 engines):**

- `/rdm:rdm-wf-dispatch-phase`
- `/rdm:rdm-wf-review-refute-fix`

**Rationale:**

- The `rdm-wf-` prefix is **the disambiguator** — it prevents collision between the skill shim and the engine it invokes. Dropping it would violate the disjointness invariant (see above).
- Keeping `rdm-wf-` requires **zero changes** to the source tree, to any engine file, to `meta.name` declarations, or to the skills that invoke them.
- All existing gates in the repository (`scripts/verify-workflow-review.sh` §2d, §2e, and `observe-workflow-listing.sh`) remain green without modification because they inspect the source tree, not plugin-mode emission output.
- Future phases that implement the plugin manifest generator can apply a straightforward name-rewrite transform at emission time (`rdm-wf-dispatch-phase` → `/rdm:rdm-wf-dispatch-phase`) without touching any shipping code.

## Decision 3: Shim Invocation Form

**Decision:** Use the namespaced `Workflow()` invocation form in emitted skill shims.

**Invocation form:**

```javascript
Workflow({
  name: "rdm:rdm-wf-dispatch-phase",
  args: { …args }
})
```

**Alternative form (not chosen):**

```javascript
Workflow({
  scriptPath: `${CLAUDE_PLUGIN_ROOT}/workflows/rdm-wf-dispatch-phase.js`,
  args: { …args }
})
```

**Rationale:**

- The namespaced form (`name: "rdm:<engine-name>"`) is the primary invocation style used by `claude-security` and is the idiomatic approach in Claude Code plugins.
- It does not require runtime knowledge of `CLAUDE_PLUGIN_ROOT` or reliance on file paths — the Workflow tool resolves the name natively.
- It reduces coupling between the skill shim and the file-system layout, making the shim more portable and less sensitive to directory restructuring.
- Both forms are supported by official plugins, so the choice is one of style and maintainability, not feasibility.

Official precedent: `claude-security` plugin uses `Workflow({ name: "claude-security:<engine-name>", args: {…} })` throughout its skill shims.

## Decision 4: Runtime Arguments Delivery

**Decision:** Shims must resolve `rdmBin` and optional `project` arguments for the engines.

Workflow engines (e.g., `rdm-wf-dispatch-phase.js`) accept these arguments:

- **`rdmBin` (OPTIONAL):** Path to the rdm binary. An absent key defaults to a plain `rdm` on `PATH`; an explicit string is used verbatim, and the sentinel `"rdm"` requests PATH resolution deliberately. A present-but-non-string value still throws, since degrading a typo to PATH would reintroduce the silent-wrong-binary hazard. The engine never probes the filesystem. Canonical contract: [`docs/workflow-schemas.md`](workflow-schemas.md) § "Environment args: `rdmBin` and `project`".
- **`project` (OPTIONAL):** The rdm project name. If absent, rdm uses `RDM_PROJECT` environment variable or the `default_project` configured in `rdm.toml`.

This reverses an earlier fail-closed stance recorded in this document. That stance guarded a real hazard — inside the rdm source repo a bare `rdm` is a stale installed build the development-build rule forbids — but the hazard is dogfood-scoped, and a plugin consumer has no repo-local build path to pass. The compensating control now lives where the hazard does: `RDM_BIN` in this repo's `.mise.toml` pins the local development build and the calling skill resolves it, gated by `scripts/verify-workflow-dispatch.sh` § 9c-dogfood.

### Resolution Strategy for Plugin-Installed Shims

When rdm is installed as a plugin, the consumer tree has no repo-local `./target/debug/rdm` path. The emitted skill shim carries a "Resolving `rdmBin` (plugin install)" section resolving it at runtime using these strategies, in order of precedence:

1. **Explicit `--rdm-bin` CLI flag** (if the skill supports it): Consumer passes the path explicitly.
2. **`RDM_BIN` environment variable:** Consumer has set `RDM_BIN=/path/to/rdm` in their environment.
3. **`rdm` in `PATH`:** Shim executes `rdm` directly, relying on PATH lookup (requires global installation or consumer having rdm in PATH).

Strategy 3 is also the engine's own default for an omitted `rdmBin`, so a normally installed rdm needs no configuration — strategies 1 and 2 are overrides for a binary that is not on `PATH`. If all strategies fail, the shim must emit a **clear, actionable error message:**

```
Error: rdm binary not found. Install rdm, then set RDM_BIN=/path/to/rdm, put rdm on your PATH, or pass --rdm-bin /path/to/rdm.
```

### Project Resolution

If the `project` argument is not supplied to the Workflow call, rdm uses its standard resolution chain:

1. `--project` CLI flag (if passed by the shim)
2. `RDM_PROJECT` environment variable
3. `default_project` in `rdm.toml` (searched up from the current working directory)

If rdm cannot resolve the project, it emits a clear error message directing the consumer to set one of the above.

### Notes on Implementation

- The skill shim (e.g., `rdm-dispatch-phase`) is generated in Phase 2 and must contain the `rdmBin` resolution logic at the point where it invokes the Workflow tool.
- This does not change the Workflow engines themselves (e.g., `rdm-wf-dispatch-phase.js`). They treat `rdmBin` identically in the plugin and raw-skills distributions: absent means a plain `rdm` on `PATH`, and only a non-string value is refused.
- Error messages must be tested as part of Phase 3's integration tests (the harness that validates plugin installation and command execution).

## Fixed Plugin Layout

The following layout decisions were established in the roadmap's "Central constraint — RESOLVED" section. This section restates them so later phases have a single canonical reference.

### Directory Structure

```
<plugin-root>/
  .claude-plugin/
    plugin.json              # Plugin manifest (standard Claude Code format)
  skills/
    roadmap/                 # Skill implementations (directory names: rdm- prefix dropped per Decision 1)
    dispatch-phase/
    autopilot/
    backlog/
    do/
    document/
    estimate/
    land/
    plan-review/
    review/
    revise/
  workflows/
    rdm-wf-dispatch-phase.js # Workflow engines (directory names: rdm-wf- prefix kept per Decision 2)
    rdm-wf-review-refute-fix.js
```

### Manifest Configuration

The plugin manifest (`plugin.json`) is a standard Claude Code plugin manifest with these properties:

- **`name`:** `"rdm"` — the plugin namespace. This is **not** configurable; it is the vendor name.
- **`version`:** Matches the rdm crate version (e.g., `"0.18.1"`).
- **No `workflows` field:** The `workflows/` directory is convention-discovered by the Claude Code runtime. The absence of a manifest key means the runtime uses the default directory location.

### Why Convention-Discovery (No Manifest Key)

Both official plugins (`claude-security` and `code-modernization`) omit the `workflows` key, relying on the convention that workflows live in `workflows/` at the plugin root. This keeps the manifest minimal and leverages the platform's default behavior. If a future phase needs to relocate workflows to a non-standard directory, the manifest can include a `workflows: "path/to/dir"` field at that time.

### Why the Directory is a Sibling, Not Nested

Placing `workflows/` as a sibling of `skills/` (not inside `.claude-plugin/`) follows the official structure demonstrated by `claude-security`. This layout treats workflows as a first-class plugin component alongside skills, making the structure immediately recognizable to users familiar with Claude Code conventions.

## Coexistence with Source-Tree Gates

The naming decisions above are **plugin-mode transformations applied at emission time**, not edits to the source tree. This section explains how they coexist with the repository's existing validation gates.

### Source-Tree Gates (Unchanged)

Three gates validate the source tree and raw-skills emission:

1. **`scripts/verify-workflow-review.sh` § 2d (`check_listing_disjoint`):** Asserts that engine `meta.name` fields and skill frontmatter names form disjoint listing entries in the raw output. The gate inspects `.claude/workflows/` and `.claude/skills/` only.

2. **`scripts/verify-workflow-review.sh` § 2e (anchored bare-name sweep):** Searches for engine names (without `rdm-wf-` prefix) across multiple roots: `scripts/`, `rdm-cli/`, `rdm-core/`, `docs/`, `CLAUDE.md`, and `README.md`. The goal is to catch unintended references to bare engine names (which would be ambiguous before the plugin context renames them).

3. **`scripts/observe-workflow-listing.sh`:** Runs `claude -p` to capture the real listing and asserts engine/skill name disjointness. This also operates on the raw surface, not plugin-mode output.

### Plugin-Mode Transformation (Applied at Emission)

Phases 2–3 will emit plugin-mode artifacts (manifest, skill shims, workflow files bundled at the plugin root). At that point:

- The raw-skills surface (`--skills --out`) remains byte-for-byte identical to today's output.
- The plugin surface (new `--plugin --out` CLI) applies name transformations:
  - Skill names: `rdm-roadmap` → `roadmap` (drop `rdm-` prefix)
  - Engine names: `rdm-wf-dispatch-phase` → `rdm-wf-dispatch-phase` (keep `rdm-wf-` prefix; the plugin namespace `rdm:` is added by the platform)
- The source tree (`.claude/workflows/`, `.claude/skills/`) is never modified.

### Extending the Disjointness Check to Plugin Mode

**Important for Phases 2–3:** The §2d and §2e gates are source-tree-only and do not inspect plugin-mode emission output. To ensure the disjointness invariant is enforced downstream, Phases 2–3 must:

- Add a new gate (or extend existing ones) that validates the **emitted plugin artifact**, not the source tree.
- Assert that emitted skill names and engine names (after plugin-mode transformations) still form disjoint listing entries.
- Ensure no phase accidentally drops both `rdm-` and `rdm-wf-` prefixes, which would re-introduce the collision.

This gate will run as part of Phase 3's harness (`rdm-cli plugin --check` or equivalent) and must pass before any plugin artifact is considered shipping-ready.

### Path Examples in Documentation

All path examples in rdm's documentation use the **source-tree path**, not the plugin-mode name transform:

- Workflow engine file: `.claude/workflows/rdm-wf-dispatch-phase.js` (not `rdm-wf-dispatch-phase.js` alone)
- Skill file: `.claude/skills/rdm-dispatch-phase/SKILL.md` (not `dispatch-phase/SKILL.md`)
- In plugin context, these are invoked as: `/rdm:rdm-wf-dispatch-phase` and `rdm:dispatch-phase` (with namespacing applied by the platform)

This convention keeps documentation independent of the distribution channel. A consumer of the raw-skills output or the plugin sees the same workflow engine name in logs and error messages (the platform-supplied namespace prefix).

## Summary of Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| **Skill names** | Drop `rdm-` → `rdm:roadmap` | Plugin namespace provides collision protection; reduces verbosity; aligns with thin-shim naming. |
| **Engine names** | Keep `rdm-wf-` → `/rdm:rdm-wf-dispatch-phase` | Preserves disjointness invariant; requires zero source-tree changes; all existing gates pass unchanged. |
| **Shim invocation** | Namespaced form: `Workflow({ name: "rdm:rdm-wf-dispatch-phase" })` | Idiomatic for Claude Code plugins; native Workflow-tool support; reduces file-system coupling. |
| **rdmBin resolution** | Optional, defaulting to `rdm` on PATH; shim resolves via flag, then env var, then PATH; fail with actionable error if none resolves. | Enables plugin use in environments without repo-local binaries; PATH is the right answer for nearly every consumer, and the flag/env var cover the rest. |
| **Disambiguator** | `rdm-wf-` prefix survives | Prevents collision between `rdm-dispatch-phase` skill and `rdm-wf-dispatch-phase` engine. |

## Marketplace Distribution

### The Checked-In Plugin Tree

The generated plugin tree is checked into this repo at a single canonical path:

| Artifact | Path |
|----------|------|
| Plugin tree | `plugins/rdm/` |
| Plugin manifest | `plugins/rdm/.claude-plugin/plugin.json` |
| Marketplace manifest | `.claude-plugin/marketplace.json` (repo root) |

Two distinct `.claude-plugin/` directories therefore exist, and they are not interchangeable: the **repo-root** one holds the *marketplace* manifest, and the one **inside `plugins/rdm/`** holds the *plugin* manifest. `claude plugin validate` is pointed at the containing directory, not at the JSON file.

The marketplace entry's `source` is `./plugins/rdm`, resolved relative to the repo root. The entry deliberately carries **no `version` field** — that would re-couple the repo to the crate version (see below).

### Canonical Regeneration Command

**Never hand-edit anything under `plugins/rdm/`.** It is generator output. Regenerate it with exactly:

```bash
env -u RDM_ROOT -u RDM_PROJECT cargo run -q -- agent-config claude --plugin --out plugins/rdm
```

(equivalently `env -u RDM_ROOT -u RDM_PROJECT ./target/debug/rdm agent-config claude --plugin --out plugins/rdm`).

Two details are load-bearing:

- **`env -u RDM_ROOT -u RDM_PROJECT`** — a developer with those variables set in their shell would otherwise produce a different tree and see false drift.
- **No `--project` flag.** Emission is `--project`-sensitive: passing `--project rdm` bakes this repo's own project name into all 11 `SKILL.md` bodies. Omitting it emits the generic `--project <PROJECT>` placeholder a downstream consumer needs.

Emission is deterministic — two runs are byte-identical — and needs no plan repo.

This adds a **third** in-repo copy of the two Workflow engine scripts, alongside `.claude/workflows/` and `rdm-core/src/templates/workflows/`. After regenerating, re-run `scripts/verify-agent-config-distribution.sh` and `scripts/verify-workflow-review.sh` as well as the two harnesses below.

### Why the Drift Gate is Version-Normalized

The plugin manifest's `version` comes from `env!("CARGO_PKG_VERSION")`, which inherits the workspace version from `Cargo.toml`. `.github/workflows/prepare-release.yml` sed-bumps that version and stages exactly `Cargo.toml Cargo.lock CHANGELOG.md` (line 104) before committing and pushing to `main` — **it regenerates nothing**. So `plugins/rdm/.claude-plugin/plugin.json` goes stale on every release *by design*.

A naive byte-identity gate over a version-bearing checked-in tree would therefore go red on `main` the moment a release lands. That is the same failure class as asserting on `CHANGELOG.md` prose, and it is what blocked v0.18.1 (CI run 30815546603).

The resolution, implemented in `scripts/verify-plugin-install.sh`:

- The checked-in tree is treated as **version-agnostic**. The drift gate replaces the manifest `version` value with a fixed placeholder on **both** sides before diffing, so a crate-version bump can never move the diff. Everything else — every other manifest field, every `SKILL.md` byte, every workflow byte, and the file set itself — stays under exact byte-identity.
- Version currency is asserted **only against freshly generated output**, never against committed bytes.
- The expected version is read from `Cargo.toml`'s `[workspace.package] version`. Note that **`rdm --version` does not exist** (clap rejects the flag), so Cargo.toml is the source of truth.

A paired self-test proves both halves: a planted `99.99.99` bump leaves the drift gate green while turning the runtime version assertion red, and a mutated non-version manifest field turns the drift gate red (proving the normalization is surgical rather than a blanket neuter).

### Consumer Installation

```bash
claude plugin marketplace add edpaget/rdm
claude plugin install rdm@rdm
```

This installs the 11 `rdm:<name>` skills and the two `rdm:rdm-wf-<engine>` Workflow engines. A local checkout can be installed the same way by pointing `marketplace add` at the repo directory.

### The Two Harnesses

| Script | Hermetic? | Run by CI? |
|--------|-----------|------------|
| `scripts/verify-plugin-install.sh` | Yes — pure POSIX shell + coreutils, no `python3`/`node`/`jq`/`claude` | **Yes**, via `.github/workflows/ci.yml`'s `for f in scripts/verify-*.sh` glob |
| `scripts/observe-plugin-install.sh` | No — requires the `claude` CLI | **No.** The `observe-` name keeps it outside the glob; it is developer-run |

`verify-plugin-install.sh` gates the version-normalized drift of `plugins/rdm/` against generator output, the runtime manifest-version assertion (fresh output only), marketplace shape plus `source` resolution with a non-empty-entry floor, workflow byte-identity, and the 11-skill inventory with frontmatter validity — each behind a planted-corruption self-test proving it is non-vacuous.

`observe-plugin-install.sh` performs a real offline install (`validate --strict` → `marketplace add` → `install rdm@rdm` → assert the installed tree) into an mktemp'd `CLAUDE_CONFIG_DIR`, and proves the invoking user's real `~/.claude` is byte-unchanged. It exits **2** with a NOTICE — deliberately distinguishable from both pass (0) and fail (1) — when `claude` is absent from `PATH`.

Two CLI gaps shape that split, both verified against `claude` 2.1.220:

1. `claude plugin validate --strict` **false-passes** a marketplace whose plugin `source` points at a nonexistent directory (exit 0 on `"source": "./does-not-exist"`, with all other warnings cleared). Source resolution is therefore owned by our own harness, with a non-empty-entry floor so "every entry resolves" cannot be vacuously true of an empty list.
2. `claude plugin details` reports Skills / Agents / Hooks / MCP servers / LSP servers and has **no Workflows category at all** — confirmed against the official `claude-security` plugin, which ships workflows and shows none. Workflows *are* fully supported by the runtime; the inventory simply does not enumerate them. So workflow presence is asserted **on the filesystem**, never via `plugin details`.

## Next Steps for Later Phases

- **Phase 2:** Implement the plugin manifest generator in `rdm-core/src/agent_config.rs` and the `--plugin` CLI in `rdm-cli`. Apply the name transformations at emission time.
- **Phase 3:** Extend the disjointness validation to plugin-mode artifacts. Update or create a gate that checks the emitted plugin for naming collisions. Add the `agent-config claude --plugin` CLI surface.
- **Phase 4:** Package the plugin for marketplace distribution and implement the install harness.
- **Phase 5:** Document the dogfood resolution and recommended distribution path.

All path examples in code, CLI output, and documentation must continue to use the source-tree prefix (`rdm-wf-`), even in plugin contexts, to maintain consistency across distribution channels.

## Local run transcript: `scripts/observe-plugin-install.sh`

Landing evidence for the real-install half. Captured 2026-08-03 on macOS (darwin 25.5.0) against `claude` **2.1.220 (Claude Code)** and rdm crate version **0.18.1**, fully offline — no network, no auth, no `ANTHROPIC_API_KEY`.

What it proves: the committed plugin manifest and the marketplace manifest both pass `claude plugin validate --strict`; the marketplace adds and the plugin installs cleanly into an isolated `CLAUDE_CONFIG_DIR`; `plugin list --json` reports exactly one entry, `id=rdm@rdm`, `enabled=true`, at the crate version; the installed tree carries all 11 skills and both Workflow engines byte-identical to the emitted bytes; and the invoking user's real `~/.claude` config is byte-unchanged with no `rdm` cache or marketplace directory created.

This script is **developer-run and carries no CI coverage** — CI runs only `scripts/verify-plugin-install.sh`.

```

==> 1. Snapshotting the invoking user's real ~/.claude config (before)
    FILE 2750462618 1528 /Users/edward/.claude/settings.json
    FILE 616204153 24 /Users/edward/.claude/plugins/config.json
    FILE 1133680374 1419 /Users/edward/.claude/plugins/installed_plugins.json
    FILE 553281503 277 /Users/edward/.claude/plugins/known_marketplaces.json
    ABSENT-DIR /Users/edward/.claude/plugins/cache/rdm
    ABSENT-DIR /Users/edward/.claude/plugins/marketplaces/rdm
[ok] before-snapshot captured

==> 1b. Isolation: CLAUDE_CONFIG_DIR=/var/folders/wh/d1mw3dm11z1_pglt1_w9t0mw0000gn/T/tmp.jeP0c4ERm3 (mktemp'd, trap-cleaned)
[ok] config isolated

==> 2. Building a temp marketplace at /var/folders/wh/d1mw3dm11z1_pglt1_w9t0mw0000gn/T/tmp.4Q36fgK5Gk
[ok] marketplace manifest copied and a fresh plugin tree emitted into /var/folders/wh/d1mw3dm11z1_pglt1_w9t0mw0000gn/T/tmp.4Q36fgK5Gk/plugins/rdm

==> 3. claude plugin validate --strict on the COMMITTED plugin tree (read-only, in place)
Validating plugin manifest: /Users/edward/Projects/rdm__worktrees/roadmap-plugin-distribution/plugins/rdm/.claude-plugin/plugin.json

✔ Validation passed
[ok] committed plugin manifest validates under --strict

==> 3b. claude plugin validate --strict on the temp marketplace
Validating marketplace manifest: /var/folders/wh/d1mw3dm11z1_pglt1_w9t0mw0000gn/T/tmp.4Q36fgK5Gk/.claude-plugin/marketplace.json

✔ Validation passed
[ok] marketplace manifest validates under --strict

==> 4. claude plugin marketplace add /var/folders/wh/d1mw3dm11z1_pglt1_w9t0mw0000gn/T/tmp.4Q36fgK5Gk
Adding marketplace…✔ Successfully added marketplace: rdm (declared in user settings)
[ok] marketplace added into the isolated config

==> 4b. claude plugin install rdm@rdm
Installing plugin "rdm@rdm"...✔ Successfully installed plugin: rdm@rdm (scope: user)
[ok] plugin installed

==> 5. claude plugin list --json: exactly one entry, enabled, at the crate version
    [
      {
        "id": "rdm@rdm",
        "version": "0.18.1",
        "scope": "user",
        "enabled": true,
        "installPath": "/var/folders/wh/d1mw3dm11z1_pglt1_w9t0mw0000gn/T/tmp.jeP0c4ERm3/plugins/cache/rdm/rdm/0.18.1",
        "installedAt": "2026-08-03T20:11:21.254Z",
        "lastUpdated": "2026-08-03T20:11:21.254Z"
      }
    ]
[ok] one entry: id=rdm@rdm, enabled=true, version=0.18.1
[ok] installPath resolves inside the isolated config: /var/folders/wh/d1mw3dm11z1_pglt1_w9t0mw0000gn/T/tmp.jeP0c4ERm3/plugins/cache/rdm/rdm/0.18.1

==> 5b. Installed skill inventory equals the emitted inventory
[ok] 11 skills installed: autopilot backlog dispatch-phase do document estimate land plan-review review revise roadmap 

==> 5c. Installed workflow scripts, asserted ON THE FILESYSTEM (never via plugin details — see gap 2)
[ok] 2 workflow scripts installed byte-identical: rdm-wf-dispatch-phase.js rdm-wf-review-refute-fix.js 

==> 5d. Corroboration only: claude plugin details rdm
    rdm 0.18.1
      rdm's planning lane for Claude Code: skills for creating roadmaps, implementing and reviewing phases and tasks, and landing finished work, plus the Workflow engines they dispatch.
      Source: rdm@rdm
    
    Component inventory
      Skills (11)  autopilot, backlog, dispatch-phase, do, document, estimate, land, plan-review, review, revise, roadmap
      Agents (0)
      Hooks (0)
      MCP servers (0)
      LSP servers (0)
    
    Projected token cost
      Always-on:   ~367 tok   added to every session
    
    Per-component (rounded)
      component       always-on  on-invoke
      land                  ~60      ~1.8k
      do                    ~30      ~4.5k
      roadmap              < 20       ~540
      backlog               ~50      ~1.7k
      document             < 20       ~480
      review               < 20      ~6.4k
      estimate              ~20       ~760
      revise                ~30      ~1.4k
      autopilot             ~60      ~3.8k
      dispatch-phase        ~60      ~2.3k
      plan-review           ~20      ~6.7k
    
      On-invoke cost is paid each time a skill or agent fires.
      Token counts are estimates and may differ from actual usage.
[ok] details rendered (note: it has no Workflows category — 5c is the authority)

==> 6. Snapshotting the real ~/.claude config (after) and requiring it byte-unchanged
[ok] real ~/.claude config is byte-identical before and after, with no rdm cache or marketplace directory

==> 6b. Positive proof the write landed in the isolated dir instead
[ok] /var/folders/wh/d1mw3dm11z1_pglt1_w9t0mw0000gn/T/tmp.jeP0c4ERm3/plugins/cache/rdm/rdm/0.18.1 exists

==> All plugin-install observations passed (developer-run; NOT covered by CI).
```

The missing-`claude` path is likewise exercised locally:

```
$ env PATH=/usr/bin:/bin sh scripts/observe-plugin-install.sh; echo "EXIT=$?"
[NOTICE] claude was not found on PATH — the real-install observation was SKIPPED.
          This is NOT a pass. Install the Claude Code CLI and re-run to observe.
          The hermetic half (scripts/verify-plugin-install.sh) covers everything CI gates.
EXIT=2
```

## Which copy runs?

rdm is distributed via three surfaces, each with a distinct purpose and audience:

### 1. Source of Truth: `rdm-core/src/templates/`

The **authoritative source** for all distributed artifacts — the skills, workflow engines, and templates that all other surfaces derive from. When you modify a skill or workflow, you edit the template in this tree:

- **Skills:** `rdm-core/src/templates/skill-*.md` (11 skills: autopilot, backlog, dispatch-phase, do, document, estimate, land, plan-review, review, revise, roadmap)
- **Workflows:** `rdm-core/src/templates/workflows/rdm-wf-*.js` (2 engines: rdm-wf-dispatch-phase.js, rdm-wf-review-refute-fix.js)
- **Generators:** `rdm-cli`'s `agent-config` command emits these templates verbatim (with substitutions for `{rdm_bin}`, `{project}`, etc.) to create downstream artifacts.

Invocation: the emitted skills invoke bare `rdm` (e.g., `./target/debug/rdm phase list …`).

### 2. Plugin Installation: the Emitted Plugin Tree

What **downstream consumers install** via `claude plugin marketplace add` and `claude plugin install`. Built by `rdm agent-config claude --plugin --out <dir>`, emitted to `plugins/rdm/` in this repo for release. Consumers never see the templates — they only see the installed plugin.

**Layout:** 11 skills (`rdm:roadmap`, `rdm:dispatch-phase`, etc., with the `rdm-` prefix dropped per the naming decision) and 2 workflow engines (`rdm:rdm-wf-dispatch-phase`, `rdm:rdm-wf-review-refute-fix`, with the `rdm-wf-` prefix kept for disambiguation).

**Invocation:** skills resolve as `rdm:<name>` (e.g., `rdm:roadmap`), and they invoke workflows via `Workflow({ name: "rdm:rdm-wf-dispatch-phase", … })`.

**rdmBin Resolution:** the emitted plugin shims must resolve the rdm binary at runtime using these strategies, in order of precedence: `--rdm-bin` flag, `RDM_BIN` environment variable, or `rdm` on PATH. See § "Decision 4: Runtime Arguments Delivery" for details.

### 3. This Repo's Lane: `.claude/skills/` and `.claude/workflows/`

The **local copies** that **this repo** (`rdm` source) runs internally — deliberately different from the templates to support development and testing. This repo **never installs its own plugin**; it uses these local copies instead.

**Hand-maintained dogfood:** Most `.claude/skills/` files are curated local copies of the skills, tuned for development:
- `.claude/skills/rdm-roadmap/SKILL.md`
- `.claude/skills/rdm-do/SKILL.md`
- `.claude/skills/rdm-revise/SKILL.md`
- `.claude/skills/rdm-backlog/SKILL.md`
- `.claude/skills/rdm-autopilot/SKILL.md`
- etc.

These are hand-edited to accommodate repo-local paths, hoisted arguments, and worktree-specific logic. Edit them freely — they are not code.

**Generated artifacts (drift-gated):** The following files are **generated** by the stamping scripts and **must never be hand-edited**:
- `.claude/skills/rdm-review/SKILL.md` — stamped from `.claude/workflows/lib/review.mjs` by `scripts/gen-skill-review.sh`
- `.claude/skills/rdm-plan-review/SKILL.md` — stamped from `.claude/workflows/lib/review.mjs` by `scripts/gen-skill-review.sh` (plan mode variant)
- `.claude/workflows/rdm-wf-estimate.js` — stamped from `.claude/workflows/lib/estimate.mjs` by `scripts/gen-workflow-estimate.sh`

**Generated with hand-copied driver blocks:** Some workflow files contain large stamped blocks (generated by a script) plus a smaller trailing hand-copied driver section. These must never be hand-edited in the generated portions, but the driver blocks are hand-maintained:
- `.claude/workflows/rdm-wf-dispatch-phase.js` — lines 61–1787 are generated by `scripts/gen-workflow-review.sh` (stamped from `.claude/workflows/lib/review.mjs`), and lines 1792+ contain the hand-copied `dispatch-outcome` driver block verified byte-identical by `scripts/verify-workflow-dispatch.sh` against `.claude/workflows/lib/dispatch-phase.mjs`
- `.claude/workflows/rdm-wf-review-refute-fix.js` — lines 52–1778 are generated by `scripts/gen-workflow-review.sh` (stamped from `.claude/workflows/lib/review.mjs`); the trailing driver code is structurally gated by `scripts/verify-workflow-review-outcome.sh`, not byte-checked against a single lib module
- `.claude/workflows/rdm-wf-plan-review.js` — large stamped block (generated by `scripts/gen-workflow-review.sh` from `.claude/workflows/lib/review.mjs`) followed by a hand-copied `plan-review-driver` block verified byte-identical by `scripts/verify-workflow-review.sh` against `.claude/workflows/lib/plan-review.mjs`

**Byte-checked (not generated, hand-maintained driver blocks only):** The following files contain hand-copied driver blocks without generated sections, verified byte-identical by their CI harnesses:
- `.claude/workflows/rdm-wf-backlog.js` — byte-checked against `.claude/workflows/lib/backlog.mjs` by `scripts/verify-workflow-backlog.sh`
- `.claude/workflows/rdm-wf-document.js` — byte-checked against `.claude/workflows/lib/document.mjs` by `scripts/verify-workflow-document.sh`

When you hand-edit the generated portions of a file (lines between the `// >>> … <<<` markers), the corresponding drift gate turns red. Regenerate the file with the canonical generator script (e.g., `scripts/gen-workflow-review.sh`, no `--check`), then the gate passes again.

When you hand-edit a byte-checked driver block (which should never happen), the drift gate turns red. Restore the file from git.

**Invocation:** this repo's skills invoke `./target/debug/rdm` (the local debug binary built during development).

### Why This Repo Doesn't Install Its Own Plugin

This repo runs its local `.claude/` lane and never installs the distributed plugin, for two reasons:

1. **Development-build requirement:** rdm's own development is guided by the hard rule "ALWAYS run `cargo build` before any rdm command; ALWAYS use `./target/debug/rdm`, never a globally installed `rdm`." The plugin surface is designed for distributed consumers who have no local source repo — it has no way to invoke an unbuilt binary. Installing the plugin here would break this invariant and cause silent use of a stale installed version instead of the freshly built one.

2. **Generated-artifact drift-gate root:** The `.claude/` tree is the drift-gate root for the stamping scripts and their CI harnesses. Every harness verifies that committed generated files are byte-identical to fresh output; this verification is anchored to the repo-local committed copies. If this repo installed the plugin (which replaces the source-tree paths), the harnesses would have no local copy to check against and could not enforce drift detection. Downstream consumers, by contrast, never see `rdm-core/src/templates/` and don't need to run the generators — they install the pre-built plugin as-is.

---

**Summary table:**

| Surface | Purpose | Consumer | Invocation | Binary Path |
|---------|---------|----------|-----------|-------------|
| `rdm-core/src/templates/` | Authoritative source | Generator input | N/A | N/A |
| Emitted plugin (`plugins/rdm/`) | Distributed installation | End users | `rdm:<skill>`, `rdm:rdm-wf-<engine>` | Resolved at runtime from `--rdm-bin` flag, `RDM_BIN` env var, or `rdm` on PATH |
| Local `.claude/` lane | Development & testing | This repo | `./target/debug/rdm` | Built local binary |
