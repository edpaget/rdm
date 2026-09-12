# REST API

rdm includes a REST API server for programmatic integrations beyond the CLI.

## Starting the Server

```bash
rdm serve --port 8400
```

The server binds to `127.0.0.1` by default. Use `--host` to change the bind address.

## Endpoints

Endpoints mirror the CLI commands. All endpoints accept and return JSON by default.

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/projects` | List all projects |
| `GET` | `/projects/:project/roadmaps` | List roadmaps with progress |
| `GET` | `/projects/:project/roadmaps/:roadmap` | Show roadmap details with phases |
| `POST` | `/projects/:project/roadmaps` | Create a new roadmap |
| `PATCH` | `/projects/:project/roadmaps/:roadmap/phases/:phase` | Update a phase |
| `GET` | `/projects/:project/tasks` | List tasks with optional filters |
| `GET` | `/projects/:project/tasks/:task` | Show task details |
| `POST` | `/projects/:project/tasks` | Create a new task |
| `PATCH` | `/projects/:project/tasks/:task` | Update a task |

## Mutations Are Staged, Not Committed

A running server is **one session**, and every write it makes is attributed to
that session's changeset. By default the server **stages** mutations to disk and
commits nothing — a `POST` or `PATCH` that returns `201`/`200` has written the
markdown file, but has created no git commit. Landing them is a separate,
explicit step.

That default is deliberately loud on three surfaces rather than one, because a
boot-time warning on a server that has been up for a week is indistinguishable
from no warning at all:

- a `WARN` line on stderr at boot,
- a `WARN` line on stderr for **each** mutation, and
- response headers.

### Response headers

| Header | Set on | Meaning |
|--------|--------|---------|
| `x-rdm-changeset` | every response, when the server resolved a changeset id | The changeset this server's mutations belong to. Pass it to `rdm commit --changeset <id>` to land them. |
| `x-rdm-staged` | mutating responses only (anything other than `GET`/`HEAD`/`OPTIONS`), and only under the staging-only default | The plain-ASCII warning text: `WARN: mutation staged, NOT committed. Changeset '<id>'. Reconcile with: rdm commit --changeset <id>` |

Under `--autocommit` the write did reach a commit, so `x-rdm-staged` is absent —
its presence is exactly the signal that something is pending.

### Reconciling

The named recovery command is the one carried in `x-rdm-staged`:

```bash
rdm commit --changeset <id>     # land what the server staged
rdm session list                # find the id if you did not capture a header
```

### Committing on every mutation

Pass `--autocommit` to the `rdm-server` binary, or set
`RDM_SERVER_AUTOCOMMIT=1` (`true` is also accepted; any other value keeps the
staging-only default, so an unparseable value never silently opts in). Each
mutation then lands a scoped commit of its own. `--changeset <id>` (else
`RDM_SESSION`) pins which changeset the server attributes its writes to; a
trailing `--changeset` with no argument, or a blank id, falls back to the
ordinary session-resolution chain rather than naming a changeset `""`.

These two flags are read by the standalone `rdm-server` binary. The `rdm serve`
CLI subcommand starts the same router on the staging-only default but does not
currently parse them, and does not pre-resolve a changeset id — so it emits no
`x-rdm-changeset` header and its `x-rdm-staged` text renders the id as
`<unresolved>`. Use `rdm session list` to find the changeset in that case.

### Concurrent writers

Because scoping is per changeset, a server and a CLI session working in the same
plan repo do not sweep up each other's uncommitted work. Two writers racing on
the *same file* are a different matter, and are refused rather than silently
resolved — see the `409` rows below.

## Content Negotiation

The server uses the `Accept` header to determine the response format:

- **`application/hal+json`** — JSON with `_links` for discoverability and `_embedded` for related resources. API consumers should prefer this format.
- **`text/html`** — HTML rendered from Askama templates. Browsers get a human-readable view automatically.
- **`application/json`** — Plain JSON without HAL links.

## Error Responses

Errors follow [RFC 9457 Problem Details](https://www.rfc-editor.org/rfc/rfc9457):

```json
{
  "type": "urn:rdm:error:project-not-found",
  "title": "Project not found",
  "status": 404,
  "detail": "No project with slug 'foo' exists in this plan repo."
}
```

Each `rdm-core` error variant maps to a specific HTTP status code. The mapping is exhaustive — adding a new error variant to core forces the server to handle it at compile time.

### Lost-update refusals (`409 Conflict`)

Two writers racing on the same document are refused rather than silently
resolved. Both refusals carry the core error's own message verbatim as `detail`,
and both state that **nothing was written or committed** — so a `409` is always
safe to retry after re-reading:

| Core error | When | What to do |
|------------|------|------------|
| `StaleWrite` | The document changed on disk between this request reading it and writing it back — another session wrote it concurrently. | Re-issue the request so the change applies on top of the current content. |
| `ChangesetPathOverwritten` | At commit time, another session had overwritten a path this changeset wrote, so committing would land their content under your message. | Re-run the request that produced your change, then commit. |
