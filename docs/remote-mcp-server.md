# Remote MCP Server — Technical Design

Status: **design** — supersedes the retired `remote-mcp-server` roadmap, which bundled
four independently-shippable tracks into five phases. This document is the single
authoritative design; the implementation is planned as four small roadmaps
(see [Sequencing](#sequencing--roadmap-decomposition)).

## Problem

`rdm-mcp` today is a stdio-only, single-tenant MCP server: `run()` serves
`stdio()` and nothing else, and the server owns exactly one plan repo
(`RdmMcpServer { store: Mutex<AppStore>, plan_root, auto_init }`) chosen at
process start. That is the right shape for a local agent session, but it cannot
serve a plan repo that lives on a shared host: Claude Code web (or any remote
MCP client) needs an HTTP endpoint, per-user plan-repo scoping, and
authentication.

### Relationship to the existing Claude Code web integration

The repo already ships one answer to "use rdm from a web sandbox":
`docs/claude-code-web.md` — a `SessionStart` hook that clones the plan repo
into the sandbox and runs rdm locally over stdio/CLI. The remote MCP server is
a **complementary** second answer, not a replacement:

| | Sandbox clone (shipped) | Remote MCP connector (this design) |
|---|---|---|
| Infrastructure | none — just git credentials | a deployed, TLS-terminated server |
| Plan repo state | per-sandbox clone, synced via git push/pull | one live repo on the server, always current |
| Multi-user | each user clones independently | per-user scoping + tokens on one host |
| Offline/local CLI | full rdm CLI available | MCP tools only |

The sandbox-clone path remains the zero-infrastructure default. The remote
connector is for teams that want a persistent, shared, always-current plan repo
without per-session cloning. The deployment roadmap's final phase reconciles
the two docs so users see one coherent decision, not two competing features.

## Architecture

### Transport (roadmap: `mcp-http-transport`)

HTTP lives in `rdm-mcp`, not `rdm-server`. `rdm-server` stays what it is — the
axum REST/web-UI server for humans. The MCP protocol layer comes from rmcp
(the workspace pins **rmcp 2.0**; note the retired roadmap incorrectly said
1.4): enable rmcp's `transport-streamable-http-server` feature (name confirmed
against the vendored rmcp 2.0.0 crate) and add an HTTP entry point alongside
the stdio one. Two realities of that API shape the plan: the feature yields a
tower `Service` (`StreamableHttpService`), not a bound server — so `rdm-mcp`
gains an axum dependency to mount it, bind a listener, and serve — and its
constructor takes a **per-session server factory**, not a built `RdmMcpServer`
instance. Everything HTTP (the rmcp feature + axum) is gated behind a new
`http` cargo feature on `rdm-mcp` so stdio-only builds stay lean.

`rdm-mcp` is a **lib crate** — there is no rdm-mcp binary. The CLI owns
process entry: the `Mcp` subcommand (`rdm-cli/src/cli.rs`, currently
zero-arg → `rdm_mcp::run`) grows `--http` / `--bind <addr>` flags; stdio
remains the default so existing `.mcp.json` configs keep working. A `/healthz`
route ships with the HTTP mode (mirror `rdm-server/src/handlers/health.rs`;
keep it trivial rather than sharing code across crates).

### Multi-tenancy (roadmap: `mcp-multi-tenant`)

Today every tool locks the single shared store, and `maybe_auto_init` swaps it
in place (`*self.store.lock() = new_store`). Remote mode replaces "one store
per process" with "one store per request identity":

```
request → identity (from auth, or "local" on stdio)
        → PlanRepoResolver::resolve(identity) → plan_root
        → open AppStore for that root (per request; cache only if profiling demands it)
```

- `PlanRepoResolver` trait with two impls: `StaticResolver` (single root —
  wraps today's behavior, used by stdio mode) and `MapResolver` (identity →
  plan_root map from `rdm-mcp.toml`). `resolve` returns a hand-written error
  enum (`UnknownIdentity`, …) — matchable, so the auth layer can map it to
  401/403 — and the config schema type itself lives in `rdm-core::config`
  next to `Config`/`GlobalConfig`/`ServerConfig`; only the resolver impls are
  MCP-specific.
- **Identity seam** (owned by the multi-tenant roadmap, fed by mcp-auth
  later): an `Identity` value attached per session/request — "local" on
  stdio, a fixed default on HTTP until auth lands, plus a test-only
  header-injection hook so isolation tests can drive multiple identities
  before bearer auth exists. The auth middleware later populates this same
  seam.
- The refactor lands in two stages: first a behavior-identical
  store-acquisition helper with its final signature migrates all ~30
  handlers; then only the helper's internals flip to per-request resolution.
  Two bespoke in-place store-swap sites (`maybe_auto_init` **and** the
  `rdm_init` tool) move to per-resolved-root init; the cwd-bound worktree
  tools stay stdio-only and return a clean "not supported over remote MCP"
  error in HTTP mode.
- Removing the process-wide `Mutex<AppStore>` must not remove serialization:
  a per-resolved-root keyed lock keeps concurrent same-root requests
  serialized (working-tree writes + INDEX.md regeneration would otherwise
  race) while different roots proceed independently. GitStore staging is
  on-disk (no in-memory buffer), so it survives per-request store lifetimes
  for free — verified by a mutate-then-commit-across-requests test.
- Isolation is a correctness requirement: integration tests must alternate
  identities on one server instance and prove no state leaks across tenants,
  plus a same-tenant concurrent-mutation test for the keyed lock.

### Authentication (roadmap: `mcp-auth`)

There is currently **zero** auth infrastructure anywhere in the workspace (the
`rdm_author` cookie in rdm-server is an identity hint, not auth). Design:

- **Bearer tokens now, OAuth later.** Static bearer tokens are the MCP
  remote-connector baseline and require no third-party identity provider. The
  middleware seam (token → identity) is the same one an OAuth flow would fill
  in later.
- **Token store:** file-based, colocated with server config; secrets stored
  only as **argon2id hashes** (new workspace dependency). Plaintext is shown
  exactly once, at issuance, in the form `<token_id>.<secret>` — the id
  prefix makes verification a single record lookup plus one argon2 verify,
  never a scan over all hashes. The store file is 0600, and the middleware
  re-reads it per auth attempt so a CLI `revoke` takes effect on a running
  server without restart. An authenticated identity with no `[users.*]`
  mapping gets 403 (distinct from 401); `/healthz` bypasses auth.
- **CLI surface** (in rdm-cli, since rdm-mcp is a lib):
  `rdm mcp token issue --user <id>` (prints the one-time secret),
  `rdm mcp token revoke <id>`, `rdm mcp token list`.
- **Middleware:** HTTP-mode-only layer that maps `Authorization: Bearer …` to
  an identity fed to the `PlanRepoResolver`; anything else → 401. Stdio mode
  bypasses auth entirely (local trust, unchanged).
- **Rate limiting:** in-memory failed-auth throttle (per source), to blunt
  token guessing. Deliberately simple; not a general rate limiter.

### Configuration: `rdm-mcp.toml`

Single authoritative schema, defined once here (the retired roadmap specified
it three times in three phases):

```toml
bind = "127.0.0.1:8321"          # HTTP mode bind address (CLI --bind overrides)
token_store = "tokens.toml"       # argon2-hashed credentials, relative to this file

[users.alice]
plan_root = "/srv/rdm/alice-plans"

[users.bob]
plan_root = "/srv/rdm/bob-plans"

[rate_limit]
max_failures = 10                 # failed auths per window per source
window_secs = 60
trust_forwarded_for = false       # key on rightmost XFF hop only behind a trusted proxy
```

TLS is intentionally absent: terminate TLS at a reverse proxy (see
Deployment), keeping rdm free of certificate management.

### Deployment (roadmap: `mcp-deployment`)

- Multi-stage Dockerfile → minimal runtime image running
  `rdm mcp --http --bind 0.0.0.0:8321` with config + plan repos on volumes.
- Reference `docker-compose.yml` with a Caddy reverse proxy for TLS.
- Sample systemd unit for bare-metal hosts.
- Release integration: build/push a GHCR image from the existing cargo-dist
  release workflow. Note `rdm-mcp` and `rdm-server` currently set
  `package.metadata.dist { dist = false }` — the container is a new artifact,
  not a change to existing installers.
- `rdm agent-config --mode web-connector` (extending
  `write_mcp_json` in `rdm-cli/src/commands/agent_config.rs`) emits the
  remote-connector client config (URL + token placeholder), plus
  `scripts/mcp-smoke.sh` to validate a live deployment end-to-end.

## Done: convention over remote MCP

The `Done:` post-merge/post-commit hook pipeline runs where the plan repo
lives. With a remote server, the plan repo is on the server host, so hooks
installed there continue to work; nothing in this design changes the hook
contract. The smoke test should include one mutation + `rdm_commit` round-trip
to prove the git-backed store behaves identically over HTTP.

## Sequencing & roadmap decomposition

Four roadmaps, strictly ordered by dependency:

1. **`mcp-http-transport`** — rmcp 2.0 streamable-HTTP feature + `serve_http`;
   CLI `--http/--bind`; `/healthz`; HTTP integration test. Depends on nothing.
2. **`mcp-multi-tenant`** — resolver trait, `rdm-mcp.toml` MapResolver, the
   store-ownership refactor, isolation tests. Depends on transport.
3. **`mcp-auth`** — token store, token CLI, bearer middleware, rate limiting,
   auth tests. Depends on multi-tenant (identity feeds the resolver).
4. **`mcp-deployment`** — container/compose/systemd, GHCR release, connector
   agent-config + smoke script, doc reconciliation. Depends on auth.

**Conflict management:** `mcp-multi-tenant` rewrites the store-ownership core
of `rdm-mcp/src/server.rs` — the same file the outstanding `mcp-hardening`
(auto-init paths) and `mcp-server-parity` (tool surface) roadmaps edit. Land
those two small roadmaps **before** starting `mcp-multi-tenant`; whichever
lands second otherwise eats a large rebase. `mcp-http-transport` touches the
entry points only and can proceed in parallel with them.
