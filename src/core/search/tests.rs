use super::debug::debug_parsed_turn_summary;
use super::*;
use crate::core::protocol::{FINAL_FORCE_ANSWER, ParsedModelTurn, parse_response};
use crate::core::repo::RepoMap;
use crate::executor::{InstantContextTiming, InstantContextToolUpdate, ToolExecutionStatus};
use crate::protobuf::{ProtobufEncoder, connect_frame_encode};
use std::collections::VecDeque;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

#[test]
fn build_user_content_includes_compact_scope_snapshot() {
    let repo_map = RepoMap {
        tree: "/codebase\nCargo.toml\nsrc/\nsrc/lib.rs".to_string(),
        depth: 2,
        size_bytes: 35,
        fell_back: false,
    };

    let content = build_user_content("find parser", &repo_map);

    assert!(content.contains("Workspace scope snapshot (depth 2):"));
    assert!(content.contains("/codebase"));
    assert!(content.contains("Cargo.toml"));
    assert!(content.contains("src/"));
    assert!(content.contains(r#"Please find the code context for the query: "(find parser)""#));
    assert!(content.contains(r#"Constrain search to directory: "(/codebase)""#));
}

#[test]
fn restricted_exec_results_hide_internal_status_and_timing() {
    let updates = vec![
        InstantContextToolUpdate {
            step_id: "step-1".to_string(),
            tool_call_id: "command1".to_string(),
            tool_name: "command1".to_string(),
            command: serde_json::json!({"type": "rg", "pattern": "main", "path": "/codebase"}),
            status: ToolExecutionStatus::Completed,
            output: "/codebase/src/main.rs:1:fn main() {}".to_string(),
            timing: InstantContextTiming { duration_ms: 12 },
        },
        InstantContextToolUpdate {
            step_id: "step-1".to_string(),
            tool_call_id: "command2".to_string(),
            tool_name: "command2".to_string(),
            command: serde_json::json!({"type": "readfile", "file": "/codebase/src/lib.rs"}),
            status: ToolExecutionStatus::TimedOut,
            output: "Error: tool timed out".to_string(),
            timing: InstantContextTiming { duration_ms: 1_000 },
        },
    ];

    let output = format_restricted_exec_results(&updates);

    assert!(output.contains("command1_result:\n/codebase/src/main.rs:1:fn main() {}"));
    assert!(output.contains("command2_result:\nError: tool timed out"));
    assert!(!output.contains("status="));
    assert!(!output.contains("duration_ms="));
}

#[test]
fn restricted_exec_commands_keep_standard_command_shape() {
    let args = serde_json::json!({
        "command1": {
            "type": "rg",
            "pattern": "needle",
            "path": "/codebase/src"
        }
    });

    assert!(has_valid_restricted_exec_command(&args, 8));
    let commands = restricted_exec_commands(&args, 8);

    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].args, args["command1"]);
}

#[test]
fn restricted_exec_commands_normalize_model_shorthand_commands() {
    let args = serde_json::json!({
        "command1": {
            "rg": {
                "pattern": "auth",
                "path": "/codebase/src",
                "exclude": ["test"]
            }
        },
        "command2": {
            "readfile": "/codebase/src/core.rs",
            "start_line": 1,
            "end_line": 10
        },
        "command3": {
            "rg": "parse_answer",
            "path": "/codebase/src/core"
        },
        "command4": {
            "glob": "**/*.rs",
            "path": "/codebase/src"
        }
    });

    assert!(has_valid_restricted_exec_command(&args, 8));
    let commands = restricted_exec_commands(&args, 8);

    assert_eq!(commands.len(), 4);
    assert_eq!(commands[0].args["type"], "rg");
    assert_eq!(commands[0].args["pattern"], "auth");
    assert_eq!(commands[0].args["path"], "/codebase/src");
    assert_eq!(commands[1].args["type"], "readfile");
    assert_eq!(commands[1].args["file"], "/codebase/src/core.rs");
    assert_eq!(commands[1].args["start_line"], 1);
    assert_eq!(commands[2].args["type"], "rg");
    assert_eq!(commands[2].args["pattern"], "parse_answer");
    assert_eq!(commands[2].args["path"], "/codebase/src/core");
    assert_eq!(commands[3].args["type"], "glob");
    assert_eq!(commands[3].args["pattern"], "**/*.rs");
    assert_eq!(commands[3].args["path"], "/codebase/src");
}

#[test]
fn restricted_exec_commands_infer_missing_command_types_from_fields() {
    let args = serde_json::json!({
        "command1": {
            "file": "/codebase/src/core.rs",
            "start_line": 1,
            "end_line": 10
        },
        "command2": {
            "pattern": "parse_answer",
            "path": "/codebase/src"
        },
        "command3": {
            "pattern": "**/*.rs",
            "path": "/codebase/src",
            "type_filter": "file"
        },
        "command4": {
            "path": "/codebase/src",
            "levels": 2
        },
        "command5": {
            "path": "/codebase/src",
            "all": false
        }
    });

    assert!(has_valid_restricted_exec_command(&args, 8));
    let commands = restricted_exec_commands(&args, 8);

    assert_eq!(commands.len(), 5);
    assert_eq!(commands[0].args["type"], "readfile");
    assert_eq!(commands[1].args["type"], "rg");
    assert_eq!(commands[2].args["type"], "glob");
    assert_eq!(commands[3].args["type"], "tree");
    assert_eq!(commands[4].args["type"], "ls");
}

#[test]
fn search_progress_turn_log_uses_configured_search_turn_count() {
    assert_eq!(format_search_progress_turn(3, 0, 4, false), "Turn 4/4");
    assert_eq!(format_search_progress_turn(4, 0, 5, false), "Turn 5/5");
    assert_eq!(format_search_progress_turn(5, 0, 6, false), "Turn 6/6");
    assert_eq!(
        format_search_progress_turn(5, 1, 5, false),
        "Compensation turn 1/2"
    );
    assert_eq!(
        format_search_progress_turn(6, 2, 5, false),
        "Compensation turn 2/2"
    );
    assert_eq!(
        format_search_progress_turn(3, 0, 3, true),
        "Final answer turn"
    );
}

#[test]
fn debug_parsed_turn_summary_includes_model_values() {
    let parsed = ParsedModelTurn::ToolCalls {
        thinking: "look\naround".to_string(),
        calls: vec![crate::core::protocol::ParsedToolCall {
            id: "tool-1".to_string(),
            name: "restricted_exec".to_string(),
            args: serde_json::json!({
                "command1": {
                    "type": "rg",
                    "pattern": "format_search_success",
                    "path": "/codebase/src"
                }
            }),
        }],
    };

    let summary = debug_parsed_turn_summary(&parsed);

    assert!(summary.contains("tool_calls count=1"));
    assert!(summary.contains("restricted_exec"));
    assert!(summary.contains("format_search_success"));
    assert!(summary.contains("look\\naround"));
}

#[test]
fn debug_preview_truncates_and_escapes_control_characters() {
    assert_eq!(debug_preview("a\nb\tc", 20), "a\\nb\\tc");
    assert_eq!(debug_preview("abcdef", 3), "abc...");
}

#[test]
fn debug_model_response_writes_model_values_to_log() {
    let tmp = TempDir::new().unwrap();
    let log_path = tmp.path().join("debug.log");
    let debug = DebugLogger {
        path: Some(log_path.clone()),
    };
    let mut response = ProtobufEncoder::new();
    response.write_string(1, "model text value");
    let frame = connect_frame_encode(&response.to_bytes(), false);
    let parsed = parse_response(&frame);

    debug_model_response(&debug, "Turn 1/4", &frame, &parsed);

    let log = fs::read_to_string(log_path).unwrap();
    assert!(log.contains("model_response turn=Turn 1/4"));
    assert!(log.contains("model_response_strings"));
    assert!(log.contains("model text value"));
}

#[tokio::test]
async fn search_loop_success_with_mock_streaming() {
    let mut t1_encoder = ProtobufEncoder::new();
    t1_encoder.write_string(1, "thinking about doing search");
    t1_encoder.write_string(
            2,
            r#"[TOOL_CALLS]restricted_exec[ARGS]{"command1":{"type":"readfile","file":"/codebase/test.txt"}}"#,
        );
    let t1_frame = connect_frame_encode(&t1_encoder.to_bytes(), false);

    let mut t2_encoder = ProtobufEncoder::new();
    t2_encoder.write_string(1, "found answer");
    t2_encoder.write_string(
            2,
            r#"[TOOL_CALLS]answer[ARGS]{"answer": "<ANSWER><file path=\"/codebase/test.txt\"><range>1-10</range></file></ANSWER>"}"#,
        );
    let t2_frame = connect_frame_encode(&t2_encoder.to_bytes(), false);

    let responses = Arc::new(Mutex::new(VecDeque::from([t1_frame, t2_frame])));
    let call_count = Arc::new(AtomicUsize::new(0));
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("test.txt"), "line1\nline2").unwrap();

    let mut options = SearchOptions::new("find main", tmp.path());
    options.api_key = Some("sk-ws-01-key".to_string());
    options.jwt = Some("mocked.jwt.token".to_string());
    options.max_turns = 2;

    let result = search_with_streaming(options, None, {
        let responses = Arc::clone(&responses);
        let call_count = Arc::clone(&call_count);
        move |_, _, _, _| {
            let responses = Arc::clone(&responses);
            let call_count = Arc::clone(&call_count);
            async move {
                call_count.fetch_add(1, Ordering::SeqCst);
                Ok(responses.lock().unwrap().pop_front().unwrap())
            }
        }
    })
    .await;

    assert_eq!(result.files.len(), 1);
    assert_eq!(result.files[0].path, "test.txt");
    assert_eq!(result.files[0].ranges, vec![(1, 2)]);
    assert_eq!(call_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn search_loop_supports_restricted_exec_then_answer() {
    let t1_frame = connect_frame_encode(
            br#"{"output":"find candidates","tool_calls":[{"id":"q1","name":"restricted_exec","args":{"command1":{"type":"glob","pattern":"test.txt","path":"/codebase","type_filter":"file"},"command2":{"type":"readfile","file":"/codebase/test.txt","start_line":1,"end_line":1}}}]}"#,
            false,
        );

    let mut t2_encoder = ProtobufEncoder::new();
    t2_encoder.write_string(
            1,
            r#"[TOOL_CALLS]answer[ARGS]{"answer": "<ANSWER><file path=\"/codebase/test.txt\"><range>1-1</range></file></ANSWER>"}"#,
        );
    let t2_frame = connect_frame_encode(&t2_encoder.to_bytes(), false);

    let responses = Arc::new(Mutex::new(VecDeque::from([t1_frame, t2_frame])));
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("test.txt"), "line1").unwrap();

    let mut options = SearchOptions::new("find test", tmp.path());
    options.api_key = Some("sk-ws-01-key".to_string());
    options.jwt = Some("mocked.jwt.token".to_string());
    options.max_turns = 2;

    let result = search_with_streaming(options, None, {
        let responses = Arc::clone(&responses);
        move |_, _, _, _| {
            let responses = Arc::clone(&responses);
            async move { Ok(responses.lock().unwrap().pop_front().unwrap()) }
        }
    })
    .await;

    assert_eq!(result.files.len(), 1);
    assert_eq!(result.files[0].path, "test.txt");
    assert_eq!(result.files[0].ranges, vec![(1, 1)]);
}

#[tokio::test]
async fn search_loop_reports_remaining_search_turns_after_valid_tool_round() {
    let t1_frame = connect_frame_encode(
            br#"{"output":"read context","tool_calls":[{"id":"q1","name":"restricted_exec","args":{"command1":{"type":"readfile","file":"/codebase/test.txt","start_line":1,"end_line":1}}}]}"#,
            false,
        );
    let mut answer_encoder = ProtobufEncoder::new();
    answer_encoder.write_string(
            1,
            r#"[TOOL_CALLS]answer[ARGS]{"answer": "<ANSWER><file path=\"/codebase/test.txt\"><range>1-1</range></file></ANSWER>"}"#,
        );
    let answer_frame = connect_frame_encode(&answer_encoder.to_bytes(), false);

    let responses = Arc::new(Mutex::new(VecDeque::from([t1_frame, answer_frame])));
    let requests = Arc::new(Mutex::new(Vec::<String>::new()));
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("test.txt"), "line1").unwrap();

    let mut options = SearchOptions::new("find test", tmp.path());
    options.api_key = Some("sk-ws-01-key".to_string());
    options.jwt = Some("mocked.jwt.token".to_string());
    options.max_turns = 3;

    let result = search_with_streaming(options, None, {
        let responses = Arc::clone(&responses);
        let requests = Arc::clone(&requests);
        move |proto, _, _, _| {
            let responses = Arc::clone(&responses);
            let requests = Arc::clone(&requests);
            async move {
                requests
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&proto).to_string());
                Ok(responses.lock().unwrap().pop_front().unwrap())
            }
        }
    })
    .await;

    assert!(result.error.is_none());
    assert_eq!(result.files.len(), 1);

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].contains("Search turns used: 1. Search turns remaining: 2."));
    assert!(!requests[1].contains(FINAL_FORCE_ANSWER));
}

