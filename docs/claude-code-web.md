# Claude Code web sandbox integration

Run a Claude Code web session against your rdm plan repo without granting
filesystem access or baking plan data into your source repo. The session-start
hook in this template clones (or fast-forwards) your plan repo into the
sandbox, installs rdm if missing, and points rdm's global config at the
resolved path. After that, every `rdm` call in the session works as if the
plan repo had always been there.

## How it works

1. The user opens a Claude Code web session on your source repo.
2. The sandbox boots and the `SessionStart` hook fires.
3. The hook script:
   - installs rdm from a GitHub Release if it's not on `PATH`;
   - runs `rdm bootstrap --plan-repo "$RDM_PLAN_REPO"` to clone or fast-forward
     the plan repo into `$XDG_DATA_HOME/rdm/plan-repo`;
   - writes `root` (and optionally `default_project`) into the global rdm
     config so later calls don't need `$RDM_ROOT` in the shell env.
4. Claude Code tools run normally for the rest of the session; `rdm` commands
   operate on the cloned plan repo. When the session ends, any `Done:` lines
   in your source-repo commits are picked up by the plan repo's local
   post-merge hook the next time you pull locally (see CLAUDE.md's Done
   convention).

## Required env vars

Set these in your Claude Code web sandbox settings (or in your devcontainer
`containerEnv`):

| Name                  | Required | Purpose                                                           |
| --------------------- | :------: | ----------------------------------------------------------------- |
| `RDM_PLAN_REPO`       |   yes    | Git URL of the plan repo (HTTPS or SSH).                          |
| `RDM_PLAN_REPO_TOKEN` | if priv  | Access token for private HTTPS plan repos. See Credentials below. |
| `RDM_DEFAULT_PROJECT` |    no    | Project name inside the plan repo to treat as the default.        |
| `RDM_PLAN_REPO_PATH`  |    no    | Override the local path the plan repo gets cloned into.           |

## Credentials

Pick one of the following, scoped as narrowly as possible — if a token leaks
from a sandbox, blast radius should be a single repo.

### Option 1: GitHub fine-grained PAT (recommended)

1. Create a [fine-grained personal access
   token](https://github.com/settings/personal-access-tokens/new) with:
   - **Resource owner**: your GitHub account or org that owns the plan repo.
   - **Repository access**: "Only select repositories" → pick the single
     plan repo.
   - **Repository permissions**: `Contents: Read and write` (use
     `Contents: Read-only` for read-only sessions).
2. Set it as `RDM_PLAN_REPO_TOKEN` in the Claude Code web sandbox's
   secret/env settings for the source repo.
3. `rdm bootstrap` reads the token from the env var and injects it into the
   clone URL. The token is persisted in the sandbox's `.git/config` so
   subsequent fast-forward pulls work without re-auth, and `rdm bootstrap
   doctor` briefly passes the token to `curl` as an `Authorization` header
   (visible in `/proc/<pid>/cmdline` on Linux for the subprocess's lifetime).
   Both are acceptable because sandboxes are ephemeral; do not use this flow
   on a shared long-lived machine.

### Option 2: SSH deploy key

1. Generate a keypair (`ssh-keygen -t ed25519`) and add the public key as a
   [deploy key](https://docs.github.com/en/authentication/connecting-to-github-with-ssh/managing-deploy-keys)
   on the plan repo (allow write access if the agent should push back).
2. Mount the private key into the sandbox via its secret mechanism.
3. Use an SSH URL (`git@github.com:owner/plan-repo.git`) for `RDM_PLAN_REPO`.
4. Leave `RDM_PLAN_REPO_TOKEN` unset — `rdm bootstrap` skips token injection
   for SSH URLs.

### Checking the setup

Run `rdm bootstrap doctor` from inside the sandbox (or locally with the same
env) to verify the binary is on PATH, the plan-repo URL is set, the token is
present (if needed), and — for GitHub HTTPS URLs — that the token has access
to the repo via `GET /repos/:owner/:repo`. The doctor exits non-zero when a
critical check fails, so a CI job can use it as a readiness gate.

## Wiring options

Pick one of the following.

### Option A: Claude Code hook

1. Copy the template into your source repo:

   ```bash
   # From a clone of the rdm repo:
   scripts/install-claude-code-web-template.sh /path/to/your/source-repo
   ```

   This drops:
   - `.claude/hooks/SessionStart.sh`
   - `.claude/settings.claude-code-web.json.example`

2. Merge the contents of `.claude/settings.claude-code-web.json.example` into
   your source repo's `.claude/settings.json` (create it if it doesn't exist).
   The example registers a `SessionStart` hook that runs the installed script.
   It uses `${CLAUDE_PROJECT_DIR}`, which Claude Code injects into hook
   commands as the absolute path of your source repo — no additional setup
   required.

3. Set `RDM_PLAN_REPO` (and optionally `RDM_DEFAULT_PROJECT`) in Claude Code's
   sandbox environment variables for the repo.

### Option B: Devcontainer

If your repo already uses a `devcontainer.json`, merge the fields from
`templates/claude-code-web/devcontainer.json.fragment` into it. It runs the
install-then-bootstrap sequence during container creation and start.

## Troubleshooting

- **`[rdm hook] RDM_PLAN_REPO is not set; skipping plan-repo bootstrap.`** —
  expected when the env var isn't set. Set it in the sandbox config and
  re-open the session.
- **`failed to clone plan repo`** — usually an auth error on a private repo.
  Run `rdm bootstrap doctor` to isolate the cause (missing token, token
  rejected, repo not visible to token, etc.) and see [Credentials](#credentials)
  for the minimum-scope token setup.
- **`rdm: command not found` after the hook runs** — the hook installs to
  `$HOME/.local/bin` and prepends that to `PATH` in its own subshell. Claude
  Code tool invocations inherit `PATH` from the parent shell, so if the hook
  runs in a completely isolated subshell you may need to also set
  `PATH=$HOME/.local/bin:$PATH` in Claude Code env or your shell profile.
- **Plan repo is out of date** — rerun the hook (sessions rerun it on start).
  It performs a fast-forward pull on each invocation.
- **Hook output not visible** — Claude Code shows SessionStart stdout in the
  session log; if you're in a devcontainer context, check the
  `postStartCommand` output instead.
