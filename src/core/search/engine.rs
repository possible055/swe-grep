use super::debug::{
    DEBUG_PREVIEW_CHARS, DebugLogger, debug_answer_result, debug_model_response, debug_preview,
};
use super::restricted_exec::{
    format_restricted_exec_results, has_valid_restricted_exec_command, restricted_exec_commands,
};
use crate::executor::{
    InstantContextTiming as ExecutorTiming, InstantContextToolCall as ExecutorToolCall,
    InstantContextToolUpdate as ExecutorToolUpdate, ToolExecutionStatus as ExecutorStatus,
    ToolExecutor,
};
use serde_json::Value;
use std::future::Future;

use super::super::auth::{get_api_key, get_cached_jwt};
use super::super::error::FastContextError;
use super::super::http;
use super::super::protocol::{
    ChatMessage, FINAL_FORCE_ANSWER, MessageRole, ParsedModelTurn, build_request,
    build_system_prompt, get_tool_definitions, parse_response, trim_messages,
};
use super::super::repo::{RepoMap, get_repo_map_with_executor, parse_answer};
use super::super::types::{SearchError, SearchMeta, SearchOptions, SearchResult, unique_patterns};

const MAX_COMPENSATED_TURNS: usize = 2;

#[derive(Debug, Clone, PartialEq)]
struct InstantContextStep {
    id: String,
    tool_calls: Vec<ExecutorToolCall>,
}

impl InstantContextStep {
    fn from_calls(id: String, calls: Vec<ExecutorToolCall>) -> Self {
        let tool_calls = calls;
        Self { id, tool_calls }
    }
}

fn failed_tool_updates(step: &InstantContextStep, message: &str) -> Vec<ExecutorToolUpdate> {
    let pending = step.tool_calls.iter().map(|call| ExecutorToolUpdate {
        step_id: step.id.clone(),
        tool_call_id: call.id.clone(),
        tool_name: call.name.clone(),
        command: call.args.clone(),
        status: ExecutorStatus::Pending,
        output: String::new(),
        timing: ExecutorTiming::default(),
    });
    let terminal = step.tool_calls.iter().map(|call| ExecutorToolUpdate {
        step_id: step.id.clone(),
        tool_call_id: call.id.clone(),
        tool_name: call.name.clone(),
        command: Value::Null,
        status: ExecutorStatus::Error,
        output: format!("Error: {message}"),
        timing: ExecutorTiming::default(),
    });
    pending.chain(terminal).collect()
}

pub async fn search(
    options: SearchOptions,
    on_progress: Option<&(dyn Fn(&str) + Sync)>,
) -> SearchResult {
    search_with_streaming(
        options,
        on_progress,
        |proto, timeout_ms, max_retries, ls_version| async move {
            http::streaming_request(&proto, timeout_ms, max_retries, ls_version.as_deref()).await
        },
    )
    .await
}

