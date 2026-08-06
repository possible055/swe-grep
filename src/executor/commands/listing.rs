use super::super::helpers::{compare_dirs_first_case_insensitive, matches_type, pattern_matches};
use super::super::{ToolExecutor, TruncationProfile};
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

impl ToolExecutor {
    pub(super) fn execute_tree(&self, path: &str, levels: Option<usize>) -> String {
        if path.is_empty() {
            return "Error: missing or invalid path".to_string();
        }
        let real_path = self.real(path);
        if !real_path.is_dir() {
            return format!("Error: dir not found: {path}");
        }

        let mut lines = vec![path.to_string()];
        lines.extend(self.generate_tree_lines(&real_path, levels, 1));
        let stdout = lines.join("\n");
        let remapped = self.remap(&stdout);
        self.truncate_text(&remapped, TruncationProfile::General)
    }

    fn generate_tree_lines(
        &self,
        dir_path: &Path,
        max_depth: Option<usize>,
        current_depth: usize,
    ) -> Vec<String> {
        if max_depth.is_some_and(|depth| current_depth > depth) {
            return Vec::new();
        }

        let Ok(entries) = fs::read_dir(dir_path) else {
            return Vec::new();
        };
        let mut items = entries
            .flatten()
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        items.sort_by(|a, b| compare_dirs_first_case_insensitive(a, b));

        let filtered = items
            .into_iter()
            .filter(|item| {
                let Some(_name) = item.file_name().and_then(OsStr::to_str) else {
                    return false;
                };
                let is_dir = item.is_dir();
                if !self.is_visible_path(item, is_dir) {
                    return false;
                }
                true
            })
            .collect::<Vec<_>>();

        let mut lines = Vec::new();
        let count = filtered.len();
        for (index, item) in filtered.iter().enumerate() {
            let is_last = index == count - 1;
            let prefix = if is_last { "└── " } else { "├── " };
            let name = item
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_string();
            lines.push(format!("{prefix}{name}"));

            if item.is_dir() {
                let sub_lines = self.generate_tree_lines(item, max_depth, current_depth + 1);
                let indent = if is_last { "    " } else { "│   " };
                for sub_line in sub_lines {
                    lines.push(format!("{indent}{sub_line}"));
                }
            }
        }

        lines
    }

    pub(super) fn execute_ls(&self, path: &str, long_format: bool, all_files: bool) -> String {
        if path.is_empty() {
            return "Error: missing or invalid path".to_string();
        }
        let real_path = self.real(path);
        if !real_path.is_dir() {
            return format!("Error: dir not found: {path}");
        }

        let mut entries = match fs::read_dir(real_path) {
            Ok(entries) => entries
                .flatten()
                .filter_map(|entry| {
                    let name = entry.file_name().into_string().ok()?;
                    let path = entry.path();
                    let is_dir = entry.file_type().map(|file_type| file_type.is_dir()).ok()?;
                    if !self.is_visible_path(&path, is_dir) {
                        return None;
                    }
                    Some((name, path))
                })
                .collect::<Vec<_>>(),
            Err(err) => return format!("Error: {err}"),
        };
        entries.sort_by(|(a, _), (b, _)| a.cmp(b));

        if !all_files {
            entries.retain(|(name, _)| !name.starts_with('.'));
        }

        if !long_format {
            let names = entries
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            return self.truncate_text(&names, TruncationProfile::General);
        }

        let mut lines = vec![format!("total {}", entries.len())];
        for (name, item_path) in entries {
            let type_char = if item_path.is_dir() { "d" } else { "-" };
            let size = item_path.metadata().map(|meta| meta.len()).unwrap_or(0);
            lines.push(format!(
                "{type_char}rwxr-xr-x  1 user  staff {size:>8} Jan 01 00:00 {name}"
            ));
        }
        self.truncate_text(&self.remap(&lines.join("\n")), TruncationProfile::General)
    }

    pub(super) fn execute_glob(&self, pattern: &str, path: &str, type_filter: &str) -> String {
        if pattern.is_empty() {
            return "Error: missing or invalid pattern".to_string();
        }
        if path.is_empty() {
            return "Error: missing or invalid path".to_string();
        }

        let real_path = self.real(path);
        if !real_path.is_dir() {
            return format!("Error: dir not found: {path}");
        }

        let mut matches = Vec::new();
        if pattern.contains("**") {
            let clean_pattern = pattern.strip_prefix("**/").unwrap_or(pattern);
            for entry in WalkDir::new(&real_path)
                .into_iter()
                .filter_entry(|entry| {
                    entry.depth() == 0
                        || self.is_visible_path(entry.path(), entry.file_type().is_dir())
                })
                .flatten()
            {
                let item = entry.path();
                if !matches_type(item, type_filter) {
                    continue;
                }
                let rel = item
                    .strip_prefix(&real_path)
                    .unwrap_or(item)
                    .to_string_lossy()
                    .replace('\\', "/");
                let name = item.file_name().and_then(OsStr::to_str).unwrap_or_default();
                if pattern_matches(pattern, &rel)
                    || pattern_matches(clean_pattern, &rel)
                    || pattern_matches(clean_pattern, name)
                {
                    matches.push(item.to_path_buf());
                }
            }
        } else if let Ok(entries) = fs::read_dir(&real_path) {
            for entry in entries.flatten() {
                let item = entry.path();
                if !matches_type(&item, type_filter) {
                    continue;
                }
                if !self.is_visible_path(&item, item.is_dir()) {
                    continue;
                }
                let name = item.file_name().and_then(OsStr::to_str).unwrap_or_default();
                if pattern_matches(pattern, name) {
                    matches.push(item);
                }
            }
        }

        matches.sort();
        matches.truncate(100);
        let out = matches
            .iter()
            .map(|path| self.remap(&path.to_string_lossy()))
            .collect::<Vec<_>>()
            .join("\n");
        if out.is_empty() {
            "(no matches)".to_string()
        } else {
            self.truncate_text(&out, TruncationProfile::General)
        }
    }
}