#[tokio::test]
async fn search_loop_injects_force_answer_after_last_search_turn() {
    let t1_frame = connect_frame_encode(
            br#"{"output":"first read","tool_calls":[{"id":"q1","name":"restricted_exec","args":{"command1":{"type":"readfile","file":"/codebase/test.txt","start_line":1,"end_line":1}}}]}"#,
            false,
        );
    let t2_frame = connect_frame_encode(
            br#"{"output":"second read","tool_calls":[{"id":"q2","name":"restricted_exec","args":{"command1":{"type":"readfile","file":"/codebase/test.txt","start_line":2,"end_line":2}}}]}"#,
            false,
        );
    let mut answer_encoder = ProtobufEncoder::new();
    answer_encoder.write_string(
            1,
            r#"[TOOL_CALLS]answer[ARGS]{"answer": "<ANSWER><file path=\"/codebase/test.txt\"><range>1-2</range></file></ANSWER>"}"#,
        );
    let answer_frame = connect_frame_encode(&answer_encoder.to_bytes(), false);

    let responses = Arc::new(Mutex::new(VecDeque::from([
        t1_frame,
        t2_frame,
        answer_frame,
    ])));
    let requests = Arc::new(Mutex::new(Vec::<String>::new()));
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("test.txt"), "line1\nline2").unwrap();

    let mut options = SearchOptions::new("find test", tmp.path());
    options.api_key = Some("sk-ws-01-key".to_string());
    options.jwt = Some("mocked.jwt.token".to_string());
    options.max_turns = 2;

    let result = search_with_streaming(options, None, {
        let responses = Arc::clone(&responses);
        let requests = Arc::clone(&requests);
        move |proto, _, _, _| {
            let responses = Arc::clone(&responses);
            let requests = Arc::clone(&requests);
            async move {
                requests
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&proto).to_string());
                Ok(responses.lock().unwrap().pop_front().unwrap())
            }
        }
    })
    .await;

    assert!(result.error.is_none());
    assert_eq!(result.files.len(), 1);

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[1].contains("Search turns used: 1. Search turns remaining: 1."));
    assert!(!requests[1].contains(FINAL_FORCE_ANSWER));
    assert!(requests[2].contains(FINAL_FORCE_ANSWER));
}

