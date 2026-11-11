# Model + effort profiles

`rdm model resolve <step>` turns a dispatch step into a **profile**: a model id
plus a reasoning effort, for one host. The policy lives in
`rdm-core/src/model_policy.rs`; the CLI is a thin view over it.

Where the effort is consumed: `rdm-dispatch-phase` resolves every role with
`--format json` and runs the planner and implementer as the `rdm-effort-<effort>`
agent definitions plus the resolved model, and passes `findEffort`/`verifyEffort`
to the review engines' finders and refuters; the Codex runtime runs every
judgment at the `--host codex` profile's model and effort
([`codex-runtime.md`](codex-runtime.md)).

## Tiers

Four tiers, smallest first: `small < medium < large < frontier`. Phases and
tasks keep storing `model: small|medium|large` (no migration); `frontier` is
accepted wherever a tier is parsed (`--tier`, phase `--model`,
`[models.steps]`, `review_floor`) but **no built-in default and no difficulty
ever resolves to it** — reaching it takes an explicit operator choice.

## Default table

| Tier | claude | codex |
|---|---|---|
| small | opus @ low | gpt-6-sol @ medium |
| medium | opus @ medium | gpt-6-sol @ high |
| large | opus @ high | gpt-6-astra @ medium |
| frontier | opus @ xhigh | gpt-6-astra @ xhigh |

Each host's four rungs are points on that host's own cost/quality frontier,
with effort as the main axis; tiers are not aligned across hosts. The
rationale, and why no default is Sonnet, Fable, Haiku or `gpt-6-luna`, is the
`model-effort-profiles` roadmap's "Benchmark evidence" section. An operator can
still map any model in config.

The Codex ids were confirmed against the installed catalog: `codex-cli
0.155.1`, `codex debug models`, 2026-09-23 — `gpt-6-astra`, `gpt-6-sol` and
`gpt-6-luna` listed (no `gpt-6-terra`), each supporting efforts `low`,
`medium`, `high`, `xhigh`, `max`.

## Efforts per host

- `claude`: `low`, `medium`, `high`, `xhigh`, `max` (`claude --effort`).
- `codex`: `low`, `medium`, `high`, `xhigh`. `max` is refused because the
  shipped Codex runtime's process guard rejects it, and rdm never emits an
  effort its runtime will not run.

An unknown effort fails when `rdm.toml` loads (the error lists the five valid
values); a host-unsupported one fails validation with
`invalid value 'max' for 'models.profiles.codex.<tier>.effort' — valid values:
low, medium, high, xhigh`. `rdm model` refuses to run over such a config rather
than falling back to defaults.

## Choosing the host

`rdm model resolve` and `rdm model show` take `--host claude|codex`, default
`claude`. There is no config key: each runtime knows which host it is.

```bash
rdm model resolve plan --tier small --format json
# {"step":"plan","host":"claude","preset":null,"tier":"medium","model":"opus","effort":"medium"}
rdm model resolve review-verify --host codex     # gpt-6-astra
rdm model show --host codex
```

Plain-text `resolve` output stays a bare model id, so
`model=$(rdm model resolve …)` consumers are unaffected.

## Configuration

```toml
[models]
small = "opus"                 # legacy form: claude model only, default effort
review_floor = "medium"
[models.steps]
implement = "large"
[models.profiles.claude.large]
effort = "xhigh"
[models.profiles.codex.frontier]
model = "gpt-6-astra"
effort = "xhigh"
```

Per host and tier:

- **model**: profile `model` → (claude only) legacy `[models] small/medium/large`
  → built-in table.
- **effort**: profile `effort` → built-in table.

A partial profile keeps the other field's fallback. The legacy keys set only
the `claude` host's model at the tier's default effort; they never affect
`codex` (they have always held Claude aliases, which Codex rejects). There is
no legacy key for `frontier`.

## Tier resolution

1. The caller's `--tier` hint, else the `[models.steps]` override, else the
   step's default (`plan`/`implement`/`review-find` medium, `review-verify`
   and `review-consolidate` large, `mechanical` small).
2. Clamp up to the step's floor: `plan` to **medium** (built in, not
   configurable — the planner is never sized below medium);
   `review-find`/`review-verify`/`review-consolidate` to `review_floor`
   (default medium);
   `implement` follows the item tier and `mechanical` is unfloored.
3. Map the tier to the host's profile.

## Presets

A **preset** is a named, sparse overlay on the profile table. It changes what
a tier or a role *runs on*, never which tier is picked: tier resolution above
(hint, `[models.steps]`, defaults, `PLAN_FLOOR`, `review_floor`) is untouched.

