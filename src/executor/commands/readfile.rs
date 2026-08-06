use super::super::{ToolExecutor, TruncationProfile};
use std::fs;

impl ToolExecutor {
    pub(super) fn execute_readfile(
        &self,
        file: &str,
        start_line: Option<usize>,
        end_line: Option<usize>,
    ) -> String {
        if file.is_empty() {
            return "Error: missing or invalid file path".to_string();
        }
        let real_path = self.real(file);
        if !real_path.is_file() {
            return format!("Error: file not found: {file}");
        }
        if !self.is_visible_path(&real_path, false) {
            return format!("Error: file not visible: {file}");
        }

        let content = match fs::read(&real_path) {
            Ok(content) => String::from_utf8_lossy(&content).into_owned(),
            Err(err) => return format!("Error: {err}"),
        };

        let all_lines: Vec<&str> = content.split('\n').collect();
        let start = start_line.unwrap_or(1).saturating_sub(1);
        let end = end_line.unwrap_or(all_lines.len()).min(all_lines.len());
        let selected = if start < end && start < all_lines.len() {
            &all_lines[start..end]
        } else {
            &[]
        };

        let out = selected
            .iter()
            .enumerate()
            .map(|(idx, line)| format!("{}:{line}", start + idx + 1))
            .collect::<Vec<_>>()
            .join("\n");
        self.truncate_text(&out, TruncationProfile::ReadfileExpanded)
    }
}