pub async fn search_with_streaming<F, Fut>(
    mut options: SearchOptions,
    on_progress: Option<&(dyn Fn(&str) + Sync)>,
    mut streaming: F,
) -> SearchResult
where
    F: FnMut(Vec<u8>, u64, u32, Option<String>) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, FastContextError>>,
{
    let project_root = options
        .project_root
        .canonicalize()
        .unwrap_or_else(|_| options.project_root.clone());
    options.project_root = project_root.clone();

    let log = |message: &str| {
        if let Some(on_progress) = on_progress {
            on_progress(message);
        }
    };
    let debug = DebugLogger::from_env();
    if let Some(path) = debug.path() {
        log(&format!("Debug log: {}", path.display()));
        debug.log(format!(
            "search_start project_root={} max_turns={} max_commands={} query={}",
            project_root.display(),
            options.max_turns,
            options.max_commands,
            debug_preview(&options.query, DEBUG_PREVIEW_CHARS)
        ));
    }

    let api_key = match get_api_key(options.api_key.as_deref(), true) {
        Ok(api_key) => api_key,
        Err(err) => {
            return SearchResult {
                error: Some(SearchError::new("API_KEY_ERROR", err)),
                meta: SearchMeta {
                    project_root: Some(project_root.to_string_lossy().into_owned()),
                    error_code: Some("API_KEY_ERROR".to_string()),
                    ..SearchMeta::default()
                },
                ..SearchResult::default()
            };
        }
    };

    let timeout_ms = options.timeout_ms;

    let jwt = match options.jwt.clone() {
        Some(jwt) => jwt,
        None => match get_cached_jwt(&api_key, timeout_ms).await {
            Ok(jwt) => jwt,
            Err(err) => {
                return SearchResult {
                    error: Some(SearchError::new(err.code.clone(), err.message)),
                    meta: SearchMeta {
                        project_root: Some(project_root.to_string_lossy().into_owned()),
                        error_code: Some(err.code),
                        ..SearchMeta::default()
                    },
                    ..SearchResult::default()
                };
            }
        },
    };

    let executor = ToolExecutor::with_limits_and_filter(
        &project_root,
        options.result_max_lines,
        options.line_max_chars,
        options.path_filter.clone(),
    );
    for warning in executor.path_filter_warnings() {
        log(&format!("Path filter warning: {warning}"));
    }
    options.max_commands = options.max_commands.clamp(1, 8);
    let tool_defs = get_tool_definitions(options.max_commands);
    let system_prompt = build_system_prompt(options.max_turns, options.max_commands);

    let repo_map = get_repo_map_with_executor(options.tree_depth, &executor);
    log(&format!(
        "Repo map: scope snapshot depth {} ({:.1}KB){}",
        repo_map.depth,
        repo_map.size_bytes as f64 / 1024.0,
        if repo_map.fell_back {
            " [fell back]"
        } else {
            ""
        }
    ));
    let user_content = build_user_content(&options.query, &repo_map);

    let mut messages = vec![
        ChatMessage::simple(MessageRole::System, system_prompt),
        ChatMessage::simple(MessageRole::User, user_content),
    ];

    let mut force_answer_injected = false;
    let mut compensated_turns = 0_usize;
    let mut turn = 0_usize;

    while turn < options.max_turns + 1 + compensated_turns {
        let progress_turn = format_search_progress_turn(
            turn,
            compensated_turns,
            options.max_turns,
            force_answer_injected,
        );
        log(&progress_turn);

        let proto = build_request(
            &api_key,
            &jwt,
            &messages,
            &tool_defs,
            options.app_version.as_deref(),
            options.ide_version.as_deref(),
        );

        let mut response = match streaming(proto, timeout_ms, 2, options.ls_version.clone()).await {
            Ok(response) => response,
            Err(err) => {
                let base_meta = search_meta(&project_root, &repo_map, Some(err.code.clone()));

                if matches!(err.code.as_str(), "PAYLOAD_TOO_LARGE" | "TIMEOUT")
                    && messages.len() > 4
                {
                    log(&format!(
                        "{} on {}: trimming context and retrying...",
                        err.code,
                        format_search_turn_ref(turn, compensated_turns, options.max_turns)
                    ));
                    trim_messages(&mut messages);
                    let retry_proto = build_request(
                        &api_key,
                        &jwt,
                        &messages,
                        &tool_defs,
                        options.app_version.as_deref(),
                        options.ide_version.as_deref(),
                    );
                    match streaming(retry_proto, timeout_ms, 2, options.ls_version.clone()).await {
                        Ok(response) => response,
                        Err(retry_err) => {
                            return SearchResult {
                                files: Vec::new(),
                                error: Some(SearchError::new(
                                    retry_err.code.clone(),
                                    format!("{} (retry failure)", retry_err.message),
                                )),
                                meta: SearchMeta {
                                    error_code: Some(retry_err.code),
                                    context_trimmed: Some(true),
                                    ..base_meta
                                },
                                ..SearchResult::default()
                            };
                        }
                    }
                } else {
                    return SearchResult {
                        files: Vec::new(),
                        error: Some(SearchError::new(err.code, err.message)),
                        meta: base_meta,
                        ..SearchResult::default()
                    };
                }
            }
        };

        let mut parsed = parse_response(&response);
        debug_model_response(&debug, &progress_turn, &response, &parsed);
        if matches!(&parsed, ParsedModelTurn::Text(text) if text.trim().is_empty()) {
            log("Empty model response on turn; retrying once...");
            let retry_proto = build_request(
                &api_key,
                &jwt,
                &messages,
                &tool_defs,
                options.app_version.as_deref(),
                options.ide_version.as_deref(),
            );
            response = match streaming(retry_proto, timeout_ms, 2, options.ls_version.clone()).await
            {
                Ok(response) => response,
                Err(err) => {
                    return SearchResult {
                        files: Vec::new(),
                        error: Some(SearchError::new(err.code.clone(), err.message)),
                        meta: search_meta(&project_root, &repo_map, Some(err.code)),
                        ..SearchResult::default()
                    };
                }
            };
            parsed = parse_response(&response);
            debug_model_response(
                &debug,
                &format!("{progress_turn} retry"),
                &response,
                &parsed,
            );
            if matches!(&parsed, ParsedModelTurn::Text(text) if text.trim().is_empty()) {
                return SearchResult {
                    files: Vec::new(),
                    error: Some(SearchError::new(
                        "MODEL_EMPTY_RESPONSE",
                        "Model returned empty response",
                    )),
                    meta: search_meta(
                        &project_root,
                        &repo_map,
                        Some("MODEL_EMPTY_RESPONSE".to_string()),
                    ),
                    ..SearchResult::default()
                };
            }
        }

        let (thinking, tool_calls) = match parsed {
            ParsedModelTurn::ToolCalls { thinking, calls } => (thinking, calls),
            ParsedModelTurn::Error(error) => {
                return SearchResult {
                    files: Vec::new(),
                    error: Some(SearchError::new("MODEL_ERROR", error)),
                    meta: search_meta(&project_root, &repo_map, Some("MODEL_ERROR".to_string())),
                    ..SearchResult::default()
                };
            }
            ParsedModelTurn::Text(text) => {
                if text.contains("<ANSWER") {
                    log("Received final answer");
                    let mut result = parse_answer(&text, &project_root);
                    result.files.truncate(options.max_results);
                    debug_answer_result(&debug, "text", &text, result.files.len());
                    result.rg_patterns = unique_patterns(executor.collected_rg_patterns());
                    result.meta = search_meta(&project_root, &repo_map, None);
                    return result;
                }

                let effective_turns_used = (turn + 1)
                    .saturating_sub(compensated_turns)
                    .min(options.max_turns);
                if !force_answer_injected {
                    messages.push(ChatMessage::simple(MessageRole::Assistant, text.clone()));
                    if effective_turns_used < options.max_turns {
                        messages.push(ChatMessage::simple(
                            MessageRole::User,
                            format_search_turn_status(effective_turns_used, options.max_turns),
                        ));
                        log("Plain-text response without answer; continuing search");
                    } else {
                        messages.push(ChatMessage::simple(MessageRole::User, FINAL_FORCE_ANSWER));
                        force_answer_injected = true;
                        log("Injected force-answer prompt after plain-text response");
                    }
                    turn += 1;
                    continue;
                }

                return SearchResult {
                    files: Vec::new(),
                    rg_patterns: unique_patterns(executor.collected_rg_patterns()),
                    raw_response: Some(text),
                    error: None,
                    meta: search_meta(&project_root, &repo_map, None),
                };
            }
        };

        if let Some(answer_call) = tool_calls.iter().find(|call| call.name == "answer") {
            let answer_xml = answer_call
                .args
                .get("answer")
                .and_then(Value::as_str)
                .unwrap_or_default();
            log("Received final answer");
            let mut result = parse_answer(answer_xml, &project_root);
            result.files.truncate(options.max_results);
            debug_answer_result(&debug, "tool_call", answer_xml, result.files.len());
            result.rg_patterns = unique_patterns(executor.collected_rg_patterns());
            result.meta = search_meta(&project_root, &repo_map, None);
            return result;
        }

        let tool_call = tool_calls
            .iter()
            .find(|call| call.name == "restricted_exec")
            .cloned()
            .or_else(|| tool_calls.into_iter().next());
        let Some(tool_call) = tool_call else {
            turn += 1;
            continue;
        };
        let (step, assistant_tool_name, assistant_tool_args, tool_ref_id) = if tool_call.name
            == "restricted_exec"
        {
            let has_valid_command =
                has_valid_restricted_exec_command(&tool_call.args, options.max_commands);
            let step = InstantContextStep::from_calls(
                format!("restricted-exec-{}", turn + 1),
                restricted_exec_commands(&tool_call.args, options.max_commands),
            );
            if !has_valid_command && compensated_turns < MAX_COMPENSATED_TURNS {
                compensated_turns += 1;
                log(&format!(
                    "No valid restricted_exec commands; scheduling compensation turn {compensated_turns}/{MAX_COMPENSATED_TURNS}"
                ));
            } else if !has_valid_command {
                log(&format!(
                    "Turn compensation skipped: max compensations ({MAX_COMPENSATED_TURNS}) reached"
                ));
            }
            (
                step,
                tool_call.name,
                tool_call.args.to_string(),
                tool_call.id,
            )
        } else {
            let command = serde_json::json!({
                "type": tool_call.name,
                "arguments": tool_call.args,
            });
            let step = InstantContextStep::from_calls(
                format!("unsupported-tool-{}", turn + 1),
                vec![ExecutorToolCall {
                    id: "command1".to_string(),
                    name: "command1".to_string(),
                    args: command,
                }],
            );
            (
                step,
                tool_call.name,
                tool_call.args.to_string(),
                tool_call.id,
            )
        };

        log(&format!(
            "Executing {} restricted_exec commands",
            step.tool_calls.len()
        ));
        let timeout_budget_ms = (timeout_ms as u128 / 2).max(1_000);
        let blocking_executor = executor.clone();
        let blocking_step_id = step.id.clone();
        let blocking_calls = step.tool_calls.clone();
        let updates = match tokio::task::spawn_blocking(move || {
            blocking_executor.exec_restricted_exec_step(
                &blocking_step_id,
                &blocking_calls,
                timeout_budget_ms,
            )
        })
        .await
        {
            Ok(updates) => updates,
            Err(error) => {
                log(&format!("Restricted exec worker failed: {error}"));
                failed_tool_updates(&step, "restricted exec worker failed")
            }
        };
        let results = format_restricted_exec_results(&updates);

        messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: thinking,
            tool_call_id: Some(tool_ref_id.clone()),
            tool_name: Some(assistant_tool_name),
            tool_args_json: Some(assistant_tool_args),
            ref_call_id: None,
        });
        messages.push(ChatMessage {
            role: MessageRole::Tool,
            content: results,
            tool_call_id: None,
            tool_name: None,
            tool_args_json: None,
            ref_call_id: Some(tool_ref_id),
        });

        let effective_turns_used = (turn + 1)
            .saturating_sub(compensated_turns)
            .min(options.max_turns);
        if effective_turns_used < options.max_turns {
            messages.push(ChatMessage::simple(
                MessageRole::User,
                format_search_turn_status(effective_turns_used, options.max_turns),
            ));
        } else if !force_answer_injected {
            messages.push(ChatMessage::simple(MessageRole::User, FINAL_FORCE_ANSWER));
            force_answer_injected = true;
            log("Injected force-answer prompt");
        }

        turn += 1;
    }

    SearchResult {
        files: Vec::new(),
        rg_patterns: unique_patterns(executor.collected_rg_patterns()),
        error: Some(SearchError::new(
            "MAX_TURNS_REACHED",
            "Max turns reached without getting an answer",
        )),
        meta: search_meta(
            &project_root,
            &repo_map,
            Some("MAX_TURNS_REACHED".to_string()),
        ),
        ..SearchResult::default()
    }
}