#[tokio::test]
async fn search_loop_empty_answer_does_not_fallback_to_tool_history() {
    let t1_frame = connect_frame_encode(
            br#"{"output":"read context","tool_calls":[{"id":"q1","name":"restricted_exec","args":{"command1":{"type":"readfile","file":"/codebase/test.txt","start_line":1,"end_line":2}}}]}"#,
            false,
        );

    let mut t2_encoder = ProtobufEncoder::new();
    t2_encoder.write_string(
        1,
        r#"[TOOL_CALLS]answer[ARGS]{"answer": "<ANSWER></ANSWER>"}"#,
    );
    let t2_frame = connect_frame_encode(&t2_encoder.to_bytes(), false);

    let responses = Arc::new(Mutex::new(VecDeque::from([t1_frame, t2_frame])));
    let requests = Arc::new(Mutex::new(Vec::<String>::new()));
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("test.txt"), "line1\nline2\nline3").unwrap();

    let mut options = SearchOptions::new("find test", tmp.path());
    options.api_key = Some("sk-ws-01-key".to_string());
    options.jwt = Some("mocked.jwt.token".to_string());
    options.max_turns = 2;

    let result = search_with_streaming(options, None, {
        let responses = Arc::clone(&responses);
        let requests = Arc::clone(&requests);
        move |proto, _, _, _| {
            let responses = Arc::clone(&responses);
            let requests = Arc::clone(&requests);
            async move {
                requests
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&proto).to_string());
                Ok(responses.lock().unwrap().pop_front().unwrap())
            }
        }
    })
    .await;

    assert!(result.files.is_empty());

    let requests = requests.lock().unwrap();
    assert!(requests[0].contains("Workspace scope snapshot (depth "));
    assert!(requests[0].contains("/codebase"));
    assert!(requests[0].contains("test.txt"));
    assert!(requests[1].contains("command1_result:"));
    assert!(!requests[1].contains("status="));
    assert!(!requests[1].contains("duration_ms="));
}

