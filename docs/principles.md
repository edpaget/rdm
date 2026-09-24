# Architectural Principles

This document codifies the architectural principles that govern the rdm codebase. These principles are intended to be enforced — by convention, by tests, by linting, and by automated review.

---

## 1. Core Is the Source of Truth

All business logic, data models, parsing, and domain rules live in `rdm-core`. **All plan-repo persistence is abstracted behind the `Store` trait** — core never reads or writes a plan document directly, and the storage implementations live in separate crates (`rdm-store-fs`, `rdm-store-git`). CLI and server crates are thin layers that wire a concrete store to core and format output.

- **New interfaces call core.** Whether it's a TUI or a WASM module, new frontends import and call `rdm-core`. They do not duplicate logic.
- **Core has no knowledge of its consumers or its storage backend.** `rdm-core` must never depend on `rdm-cli`, `rdm-server`, `rdm-store-fs`, or any interaction-layer crate. Dependencies flow strictly downward. Core reads and writes plan data through the `Store` trait, never through concrete I/O types.
- **Formatting belongs in core when reusable.** Display logic that multiple interfaces need (e.g., `format_index()`, `format_search_results()`) lives in `rdm-core::display`. Interface-specific formatting (e.g., terminal colors, HTML templates) stays in the consuming crate.
- **Direct I/O in core is confined to three non-plan-data categories.** The `Store` trait abstracts *plan documents*; it was never the seam for machinery that must exist before, around, or outside a store. Core therefore touches `std::fs` directly in exactly three cases, and no others:
  - **(a) Locating the plan repo** — `root.rs` and global-config resolution. The root has to be resolved before a `Store` can be constructed, so this cannot go through one.
  - **(b) Out-of-band coordination state** — `session/{lease,journal,process}.rs` and `lock.rs`. These are records under `.git/rdm/`, deliberately never committed, whose whole job is to *guard* a store flush. Routing them through the store would both stage coordination bookkeeping as plan data and be circular: a flush lock cannot be acquired through the flush it protects.
  - **(c) Emitting artifacts outside a plan repo** — `agent_config.rs`, which writes skill and plugin trees into a *consumer's* source repo. That repo is not a plan repo and has no store.

  Anything representing plan data goes through the `Store`, without exception. A new direct-I/O module in core must fall into one of these three categories and name which one in its module docs; anything else belongs in an I/O crate.

### Why

A single source of truth prevents logic drift between interfaces. When the rules for parsing a phase document or validating a status transition live in one place, every consumer gets the fix or feature automatically. Duplicated logic is a bug waiting to diverge. Keeping plan-data I/O out of core makes the domain logic pure and testable — every test of it runs in-memory with no filesystem setup or cleanup. The three carve-outs are drawn narrowly for the same reason the rule exists: each one is machinery a store depends on rather than data a store holds, so pushing it behind the trait would buy no testability and would invert the dependency.

---

## 2. Interaction Layers Are Thin Adapters

CLI commands, HTTP handlers, and any future interaction modes parse input, call core, and format output. They contain no business logic of their own.

- **CLI commands are argument parsing + core calls.** A `clap` handler destructures its arguments, calls the appropriate `rdm-core` function, and prints the result. If a CLI handler needs an `if` that isn't about argument validation or output formatting, that logic probably belongs in core.
- **HTTP handlers are extractors + core calls.** An `axum` handler extracts path params, query params, and request body; calls core; and returns a response. Content negotiation and error-to-status-code mapping are the handler's only responsibilities.
- **No cross-contamination.** CLI code must never import from the server crate or vice versa. Shared concerns go in core.

### Why

Thin adapters are easy to test, easy to replace, and impossible to accidentally couple to a specific interaction mode. If business logic lives in a CLI handler, the HTTP API can't use it without duplication.

---

## 3. I/O Lives Behind the Store Trait

All **plan-repo** persistence in `rdm-core` goes through the `Store` trait. Core never touches `std::fs`, `std::io`, or any concrete I/O type to read or write a plan document. Storage implementations live in dedicated crates outside core.

