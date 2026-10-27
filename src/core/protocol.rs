use super::protobuf::{ProtobufEncoder, connect_frame_decode, extract_strings};
use regex::Regex;
use serde_json::{Value, json};
use std::env;

mod repair;

use super::WS_APP;
use super::auth::{ws_app_version, ws_ide_version};
use repair::parse_tool_call_args;

#[derive(Debug, Clone)]
pub(crate) struct ChatMessage {
    pub(super) role: MessageRole,
    pub(super) content: String,
    pub(super) tool_call_id: Option<String>,
    pub(super) tool_name: Option<String>,
    pub(super) tool_args_json: Option<String>,
    pub(super) ref_call_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MessageRole {
    User,
    Assistant,
    Tool,
    System,
}

impl MessageRole {
    fn wire_value(self) -> u64 {
        match self {
            Self::User => 1,
            Self::Assistant => 2,
            Self::Tool => 4,
            Self::System => 5,
        }
    }
}

impl ChatMessage {
    pub(crate) fn simple(role: MessageRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_call_id: None,
            tool_name: None,
            tool_args_json: None,
            ref_call_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParsedToolCall {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) args: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ParsedModelTurn {
    ToolCalls {
        thinking: String,
        calls: Vec<ParsedToolCall>,
    },
    Text(String),
    Error(String),
}

fn build_metadata(
    api_key: &str,
    jwt: &str,
    app_version: Option<&str>,
    ide_version: Option<&str>,
) -> ProtobufEncoder {
    let mut meta = ProtobufEncoder::new();
    meta.write_string(1, WS_APP);
    meta.write_string(2, &ws_app_version(app_version));
    meta.write_string(3, api_key);
    meta.write_string(4, "zh-cn");

    let os_name = env::consts::OS;
    let arch = env::consts::ARCH;
    let sysname = match os_name {
        "macos" => "Darwin",
        "windows" => "Windows_NT",
        _ => "Linux",
    };
    meta.write_string(
        5,
        &json!({
            "Os": os_name,
            "Arch": arch,
            "Release": "1.0",
            "Version": "1.0",
            "Machine": arch,
            "Nodename": "localhost",
            "Sysname": sysname,
            "ProductVersion": "",
        })
        .to_string(),
    );
    meta.write_string(7, &ws_ide_version(ide_version));

    let cpu_count = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(4);
    meta.write_string(
        8,
        &json!({
            "NumSockets": 1,
            "NumCores": cpu_count,
            "NumThreads": cpu_count,
            "VendorID": "",
            "Family": "0",
            "Model": "0",
            "ModelName": "Unknown CPU",
            "Memory": 16_u64 * 1024 * 1024 * 1024,
        })
        .to_string(),
    );
    meta.write_string(12, WS_APP);
    meta.write_string(21, jwt);
    meta
}

fn build_chat_message(message: &ChatMessage) -> ProtobufEncoder {
    let mut msg = ProtobufEncoder::new();
    msg.write_varint(2, message.role.wire_value());
    msg.write_string(3, &message.content);

    if let (Some(tool_call_id), Some(tool_name), Some(tool_args_json)) = (
        message.tool_call_id.as_ref(),
        message.tool_name.as_ref(),
        message.tool_args_json.as_ref(),
    ) {
        let mut tc = ProtobufEncoder::new();
        tc.write_string(1, tool_call_id);
        tc.write_string(2, tool_name);
        tc.write_string(3, tool_args_json);
        msg.write_message(6, &tc);
    }

    if let Some(ref_call_id) = &message.ref_call_id {
        msg.write_string(7, ref_call_id);
    }

    msg
}

pub(crate) fn build_request(
    api_key: &str,
    jwt: &str,
    messages: &[ChatMessage],
    tool_defs: &str,
    app_version: Option<&str>,
    ide_version: Option<&str>,
) -> Vec<u8> {
    let mut req = ProtobufEncoder::new();
    req.write_message(1, &build_metadata(api_key, jwt, app_version, ide_version));

    for message in messages {
        let msg = build_chat_message(message);
        req.write_message(2, &msg);
    }

    req.write_string(3, tool_defs);
    req.to_bytes()
}

fn parse_tool_call(text: &str) -> Option<(String, String, Value)> {
    let text = text.replace("</s>", "");
    let regex = Regex::new(r"(?s)\[TOOL_CALLS\](\w+)\[ARGS\](\{.+)").ok()?;
    let captures = regex.captures(&text)?;
    let full_match = captures.get(0)?;
    let name = captures.get(1)?.as_str().to_string();
    let raw = captures.get(2)?.as_str().trim();

    let mut depth = 0_i32;
    let mut end = 0_usize;
    for (idx, ch) in raw.char_indices() {
        if ch == '{' {
            depth += 1;
        } else if ch == '}' {
            depth -= 1;
            if depth == 0 {
                end = idx + ch.len_utf8();
                break;
            }
        }
    }
    if end == 0 {
        end = raw.len();
    }

    let args = parse_tool_call_args(&name, &raw[..end])?;
    let thinking = text[..full_match.start()].trim().to_string();
    Some((thinking, name, args))
}

fn parse_structured_tool_call(value: &Value) -> Option<ParsedModelTurn> {
    let tool_calls = value.get("tool_calls")?.as_array()?;
    let calls = tool_calls
        .iter()
        .enumerate()
        .filter_map(|(idx, call)| {
            let name = call
                .get("name")
                .or_else(|| {
                    call.get("function")
                        .and_then(|function| function.get("name"))
                })?
                .as_str()?
                .to_string();
            let args = call
                .get("args")
                .or_else(|| call.get("arguments"))
                .or_else(|| {
                    call.get("function")
                        .and_then(|function| function.get("arguments"))
                })?;
            let args = if let Some(raw) = args.as_str() {
                serde_json::from_str(raw).ok()?
            } else {
                args.clone()
            };
            let id = call
                .get("id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("tool-call-{}", idx + 1));
            Some(ParsedToolCall { id, name, args })
        })
        .collect::<Vec<_>>();
    if calls.is_empty() {
        return None;
    }
    let thinking = value
        .get("thinking")
        .or_else(|| value.get("output"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Some(ParsedModelTurn::ToolCalls { thinking, calls })
}

pub(crate) fn parse_response(data: &[u8]) -> ParsedModelTurn {
    let frames = connect_frame_decode(data);
    let mut all_text = String::new();

    for frame_data in frames {
        let text_candidate = String::from_utf8_lossy(&frame_data).to_string();
        if text_candidate.starts_with('{')
            && let Ok(error) = serde_json::from_str::<Value>(&text_candidate)
            && let Some(error) = error.get("error")
        {
            let code = error
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let msg = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return ParsedModelTurn::Error(format!("[Error] {code}: {msg}"));
        }

        if text_candidate.starts_with('{')
            && let Ok(value) = serde_json::from_str::<Value>(&text_candidate)
            && let Some(turn) = parse_structured_tool_call(&value)
        {
            return turn;
        }

        let raw_text = text_candidate.replace('\u{fffd}', "");
        let extracted_strings = extract_strings(&frame_data);
        if raw_text.contains("[TOOL_CALLS]") {
            all_text = if extracted_strings
                .iter()
                .any(|value| value.contains("[TOOL_CALLS]"))
            {
                extracted_strings.join("")
            } else {
                raw_text
            };
            break;
        }

        for value in extracted_strings {
            if value.len() > 10 {
                all_text.push_str(&value);
            }
        }
    }

    if let Some((thinking, name, args)) = parse_tool_call(&all_text) {
        ParsedModelTurn::ToolCalls {
            thinking,
            calls: vec![ParsedToolCall {
                id: "tool-call-1".to_string(),
                name,
                args,
            }],
        }
    } else {
        ParsedModelTurn::Text(all_text)
    }
}

const SYSTEM_PROMPT_TEMPLATE: &str = r#"Search the codebase based on the user's question and provide a concise, accurate set of file paths and relevant line ranges containing all information needed to understand and correctly solve the issue.

# OBJECTIVE

* Return a minimal but precise set of relevant files that include the implementations, definitions, callers, configuration, tests, and entry points needed to plan or verify changes.
* Prioritize complete semantic blocks, such as functions, classes, implementations, or modules. If a block is too large, trim it as needed while preserving meaning.
* Provide enough code-related context to ensure the relevant mechanisms can be correctly understood.
* Exclude irrelevant code snippets to avoid polluting the context, but do not reduce the result set merely for the sake of reduction.
* Draw conclusions from the actual code. If a command fails or the returned evidence is insufficient, try different tool queries to obtain the answer.

# ENVIRONMENT

* Working directory: /codebase. Make sure to run commands in this directory, not `.`.
* Allowed sub-commands:

  * rg: Search for patterns in files.
  * readfile: Read the contents of a file, optionally within a line range.
  * tree: Display the directory structure as a tree.
  * ls: List files in a directory.
  * glob: Find files matching a glob pattern.

# TOOL USE GUIDELINES

* Use at most {max_commands} `restricted_exec` commands. Each command `type` must be one of `rg`, `readfile`, `tree`, `ls`, or `glob`.
* Each command object must put the command type in the top-level `type` field. Do not wrap commands as `{"rg": {...}}`, `{"readfile": "path"}`, or omit `type`.
* You have at most {max_turns} turns to interact with the environment by calling tools, so parallel commands are encouraged.
* `restricted_exec` arguments must have this shape:

[TOOL_CALLS]restricted_exec[ARGS]{
  "command1": {
    "type": "rg",
    "pattern": "Controller",
    "path": "/codebase/src",
    "include": ["**/*.py"]
  },
  "command2": {
    "type": "readfile",
    "file": "/codebase/src/controller.py",
    "start_line": 1,
    "end_line": 200
  },
  "command3": {
    "type": "tree",
    "path": "/codebase/src",
    "levels": 2
  },
  "command4": {
    "type": "ls",
    "path": "/codebase/src",
    "all": false
  },
  "command5": {
    "type": "glob",
    "pattern": "**/*.rs",
    "path": "/codebase/src",
    "type_filter": "file"
  }
}

# FAST-SEARCH DEFAULTS (optimize rg/tree on large repos)

* Start NARROW, then widen only if needed. Prefer searching likely code roots first (e.g., `src/`, `lib/`, `app/`, `packages/`, `services/`) instead of `/codebase`.
* Prefer fixed-string search for literals: escape patterns or keep regex simple. Use smart case; avoid case-insensitive unless necessary.
* Prefer file-type filters and globs (in include) over full-repo scans.
* Default EXCLUDES for speed (apply via the exclude array): node_modules, .git, dist, build, coverage, .venv, venv, target, out, .cache, __pycache__, vendor, deps, third_party, logs, data, *.min.*
* Skip huge files where possible; when opening files, prefer reading only relevant ranges with readfile.
* Limit directory traversal with tree levels to quickly orient before deeper inspection.

# ANSWER FORMAT

Strictly follow this format, including the tags.

* Use the answer tool with XML rooted at `ANSWER`.
* Each `file` element must have a `path` attribute.
* Each file must contain one or more inclusive `range` elements.
* Example output inside the answer tool argument:

<ANSWER><file path="/codebase/src/auth.rs"><range>10-80</range></file></ANSWER>

# NO RESULTS POLICY

If, after thorough searching, you are confident that no relevant files exist for the given query, such as when the requested function, class, or concept does not exist in the codebase, you MUST return an empty `ANSWER`:

<ANSWER></ANSWER>

Do NOT return irrelevant files, such as entry points or configuration files, merely to provide some output. An empty answer is always better than a misleading one.
"#;

pub(crate) const FINAL_FORCE_ANSWER: &str =
    "You have no turns left. Now you MUST provide your final ANSWER, even if it's not complete.";

pub fn get_tool_definitions(max_commands: usize) -> String {
    let max_commands = max_commands.clamp(1, 8);
    let command_schema = json!({
        "type": "object",
        "description": "Command to execute. Must be one of: rg, readfile, tree, ls, or glob.",
        "oneOf": [
            {
                "properties": {
                    "type": {"type": "string", "const": "rg", "description": "Search for patterns in files using ripgrep."},
                    "pattern": {"type": "string", "description": "The regex pattern to search for."},
                    "path": {"type": "string", "description": "The path to search in (file or directory)."},
                    "include": {"type": "array", "items": {"type": "string"}, "description": "File patterns to include in the search."},
                    "exclude": {"type": "array", "items": {"type": "string"}, "description": "File patterns to exclude from the search."}
                },
                "required": ["type", "pattern", "path"]
            },
            {
                "properties": {
                    "type": {"type": "string", "const": "readfile", "description": "Read contents of a file with optional line range."},
                    "file": {"type": "string", "description": "Path to the file to read."},
                    "start_line": {"type": "integer", "description": "Starting line number (1-indexed)."},
                    "end_line": {"type": "integer", "description": "Ending line number (1-indexed)."}
                },
                "required": ["type", "file"]
            },
            {
                "properties": {
                    "type": {"type": "string", "const": "tree", "description": "Display directory structure as a tree."},
                    "path": {"type": "string", "description": "Path to the directory to display."},
                    "levels": {"type": "integer", "description": "Number of directory levels to show."}
                },
                "required": ["type", "path"]
            },
            {
                "properties": {
                    "type": {"type": "string", "const": "ls", "description": "List files in a directory."},
                    "path": {"type": "string", "description": "Path to the directory to list."},
                    "long_format": {"type": "boolean", "description": "Use tab-separated type, regular-file byte size, readonly, and name fields."},
                    "all": {"type": "boolean", "description": "Show all files, including hidden files."}
                },
                "required": ["type", "path"]
            },
            {
                "properties": {
                    "type": {"type": "string", "const": "glob", "description": "Find files matching a glob pattern."},
                    "pattern": {"type": "string", "description": "Glob pattern to match."},
                    "path": {"type": "string", "description": "Path to search in."},
                    "type_filter": {"type": "string", "enum": ["file", "directory", "all"], "description": "Type of files to match."}
                },
                "required": ["type", "pattern", "path"]
            }
        ]
    });
    let mut properties = serde_json::Map::new();
    for idx in 1..=max_commands {
        properties.insert(format!("command{idx}"), command_schema.clone());
    }
    let required = vec!["command1"];

    json!([
        {
            "type": "function",
            "function": {
                "name": "restricted_exec",
                "description": "Execute restricted commands (rg, readfile, tree, ls, glob) in parallel.",
                "parameters": {
                    "type": "object",
                    "properties": properties,
                    "required": required
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "answer",
                "description": "Final answer with relevant files and line ranges.",
                "parameters": {
                    "type": "object",
                    "properties": {"answer": {"type": "string", "description": "The final answer in XML format."}},
                    "required": ["answer"]
                }
            }
        }
    ])
    .to_string()
}

pub fn build_system_prompt(max_turns: usize, max_commands: usize) -> String {
    let max_commands = max_commands.clamp(1, 8);
    SYSTEM_PROMPT_TEMPLATE
        .replace("{max_commands}", &max_commands.to_string())
        .replace("{max_turns}", &max_turns.to_string())
}

pub(crate) fn trim_messages(messages: &mut Vec<ChatMessage>) -> bool {
    if messages.len() <= 4 {
        return false;
    }
    let head = messages[..2].to_vec();
    let tail = messages[messages.len() - 2..].to_vec();
    messages.clear();
    messages.extend(head);
    messages.push(ChatMessage::simple(
        MessageRole::User,
        "[Prior search rounds omitted to reduce payload. Provide your best answer based on available context.]",
    ));
    messages.extend(tail);
    true
}

#[cfg(test)]
mod tests;