#[tokio::test]
async fn search_loop_uses_final_answer_without_tool_history_ranges() {
    let t1_frame = connect_frame_encode(
            br#"{"output":"read context","tool_calls":[{"id":"q1","name":"restricted_exec","args":{"command1":{"type":"readfile","file":"/codebase/test.txt","start_line":1,"end_line":2}}}]}"#,
            false,
        );

    let mut t2_encoder = ProtobufEncoder::new();
    t2_encoder.write_string(
            1,
            r#"[TOOL_CALLS]answer[ARGS]{"answer": "<ANSWER><file path=\"/codebase/test.txt\"><range>3-3</range></file></ANSWER>"}"#,
        );
    let t2_frame = connect_frame_encode(&t2_encoder.to_bytes(), false);

    let responses = Arc::new(Mutex::new(VecDeque::from([t1_frame, t2_frame])));
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("test.txt"), "line1\nline2\nline3").unwrap();

    let mut options = SearchOptions::new("find test", tmp.path());
    options.api_key = Some("sk-ws-01-key".to_string());
    options.jwt = Some("mocked.jwt.token".to_string());
    options.max_turns = 2;

    let result = search_with_streaming(options, None, {
        let responses = Arc::clone(&responses);
        move |_, _, _, _| {
            let responses = Arc::clone(&responses);
            async move { Ok(responses.lock().unwrap().pop_front().unwrap()) }
        }
    })
    .await;

    assert_eq!(result.files.len(), 1);
    assert_eq!(result.files[0].path, "test.txt");
    assert_eq!(result.files[0].ranges, vec![(3, 3)]);
}