- **`Store` is the only seam for plan data.** `PlanRepo<S: Store>` is generic over any `Store` implementation. Core reads, writes, and deletes plan documents through this trait — it has no other path to them.
- **Implementations are separate crates.** `rdm-store-fs` provides `FsStore` (filesystem with atomic writes via temp-file + rename). `rdm-store-git` provides `GitStore` (wraps `FsStore` and adds git commits). New backends (e.g., S3, SQLite) would be new crates implementing `Store`.
- **Staging semantics are built in.** `Store::write()` and `Store::delete()` stage changes; `Store::commit()` flushes them atomically. Reads see staged changes before commit (read-your-own-writes). This lets core batch mutations without partial writes hitting disk.
- **`MemoryStore` is a first-class implementation.** The in-memory store in core is not a test mock — it is a complete `Store` implementation with full staging semantics. Core tests use it directly.
- **Store *machinery* is not store *data*.** The §1 carve-outs — repo-root resolution, the `.git/rdm/` session lease/journal, the `lock::AdvisoryLock` primitive, and `agent_config`'s emission of artifacts into a consumer's repo — reach the filesystem directly and are closed to extension without amending this document. They live in core rather than in a backend crate because more than one backend needs the same behavior and duplicating a concurrency state machine across `rdm-store-fs` and `rdm-store-git` is the failure mode this rule exists to prevent. They are deliberately mechanism-only: no domain types, no plan-document knowledge, no `Store` access.

### Why

Keeping I/O out of the library makes core a pure-logic crate: no filesystem assumptions, no cleanup, no platform-specific behavior. Tests run in microseconds against `MemoryStore`. The trait boundary also makes the storage backend a deployment decision — CLI users get git-backed storage, the server gets filesystem storage, and tests get in-memory storage, all without changing a line of business logic.

---

## 4. Tests Observe Behavior; Be Honest About Gaps

Every behavior must be covered by automated tests. A test runs the code and observes what it does. Tests never assert that a string is present in or absent from a source, template, prose, or `CHANGELOG.md` file — planted-mutation self-tests do not make such a check evidence of behavior. Regenerating a generated artifact and comparing it to a committed copy is not a text check. A test that breaks on a correct refactor is a defect.

When a behavior cannot be exercised (a model following a prompt, the real Claude host runtime, a live account), state the gap where the item's review will see it — e.g. "verified by dogfooding" or "not covered". Do not cover it with a vacuous test.

- **Follow TDD.** Write a failing test first, then the minimum code to make it pass, then refactor.
- **Unit tests live next to the code.** Use `#[cfg(test)] mod tests` in the same file. Test internal logic through the module's public interface.
- **Core tests use `MemoryStore`.** Since core's plan-data path is I/O-free, tests of it run against the in-memory `Store` implementation. This makes tests fast, deterministic, and free of filesystem setup/cleanup. No mocking — `MemoryStore` is a real `Store` implementation, not a mock. The §1 direct-I/O carve-outs are the exception, and necessarily so: they test against a `TempDir`, because real filesystem behavior — an interrupted lock holder, a stale lease, a partially written journal — is the thing under test.
- **Integration tests use real artifacts.** CLI integration tests spawn the compiled binary with `assert_cmd`, write to a `TempDir`, and assert on stdout/stderr with `predicates`. Server integration tests start a real TCP listener and make HTTP requests with `reqwest`. These tests exercise the full stack including real filesystem I/O through `FsStore`/`GitStore`.
- **Doctests are encouraged for public API.** Examples in `///` doc comments are compiled and run by `cargo test`. They serve as both documentation and regression tests.

### Why

Tests are the primary defense against regressions. A test that asserts file text in lieu of exercising behavior is vacuous — it confirms the test exists, not that the code works. Core's `Store` abstraction lets unit tests run entirely in-memory — fast and deterministic — while CLI and server integration tests exercise the real filesystem and HTTP stack to catch I/O bugs. TDD keeps the design testable from the start. When behavior cannot be exercised (e.g., a prompt reaching Claude's host runtime, a live user account), stating the gap is more useful than a cheated test — it is an actionable record of what the item's review must cover outside automated tests.

