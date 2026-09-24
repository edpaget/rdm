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
# {"step":"plan","host":"claude","tier":"medium","model":"opus","effort":"medium"}
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
   large, `mechanical` small).
2. Clamp up to the step's floor: `plan` to **medium** (built in, not
   configurable — the planner is never sized below medium);
   `review-find`/`review-verify` to `review_floor` (default medium);
   `implement` follows the item tier and `mechanical` is unfloored.
3. Map the tier to the host's profile.
