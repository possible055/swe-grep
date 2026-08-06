# swe-grep

English | [简体中文](README_zh.md)

AI-powered semantic code search CLI. Ask questions in natural language and get the most relevant files and line ranges from your codebase.

## Features

- **Natural language search** — describe what you're looking for, not exact keywords
- **Agentic traversal** — the tool runs multiple search rounds, refining results automatically
- **Local execution** — all searches run locally via `rg`, `readfile`, `tree`, `ls`, and `glob`
- **Agent Skill integration** — installs as a skill for Cursor, Claude Code, and cross-client Agent Skills

## Requirements

- A logged-in Windsurf installation **or** a valid Windsurf API key

## Installation

### Linux

```bash
curl -fsSL https://github.com/possible055/swe-grep/releases/latest/download/install.sh | sh
```

Non-interactive installation:

```bash
curl -fsSL https://github.com/possible055/swe-grep/releases/latest/download/install.sh \
  | sh -s -- --host agents --scope user --yes
```

Uninstall (pass the same `--host` and `--scope` used during installation):

```bash
curl -fsSL https://github.com/possible055/swe-grep/releases/latest/download/install.sh \
  | sh -s -- --uninstall --host agents --scope user --yes
```

**Installer options:**

| Option | Values | Description |
| --- | --- | --- |
| `--host` | `agents`, `cursor`, `claude`, `custom` | Skill host (`codex` is an alias for `agents`) |
| `--scope` | `user` (default), `project` | Install scope |
| `--path` | directory | Custom skill parent directory (with `--host custom`) |
| `--force` | — | Back up existing installation before replacing |
| `--yes` | — | Skip confirmation prompts |

### Windows

```powershell
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; irm https://github.com/possible055/swe-grep/releases/latest/download/install.ps1 | iex
```

Non-interactive installation:

```powershell
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; & ([scriptblock]::Create((irm https://github.com/possible055/swe-grep/releases/latest/download/install.ps1))) -Client agents -Scope user -Yes
```

Uninstall:

```powershell
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; & ([scriptblock]::Create((irm https://github.com/possible055/swe-grep/releases/latest/download/install.ps1))) -Uninstall -Client agents -Scope user -Yes
```

Windows installer options mirror Linux: `-Client agents|cursor|claude|custom`, `-Scope user|project`, `-Force`, `-Yes`. Requires Windows 10+ / Windows Server 2016+ on x64.

## Usage

### Search

```bash
swe-grep search "where is authentication handled" --path /path/to/project
```

| Flag | Required | Description |
| --- | --- | --- |
| `query` | yes | Natural language search query |
| `--path` | yes | Project root directory |
| `--api-key` | no | Windsurf API key (overrides env and config) |
| `--turns` | no | Max search rounds, `4`–`6` (default: `4`) |

Increase `--turns` for broad or ambiguous queries that require deeper traversal.

### Extract API Key

```bash
swe-grep extract-key              # auto-detect and print masked key
swe-grep extract-key --save       # save to shared config for future searches
swe-grep extract-key --show       # print full key
swe-grep extract-key --db-path /path/to/state.vscdb  # read a specific database
```

`--save` writes to `~/.config/swe-tools/config.json` (macOS/Linux) or `~/swe-tool/config.json` (Windows).

## Authentication

Credential resolution order:

1. `--api-key` flag
2. `WINDSURF_API_KEY` environment variable
3. Saved config file (via `extract-key --save`)
4. Auto-discovered Windsurf database

Auto-discovery searches for `state.vscdb` in:

| Platform | Path |
| --- | --- |
| macOS | `~/Library/Application Support/Windsurf/User/globalStorage/state.vscdb` |
| Windows | `%APPDATA%/Windsurf/User/globalStorage/state.vscdb` |
| Linux | `$XDG_CONFIG_HOME/Windsurf/User/globalStorage/state.vscdb` or `~/.config/Windsurf/User/globalStorage/state.vscdb` |
| WSL | `/mnt/c/Users/*/AppData/Roaming/Windsurf/User/globalStorage/state.vscdb`, then Linux fallback |

Devin builds using the same format under `devin/User/globalStorage` are also checked.

## Configuration

### Path Filtering

Search visibility is determined by:

1. `exclude.txt` — patterns to exclude
2. `include.txt` — patterns to include
3. `.gitignore` — from project root through nested directories

Patterns use gitignore syntax. Hidden dot-paths (e.g. `.cache/`) are excluded unless explicitly included.

Filter files are loaded from the shared runtime directory:

| Platform | Location |
| --- | --- |
| Linux | `${XDG_DATA_HOME:-$HOME/.local/share}/swe-grep/runtime/` |
| Windows | `%LOCALAPPDATA%\swe-grep\runtime\` |

Disable path filtering: `export SWEGREP_PATH_FILTER=0`

### Environment Variables

The CLI loads `.env` from the shared runtime directory at startup. All packaged values are commented out by default.

| Variable | Default | Description |
| --- | --- | --- |
| `WINDSURF_API_KEY` | — | Windsurf API key |
| `SWEGREP_RG_PATH` | — | Custom `rg` binary path |
| `WS_APP_VER` | `1.48.2` | Windsurf app version metadata |
| `WS_LS_VER` | `1.9544.35` | Windsurf language server version metadata |
| `SWEGREP_PATH_FILTER` | `1` | Enable path filtering (`1/0`, `true/false`, `yes/no`, `on/off`) |
| `FC_MAX_COMMANDS` | `8` | Max parallel commands per search round (`1..8`) |
| `FC_RESULT_MAX_LINES` | `80` | Max lines for non-`readfile` tool results (`1..500`) |
| `FC_READFILE_MAX_LINES` | `400` | Max lines for `readfile` output (`1..10000`) |
| `FC_LINE_MAX_CHARS` | `300` | Max characters per line in tool output (`20..10000`) |
| `FC_MAX_TURNS` | `4` | Default max search rounds (`4..6`) |
| `FC_TIMEOUT_MS` | `30000` | Streaming timeout in ms (`1000..300000`) |
| `SWEGREP_DEBUG` | `0` | Enable diagnostics logging |
| `SWEGREP_DEBUG_LOG` | `~/.config/swe-tools/debug.log` | Debug log file path |
| `SCOPE_SNAPSHOT_TREE_DEPTH` | `4` | RepoMap snapshot depth (`0..8`) |

Invalid values fall back to defaults with a single warning.

## Build from Source

```bash
cargo build --release
./target/release/swe-grep --help
```

Development checks:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

## Acknowledgements

- [fast-context-mcp](https://github.com/SammySnake-d/fast-context-mcp)
- [fast-context-skill](https://github.com/oulkurt/fast-context-skill)