---

## 5. Matchable Error Enums in Core

`rdm-core` uses hand-written error enums that implement `std::error::Error` and `Display`. Errors are matchable — no `anyhow`, no `Box<dyn Error>`, no type erasure in the library.

- **Each error variant is a domain concept.** `Error::ProjectNotFound`, `Error::DuplicateSlug`, `Error::FrontmatterParse` — each variant represents a specific failure mode that callers can match on and handle differently.
- **Display messages are user-facing.** The `Display` impl for each variant produces an actionable message: what went wrong and what the user can do about it. No raw debug output, no backtraces, no implementation details.
- **Consumers add context, not core.** CLI and server crates may wrap core errors with `anyhow::Context` to add interaction-layer details (e.g., "while processing the `roadmap show` command"). Core itself does not use `anyhow`.
- **HTTP status mapping is mechanical.** The server crate maps each error variant to an HTTP status code and RFC 9457 Problem Details response. This mapping is exhaustive — adding a new variant to the core enum forces the server to handle it.

### Why

Matchable errors let each consumer handle failures appropriately. The CLI can print a helpful message; the server can return the right status code; a library consumer can programmatically recover. Type-erased errors force every consumer into string matching or catch-all handling.

---

## 6. Documents Are YAML Frontmatter + Markdown Body

Every persistent item — roadmaps, phases, tasks, projects — is stored as a markdown file with YAML frontmatter. The `Document<T>` generic wrapper enforces this structure.

- **Frontmatter is typed.** Each item type (`Roadmap`, `Phase`, `Task`) is a struct with `Serialize` and `Deserialize` derives. The YAML frontmatter deserializes into the struct; the markdown body is a separate `String` field.
- **Parse and render are symmetric.** `Document::parse(content)` splits frontmatter from body; `doc.render()` joins them back. Round-tripping preserves content.
- **No derived index exists.** rdm no longer generates `INDEX.md` at all — the `rdm index` command and the per-mutation regeneration it superseded have both been removed (see [`docs/index-removal.md`](index-removal.md)). `rdm list --format markdown` produces a browsable snapshot on demand instead.
- **File layout is conventional.** `projects/<name>/roadmaps/<slug>/roadmap.md`, `projects/<name>/tasks/<slug>.md` — the path encodes the hierarchy. Core functions resolve paths from slugs; consumers never construct paths manually.

### Why

Markdown with YAML frontmatter is human-readable, diff-friendly, and git-native. Typed frontmatter catches schema errors at parse time rather than at use time. A conventional file layout means the filesystem *is* the database — no separate index to keep in sync (INDEX.md was a convenience view, not a source of truth; that framing is what motivated removing it as a generated artifact entirely — see [`docs/index-removal.md`](index-removal.md)).

---

## 7. Status Enums Encode Valid Transitions

Status types (`PhaseStatus`, `TaskStatus`) are enums with `Display` and `FromStr` implementations. Valid transitions are enforced — terminal states cannot be exited.

- **Kebab-case serialization.** Statuses serialize as `"not-started"`, `"in-progress"`, `"done"` — matching the YAML frontmatter convention. Use `#[serde(rename_all = "kebab-case")]` for consistency.
- **Terminal states are documented.** `done` and `wont-fix` are terminal for tasks; `done` and `wont-fix` are terminal for phases. The core library enforces these constraints.
- **FromStr errors are actionable.** An invalid status string produces an error message listing all valid options, not just "parse error."

### Why

Encoding status rules in the type system prevents invalid states from being representable. A phase cannot be "done" and then moved back to "in-progress" without the system explicitly allowing it. This is cheaper to enforce at the type level than with runtime checks scattered across the codebase.

---

## 8. Public API Is Fully Documented

`rdm-core` enforces `#![warn(missing_docs)]`. Every public type, function, method, and module has a doc comment.