#[tokio::test]
async fn search_loop_accepts_plain_text_answer() {
    let mut t1_encoder = ProtobufEncoder::new();
    t1_encoder.write_string(
        1,
        r#"<ANSWER><file path="/codebase/test.txt"><range>1-1</range></file></ANSWER>"#,
    );
    let t1_frame = connect_frame_encode(&t1_encoder.to_bytes(), false);

    let responses = Arc::new(Mutex::new(VecDeque::from([t1_frame])));
    let call_count = Arc::new(AtomicUsize::new(0));
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("test.txt"), "line1").unwrap();

    let mut options = SearchOptions::new("find main", tmp.path());
    options.api_key = Some("sk-ws-01-key".to_string());
    options.jwt = Some("mocked.jwt.token".to_string());
    options.max_turns = 2;

    let result = search_with_streaming(options, None, {
        let responses = Arc::clone(&responses);
        let call_count = Arc::clone(&call_count);
        move |_, _, _, _| {
            let responses = Arc::clone(&responses);
            let call_count = Arc::clone(&call_count);
            async move {
                call_count.fetch_add(1, Ordering::SeqCst);
                Ok(responses.lock().unwrap().pop_front().unwrap())
            }
        }
    })
    .await;

    assert_eq!(result.files.len(), 1);
    assert_eq!(result.files[0].path, "test.txt");
    assert_eq!(call_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn search_loop_returns_raw_response_for_plain_text_after_force_answer() {
    let mut t1_encoder = ProtobufEncoder::new();
    t1_encoder.write_string(1, "I could not find a matching implementation.");
    let t1_frame = connect_frame_encode(&t1_encoder.to_bytes(), false);

    let mut t2_encoder = ProtobufEncoder::new();
    t2_encoder.write_string(1, "Still no structured answer.");
    let t2_frame = connect_frame_encode(&t2_encoder.to_bytes(), false);

    let responses = Arc::new(Mutex::new(VecDeque::from([t1_frame, t2_frame])));
    let requests = Arc::new(Mutex::new(Vec::<String>::new()));
    let tmp = TempDir::new().unwrap();

    let mut options = SearchOptions::new("find missing thing", tmp.path());
    options.api_key = Some("sk-ws-01-key".to_string());
    options.jwt = Some("mocked.jwt.token".to_string());
    options.max_turns = 1;

    let result = search_with_streaming(options, None, {
        let responses = Arc::clone(&responses);
        let requests = Arc::clone(&requests);
        move |proto, _, _, _| {
            let responses = Arc::clone(&responses);
            let requests = Arc::clone(&requests);
            async move {
                requests
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&proto).to_string());
                Ok(responses.lock().unwrap().pop_front().unwrap())
            }
        }
    })
    .await;

    assert!(result.error.is_none());
    assert_eq!(
        result.raw_response.as_deref(),
        Some("Still no structured answer.")
    );
    assert!(result.files.is_empty());

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].contains(FINAL_FORCE_ANSWER));
}

