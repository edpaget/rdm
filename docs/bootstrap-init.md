# Bootstrap and Init

## Overview

`rdm init` sets up everything you need in one command: repo directory, configuration files, and optional git remote. No environment variables or manual file editing required.

Configuration follows XDG conventions, separating user-wide preferences (global config) from plan repo data. Settings can be overridden per-repo, per-session, or per-command.

## Usage

### First-time setup

Create a plan repo at the default XDG data location (`~/.local/share/rdm` on Linux, `~/Library/Application Support/rdm` on macOS):

```bash
rdm init
```

With a default project and output format:

```bash
rdm init --default-project myproject --default-format json
```

This creates:
- The plan repo directory (with `rdm.toml`)
- Global config at `~/.config/rdm/config.toml`
- The `myproject` project directory

With staging mode (mutations don't auto-commit to git):

```bash
rdm init --stage --default-project myproject
```

### Custom repo location

```bash
rdm init --root ~/Documents/my-plans --default-project work
```

### Cloning a shared plan repo

Bootstrap from an existing remote repo:

```bash
rdm init --remote git@github.com:team/plans.git
```

The remote is validated and configured for future push/pull. Combine with other flags as needed:

```bash
rdm init --remote git@github.com:team/plans.git --default-format table
```

### Repo resolution

`RDM_ROOT` is not required after init. The repo location resolves through:

1. `--root` CLI flag
2. `RDM_ROOT` environment variable
3. `root` in global config
4. XDG data default (`$XDG_DATA_HOME/rdm`)

### Managing configuration

```bash
rdm config list                                  # all settings with sources
rdm config get default_format                    # single setting
rdm config set default_project myproject         # set in repo config
rdm config set --global default_format json      # set in global config
```

#### Configuration keys

| Key | Scope | Description |
|-----|-------|-------------|
| `root` | global only | Path to the default plan repo |
| `default_project` | repo or global | Default project for commands |
| `default_format` | repo or global | Output format: `human`, `json`, `table`, `markdown` |
| `stage` | repo or global | Whether mutations defer git commits |
| `remote.default` | repo | Default remote name for push/pull |

#### Configuration hierarchy

Highest precedence first:

1. **CLI flags** (`--root`, `--format`, `--project`, `--stage`)
2. **Environment variables** (`RDM_ROOT`, `RDM_FORMAT`, `RDM_PROJECT`, `RDM_STAGE`)
3. **Repo config** (`<repo>/rdm.toml`)
4. **Global config** (`$XDG_CONFIG_HOME/rdm/config.toml`)
5. **Built-in defaults**

> **2026-09-11 — MCP server initialization was removed.** rdm previously
> shipped an `rdm_init` MCP tool and an `auto_init` global config key so a
> client installing rdm from an MCP marketplace could initialize a plan repo
> without a terminal. The `retire-mcp` roadmap deleted the `rdm-mcp` crate
> (and `auto_init` with it, since that tool was its only consumer); a global
> config file left over from before this change that still sets `auto_init`
> continues to parse without error but the key is ignored. Initialize via
> `rdm init` from a terminal instead — see "First-time setup" above.

## Limitations

- No interactive wizard — all options are passed as flags.
- Custom templates and hook scripts are not supported during init.
- `auto_init` creates a repo with defaults only — no custom project or format.