- **`# Errors` is required on `Result`-returning functions.** List each error variant and when it occurs. This is the function's contract with its callers.
- **`# Panics` is required if the function can panic.** Describe the conditions. Callers need to know what invariants they must uphold.
- **`# Examples` are encouraged on public entry points.** Doctests serve as both documentation and regression tests.
- **`# Safety` is required on any `unsafe fn`.** Document the invariants the caller must uphold.
- **Prefer self-documenting signatures.** Descriptive parameter names and newtypes are better than `# Arguments` sections. If the type signature tells the story, don't repeat it in prose.

### Why

`rdm-core` is a library. Its public API is a contract. Undocumented functions force consumers to read the implementation to understand behavior, error conditions, and edge cases. `warn(missing_docs)` makes documentation a compile-time requirement, not an afterthought.

---

## 9. Module Public API via Re-exports

`rdm-core`'s `lib.rs` re-exports the crate's public API. Consumers import from `rdm_core::` directly, without reaching into submodules.

- **Re-export public types from `lib.rs`.** If `model.rs` defines `Roadmap`, `Phase`, and `Task`, consumers write `use rdm_core::Roadmap`, not `use rdm_core::model::Roadmap`.
- **Submodules are implementation details.** The internal organization of `rdm-core` (which types live in which file) can change without breaking consumers, as long as re-exports are updated.
- **Keep re-exports intentional.** Only export types that are part of the crate's public API. Internal helpers remain `pub(crate)` or private.

### Why

Consolidated re-exports make the crate easier to use, reduce import churn when internals are reorganized, and make the public API surface visible at a glance in `lib.rs`. When every import goes through the crate root, it's straightforward to audit what's exposed.

---

## 10. No Unsafe Without Safety Comments

`unsafe` blocks and functions require a `// SAFETY:` comment explaining the invariant that makes the usage sound. Prefer safe alternatives.

- **Justify, don't just annotate.** The safety comment must explain *why* the invariant holds, not just restate the requirement. "SAFETY: the pointer is non-null" is insufficient; "SAFETY: `alloc()` returns a non-null pointer or panics, so this pointer is guaranteed non-null" is acceptable.
- **Prefer safe alternatives.** If a safe API exists that accomplishes the same goal with acceptable performance, use it. `unsafe` is a last resort, not an optimization shortcut.

### Why

`unsafe` is Rust's escape hatch from the borrow checker and type system. Every `unsafe` block is a promise that the programmer has verified an invariant the compiler cannot check. Without a comment explaining that verification, the promise is unauditable.

---

## 11. Content Negotiation at the HTTP Boundary

The server crate serves multiple representations of the same resource based on the `Accept` header. Clients get the format they need without separate endpoints.

- **HAL+JSON for API consumers.** `application/hal+json` responses include `_links` for discoverability and `_embedded` for related resources. Clients navigate the API through links, not hardcoded URL patterns.
- **HTML for browsers.** `text/html` responses render Askama templates. The same handler serves both formats — no separate "API" and "web" route trees.
- **RFC 9457 Problem Details for errors.** Error responses use `application/problem+json` with `type`, `title`, `status`, and `detail` fields. This gives API consumers structured, parseable errors instead of raw strings.

### Why

Content negotiation lets one URL serve multiple consumers. A browser and a CLI tool can both `GET /projects/rdm/roadmaps` and receive the representation they understand. This eliminates URL proliferation and keeps the API surface small.

---

## 12. Workspace Dependency Management

All crate dependencies are declared in the workspace root `Cargo.toml` under `[workspace.dependencies]`. Individual crates reference them with `dep.workspace = true`.

- **Versions are pinned once.** A dependency's version appears in exactly one place — the workspace root. Individual crates inherit it. This prevents version skew between `rdm-core`, `rdm-cli`, and `rdm-server`.
- **Features are specified at the usage site.** If only one crate needs `serde/derive`, that crate activates the feature. The workspace root declares the base dependency; crates add features as needed.
- **Optional dependencies use feature flags.** If `rdm-server` becomes optional for CLI-only users, it is gated behind a cargo feature flag. Users who don't need the server skip its dependency tree entirely.

