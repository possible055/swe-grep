use super::super::{BoundedText, ToolExecutor, TruncationProfile, VisibleEntry};
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::fs;
use std::path::Path;
use std::time::Instant;

const GLOB_RESULT_LIMIT: usize = 100;

impl ToolExecutor {
    pub(super) fn execute_tree(
        &self,
        path: &str,
        levels: Option<usize>,
        deadline: Option<Instant>,
    ) -> String {
        if path.is_empty() {
            return "Error: missing or invalid path".to_string();
        }
        let real_path = self.real(path);
        if self.path_contains_symlink(&real_path) {
            return format!("Error: symlink paths are not followed: {path}");
        }
        if !real_path.is_dir() {
            return format!("Error: dir not found: {path}");
        }

        let mut output = self.bounded_text(TruncationProfile::General);
        output.push_line(path);
        if let Err(error) = self.append_tree_lines(&real_path, levels, 1, "", deadline, &mut output)
        {
            return error;
        }
        self.remap(&output.finish())
    }

    fn append_tree_lines(
        &self,
        dir_path: &Path,
        max_depth: Option<usize>,
        current_depth: usize,
        parent_prefix: &str,
        deadline: Option<Instant>,
        output: &mut BoundedText,
    ) -> Result<(), String> {
        if max_depth.is_some_and(|depth| current_depth > depth) {
            return Ok(());
        }
        if deadline_expired(deadline) {
            return Err("Error: tool timed out".to_string());
        }

        let entries = self
            .visible_entries(dir_path)
            .map_err(|error| format!("Error: {}: {error}", dir_path.display()))?;
        let count = entries.len();
        for (index, entry) in entries.iter().enumerate() {
            if deadline_expired(deadline) {
                return Err("Error: tool timed out".to_string());
            }
            if output.is_full() {
                output.mark_truncated();
                return Ok(());
            }

            let is_last = index + 1 == count;
            let connector = if is_last { "└── " } else { "├── " };
            let name = entry.name.to_string_lossy();
            output.push_line(&format!("{parent_prefix}{connector}{name}"));

            if entry.is_dir() {
                let branch = if is_last { "    " } else { "│   " };
                let child_prefix = format!("{parent_prefix}{branch}");
                self.append_tree_lines(
                    &entry.path,
                    max_depth,
                    current_depth + 1,
                    &child_prefix,
                    deadline,
                    output,
                )?;
            }
        }
        Ok(())
    }

    pub(super) fn execute_ls(
        &self,
        path: &str,
        long_format: bool,
        all_files: bool,
        deadline: Option<Instant>,
    ) -> String {
        if path.is_empty() {
            return "Error: missing or invalid path".to_string();
        }
        let real_path = self.real(path);
        if self.path_contains_symlink(&real_path) {
            return format!("Error: symlink paths are not followed: {path}");
        }
        if !real_path.is_dir() {
            return format!("Error: dir not found: {path}");
        }

        let mut entries = match self.visible_entries(&real_path) {
            Ok(entries) => entries,
            Err(error) => return format!("Error: {error}"),
        };
        if !all_files {
            entries.retain(|entry| !entry.name.to_string_lossy().starts_with('.'));
        }

        let mut output = self.bounded_text(TruncationProfile::General);
        if long_format {
            output.push_line(&format!("total {}", entries.len()));
        }
        for entry in entries {
            if deadline_expired(deadline) {
                return "Error: tool timed out".to_string();
            }
            let name = entry.name.to_string_lossy();
            let line = if long_format {
                match long_format_line(&entry, &name) {
                    Ok(line) => line,
                    Err(error) => return format!("Error: {}: {error}", entry.path.display()),
                }
            } else {
                name.into_owned()
            };
            if !output.push_line(&line) {
                break;
            }
        }
        self.remap(&output.finish())
    }

