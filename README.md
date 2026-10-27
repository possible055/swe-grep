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
curl --proto '=https' --tlsv1.2 -fsSL --retry 3 --connect-timeout 10 \
  https://github.com/possible055/swe-grep/releases/latest/download/install.sh -o /tmp/install.sh \
  && sh /tmp/install.sh \
  && rm -f /tmp/install.sh
```

Non-interactive installation:

```bash
curl --proto '=https' --tlsv1.2 -fsSL --retry 3 --connect-timeout 10 \
  https://github.com/possible055/swe-grep/releases/latest/download/install.sh -o /tmp/install.sh \
  && sh /tmp/install.sh --host agents --scope user --yes \
  && rm -f /tmp/install.sh
```

To upgrade an existing installation while preserving your configuration (`.env`, `include.txt`, `exclude.txt`), pass `--force`:

```bash
curl --proto '=https' --tlsv1.2 -fsSL --retry 3 --connect-timeout 10 \
  https://github.com/possible055/swe-grep/releases/latest/download/install.sh -o /tmp/install.sh \
  && sh /tmp/install.sh --force \
  && rm -f /tmp/install.sh
```

Uninstall (pass the same `--host` and `--scope` used during installation):

```bash
curl --proto '=https' --tlsv1.2 -fsSL --retry 3 --connect-timeout 10 \
  https://github.com/possible055/swe-grep/releases/latest/download/install.sh -o /tmp/install.sh \
  && sh /tmp/install.sh --uninstall --host agents --scope user --yes \
  && rm -f /tmp/install.sh
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
$ProgressPreference = 'SilentlyContinue'; $installer = Join-Path $env:TEMP "install.ps1"; irm https://github.com/possible055/swe-grep/releases/latest/download/install.ps1 -OutFile $installer -ErrorAction Stop; powershell.exe -NoProfile -ExecutionPolicy Bypass -File $installer; Remove-Item -Force $installer
```

Non-interactive installation:

```powershell
$ProgressPreference = 'SilentlyContinue'; $installer = Join-Path $env:TEMP "install.ps1"; irm https://github.com/possible055/swe-grep/releases/latest/download/install.ps1 -OutFile $installer -ErrorAction Stop; powershell.exe -NoProfile -ExecutionPolicy Bypass -File $installer -Client agents -Scope user -Yes; Remove-Item -Force $installer
```

To upgrade an existing installation while preserving your configuration (`.env`, `include.txt`, `exclude.txt`), pass `-Force`:

```powershell
$ProgressPreference = 'SilentlyContinue'; $installer = Join-Path $env:TEMP "install.ps1"; irm https://github.com/possible055/swe-grep/releases/latest/download/install.ps1 -OutFile $installer -ErrorAction Stop; powershell.exe -NoProfile -ExecutionPolicy Bypass -File $installer -Force; Remove-Item -Force $installer
```

Uninstall:

```powershell
$ProgressPreference = 'SilentlyContinue'; $installer = Join-Path $env:TEMP "install.ps1"; irm https://github.com/possible055/swe-grep/releases/latest/download/install.ps1 -OutFile $installer -ErrorAction Stop; powershell.exe -NoProfile -ExecutionPolicy Bypass -File $installer -Uninstall -Client agents -Scope user -Yes; Remove-Item -Force $installer
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

### Self Update

Upgrade `swe-grep` in-place to the latest release while preserving custom configurations (`.env`, `include.txt`, `exclude.txt`) and automatically syncing managed Agent Skills:

```bash
swe-grep self-update
```

Optional flags:

```bash
swe-grep self-update --check                  # Check if a new version is available
swe-grep self-update --force                  # Reinstall even if already at latest version
swe-grep self-update --yes                    # Non-interactive mode (auto-confirm)
swe-grep self-update --tag v0.1.9 --force     # Reinstall or downgrade to a specific tag
swe-grep self-update --tag v0.1.11 --release-base-url URL  # Download from a mirror
```

`swe-grep update` is also available as an alias.

Custom mirrors require `--tag` to identify the target version. The base URL must point directly to that release's asset directory; the tag is not appended to the URL.

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

When disabled, swe-grep does not apply `exclude.txt`, `include.txt`, `.gitignore`, or the default hidden-path filter. Explicit filters supplied by a tool call still apply, and `ls` continues to hide dot-paths unless its `all` option is enabled.

Local tool directory output is deterministic: directories are listed before other entries, then names are sorted case-insensitively with the original name as the tie-breaker. `ls` long-format rows are tab-separated `type`, regular-file byte size (`-` otherwise), `readonly`, and `name` fields.

### Environment Variables

The CLI loads `.env` from the shared runtime directory at startup. All packaged values are commented out by default.

| Variable | Default | Description |
| --- | --- | --- |
| `WINDSURF_API_KEY` | — | Windsurf API key |
| `SWEGREP_RG_PATH` | — | Custom `rg` binary path |
| `WS_APP_VER` | `1.48.2` | Metadata `extension_version`; matches the official `codeiumVersion` |
| `WS_IDE_VER` | `1.126.0` | Metadata `ide_version` (IDE/app version) |
| `WS_LS_VER` | `1.9544.35` | Language-server version in the streaming `Baggage` header |
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
cargo bench --bench local_tools
```

The benchmark reports warm-cache p50/p95 latency and output size. Measure peak RSS externally when needed, for example with `/usr/bin/time -v cargo bench --bench local_tools` on Linux; benchmark results are not a CI gate.

## Acknowledgements

- [fast-context-mcp](https://github.com/SammySnake-d/fast-context-mcp)
- [fast-context-skill](https://github.com/oulkurt/fast-context-skill)
