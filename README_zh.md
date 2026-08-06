# swe-grep

[English](README.md) | 简体中文

AI 驱动的语义代码搜索 CLI。用自然语言提问，从代码库中获取最相关的文件和行范围。

## 特性

- **自然语言搜索** — 描述你要找的内容，无需精确关键词
- **多轮代理遍历** — 工具自动执行多轮搜索，逐步细化结果
- **本地执行** — 所有搜索通过 `rg`、`readfile`、`tree`、`ls` 和 `glob` 在本地运行
- **Agent Skill 集成** — 可作为 Cursor、Claude Code 和跨客户端 Agent Skills 的 Skill 安装

## 环境要求

- 已登录的 Windsurf 客户端 **或** 有效的 Windsurf API 密钥

## 安装

### Linux

```bash
curl -fsSL https://github.com/possible055/swe-grep/releases/latest/download/install.sh | sh
```

非交互式安装：

```bash
curl -fsSL https://github.com/possible055/swe-grep/releases/latest/download/install.sh \
  | sh -s -- --host agents --scope user --yes
```

卸载（传入安装时使用的相同 `--host` 和 `--scope`）：

```bash
curl -fsSL https://github.com/possible055/swe-grep/releases/latest/download/install.sh \
  | sh -s -- --uninstall --host agents --scope user --yes
```

**安装器选项：**

| 选项 | 可选值 | 说明 |
| --- | --- | --- |
| `--host` | `agents`、`cursor`、`claude`、`custom` | Skill 宿主（`codex` 为 `agents` 的别名） |
| `--scope` | `user`（默认）、`project` | 安装范围 |
| `--path` | 目录 | 自定义 Skill 父目录（配合 `--host custom`） |
| `--force` | — | 替换前备份已有安装 |
| `--yes` | — | 跳过确认提示 |

### Windows

```powershell
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; irm https://github.com/possible055/swe-grep/releases/latest/download/install.ps1 | iex
```

非交互式安装：

```powershell
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; & ([scriptblock]::Create((irm https://github.com/possible055/swe-grep/releases/latest/download/install.ps1))) -Client agents -Scope user -Yes
```

卸载：

```powershell
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; & ([scriptblock]::Create((irm https://github.com/possible055/swe-grep/releases/latest/download/install.ps1))) -Uninstall -Client agents -Scope user -Yes
```

Windows 安装器选项与 Linux 对应：`-Client agents|cursor|claude|custom`、`-Scope user|project`、`-Force`、`-Yes`。要求 Windows 10+ / Windows Server 2016+，x64 环境。

## 使用方法

### 搜索

```bash
swe-grep search "where is authentication handled" --path /path/to/project
```

| 参数 | 必填 | 说明 |
| --- | --- | --- |
| `query` | 是 | 自然语言搜索查询 |
| `--path` | 是 | 项目根目录 |
| `--api-key` | 否 | Windsurf API 密钥（覆盖环境变量和配置文件） |
| `--turns` | 否 | 最大搜索轮数，`4`–`6`（默认：`4`） |

对于广泛或模糊的查询，可增大 `--turns` 以进行更深入的遍历。

### 提取 API 密钥

```bash
swe-grep extract-key              # 自动检测并打印掩码密钥
swe-grep extract-key --save       # 保存到共享配置，供后续搜索使用
swe-grep extract-key --show       # 打印完整密钥
swe-grep extract-key --db-path /path/to/state.vscdb  # 读取指定数据库
```

`--save` 写入 `~/.config/swe-tools/config.json`（macOS/Linux）或 `~/swe-tool/config.json`（Windows）。

## 身份验证

凭据解析顺序：

1. `--api-key` 参数
2. `WINDSURF_API_KEY` 环境变量
3. 已保存的配置文件（通过 `extract-key --save`）
4. 自动发现的 Windsurf 数据库

自动发现会在以下位置搜索 `state.vscdb`：

| 平台 | 路径 |
| --- | --- |
| macOS | `~/Library/Application Support/Windsurf/User/globalStorage/state.vscdb` |
| Windows | `%APPDATA%/Windsurf/User/globalStorage/state.vscdb` |
| Linux | `$XDG_CONFIG_HOME/Windsurf/User/globalStorage/state.vscdb` 或 `~/.config/Windsurf/User/globalStorage/state.vscdb` |
| WSL | `/mnt/c/Users/*/AppData/Roaming/Windsurf/User/globalStorage/state.vscdb`，然后使用 Linux 回退路径 |

Devin 构建版本在 `devin/User/globalStorage` 下使用相同格式，也会被检查。

## 配置

### 路径过滤

搜索可见性由以下顺序决定：

1. `exclude.txt` — 排除模式
2. `include.txt` — 包含模式
3. `.gitignore` — 从项目根目录到嵌套子目录

模式使用 gitignore 语法。隐藏点路径（如 `.cache/`）默认排除，除非显式包含。

过滤文件从共享 runtime 目录加载：

| 平台 | 位置 |
| --- | --- |
| Linux | `${XDG_DATA_HOME:-$HOME/.local/share}/swe-grep/runtime/` |
| Windows | `%LOCALAPPDATA%\swe-grep\runtime\` |

禁用路径过滤：`export SWEGREP_PATH_FILTER=0`

### 环境变量

CLI 启动时会从共享 runtime 目录加载 `.env` 文件。打包的值默认全部注释。

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `WINDSURF_API_KEY` | — | Windsurf API 密钥 |
| `SWEGREP_RG_PATH` | — | 自定义 `rg` 二进制文件路径 |
| `WS_APP_VER` | `1.48.2` | Windsurf 应用版本元数据 |
| `WS_LS_VER` | `1.9544.35` | Windsurf 语言服务器版本元数据 |
| `SWEGREP_PATH_FILTER` | `1` | 启用路径过滤（`1/0`、`true/false`、`yes/no`、`on/off`） |
| `FC_MAX_COMMANDS` | `8` | 每轮搜索的最大并行命令数（`1..8`） |
| `FC_RESULT_MAX_LINES` | `80` | 非 `readfile` 工具结果的最大行数（`1..500`） |
| `FC_READFILE_MAX_LINES` | `400` | `readfile` 输出的最大行数（`1..10000`） |
| `FC_LINE_MAX_CHARS` | `300` | 工具输出中每行保留的最大字符数（`20..10000`） |
| `FC_MAX_TURNS` | `4` | 默认最大搜索轮数（`4..6`） |
| `FC_TIMEOUT_MS` | `30000` | 流式传输超时，单位毫秒（`1000..300000`） |
| `SWEGREP_DEBUG` | `0` | 启用诊断日志 |
| `SWEGREP_DEBUG_LOG` | `~/.config/swe-tools/debug.log` | Debug 日志文件路径 |
| `SCOPE_SNAPSHOT_TREE_DEPTH` | `4` | RepoMap 快照深度（`0..8`） |

无效值会回退到默认值并输出一条警告。

## 从源码构建

```bash
cargo build --release
./target/release/swe-grep --help
```

开发检查：

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

## 致谢

- [fast-context-mcp](https://github.com/SammySnake-d/fast-context-mcp)
- [fast-context-skill](https://github.com/oulkurt/fast-context-skill)