#[tokio::test]
async fn search_loop_injects_force_answer_after_plain_text_on_last_search_turn() {
    let t1_frame = connect_frame_encode(
            br#"{"output":"read context","tool_calls":[{"id":"q1","name":"restricted_exec","args":{"command1":{"type":"readfile","file":"/codebase/test.txt","start_line":1,"end_line":1}}}]}"#,
            false,
        );

    let mut t2_encoder = ProtobufEncoder::new();
    t2_encoder.write_string(1, "I could not find a matching implementation.");
    let t2_frame = connect_frame_encode(&t2_encoder.to_bytes(), false);

    let mut answer_encoder = ProtobufEncoder::new();
    answer_encoder.write_string(
            1,
            r#"[TOOL_CALLS]answer[ARGS]{"answer": "<ANSWER><file path=\"/codebase/test.txt\"><range>1-1</range></file></ANSWER>"}"#,
        );
    let answer_frame = connect_frame_encode(&answer_encoder.to_bytes(), false);

    let responses = Arc::new(Mutex::new(VecDeque::from([
        t1_frame,
        t2_frame,
        answer_frame,
    ])));
    let requests = Arc::new(Mutex::new(Vec::<String>::new()));
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("test.txt"), "line1").unwrap();

    let mut options = SearchOptions::new("find test", tmp.path());
    options.api_key = Some("sk-ws-01-key".to_string());
    options.jwt = Some("mocked.jwt.token".to_string());
    options.max_turns = 2;

    let result = search_with_streaming(options, None, {
        let responses = Arc::clone(&responses);
        let requests = Arc::clone(&requests);
        move |proto, _, _, _| {
            let responses = Arc::clone(&responses);
            let requests = Arc::clone(&requests);
            async move {
                requests
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&proto).to_string());
                Ok(responses.lock().unwrap().pop_front().unwrap())
            }
        }
    })
    .await;

    assert!(result.error.is_none());
    assert_eq!(result.files.len(), 1);

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[2].contains(FINAL_FORCE_ANSWER));
    assert!(requests[2].contains("I could not find a matching implementation."));
}

