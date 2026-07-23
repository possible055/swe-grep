# swe-grep

English | [简体中文](README_zh.md)

`swe-grep` is a Rust CLI for Windsurf Context subagent-compatible code search. It sends a natural-language query to `GetDevstralStream`, executes `restricted_exec` local search commands against your working tree, and returns the most relevant files and line ranges.

## Scope

- Implements a Windsurf Context subagent-compatible `restricted_exec` fast-context loop.
- Executes local equivalents of the official fast-context subcommands: `rg`, `readfile`, `tree`, `ls`, and `glob`.
- Returns compact file/range results for follow-up inspection.

## Requirements

- A logged-in Windsurf installation or a valid Windsurf API key

## Install on Linux

Install the latest static Linux release interactively, then choose cross-client Agent Skills, Cursor, Claude Code, or a custom skill directory:

```bash
curl -fsSL https://github.com/possible055/swe-grep/releases/latest/download/install.sh | sh
```

For a non-interactive cross-client user installation:

```bash
curl -fsSL https://github.com/possible055/swe-grep/releases/latest/download/install.sh \
  | sh -s -- --host agents --scope user --yes
```

The installer verifies the release archive SHA-256 checksum before extracting it. It installs the shared runtime at `${XDG_DATA_HOME:-$HOME/.local/share}/swe-grep/runtime`, always creates `~/.local/bin/swe-grep`, and persists that launcher through `~/.profile`. `--host agents` uses the cross-client `~/.agents/skills` convention, `--host cursor` uses `~/.cursor/skills`, and `--host claude` uses `~/.claude/skills`. The former `--host codex` name remains available as an alias for `agents`. Use `--scope project` to install below the current Git repository, or `--host custom --path /path/to/skills` for another skill parent directory. If another `swe-grep` command is already present in `PATH`, the installer stops without changing its precedence.

Remove a managed Linux installation by passing the same host, scope, or custom path used during installation:

```bash
curl -fsSL https://github.com/possible055/swe-grep/releases/latest/download/install.sh \
  | sh -s -- --uninstall --host agents --scope user --yes
```

Each command removes one managed Skill. The shared runtime, launcher, and installer-owned `.profile` block are removed after the last managed Skill is uninstalled. Modified `SKILL.md` files are never deleted automatically, non-installer files in Skill directories are preserved, and unrelated shell configuration is left unchanged. Timestamped backups created by `--force` are intentionally retained.

## Install on Windows

Run the interactive installer from Windows PowerShell 5.1 or later:

```powershell
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; irm https://github.com/possible055/swe-grep/releases/latest/download/install.ps1 | iex
```

For a non-interactive cross-client user installation:

```powershell
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; & ([scriptblock]::Create((irm https://github.com/possible055/swe-grep/releases/latest/download/install.ps1))) -Client agents -Scope user -Yes
```

The Windows installer supports `-Client agents|cursor|claude|custom`; `codex` remains an alias for `agents`. It verifies the release SHA-256 checksum, installs the shared runtime at `%LOCALAPPDATA%\swe-grep\runtime`, and always creates an isolated launcher at `%LOCALAPPDATA%\swe-grep\bin\swe-grep.cmd`. Only the launcher directory is added to the User `PATH`, so the bundled `rg.exe` is not exposed. Existing commands with the same name cause installation to stop without changing their precedence. Windows 10 or Windows Server 2016 and an x64 environment are required.

Remove a managed Windows installation by passing the same client, scope, or custom path used during installation:

```powershell
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; & ([scriptblock]::Create((irm https://github.com/possible055/swe-grep/releases/latest/download/install.ps1))) -Uninstall -Client agents -Scope user -Yes
```

Each command removes one managed Skill. The shared runtime and launcher are removed after the last managed Skill is uninstalled. The installer removes the launcher directory from User `PATH` only when it originally added that exact entry; every other entry and separator is preserved. Modified `SKILL.md` and non-installer files are left untouched. Timestamped backups created by `-Force` are intentionally retained.

## Build

```bash
cargo build --release
./target/release/swe-grep --help
```

## Development Checks

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

## Usage

### Search Code

```bash
swe-grep search "where is authentication handled" --path /path/to/project
```

Optional flags:

```bash
swe-grep search "query" \
  --path /path/to/project \
  --api-key sk-ws-01-... \
  --turns 4
```