    pub(super) fn execute_glob(
        &self,
        pattern: &str,
        path: &str,
        type_filter: &str,
        deadline: Option<Instant>,
    ) -> String {
        if pattern.is_empty() {
            return "Error: missing or invalid pattern".to_string();
        }
        if path.is_empty() {
            return "Error: missing or invalid path".to_string();
        }

        let real_path = self.real(path);
        if self.path_contains_symlink(&real_path) {
            return format!("Error: symlink paths are not followed: {path}");
        }
        if !real_path.is_dir() {
            return format!("Error: dir not found: {path}");
        }
        let recursive = pattern.contains("**");
        let matcher = match compile_glob_matcher(pattern, recursive) {
            Ok(matcher) => matcher,
            Err(error) => return format!("Error: invalid glob pattern '{pattern}': {error}"),
        };

        let mut matches = Vec::with_capacity(GLOB_RESULT_LIMIT + 1);
        if let Err(error) = self.collect_glob_matches(
            &real_path,
            &real_path,
            &matcher,
            recursive,
            type_filter,
            deadline,
            &mut matches,
        ) {
            return error;
        }
        if matches.is_empty() {
            return "(no matches)".to_string();
        }

        let was_truncated = matches.len() > GLOB_RESULT_LIMIT;
        matches.truncate(GLOB_RESULT_LIMIT);
        let mut output = self.bounded_text(TruncationProfile::General);
        for entry in matches {
            if !output.push_line(&self.virtual_path(&entry.path)) {
                break;
            }
        }
        if was_truncated {
            output.mark_truncated();
        }
        output.finish()
    }

    // Keep recursive traversal state explicit instead of introducing a general walker abstraction.
    #[allow(clippy::too_many_arguments)]
    fn collect_glob_matches(
        &self,
        root: &Path,
        dir: &Path,
        matcher: &GlobSet,
        recursive: bool,
        type_filter: &str,
        deadline: Option<Instant>,
        matches: &mut Vec<VisibleEntry>,
    ) -> Result<bool, String> {
        if deadline_expired(deadline) {
            return Err("Error: tool timed out".to_string());
        }
        let entries = self
            .visible_entries(dir)
            .map_err(|error| format!("Error: {}: {error}", dir.display()))?;
        for entry in entries {
            if deadline_expired(deadline) {
                return Err("Error: tool timed out".to_string());
            }
            let rel = entry
                .path
                .strip_prefix(root)
                .unwrap_or(&entry.path)
                .to_string_lossy()
                .replace('\\', "/");
            let name = entry.name.to_string_lossy();
            if matches_type(&entry, type_filter)
                && (matcher.is_match(&rel) || matcher.is_match(name.as_ref()))
            {
                matches.push(entry.clone());
                if matches.len() > GLOB_RESULT_LIMIT {
                    return Ok(false);
                }
            }
            if recursive
                && entry.is_dir()
                && !self.collect_glob_matches(
                    root,
                    &entry.path,
                    matcher,
                    true,
                    type_filter,
                    deadline,
                    matches,
                )?
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn compile_glob_matcher(pattern: &str, recursive: bool) -> Result<GlobSet, globset::Error> {
    let mut builder = GlobSetBuilder::new();
    let normalized = pattern.replace('\\', "/");
    builder.add(Glob::new(&normalized)?);
    if recursive
        && let Some(clean_pattern) = normalized.strip_prefix("**/")
        && clean_pattern != normalized
    {
        builder.add(Glob::new(clean_pattern)?);
    }
    builder.build()
}

fn matches_type(entry: &VisibleEntry, type_filter: &str) -> bool {
    match type_filter {
        "file" => entry.is_file(),
        "directory" => entry.is_dir(),
        _ => true,
    }
}

fn long_format_line(entry: &VisibleEntry, name: &str) -> std::io::Result<String> {
    let metadata = fs::symlink_metadata(&entry.path)?;
    let kind = if entry.is_dir() {
        "directory"
    } else if entry.is_file() {
        "file"
    } else if entry.is_symlink() {
        "symlink"
    } else {
        "other"
    };
    let size = if entry.is_file() {
        metadata.len().to_string()
    } else {
        "-".to_string()
    };
    Ok(format!(
        "{kind}\t{size}\t{}\t{name}",
        metadata.permissions().readonly()
    ))
}

fn deadline_expired(deadline: Option<Instant>) -> bool {
    deadline.is_some_and(|deadline| Instant::now() >= deadline)
}