### Why

Centralized dependency management prevents the "works on my crate" problem where two crates in the same workspace use different versions of the same library. It also makes dependency auditing (`cargo deny`) straightforward — there's one place to check.

---

## 13. Conventional Commits and Changelogs

Every commit follows the [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) format. Every user-facing change gets a changelog entry.

- **Commit format: `type(scope): description`.** Types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`. Scopes: `core`, `cli`, `server`, or omit for cross-cutting.
- **Changelog follows Keep a Changelog.** An `[Unreleased]` section collects pending changes. Categories: Added, Changed, Deprecated, Removed, Fixed, Security. Entries move to a versioned section on release.
- **The entry lands in the same commit as the change.** A user-facing change and its changelog entry are one commit — entries are never deferred to a follow-up or batched at release time.
- **Commits tell *why*, not *what*.** The diff shows what changed. The commit message explains the motivation.
- **No test, harness, or CI step may assert on `CHANGELOG.md`.** Never assert that `[Unreleased]` contains a word, names a file, or describes a feature; never assert it is co-staged with a code change; never write a planted-mutation self-test over the changelog body. Release automation moves the whole `[Unreleased]` body into a versioned section, so any such check goes red on `main` the moment a release lands. The changelog rule is enforced by review, not by a gate. Assert on the code or the emitted artifact, never on the prose describing it.

### Why

Structured commits enable automated changelog generation, semantic versioning, and bisect-friendly history. A developer reading `git log` can quickly understand the intent behind each change without reading the diff. Entries are kept current by review and developer discipline. A test over changelog prose goes red on `main` the moment a release moves `[Unreleased]` into a versioned section, blocking the release — the very failure this rule exists to prevent.

---

## 14. No Guards Verifying Something Else Is Still True

A gate that only confirms a value already fetched or a guard keeping hand-written prose in sync with code is a form of vacuous test. Two specific shapes are banned.

- **One:** a second read across an agent boundary to confirm a value already fetched elsewhere. The value was authoritative at the first read. A concurrent change will expose the gap in the same way whether the check exists or not — it is pretense of protection rather than the real thing.
- **Two:** a guard that keeps hand-written prose (comments, documentation, type annotations) in sync with code. Stale prose is accepted and the project learns from practice; a defect that results from it is a real bug and belongs in the backlog as a task, not hidden in a test.
- **Exception:** regenerating a generated artifact (e.g. a skill template, a plugin tree, an `INDEX.md` snapshot) and comparing it to the committed copy is **not** this pattern. The regeneration runs the generator function — exercising behavior per §4 — and the comparison is the assertion, not the guard. A `--check` drift gate is valid.

### Why

Guards that re-read a value betray a false premise — that a second read is cheaper or more reliable than the first. Hand-written-prose guards are pretense that stale docs are worse than a brittle test; the test breaks on every refactor that doesn't touch the prose, and the prod bug happens anyway. Learning from defects (including documentation defects) is the right response; encoding the prose state in a test is not.

---

## 15. Acceptance Criteria Are Satisfiable from the Artifacts

Acceptance criteria never require a future human-driven run outside the work unit. Everything needed to verify the criterion is either (a) the code, tests, or documentation the work produces, or (b) something already in the repo before the work starts.

### Why

An AC that requires a human to follow steps after the work lands is a hidden acceptance gate, not a way to verify completion. It delays signoff and is easy to forget. If an AC requires manual verification, the work isn't done.

---

## 16. Nothing Is Built for the rdm Repo Alone

If it isn't shipped in the same unit of work, it isn't built. This applies to features the repo uses but does not ship: dogfood-only workflows, unpublished skill or agent definitions, and autonomous-lane engines. **Tests, generators, and CI verification harnesses that exercise the shipped code are not covered by this rule.**

### Why

Dogfood-only work rots into hardcoded forks that diverge from the shipped path. Building twice — once for internal use and once for distribution — is a recipe for drift. Ship early, use what's shipped, and iterate on one codebase.
