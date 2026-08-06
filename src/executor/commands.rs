mod listing;
mod readfile;
mod rg;

use super::{
    InstantContextTiming, InstantContextToolCall, InstantContextToolUpdate, ToolExecutionStatus,
    ToolExecutor,
};
use serde_json::Value;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

impl ToolExecutor {
    pub fn exec_command(&self, cmd: &Value) -> String {
        let Some(cmd) = cmd.as_object() else {
            return "Error: missing or invalid command".to_string();
        };
        let command_type = cmd.get("type").and_then(Value::as_str).unwrap_or_default();
        match command_type {
            "rg" => self.execute_rg(
                cmd.get("pattern")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                cmd.get("path").and_then(Value::as_str).unwrap_or_default(),
                string_array(cmd.get("include")).as_deref(),
                string_array(cmd.get("exclude")).as_deref(),
            ),
            "readfile" => self.execute_readfile(
                cmd.get("file").and_then(Value::as_str).unwrap_or_default(),
                cmd.get("start_line")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize),
                cmd.get("end_line")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize),
            ),
            "tree" => self.execute_tree(
                cmd.get("path").and_then(Value::as_str).unwrap_or_default(),
                cmd.get("levels")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize),
            ),
            "ls" => self.execute_ls(
                cmd.get("path").and_then(Value::as_str).unwrap_or_default(),
                cmd.get("long_format")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                cmd.get("all").and_then(Value::as_bool).unwrap_or(false),
            ),
            "glob" => self.execute_glob(
                cmd.get("pattern")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                cmd.get("path").and_then(Value::as_str).unwrap_or_default(),
                cmd.get("type_filter")
                    .and_then(Value::as_str)
                    .unwrap_or("file"),
            ),
            _ => format!("Error: unknown command type '{command_type}'"),
        }
    }

    pub fn exec_restricted_exec_step(
        &self,
        step_id: &str,
        tool_calls: &[InstantContextToolCall],
        timeout_budget_ms: u128,
    ) -> Vec<InstantContextToolUpdate> {
        if tool_calls.is_empty() {
            return Vec::new();
        }

        let pending = tool_calls
            .iter()
            .map(|call| InstantContextToolUpdate {
                step_id: step_id.to_string(),
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                command: call.args.clone(),
                status: ToolExecutionStatus::Pending,
                output: String::new(),
                timing: InstantContextTiming::default(),
            })
            .collect::<Vec<_>>();

        if timeout_budget_ms == 0 {
            let completed = tool_calls
                .iter()
                .map(|call| InstantContextToolUpdate {
                    step_id: step_id.to_string(),
                    tool_call_id: call.id.clone(),
                    tool_name: call.name.clone(),
                    command: Value::Null,
                    status: ToolExecutionStatus::TimedOut,
                    output: "Error: tool timed out".to_string(),
                    timing: InstantContextTiming { duration_ms: 0 },
                })
                .collect::<Vec<_>>();

            return pending.into_iter().chain(completed).collect();
        }

        let timeout = Duration::from_millis(timeout_budget_ms.min(u64::MAX as u128) as u64);
        let receivers = tool_calls
            .iter()
            .map(|call| {
                let executor = self.clone();
                let call_name = call.name.clone();
                let mut cmd = call.args.clone();
                if let Some(map) = cmd.as_object_mut() {
                    map.entry("type".to_string())
                        .or_insert_with(|| Value::String(call_name));
                }
                let (tx, rx) = mpsc::channel();
                thread::spawn(move || {
                    let started = Instant::now();
                    let output = executor.exec_command(&cmd);
                    let duration_ms = started.elapsed().as_millis();
                    let status = if output.trim_start().starts_with("Error:") {
                        ToolExecutionStatus::Error
                    } else {
                        ToolExecutionStatus::Completed
                    };
                    let _ = tx.send((cmd, status, output, duration_ms));
                });
                rx
            })
            .collect::<Vec<_>>();

        let outputs = receivers
            .into_iter()
            .map(|rx| match rx.recv_timeout(timeout) {
                Ok(result) => result,
                Err(mpsc::RecvTimeoutError::Timeout) => (
                    Value::Null,
                    ToolExecutionStatus::TimedOut,
                    "Error: tool timed out".to_string(),
                    timeout_budget_ms,
                ),
                Err(mpsc::RecvTimeoutError::Disconnected) => (
                    Value::Null,
                    ToolExecutionStatus::Error,
                    "Error: command panicked".to_string(),
                    0,
                ),
            })
            .collect::<Vec<_>>();

        let completed = tool_calls
            .iter()
            .zip(outputs)
            .map(
                |(call, (command, status, output, duration_ms))| InstantContextToolUpdate {
                    step_id: step_id.to_string(),
                    tool_call_id: call.id.clone(),
                    tool_name: call.name.clone(),
                    command,
                    status,
                    output,
                    timing: InstantContextTiming { duration_ms },
                },
            )
            .collect::<Vec<_>>();

        pending.into_iter().chain(completed).collect()
    }
}

fn string_array(value: Option<&Value>) -> Option<Vec<String>> {
    value.and_then(Value::as_array).map(|values| {
        values
            .iter()
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>()
    })
}

#[cfg(test)]
mod tests;
