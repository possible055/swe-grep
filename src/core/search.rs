mod debug;
mod engine;
mod restricted_exec;

pub use engine::{search, search_with_streaming};

use super::types::SearchOptions;
use super::{SearchOutputConfig, format_search_error, format_search_success};

#[cfg(test)]
use debug::{DebugLogger, debug_model_response, debug_preview};
#[cfg(test)]
use engine::{build_user_content, format_search_progress_turn};
#[cfg(test)]
use restricted_exec::{
    format_restricted_exec_results, has_valid_restricted_exec_command, restricted_exec_commands,
};

pub async fn search_with_content(options: SearchOptions) -> String {
    let max_turns = options.max_turns;
    let max_results = options.max_results;
    let max_commands = options.max_commands;
    let result = search(options.clone(), None).await;

    if result.error.is_some() {
        return format_search_error(
            &result,
            Some(SearchOutputConfig {
                max_turns,
                max_results,
                max_commands,
            }),
        );
    }

    let mut output = format_search_success(&result);
    if result.meta.tree_depth.is_some() {
        let fb_note = if result.meta.fell_back == Some(true) {
            " (fell back)"
        } else {
            ""
        };
        output.push_str(&format!(
            "\n\n[config] tree_depth={}{}{}, tree_size={}KB, max_turns={max_turns}, max_results={max_results}",
            result.meta.tree_depth.unwrap_or_default(),
            fb_note,
            "",
            result.meta.tree_size_kb.unwrap_or_default()
        ));
    }

    output
}

#[cfg(test)]
mod tests;
