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
| `GET` | `/index` | Get the generated index |

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