- `--path` is required and should point to the project root.
- `--turns` accepts values from `4` to `6`.

### Extract a Windsurf API Key

```bash
swe-grep extract-key
swe-grep extract-key --save
swe-grep extract-key --show
swe-grep extract-key --db-path /path/to/state.vscdb
```

`--save` writes the extracted key to the shared `swe-tools` config:

- macOS / Linux: `~/.config/swe-tools/config.json`
- Windows: `~/swe-tool/config.json`

## Authentication

`search` resolves credentials in this order:

1. `--api-key`
2. `WINDSURF_API_KEY`
3. Saved config file
4. Auto-discovered Windsurf database

Auto-discovery looks for `state.vscdb` in these locations:

| Platform | Path |
| --- | --- |
| macOS | `~/Library/Application Support/Windsurf/User/globalStorage/state.vscdb` |
| Windows | `%APPDATA%/Windsurf/User/globalStorage/state.vscdb` |
| Linux | `$XDG_CONFIG_HOME/Windsurf/User/globalStorage/state.vscdb` or `~/.config/Windsurf/User/globalStorage/state.vscdb` |
| WSL | First matching `/mnt/c/Users/*/AppData/Roaming/Windsurf/User/globalStorage/state.vscdb`, then the Linux fallback above |

Recent Devin builds use the same `state.vscdb` format under `devin/User/globalStorage`.
Auto-discovery checks the matching `devin` paths after the Windsurf paths.

## Path Filtering

Local search tools share the same path filtering policy.

By default, visibility is determined in this order:

1. `exclude.txt`
2. `include.txt`
3. `.gitignore` files from the project root through nested subdirectories

Additional rules:

- Patterns use gitignore-style syntax.
- Empty lines and lines starting with `#` are ignored.
- Hidden dot-paths such as `.cache/` are excluded unless explicitly included.

Filter files are loaded only from the shared runtime directory:

| Platform | Location |
| --- | --- |
| Linux | `${XDG_DATA_HOME:-$HOME/.local/share}/swe-grep/runtime/{include.txt,exclude.txt}` |
| Windows | `%LOCALAPPDATA%\swe-grep\runtime\{include.txt,exclude.txt}` |

Disable shared path filtering with:

```bash
export SWEGREP_PATH_FILTER=0
```

## Environment Variables

At startup, the CLI optionally loads the shared runtime environment file:

- Linux: `${XDG_DATA_HOME:-$HOME/.local/share}/swe-grep/runtime/.env`
- Windows: `%LOCALAPPDATA%\swe-grep\runtime\.env`

Packaged `.env` values are commented by default.

Supported variables:

| Variable | Default | Description |
| --- | --- | --- |
| `WINDSURF_API_KEY` | none | Windsurf API key used by `search` |
| `SWEGREP_RG_PATH` | none | Custom `rg` binary path; overrides bundled `rg` and `PATH` |
| `WS_APP_VER` | `1.48.2` | Windsurf app version metadata |
| `WS_LS_VER` | `1.9544.35` | Windsurf language server version metadata |
| `SWEGREP_PATH_FILTER` | `1` | Enable shared path filtering; use `0`, `false`, `no`, or `off` to disable |
| `FC_MAX_COMMANDS` | `8` | Max parallel restricted commands per search round |
| `FC_RESULT_MAX_LINES` | `80` | Max lines for Windsurf-style non-`readfile` local tool results |
| `FC_READFILE_MAX_LINES` | `400` | Max lines for Windsurf-style `readfile` tool output |
| `FC_LINE_MAX_CHARS` | `300` | Max characters kept per line for Windsurf-style tool output |
| `FC_MAX_TURNS` | `4` | Default maximum search rounds for `search` |
| `FC_TIMEOUT_MS` | `30000` | Streaming timeout in milliseconds |
| `SWEGREP_DEBUG` | `0` | Enable model-response diagnostics when set to `1`, `true`, `yes`, or `on` |
| `SWEGREP_DEBUG_LOG` | `~/.config/swe-tools/debug.log` | Debug log path for parsed model responses and final answer XML |

## Acknowledgements

This project was inspired by and built upon ideas from the following projects:

- [fast-context-mcp](https://github.com/SammySnake-d/fast-context-mcp)
- [fast-context-skill](https://github.com/oulkurt/fast-context-skill)
