use super::*;
use crate::core::protobuf::{ProtobufEncoder, connect_frame_encode, decode_varint};

#[test]
fn trim_messages_keeps_head_bridge_and_tail() {
    let mut messages = vec![
        ChatMessage::simple(MessageRole::System, "system"),
        ChatMessage::simple(MessageRole::User, "user"),
        ChatMessage::simple(MessageRole::Assistant, "thinking 1"),
        ChatMessage::simple(MessageRole::Tool, "result 1"),
        ChatMessage::simple(MessageRole::Assistant, "thinking 2"),
        ChatMessage::simple(MessageRole::Tool, "result 2"),
    ];

    assert!(trim_messages(&mut messages));
    assert_eq!(messages.len(), 5);
    assert_eq!(messages[0].content, "system");
    assert_eq!(messages[1].content, "user");
    assert!(messages[2].content.contains("omitted"));
    assert_eq!(messages[3].content, "thinking 2");
    assert_eq!(messages[4].content, "result 2");
}

#[test]
fn message_role_wire_values_match_existing_protocol_numbers() {
    assert_eq!(MessageRole::User.wire_value(), 1);
    assert_eq!(MessageRole::Assistant.wire_value(), 2);
    assert_eq!(MessageRole::Tool.wire_value(), 4);
    assert_eq!(MessageRole::System.wire_value(), 5);
}

#[test]
fn build_system_prompt_contains_required_protocol_keywords() {
    let prompt = build_system_prompt(3, 6);

    for keyword in [
        "/codebase",
        "restricted_exec",
        "answer",
        "ANSWER",
        "rg",
        "readfile",
        "tree",
        "ls",
        "glob",
        "FAST-SEARCH",
        "6",
        "3",
    ] {
        assert!(prompt.contains(keyword), "missing keyword: {keyword}");
    }
    assert!(!prompt.contains("{max_commands}"));
    assert!(!prompt.contains("{max_turns}"));
    assert!(!prompt.contains("{max_results}"));
}

#[test]
fn final_force_answer_prompt_matches_official_source() {
    assert_eq!(
        FINAL_FORCE_ANSWER,
        "You have no turns left. Now you MUST provide your final ANSWER, even if it's not complete."
    );
}

#[test]
fn build_request_writes_official_version_metadata() {
    let proto = build_request(
        "test-api-key",
        "test.jwt.token",
        &[ChatMessage::simple(MessageRole::User, "query")],
        "[]",
        Some("app-version"),
        Some("ide-version"),
    );
    let (tag, offset) = decode_varint(&proto, 0);
    assert_eq!(tag, 0x0a);
    let (length, offset) = decode_varint(&proto, offset);
    let metadata = &proto[offset..offset + length as usize];
    let mut fields = std::collections::BTreeMap::new();
    let mut offset = 0;
    while offset < metadata.len() {
        let (tag, next) = decode_varint(metadata, offset);
        assert_eq!(tag & 7, 2);
        let (length, next) = decode_varint(metadata, next);
        offset = next + length as usize;
        fields.insert(tag >> 3, &metadata[next..offset]);
    }
    assert_eq!(fields.get(&2).copied(), Some(b"app-version".as_slice()));
    assert_eq!(fields.get(&7).copied(), Some(b"ide-version".as_slice()));
}

#[test]
fn tool_definitions_expose_only_restricted_exec_and_answer() {
    let defs: serde_json::Value = serde_json::from_str(&get_tool_definitions(8)).unwrap();
    let tools = defs.as_array().unwrap();

    let names = tools
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["restricted_exec", "answer"]);

    let schema = &tools[0]["function"]["parameters"]["properties"];
    assert!(schema.get("command1").is_some());
    assert!(schema.get("command8").is_some());
    assert!(schema.get("command9").is_none());

    let serialized = serde_json::to_string(&defs).unwrap();
    assert!(serialized.contains("\"rg\""));
    assert!(serialized.contains("\"readfile\""));
    assert!(serialized.contains("\"tree\""));
    assert!(serialized.contains("\"ls\""));
    assert!(serialized.contains("\"glob\""));
}

