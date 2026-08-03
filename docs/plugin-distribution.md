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

- **Emitted skills (11 total):** autopilot, backlog, dispatch-phase, do, document, estimate, land, plan-review, review, revise, roadmap
- **Emitted workflow engines (2 total):** rdm-wf-dispatch-phase, rdm-wf-review-refute-fix

Note: `dispatch-phase` appears in **both** lists — it is a thin skill shim that invokes the `rdm-wf-dispatch-phase` workflow engine.

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

Workflow engines (e.g., `rdm-wf-dispatch-phase.js`) require these arguments:

- **`rdmBin` (REQUIRED):** Path to the rdm binary. No ambient fallback; an absent key throws before the first `agent()` call.
- **`project` (OPTIONAL):** The rdm project name. If absent, rdm uses `RDM_PROJECT` environment variable or the `default_project` configured in `rdm.toml`.

### Resolution Strategy for Plugin-Installed Shims

When rdm is installed as a plugin, the consumer tree has no repo-local `./target/debug/rdm` path. The emitted skill shim must resolve `rdmBin` at runtime using these strategies, in order of precedence:

1. **Explicit `--rdm-bin` CLI flag** (if the skill supports it): Consumer passes the path explicitly.
2. **`RDM_BIN` environment variable:** Consumer has set `RDM_BIN=/path/to/rdm` in their environment.
3. **`rdm` in `PATH`:** Shim executes `rdm` directly, relying on PATH lookup (requires global installation or consumer having rdm in PATH).
4. **Standard installation locations** (optional, as a convenience): Check `/usr/local/bin/rdm`, `/opt/homebrew/bin/rdm` (macOS), etc.

If all strategies fail, the shim must emit a **clear, actionable error message:**

```
Error: rdm binary not found. Please:
  1. Install rdm: [link to installation docs]
  2. Set RDM_BIN=/path/to/rdm, OR
  3. Ensure 'rdm' is in your PATH, OR
  4. Pass --rdm-bin /path/to/rdm (if supported by this command)
```

### Project Resolution

If the `project` argument is not supplied to the Workflow call, rdm uses its standard resolution chain:

1. `--project` CLI flag (if passed by the shim)
2. `RDM_PROJECT` environment variable
3. `default_project` in `rdm.toml` (searched up from the current working directory)

If rdm cannot resolve the project, it emits a clear error message directing the consumer to set one of the above.

### Notes on Implementation

- The skill shim (e.g., `rdm-dispatch-phase`) is generated in Phase 2 and must contain the `rdmBin` resolution logic at the point where it invokes the Workflow tool.
- This does not change the Workflow engines themselves (e.g., `rdm-wf-dispatch-phase.js`). They continue to fail fast if `rdmBin` is missing, exactly as they do in the raw-skills distribution.
- Error messages must be tested as part of Phase 3's integration tests (the harness that validates plugin installation and command execution).

## Fixed Plugin Layout

The following layout decisions were established in the roadmap's "Central constraint — RESOLVED" section. This section restates them so later phases have a single canonical reference.

### Directory Structure

```
<plugin-root>/
  .claude-plugin/
    plugin.json              # Plugin manifest (standard Claude Code format)
  skills/
    rdm-roadmap/             # Skill implementations (renamed from SKILL.md)
    rdm-dispatch-phase/
    …                        # (11 total skills)
  workflows/
    rdm-wf-dispatch-phase.js # Workflow engines (2 total)
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
| **rdmBin resolution** | Required; resolve via flag, env var, PATH, or standard locations; fail with actionable error if not found. | Enables plugin use in environments without repo-local binaries; clear error message guides consumer setup. |
| **Disambiguator** | `rdm-wf-` prefix survives | Prevents collision between `rdm-dispatch-phase` skill and `rdm-wf-dispatch-phase` engine. |

## Next Steps for Later Phases

- **Phase 2:** Implement the plugin manifest generator in `rdm-core/src/agent_config.rs` and the `--plugin` CLI in `rdm-cli`. Apply the name transformations at emission time.
- **Phase 3:** Extend the disjointness validation to plugin-mode artifacts. Update or create a gate that checks the emitted plugin for naming collisions. Add the `agent-config claude --plugin` CLI surface.
- **Phase 4:** Package the plugin for marketplace distribution and implement the install harness.
- **Phase 5:** Document the dogfood resolution and recommended distribution path.

All path examples in code, CLI output, and documentation must continue to use the source-tree prefix (`rdm-wf-`), even in plugin contexts, to maintain consistency across distribution channels.
