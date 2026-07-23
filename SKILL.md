---
name: swe-grep
description: Use swe-grep for semantic codebase search in local repositories. Best for broad natural-language questions about implementation paths, feature flow, and relevant files. Use when the user asks where a behavior, API, feature, command, or configuration is implemented, which files are most relevant before making a change, how an authentication, request, parsing, execution, or persistence flow is wired, or where to continue investigation when exact identifiers are unknown; for exact keyword lookup, use ripgrep instead.
metadata:
  short-description: Semantic codebase search with swe-grep
---

# SWE Grep

## Codebase Search

Run codebase search from the target repository root:
```sh
swe-grep search "<natural language query>" --path <repo-path> --turns 4
```

**Parameters:**
* `<natural language query>`: A broad semantic question about the codebase.
* `--path`: Target repository root to search. Use an absolute path when possible.
* `--turns <4-6>`: Maximum search rounds. Use `4` by default for normal questions.

Increase to `5` or `6` when the task is broad, ambiguous, or likely to require deeper codebase traversal.

## Extract Key

When credentials are missing, try extracting and saving them:

```sh
swe-grep extract-key --save
```

**Parameters:**
* `--save`: Save the extracted key to the shared `swe-tools` config.
* `--show`: Print the full key instead of a masked key.
* `--db-path <path>`: Read a specific Windsurf `state.vscdb` file instead of auto-detecting it.