#[test]
fn parse_response_extracts_text_tool_call() {
    let mut encoder = ProtobufEncoder::new();
    encoder.write_string(1, "thinking");
    encoder.write_string(
            2,
            r#"[TOOL_CALLS]restricted_exec[ARGS]{"command1":{"type":"rg","pattern":"main","path":"/codebase/src"}}"#,
        );
    let frame = connect_frame_encode(&encoder.to_bytes(), false);

    let ParsedModelTurn::ToolCalls { thinking, calls } = parse_response(&frame) else {
        panic!("expected tool call");
    };
    assert_eq!(thinking, "thinking");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "restricted_exec");
    assert_eq!(calls[0].args["command1"]["type"], "rg");
    assert_eq!(calls[0].args["command1"]["pattern"], "main");
}

#[test]
fn parse_response_repairs_restricted_exec_command_fragments() {
    let mut encoder = ProtobufEncoder::new();
    encoder.write_string(1, "thinking");
    encoder.write_string(
            2,
            r#"[TOOL_CALLS]restricted_exec[ARGS]{"command1":{"rg":{"pattern":"SearchResult","path":"/codebase/src"},"command2":{"file":"/codebase/src/core.rs","start_line":1,"end_line":20}"#,
        );
    let frame = connect_frame_encode(&encoder.to_bytes(), false);

    let ParsedModelTurn::ToolCalls { calls, .. } = parse_response(&frame) else {
        panic!("expected repaired tool call");
    };

    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "restricted_exec");
    assert_eq!(calls[0].args["command1"]["rg"]["pattern"], "SearchResult");
    assert_eq!(calls[0].args["command1"]["rg"]["path"], "/codebase/src");
    assert_eq!(calls[0].args["command2"]["file"], "/codebase/src/core.rs");
    assert_eq!(calls[0].args["command2"]["start_line"], 1);
    assert_eq!(calls[0].args["command2"]["end_line"], 20);
}

#[test]
fn parse_response_extracts_structured_restricted_exec() {
    let frame = connect_frame_encode(
            br#"{"output":"thinking","tool_calls":[{"id":"a","name":"restricted_exec","args":{"command1":{"type":"glob","pattern":"**/*.rs","path":"/codebase","type_filter":"file"},"command2":{"type":"readfile","file":"/codebase/src/lib.rs","start_line":1,"end_line":20}}}]}"#,
            false,
        );

    let ParsedModelTurn::ToolCalls { thinking, calls } = parse_response(&frame) else {
        panic!("expected structured tool calls");
    };

    assert_eq!(thinking, "thinking");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "a");
    assert_eq!(calls[0].name, "restricted_exec");
    assert_eq!(calls[0].args["command1"]["type"], "glob");
    assert_eq!(calls[0].args["command2"]["type"], "readfile");
    assert_eq!(calls[0].args["command2"]["file"], "/codebase/src/lib.rs");
}

#[test]
fn parse_response_extracts_answer_tool_call() {
    let mut encoder = ProtobufEncoder::new();
    encoder.write_string(
        1,
        r#"[TOOL_CALLS]answer[ARGS]{"answer":"<ANSWER></ANSWER>"}"#,
    );
    let frame = connect_frame_encode(&encoder.to_bytes(), false);

    let ParsedModelTurn::ToolCalls { calls, .. } = parse_response(&frame) else {
        panic!("expected answer");
    };
    assert_eq!(calls[0].name, "answer");
    assert_eq!(calls[0].args["answer"], "<ANSWER></ANSWER>");
}

#[test]
fn parse_response_handles_error_frame() {
    let frame = connect_frame_encode(
        br#"{"error":{"code":"TIMEOUT","message":"request timed out"}}"#,
        false,
    );

    assert_eq!(
        parse_response(&frame),
        ParsedModelTurn::Error("[Error] TIMEOUT: request timed out".to_string())
    );
}

#[test]
fn parse_response_returns_text_without_tool_call() {
    let mut encoder = ProtobufEncoder::new();
    encoder.write_string(1, "plain model text");
    let frame = connect_frame_encode(&encoder.to_bytes(), false);

    assert_eq!(
        parse_response(&frame),
        ParsedModelTurn::Text("plain model text".to_string())
    );
}
