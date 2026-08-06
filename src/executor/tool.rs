use crate::config;
use crate::path_filter::{PathFilter, PathFilterConfig};
use std::env;
use std::ffi::OsString;
use std::fs::{self, FileType};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TruncationProfile {
    General,
    ReadfileExpanded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OutputLimits {
    pub(crate) max_lines: usize,
    pub(crate) line_max_chars: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct VisibleEntry {
    pub(crate) path: PathBuf,
    pub(crate) name: OsString,
    pub(crate) file_type: FileType,
    sort_key: String,
}

impl VisibleEntry {
    pub(crate) fn is_dir(&self) -> bool {
        self.file_type.is_dir()
    }

    pub(crate) fn is_file(&self) -> bool {
        self.file_type.is_file()
    }

    pub(crate) fn is_symlink(&self) -> bool {
        self.file_type.is_symlink()
    }
}

#[derive(Debug)]
pub(crate) struct BoundedText {
    lines: Vec<String>,
    max_lines: usize,
    line_max_chars: usize,
    truncated: bool,
}

impl BoundedText {
    pub(crate) fn new(max_lines: usize, line_max_chars: usize) -> Self {
        Self {
            lines: Vec::with_capacity(max_lines),
            max_lines,
            line_max_chars,
            truncated: false,
        }
    }

    pub(crate) fn push_line(&mut self, line: &str) -> bool {
        if self.lines.len() >= self.max_lines {
            self.truncated = true;
            return false;
        }
        self.lines.push(truncate_line(line, self.line_max_chars));
        true
    }

    pub(crate) fn is_full(&self) -> bool {
        self.lines.len() >= self.max_lines
    }

    pub(crate) fn mark_truncated(&mut self) {
        self.truncated = true;
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub(crate) fn finish(self) -> String {
        let mut result = self.lines.join("\n");
        if self.truncated {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str("... (lines truncated) ...");
        }
        result
    }
}

#[derive(Debug, Clone)]
pub struct ToolExecutor {
    pub(super) root: PathBuf,
    pub(super) collected_rg_patterns: Arc<Mutex<Vec<String>>>,
    general_result_max_lines: usize,
    readfile_result_max_lines: usize,
    tool_line_max_chars: usize,
    path_filter: Arc<PathFilter>,
}

impl ToolExecutor {
    pub fn new(project_root: impl AsRef<Path>) -> Self {
        Self::with_limits(project_root, None, None)
    }

    pub fn with_limits(
        project_root: impl AsRef<Path>,
        result_max_lines: Option<usize>,
        line_max_chars: Option<usize>,
    ) -> Self {
        Self::with_limits_and_filter(
            project_root,
            result_max_lines,
            line_max_chars,
            PathFilterConfig::default(),
        )
    }

    pub fn with_limits_and_filter(
        project_root: impl AsRef<Path>,
        result_max_lines: Option<usize>,
        line_max_chars: Option<usize>,
        path_filter_config: PathFilterConfig,
    ) -> Self {
        Self::with_resolved_limits_and_filter(
            project_root,
            config::fc_result_max_lines(result_max_lines),
            config::fc_readfile_max_lines(),
            config::fc_line_max_chars(line_max_chars),
            path_filter_config,
        )
    }

    fn with_resolved_limits_and_filter(
        project_root: impl AsRef<Path>,
        general_result_max_lines: usize,
        readfile_result_max_lines: usize,
        tool_line_max_chars: usize,
        path_filter_config: PathFilterConfig,
    ) -> Self {
        let root = project_root
            .as_ref()
            .canonicalize()
            .unwrap_or_else(|_| normalize_path(project_root.as_ref()));
        let path_filter = Arc::new(PathFilter::new(&root, path_filter_config));

        Self {
            root,
            collected_rg_patterns: Arc::new(Mutex::new(Vec::new())),
            general_result_max_lines,
            readfile_result_max_lines,
            tool_line_max_chars,
            path_filter,
        }
    }

    pub fn collected_rg_patterns(&self) -> Vec<String> {
        self.collected_rg_patterns
            .lock()
            .map(|patterns| patterns.clone())
            .unwrap_or_default()
    }

    pub fn path_filter_warnings(&self) -> &[String] {
        self.path_filter.warnings()
    }

    pub fn path_filter_enabled(&self) -> bool {
        self.path_filter.is_enabled()
    }

    pub fn real(&self, virtual_path: &str) -> PathBuf {
        if virtual_path.is_empty() {
            return self.root.clone();
        }

        let raw_path =
            if virtual_path.starts_with("/codebase") || virtual_path.starts_with("\\codebase") {
                let rel = virtual_path
                    .trim_start_matches("/codebase")
                    .trim_start_matches("\\codebase")
                    .trim_start_matches(['/', '\\']);
                self.root.join(rel)
            } else {
                let path = PathBuf::from(virtual_path);
                if path.is_absolute() {
                    path
                } else {
                    env::current_dir()
                        .unwrap_or_else(|_| self.root.clone())
                        .join(path)
                }
            };

        let resolved = normalize_path(&raw_path);
        if resolved.starts_with(&self.root) {
            resolved
        } else {
            self.root.clone()
        }
    }

    pub(super) fn remap(&self, text: &str) -> String {
        let root_native = self.root.to_string_lossy();
        let root_slash = self.root.to_string_lossy().replace('\\', "/");
        let mut remapped = text.replace(root_native.as_ref(), "/codebase");
        if root_native != root_slash {
            remapped = remapped.replace(&root_slash, "/codebase");
        }
        remapped
    }

    pub(crate) fn output_limits(&self, profile: TruncationProfile) -> OutputLimits {
        match profile {
            TruncationProfile::General => OutputLimits {
                max_lines: self.general_result_max_lines,
                line_max_chars: self.tool_line_max_chars,
            },
            TruncationProfile::ReadfileExpanded => OutputLimits {
                max_lines: self.readfile_result_max_lines,
                line_max_chars: self.tool_line_max_chars,
            },
        }
    }

    pub(crate) fn bounded_text(&self, profile: TruncationProfile) -> BoundedText {
        let limits = self.output_limits(profile);
        BoundedText::new(limits.max_lines, limits.line_max_chars)
    }

    pub(super) fn is_visible_path(&self, path: &Path, is_dir: bool) -> bool {
        self.path_filter.is_visible(path, is_dir)
    }

    pub(crate) fn visible_entries(&self, dir: &Path) -> io::Result<Vec<VisibleEntry>> {
        let mut entries = Vec::new();
        for result in fs::read_dir(dir)? {
            let entry = result?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            let is_dir_for_filter = file_type.is_dir() || (file_type.is_symlink() && path.is_dir());
            if !self.is_visible_path(&path, is_dir_for_filter) {
                continue;
            }
            let name = entry.file_name();
            let sort_key = name.to_string_lossy().to_lowercase();
            entries.push(VisibleEntry {
                path,
                name,
                file_type,
                sort_key,
            });
        }
        entries.sort_by(|a, b| {
            (!a.is_dir())
                .cmp(&(!b.is_dir()))
                .then_with(|| a.sort_key.cmp(&b.sort_key))
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(entries)
    }

    pub(crate) fn path_contains_symlink(&self, path: &Path) -> bool {
        let Ok(relative) = path.strip_prefix(&self.root) else {
            return false;
        };
        let mut current = self.root.clone();
        for component in relative.components() {
            current.push(component.as_os_str());
            if current
                .symlink_metadata()
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                return true;
            }
        }
        false
    }
}

fn truncate_line(line: &str, line_max_chars: usize) -> String {
    if line.is_ascii() {
        return line[..line.len().min(line_max_chars)].to_string();
    }
    match line.char_indices().nth(line_max_chars) {
        Some((index, _)) => line[..index].to_string(),
        None => line.to_string(),
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    fn executor_with_limits(
        project_root: impl AsRef<Path>,
        general_result_max_lines: usize,
        readfile_result_max_lines: usize,
        line_max_chars: usize,
    ) -> ToolExecutor {
        ToolExecutor::with_resolved_limits_and_filter(
            project_root,
            general_result_max_lines,
            readfile_result_max_lines,
            line_max_chars,
            PathFilterConfig::default(),
        )
    }

    #[test]
    fn executor_paths_are_clamped_to_root() {
        let tmp = TempDir::new().unwrap();
        let executor = ToolExecutor::new(tmp.path());

        assert_eq!(
            executor.real("/codebase"),
            tmp.path().canonicalize().unwrap()
        );
        assert_eq!(
            executor.real("/codebase/sub/file.py"),
            tmp.path().join("sub").join("file.py")
        );
        assert_eq!(
            executor.real("/codebase/../../etc/passwd"),
            tmp.path().canonicalize().unwrap()
        );
        assert_eq!(
            executor.real("/codebase/sub/../../../etc/passwd"),
            tmp.path().canonicalize().unwrap()
        );
        assert_eq!(
            executor.real("/etc/passwd"),
            tmp.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn readfile_tool_uses_expanded_defaults() {
        let tmp = TempDir::new().unwrap();
        let executor = executor_with_limits(
            tmp.path(),
            config::DEFAULT_FC_RESULT_MAX_LINES,
            config::DEFAULT_FC_READFILE_MAX_LINES,
            config::DEFAULT_FC_LINE_MAX_CHARS,
        );
        let long_line = "a".repeat(400);
        let contents = (1..=410)
            .map(|_| long_line.clone())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(tmp.path().join("test.txt"), contents).unwrap();

        let output =
            executor.exec_command(&json!({"type": "readfile", "file": "/codebase/test.txt"}));

        assert!(output.contains("400:"));
        assert!(!output.contains("401:"));
        assert!(output.ends_with("... (lines truncated) ..."));
        assert_eq!(output.lines().next().unwrap().len(), 300);
    }

    #[test]
    fn general_tools_use_shared_defaults() {
        let tmp = TempDir::new().unwrap();
        let executor = executor_with_limits(
            tmp.path(),
            config::DEFAULT_FC_RESULT_MAX_LINES,
            config::DEFAULT_FC_READFILE_MAX_LINES,
            config::DEFAULT_FC_LINE_MAX_CHARS,
        );
        for idx in 1..=90 {
            fs::write(tmp.path().join(format!("file_{idx:03}.txt")), "").unwrap();
        }

        let output = executor.exec_command(&json!({"type": "ls", "path": "/codebase"}));

        assert!(output.contains("file_080.txt"));
        assert!(!output.contains("file_081.txt"));
        assert!(output.ends_with("... (lines truncated) ..."));
    }

    #[test]
    fn readfile_limit_affects_readfile_tool() {
        let tmp = TempDir::new().unwrap();
        let executor = executor_with_limits(
            tmp.path(),
            config::DEFAULT_FC_RESULT_MAX_LINES,
            120,
            config::DEFAULT_FC_LINE_MAX_CHARS,
        );
        let contents = (1..=130)
            .map(|idx| format!("line-{idx}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(tmp.path().join("test.txt"), contents).unwrap();

        let tool_output =
            executor.exec_command(&json!({"type": "readfile", "file": "/codebase/test.txt"}));

        assert!(tool_output.contains("120:line-120"));
        assert!(!tool_output.contains("121:line-121"));
    }

    #[test]
    fn general_result_limit_affects_general_tools() {
        let tmp = TempDir::new().unwrap();
        let executor = executor_with_limits(
            tmp.path(),
            10,
            config::DEFAULT_FC_READFILE_MAX_LINES,
            config::DEFAULT_FC_LINE_MAX_CHARS,
        );
        for idx in 1..=20 {
            fs::write(tmp.path().join(format!("file_{idx:03}.txt")), "").unwrap();
        }

        let tool_output = executor.exec_command(&json!({"type": "ls", "path": "/codebase"}));

        assert!(tool_output.contains("file_010.txt"));
        assert!(!tool_output.contains("file_011.txt"));
    }

    #[test]
    fn truncation_counts_unicode_characters() {
        let tmp = TempDir::new().unwrap();
        let executor = executor_with_limits(
            tmp.path(),
            config::DEFAULT_FC_RESULT_MAX_LINES,
            config::DEFAULT_FC_READFILE_MAX_LINES,
            20,
        );
        let input = "界".repeat(21);

        let mut output = executor.bounded_text(TruncationProfile::General);
        for line in input.split('\n') {
            output.push_line(line);
        }
        let output = output.finish();

        assert_eq!(output, "界".repeat(20));
        assert_eq!(output.chars().count(), 20);
    }
}
