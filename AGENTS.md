# Working on rdm with Codex

rdm is a Rust CLI with a separate git-backed plan repository. Read
[architectural principles](docs/principles.md) before implementation. Domain
logic belongs in `rdm-core`; CLI/server are thin adapters, and plan persistence
goes through `Store`. “Zero-dependency” means users need only the binary, not
that Cargo dependencies are forbidden.

## Bootstrap

Follow [Codex setup and capability boundaries](docs/codex-support.md). For this
repository, set `RDM_BIN` to the absolute path of **this checkout's** executable
`scripts/rdm-dev.sh`; it rebuilds before invoking rdm. Never use a globally
installed `rdm` for dogfooding source changes. Carry explicit `RDM_ROOT` (the
separate plan repo), `RDM_PROJECT=rdm`, and one stable `RDM_SESSION` into every
shell call; exports in one tool call may not survive the next. Do not reuse a
different conversation's session ID. Obtain normal filesystem approval for
external plan/worktree paths; do not relax sandbox settings.

## Development and gates

Write a failing test first, then implement and run `cargo nextest run` with
appropriate crate/test filters. Before review run `cargo fmt --check`,
`cargo clippy -- -D warnings`, and the relevant broader tests/harnesses. CI
configuration is authoritative for landing checks. Shell changes use
`shellcheck` and `shfmt`. Public core APIs need docs, including `# Errors` on
fallible functions. No unsafe without a documented safety invariant.

Use Conventional Commits. Every user-facing commit includes its corresponding
`CHANGELOG.md` entry. Never write a test or harness that asserts on changelog
prose; release automation moves it. Preserve unrelated work and stage only
your files. Source git commits and session-scoped `rdm commit` in the plan repo
are separate. Do not use `rdm commit --all` to mask session problems.

Use the discoverable `.agents/skills/rdm-*` manual lane. Read the selected skill
fully. `rdm-roadmap` authors plans; `rdm-do` implements one authorized item and
stops at `needs-review`; `rdm-revise` addresses document feedback; `rdm-land`
requires explicit landing authorization and independent review evidence.
Independent plan review must clear the selected item's `needs-plan-review`
gate before implementation. Author checks do not clear independent code
review. If no independent review host is available, leave the gate pending
and hand off; never claim a workflow ran or mark the item `reviewed`/`done`.

Use one shared worktree per roadmap (`rdm worktree add <roadmap>`), inspect any
existing work before editing, and rebind `RDM_BIN` to the returned checkout.
Do not merge or add completion directives during ordinary implementation.
Landing follows [landing policy](docs/landing.md) with the explicit Codex
boundaries above; Claude-specific runtime and permission advice does not
apply. `.claude/` workflows are not executable Codex tools.

Local Codex skills are generated: change `rdm-core/src/templates/codex/`, then
run `sh scripts/gen-codex-skills.sh`. Do not hand-maintain a second policy copy.
Runtime templates are generated from canonical production modules by
`node scripts/gen-codex-runtime.mjs` (also run by the skill generator).
After generation changes run `cargo nextest run -p rdm-core -E 'test(codex)'`
and `cargo nextest run -p rdm-cli --test cli_agent_config --test codex_distribution --test distribution`.
Codex acceptance uses Rust-owned tests, not standalone `verify-*.sh` harnesses.
