use crate::config;
use crate::protobuf::{connect_frame_decode, extract_strings};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::protocol::ParsedModelTurn;

pub(super) const DEBUG_PREVIEW_CHARS: usize = 4_000;

#[derive(Debug, Clone)]
pub(super) struct DebugLogger {
    pub(super) path: Option<PathBuf>,
}

impl DebugLogger {
    pub(super) fn from_env() -> Self {
        if !config::swegrep_debug_enabled() {
            return Self { path: None };
        }

        let path = config::swegrep_debug_log_path();
        Self { path: Some(path) }
    }

    pub(super) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub(super) fn log(&self, message: impl AsRef<str>) {
        let Some(path) = &self.path else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{} {}", debug_timestamp_ms(), message.as_ref());
        }
    }
}

fn debug_timestamp_ms() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("[{millis}]")
}

pub(super) fn debug_model_response(
    debug: &DebugLogger,
    turn_label: &str,
    response: &[u8],
    parsed: &ParsedModelTurn,
) {
    if debug.path().is_none() {
        return;
    }
    debug.log(format!(
        "model_response turn={} bytes={} parsed={}",
        debug_preview(turn_label, 128),
        response.len(),
        debug_parsed_turn_summary(parsed)
    ));
    let strings = debug_response_strings(response);
    if !strings.is_empty() {
        debug.log(format!(
            "model_response_strings turn={} values={}",
            debug_preview(turn_label, 128),
            debug_preview(&strings.join(" | "), DEBUG_PREVIEW_CHARS)
        ));
    }
}

fn debug_response_strings(response: &[u8]) -> Vec<String> {
    connect_frame_decode(response)
        .into_iter()
        .flat_map(|frame| {
            let mut values = extract_strings(&frame);
            let raw = String::from_utf8_lossy(&frame).replace('\u{fffd}', "");
            if raw.len() > 10 && !values.iter().any(|value| value == &raw) {
                values.push(raw);
            }
            values
        })
        .map(|value| debug_preview(&value, DEBUG_PREVIEW_CHARS))
        .filter(|value| !value.trim().is_empty())
        .collect()
}

pub(super) fn debug_parsed_turn_summary(parsed: &ParsedModelTurn) -> String {
    match parsed {
        ParsedModelTurn::ToolCalls { thinking, calls } => {
            let call_count = calls.len();
            let call_summaries = calls
                .iter()
                .map(|call| {
                    format!(
                        "{} args={}",
                        debug_preview(&call.name, 128),
                        debug_preview(&call.args.to_string(), DEBUG_PREVIEW_CHARS)
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            format!(
                "tool_calls count={} thinking={} calls=[{}]",
                call_count,
                debug_preview(thinking, DEBUG_PREVIEW_CHARS),
                call_summaries
            )
        }
        ParsedModelTurn::Text(text) => {
            format!("text value={}", debug_preview(text, DEBUG_PREVIEW_CHARS))
        }
        ParsedModelTurn::Error(error) => {
            format!("error value={}", debug_preview(error, DEBUG_PREVIEW_CHARS))
        }
    }
}

pub(super) fn debug_answer_result(
    debug: &DebugLogger,
    source: &str,
    answer_xml: &str,
    file_count: usize,
) {
    debug.log(format!(
        "final_answer source={} files={} xml={}",
        source,
        file_count,
        debug_preview(answer_xml, DEBUG_PREVIEW_CHARS)
    ));
}

pub(super) fn debug_preview(value: &str, max_chars: usize) -> String {
    let mut preview = value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    if preview.chars().count() > max_chars {
        preview = preview.chars().take(max_chars).collect::<String>();
        preview.push_str("...");
    }
    preview
}