With no preset the policy is exactly the one described above. How a run picks
a preset is under [Selecting a preset](#selecting-a-preset).

```toml
[models.presets.fable-build.claude.steps.plan]
model = "fable"
[models.presets.fable-build.claude.steps.implement]
model = "fable"
effort = "high"
[models.presets.cheap.claude.tiers.large]
model = "opus"
effort = "medium"
```

Per host, a preset holds `tiers.<tier>` overrides (applied to every step that
resolves to that tier) and `steps.<step>` overrides (applied to that role at
whatever tier it resolves to; step keys use the kebab-case names `plan`,
`implement`, `review-find`, `review-verify`, `review-consolidate`,
`mechanical`). Each field of the resolved profile layers independently:

- **model**: preset `steps.<step>.model` → preset `tiers.<tier>.model` →
  the configured-or-default profile model.
- **effort**: preset `steps.<step>.effort` → preset `tiers.<tier>.effort` →
  the configured-or-default profile effort.

So a preset that only says "implement runs on fable" inherits its effort from
the tier the item resolved to, and a role override that sets only `model`
still picks up a tier override's `effort`.

### Built-in presets

Core ships two presets for the `claude` host; the `codex` host has none.

| Preset | Roles on `fable` (claude) | Left on the profile table |
|---|---|---|
| `fable` | `plan`, `implement`, `review-find`, `review-verify`, `review-consolidate` | `mechanical` |
| `fable-build` | `plan`, `implement` | the three review roles, `mechanical` |

Neither sets an effort, so each role keeps the effort of its resolved tier.

A `[models.presets.<name>]` table whose name is a built-in's **replaces** that
built-in entirely; the two are not merged. Asking for a name that is neither
built in nor configured fails (see below).

### Selecting a preset

`rdm model resolve` and `rdm model show` pick at most one preset per run. The
first of these that applies wins:

1. `--preset <name>` on the command line.
2. A non-blank `RDM_MODEL_PRESET` environment variable.
3. The `preset` key of the `[models]` table.
4. Otherwise, no preset.

```toml
[models]
preset = "fable-build"   # the default for every model command in this repo
```

Some details:

- **`--no-preset`** selects no preset for that one invocation, whatever the
  env var or config key say. It is a separate flag rather than a reserved
  value such as `none`, so every string stays a usable preset name. It
  cannot be combined with `--preset`.
- **A blank env var** (empty or whitespace only) counts as unset and falls
  through to the config key. A parent session can export `RDM_MODEL_PRESET=`
  to go back to the config default. The env var has no "none" value; use
  `--no-preset`.
- **The `preset` key** lives in `[models]`, so it follows the wholesale repo
  vs global rule below: a repo `[models]` table hides the global one's
  `preset` too. It is set by editing `[models]`, not through
  `rdm config get/set`. `rdm model show` displays the active selection.
- **Only the winning layer is checked.** An unknown name fails the command
  and names where it came from and the presets that exist, e.g.
  `unknown model preset 'nope' (from the RDM_MODEL_PRESET environment
  variable) — available presets: fable, fable-build`. The other sources read
  `--preset flag` and `[models] preset config key`. A bad config key does not
  fail a run whose flag or env var names a valid preset. An empty
  `--preset ""` is an unknown name, not "no preset".

What the output shows:

- `rdm model resolve <step> --format json` carries `"preset"`: the active
  preset's name, or `null` when none is selected. The text form stays a bare
  model id, so `model=$(rdm model resolve …)` consumers are unaffected.
- `rdm model show --format json` carries `"preset"` (name or `null`) and
  `"presets"`, every selectable name, built in and configured, sorted. The
  list does not depend on the selection.
- `rdm model show` text output starts with `preset: <name> (from
  flag|env|config)` (or `preset: none`) and `presets: fable, fable-build, …`.
- In `model show`, the per-step lines and `steps` entries include the
  preset's role overrides. The tier lines (`small:` … `frontier:`, and the
  JSON `profiles` / `small` … `frontier` fields) show the tier layer only:
  the configured-or-default profile with the preset's `tiers` overrides,
  without its `steps` overrides.

### Global vs repo config

Presets live inside `[models]`, so they follow its existing wholesale
precedence: a repo `rdm.toml` `[models]` table — even one with no presets —
hides every preset in the global config's `[models]`; a repo with no
`[models]` table inherits the global presets. There is no deep merge.

### Validation

Preset efforts are validated when the config loads, like profile efforts. A
host-unsupported effort fails naming the key, e.g.
`invalid value 'max' for 'models.presets.<name>.codex.steps.review-find.effort'`
or `… 'models.presets.<name>.codex.tiers.small.effort' — valid values: low,
medium, high, xhigh`.
