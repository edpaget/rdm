# Architectural Principles

This document codifies the architectural principles that govern the rdm codebase. These principles are intended to be enforced — by convention, by tests, by linting, and by automated review.

---

## 1. Core Is the Source of Truth

All business logic, data models, parsing, and domain rules live in `rdm-core`. The core crate performs **no filesystem or network I/O** — all storage is abstracted behind the `Store` trait, and I/O implementations live in separate crates (`rdm-store-fs`, `rdm-store-git`). CLI and server crates are thin layers that wire a concrete store to core and format output.

- **New interfaces call core.** Whether it's a TUI, an MCP server, or a WASM module, new frontends import and call `rdm-core`. They do not duplicate logic.
- **Core has no knowledge of its consumers or its storage backend.** `rdm-core` must never depend on `rdm-cli`, `rdm-server`, `rdm-store-fs`, or any interaction-layer crate. Dependencies flow strictly downward. Core operates on the `Store` trait, never on concrete I/O types.
- **Formatting belongs in core when reusable.** Display logic that multiple interfaces need (e.g., `format_index()`, `format_search_results()`) lives in `rdm-core::display`. Interface-specific formatting (e.g., terminal colors, HTML templates) stays in the consuming crate.

### Why

A single source of truth prevents logic drift between interfaces. When the rules for parsing a phase document or validating a status transition live in one place, every consumer gets the fix or feature automatically. Duplicated logic is a bug waiting to diverge. Keeping I/O out of core makes the library pure and testable — every core test runs in-memory with no filesystem setup or cleanup.

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

All persistence in `rdm-core` goes through the `Store` trait. Core never touches `std::fs`, `std::io`, or any concrete I/O type. Storage implementations live in dedicated crates outside core.

- **`Store` is the only I/O seam.** `PlanRepo<S: Store>` is generic over any `Store` implementation. Core reads, writes, and deletes through this trait — it has no other path to the outside world.
- **Implementations are separate crates.** `rdm-store-fs` provides `FsStore` (filesystem with atomic writes via temp-file + rename). `rdm-store-git` provides `GitStore` (wraps `FsStore` and adds git commits). New backends (e.g., S3, SQLite) would be new crates implementing `Store`.
- **Staging semantics are built in.** `Store::write()` and `Store::delete()` stage changes; `Store::commit()` flushes them atomically. Reads see staged changes before commit (read-your-own-writes). This lets core batch mutations without partial writes hitting disk.
- **`MemoryStore` is a first-class implementation.** The in-memory store in core is not a test mock — it is a complete `Store` implementation with full staging semantics. Core tests use it directly.

### Why

Keeping I/O out of the library makes core a pure-logic crate: no filesystem assumptions, no cleanup, no platform-specific behavior. Tests run in microseconds against `MemoryStore`. The trait boundary also makes the storage backend a deployment decision — CLI users get git-backed storage, the server gets filesystem storage, and tests get in-memory storage, all without changing a line of business logic.

---

## 4. All Code Must Be Tested

Every behavior must be covered by automated tests. There are no exceptions for glue code or simple wrappers.

- **Follow TDD.** Write a failing test first, then the minimum code to make it pass, then refactor.
- **Unit tests live next to the code.** Use `#[cfg(test)] mod tests` in the same file. Test internal logic through the module's public interface.
- **Core tests use `MemoryStore`.** Since core is I/O-free, all core tests run against the in-memory `Store` implementation. This makes tests fast, deterministic, and free of filesystem setup/cleanup. No mocking — `MemoryStore` is a real `Store` implementation, not a mock.
- **Integration tests use real artifacts.** CLI integration tests spawn the compiled binary with `assert_cmd`, write to a `TempDir`, and assert on stdout/stderr with `predicates`. Server integration tests start a real TCP listener and make HTTP requests with `reqwest`. These tests exercise the full stack including real filesystem I/O through `FsStore`/`GitStore`.
- **Doctests are encouraged for public API.** Examples in `///` doc comments are compiled and run by `cargo test`. They serve as both documentation and regression tests.

### Why

Tests are the primary defense against regressions. Core's `Store` abstraction lets unit tests run entirely in-memory — fast and deterministic — while CLI and server integration tests exercise the real filesystem and HTTP stack to catch I/O bugs. TDD keeps the design testable from the start rather than bolting tests on after the fact.

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
- **INDEX.md is generated, never edited.** The index is a derived view computed from individual files. It is regenerated on every write operation. This eliminates merge conflicts in multi-user workflows.
- **File layout is conventional.** `projects/<name>/roadmaps/<slug>/roadmap.md`, `projects/<name>/tasks/<slug>.md` — the path encodes the hierarchy. Core functions resolve paths from slugs; consumers never construct paths manually.

### Why

Markdown with YAML frontmatter is human-readable, diff-friendly, and git-native. Typed frontmatter catches schema errors at parse time rather than at use time. A conventional file layout means the filesystem *is* the database — no separate index to keep in sync (INDEX.md is a convenience view, not a source of truth).

---

## 7. Status Enums Encode Valid Transitions

Status types (`PhaseStatus`, `TaskStatus`) are enums with `Display` and `FromStr` implementations. Valid transitions are enforced — terminal states cannot be exited.

- **Kebab-case serialization.** Statuses serialize as `"not-started"`, `"in-progress"`, `"done"` — matching the YAML frontmatter convention. Use `#[serde(rename_all = "kebab-case")]` for consistency.
- **Terminal states are documented.** `done` and `wont-fix` are terminal for tasks; `done` is terminal for phases. The core library enforces these constraints.
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
- **Commits tell *why*, not *what*.** The diff shows what changed. The commit message explains the motivation.

### Why

Structured commits enable automated changelog generation, semantic versioning, and bisect-friendly history. A developer reading `git log` can quickly understand the intent behind each change without reading the diff.