#[tokio::test]
async fn search_loop_retries_empty_model_response_once() {
    let mut t2_encoder = ProtobufEncoder::new();
    t2_encoder.write_string(
        1,
        r#"<ANSWER><file path="/codebase/test.txt"><range>1-1</range></file></ANSWER>"#,
    );
    let t2_frame = connect_frame_encode(&t2_encoder.to_bytes(), false);

    let responses = Arc::new(Mutex::new(VecDeque::from([Vec::new(), t2_frame])));
    let call_count = Arc::new(AtomicUsize::new(0));
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("test.txt"), "line1").unwrap();

    let mut options = SearchOptions::new("find test", tmp.path());
    options.api_key = Some("sk-ws-01-key".to_string());
    options.jwt = Some("mocked.jwt.token".to_string());
    options.max_turns = 2;

    let result = search_with_streaming(options, None, {
        let responses = Arc::clone(&responses);
        let call_count = Arc::clone(&call_count);
        move |_, _, _, _| {
            let responses = Arc::clone(&responses);
            let call_count = Arc::clone(&call_count);
            async move {
                call_count.fetch_add(1, Ordering::SeqCst);
                Ok(responses.lock().unwrap().pop_front().unwrap())
            }
        }
    })
    .await;

    assert!(result.error.is_none());
    assert_eq!(result.files.len(), 1);
    assert_eq!(call_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn search_loop_errors_after_empty_response_retry_exhausted() {
    let responses = Arc::new(Mutex::new(VecDeque::from([Vec::new(), Vec::new()])));
    let call_count = Arc::new(AtomicUsize::new(0));
    let tmp = TempDir::new().unwrap();

    let mut options = SearchOptions::new("find test", tmp.path());
    options.api_key = Some("sk-ws-01-key".to_string());
    options.jwt = Some("mocked.jwt.token".to_string());
    options.max_turns = 2;

    let result = search_with_streaming(options, None, {
        let responses = Arc::clone(&responses);
        let call_count = Arc::clone(&call_count);
        move |_, _, _, _| {
            let responses = Arc::clone(&responses);
            let call_count = Arc::clone(&call_count);
            async move {
                call_count.fetch_add(1, Ordering::SeqCst);
                Ok(responses.lock().unwrap().pop_front().unwrap())
            }
        }
    })
    .await;

    let error = result.error.as_ref().unwrap();
    assert_eq!(error.code, "MODEL_EMPTY_RESPONSE");
    assert_eq!(error.message, "Model returned empty response");
    assert_eq!(call_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn search_loop_compensates_invalid_restricted_exec_once() {
    let invalid_frame = connect_frame_encode(
            br#"{"output":"bad command","tool_calls":[{"id":"q1","name":"restricted_exec","args":{}}]}"#,
            false,
        );
    let valid_frame = connect_frame_encode(
            br#"{"output":"read context","tool_calls":[{"id":"q2","name":"restricted_exec","args":{"command1":{"type":"readfile","file":"/codebase/test.txt","start_line":1,"end_line":1}}}]}"#,
            false,
        );
    let mut answer_encoder = ProtobufEncoder::new();
    answer_encoder.write_string(
            1,
            r#"[TOOL_CALLS]answer[ARGS]{"answer": "<ANSWER><file path=\"/codebase/test.txt\"><range>1-1</range></file></ANSWER>"}"#,
        );
    let answer_frame = connect_frame_encode(&answer_encoder.to_bytes(), false);

    let responses = Arc::new(Mutex::new(VecDeque::from([
        invalid_frame,
        valid_frame,
        answer_frame,
    ])));
    let requests = Arc::new(Mutex::new(Vec::<String>::new()));
    let call_count = Arc::new(AtomicUsize::new(0));
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("test.txt"), "line1").unwrap();

    let mut options = SearchOptions::new("find test", tmp.path());
    options.api_key = Some("sk-ws-01-key".to_string());
    options.jwt = Some("mocked.jwt.token".to_string());
    options.max_turns = 2;

    let result = search_with_streaming(options, None, {
        let responses = Arc::clone(&responses);
        let requests = Arc::clone(&requests);
        let call_count = Arc::clone(&call_count);
        move |proto, _, _, _| {
            let responses = Arc::clone(&responses);
            let requests = Arc::clone(&requests);
            let call_count = Arc::clone(&call_count);
            async move {
                call_count.fetch_add(1, Ordering::SeqCst);
                requests
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&proto).to_string());
                Ok(responses.lock().unwrap().pop_front().unwrap())
            }
        }
    })
    .await;

    assert!(result.error.is_none());
    assert_eq!(result.files.len(), 1);
    assert_eq!(call_count.load(Ordering::SeqCst), 3);

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[1].contains("Search turns used: 0. Search turns remaining: 2."));
    assert!(requests[2].contains("Search turns used: 1. Search turns remaining: 1."));
    assert!(!requests[2].contains(FINAL_FORCE_ANSWER));
}
