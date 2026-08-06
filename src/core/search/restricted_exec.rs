use crate::executor::{
    InstantContextToolCall as ExecutorToolCall, InstantContextToolUpdate, ToolExecutionStatus,
};
use serde_json::{Map, Value};

pub(super) fn restricted_exec_commands(args: &Value, max_commands: usize) -> Vec<ExecutorToolCall> {
    let Some(map) = args.as_object() else {
        return missing_restricted_exec_command();
    };

    let commands = (1..=max_commands.clamp(1, 8))
        .filter_map(|idx| {
            let key = format!("command{idx}");
            map.get(&key).map(|command| ExecutorToolCall {
                id: key.clone(),
                name: key,
                args: normalize_restricted_exec_command(command),
            })
        })
        .collect::<Vec<_>>();
    if commands.is_empty() {
        missing_restricted_exec_command()
    } else {
        commands
    }
}

fn missing_restricted_exec_command() -> Vec<ExecutorToolCall> {
    vec![ExecutorToolCall {
        id: "command1".to_string(),
        name: "command1".to_string(),
        args: serde_json::json!({"type": "", "error": "missing restricted_exec command object"}),
    }]
}

pub(super) fn has_valid_restricted_exec_command(args: &Value, max_commands: usize) -> bool {
    let Some(map) = args.as_object() else {
        return false;
    };

    (1..=max_commands.clamp(1, 8)).any(|idx| {
        let key = format!("command{idx}");
        map.get(&key)
            .map(normalize_restricted_exec_command)
            .and_then(|command| {
                command
                    .get("type")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .is_some_and(|command_type| !command_type.trim().is_empty())
    })
}

fn normalize_restricted_exec_command(command: &Value) -> Value {
    let Some(map) = command.as_object() else {
        return command.clone();
    };
    if map
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|command_type| !command_type.trim().is_empty())
    {
        return command.clone();
    }

    for command_type in ["rg", "readfile", "tree", "ls", "glob"] {
        if let Some(normalized) = normalize_shorthand_command(command_type, map) {
            return normalized;
        }
    }

    if let Some(command_type) = infer_restricted_exec_command_type(map) {
        let mut normalized = map.clone();
        normalized.insert("type".to_string(), Value::String(command_type.to_string()));
        return Value::Object(normalized);
    }

    command.clone()
}

fn infer_restricted_exec_command_type(map: &Map<String, Value>) -> Option<&'static str> {
    if map.get("file").and_then(Value::as_str).is_some() {
        return Some("readfile");
    }
    if map.get("pattern").and_then(Value::as_str).is_some()
        && map.get("path").and_then(Value::as_str).is_some()
    {
        if map.get("type_filter").and_then(Value::as_str).is_some() {
            return Some("glob");
        }
        return Some("rg");
    }
    if map.get("path").and_then(Value::as_str).is_some()
        && map.get("levels").and_then(Value::as_i64).is_some()
    {
        return Some("tree");
    }
    if map.get("path").and_then(Value::as_str).is_some()
        && (map.get("long_format").and_then(Value::as_bool).is_some()
            || map.get("all").and_then(Value::as_bool).is_some())
    {
        return Some("ls");
    }
    None
}

fn normalize_shorthand_command(command_type: &str, map: &Map<String, Value>) -> Option<Value> {
    let shorthand = map.get(command_type)?;
    match shorthand {
        Value::Object(inner) => {
            let mut normalized = inner.clone();
            normalized
                .entry("type".to_string())
                .or_insert_with(|| Value::String(command_type.to_string()));
            Some(Value::Object(normalized))
        }
        Value::String(value) => {
            let mut normalized = Map::new();
            normalized.insert("type".to_string(), Value::String(command_type.to_string()));
            let target_field = match command_type {
                "rg" | "glob" => "pattern",
                "readfile" => "file",
                "tree" | "ls" => "path",
                _ => return None,
            };
            normalized.insert(target_field.to_string(), Value::String(value.clone()));
            for (key, value) in map {
                if key != command_type {
                    normalized.insert(key.clone(), value.clone());
                }
            }
            Some(Value::Object(normalized))
        }
        _ => None,
    }
}

pub(super) fn format_restricted_exec_results(updates: &[InstantContextToolUpdate]) -> String {
    let mut out = String::new();
    for update in updates {
        if update.status == ToolExecutionStatus::Pending {
            continue;
        }
        out.push_str(&format!(
            "{}_result:\n{}\n",
            update.tool_call_id, update.output
        ));
    }
    out
}
