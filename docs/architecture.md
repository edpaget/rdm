# Architecture

rdm is a Rust workspace with a layered architecture. The core library is the source of truth; CLI and server are thin adapters that parse input, call core, and format output.

## Crate Layout

```
rdm (workspace)
├── rdm-core/         # library: data model, parsing, validation, display, index generation
├── rdm-cli/          # binary: CLI porcelain over rdm-core (clap)
├── rdm-server/       # binary: REST API over rdm-core (axum)
├── rdm-mcp/          # library: MCP server over rdm-core (stdio transport)
├── rdm-store-fs/     # library: filesystem Store implementation (atomic writes)
└── rdm-store-git/    # library: git-backed Store implementation (wraps rdm-store-fs)
```

## Core Is the Source of Truth

All business logic, data models, parsing, and domain rules live in `rdm-core`. The core crate performs no filesystem or network I/O — all storage is abstracted behind the `Store` trait, and I/O implementations live in separate crates (`rdm-store-fs`, `rdm-store-git`).

New interfaces — whether a TUI, an MCP server, or a WASM module — import and call `rdm-core`. They do not duplicate logic.

## The Store Trait

All persistence goes through the `Store` trait. Core never touches `std::fs` or any concrete I/O type.

- **`PlanRepo<S: Store>`** is generic over any `Store` implementation.
- **`FsStore`** (in `rdm-store-fs`) provides filesystem storage with atomic writes via temp-file + rename.
- **`GitStore`** (in `rdm-store-git`) wraps `FsStore` and adds git commits after each mutation.
- **`MemoryStore`** (in `rdm-core`) is a first-class in-memory implementation used by all core tests — not a mock.

Staging semantics are built in: `Store::write()` and `Store::delete()` stage changes; `Store::commit()` flushes them atomically. Reads see staged changes before commit (read-your-own-writes).

## Interaction Layers Are Thin Adapters

CLI commands parse arguments with `clap`, call the appropriate `rdm-core` function, and print the result. HTTP handlers extract path/query params, call core, and return a response. Neither layer contains business logic.

The server supports content negotiation: `application/hal+json` for API consumers (with `_links` for discoverability), `text/html` for browsers (via Askama templates), and RFC 9457 Problem Details for errors.

## Error Handling

- **`rdm-core`**: hand-written error enums implementing `std::error::Error` + `Display`. Each variant is a matchable domain concept (`ProjectNotFound`, `DuplicateSlug`, `FrontmatterParse`). No `anyhow` or type erasure in the library.
- **`rdm-cli` / `rdm-server`**: use `anyhow` with `.context()` for readable error chains. The server maps each core error variant to an HTTP status code exhaustively.

## Documents Are YAML Frontmatter + Markdown

Every persistent item (roadmaps, phases, tasks, projects) is stored as a markdown file with YAML frontmatter. The `Document<T>` generic wrapper enforces this structure. `Document::parse()` and `doc.render()` are symmetric — round-tripping preserves content.

`INDEX.md` is a derived view computed from individual files and regenerated on every write operation. It is never edited by hand.

## Further Reading

- [File Formats](file-formats.md) — directory layout, frontmatter fields, and file conventions
- [Architectural Principles](principles.md) — the full set of design principles governing the codebase