fn format_search_turn_status(used: usize, max_turns: usize) -> String {
    format!(
        "Search turns used: {used}. Search turns remaining: {}.",
        max_turns.saturating_sub(used)
    )
}

pub(super) fn format_search_progress_turn(
    turn: usize,
    compensated_turns: usize,
    max_turns: usize,
    force_answer_injected: bool,
) -> String {
    if force_answer_injected {
        return "Final answer turn".to_string();
    }
    if is_compensation_turn(turn, compensated_turns, max_turns) {
        return format!("Compensation turn {compensated_turns}/{MAX_COMPENSATED_TURNS}");
    }
    format!(
        "Turn {}",
        format_search_turn_ref(turn, compensated_turns, max_turns)
    )
}

fn is_compensation_turn(turn: usize, compensated_turns: usize, max_turns: usize) -> bool {
    compensated_turns > 0 && (turn + 1).saturating_sub(compensated_turns) >= max_turns
}

fn format_search_turn_ref(turn: usize, compensated_turns: usize, max_turns: usize) -> String {
    let max_turns = max_turns.max(1);
    let used = (turn + 1)
        .saturating_sub(compensated_turns)
        .max(1)
        .min(max_turns);
    format!("{used}/{max_turns}")
}

pub(super) fn build_user_content(query: &str, repo_map: &RepoMap) -> String {
    format!(
        r#"Workspace scope snapshot (depth {}):
```text
{}
```

Please find the code context for the query: "({})". Constrain search to directory: "(/codebase)"."#,
        repo_map.depth, repo_map.tree, query
    )
}

fn search_meta(
    project_root: &std::path::Path,
    repo_map: &RepoMap,
    error_code: Option<String>,
) -> SearchMeta {
    SearchMeta {
        tree_depth: Some(repo_map.depth),
        tree_size_kb: Some((repo_map.size_bytes as f64 / 1024.0 * 10.0).round() / 10.0),
        fell_back: Some(repo_map.fell_back),
        project_root: Some(project_root.to_string_lossy().into_owned()),
        error_code,
        instant_context: Some(true),
        ..SearchMeta::default()
    }
}
