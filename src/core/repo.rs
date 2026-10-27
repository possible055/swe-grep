use crate::executor::{ToolExecutor, VisibleEntry};
use crate::path_filter::PathFilterConfig;
use regex::Regex;
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path};

use super::{FileEntry, MAX_TREE_BYTES, SearchResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoMap {
    pub tree: String,
    pub depth: usize,
    pub size_bytes: usize,
    pub fell_back: bool,
}

pub fn get_repo_map(
    project_root: &Path,
    target_depth: usize,
    path_filter_config: &PathFilterConfig,
) -> RepoMap {
    let executor =
        ToolExecutor::with_limits_and_filter(project_root, None, None, path_filter_config.clone());
    get_repo_map_with_executor(target_depth, &executor)
}

pub(crate) fn get_repo_map_with_executor(target_depth: usize, executor: &ToolExecutor) -> RepoMap {
    let root = executor.root();
    let entries = match executor.visible_entries(root) {
        Ok(entries) => entries,
        Err(_) => return inaccessible_repo_map(),
    };
    if target_depth == 0 || manifest_size(root, &entries) > MAX_TREE_BYTES {
        return root_fallback(root, entries);
    }

    let mut nodes = entries
        .into_iter()
        .map(ManifestNode::new)
        .collect::<Vec<_>>();
    let mut best_tree = render_manifest(root, &nodes);
    let mut best_depth = 1;
    let mut current_size = best_tree.len();

    for depth in 2..=target_depth {
        match expand_manifest_depth(&mut nodes, 1, depth - 1, root, executor, &mut current_size) {
            Ok(true) => {
                best_tree = render_manifest(root, &nodes);
                best_depth = depth;
            }
            Ok(false) | Err(_) => break,
        }
    }

    RepoMap {
        size_bytes: best_tree.len(),
        tree: best_tree,
        depth: best_depth,
        fell_back: best_depth < target_depth,
    }
}

#[derive(Debug)]
struct ManifestNode {
    entry: VisibleEntry,
    children: Vec<ManifestNode>,
}

impl ManifestNode {
    fn new(entry: VisibleEntry) -> Self {
        Self {
            entry,
            children: Vec::new(),
        }
    }
}

fn expand_manifest_depth(
    nodes: &mut [ManifestNode],
    current_depth: usize,
    parent_depth: usize,
    project_root: &Path,
    executor: &ToolExecutor,
    size_bytes: &mut usize,
) -> std::io::Result<bool> {
    for node in nodes {
        if current_depth == parent_depth {
            if !node.entry.is_dir() {
                continue;
            }
            let entries = executor.visible_entries(&node.entry.path)?;
            let mut children = Vec::with_capacity(entries.len());
            for entry in entries {
                *size_bytes = size_bytes
                    .saturating_add(1)
                    .saturating_add(manifest_line(project_root, &entry.path, entry.is_dir()).len());
                if *size_bytes > MAX_TREE_BYTES {
                    return Ok(false);
                }
                children.push(ManifestNode::new(entry));
            }
            node.children = children;
        } else if !expand_manifest_depth(
            &mut node.children,
            current_depth + 1,
            parent_depth,
            project_root,
            executor,
            size_bytes,
        )? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn manifest_size(project_root: &Path, entries: &[VisibleEntry]) -> usize {
    entries.iter().fold("/codebase".len(), |size, entry| {
        size.saturating_add(1)
            .saturating_add(manifest_line(project_root, &entry.path, entry.is_dir()).len())
    })
}

fn render_manifest(project_root: &Path, nodes: &[ManifestNode]) -> String {
    let mut tree = String::from("/codebase");
    append_manifest_nodes(&mut tree, project_root, nodes);
    tree
}

fn append_manifest_nodes(tree: &mut String, project_root: &Path, nodes: &[ManifestNode]) {
    for node in nodes {
        tree.push('\n');
        tree.push_str(&manifest_line(
            project_root,
            &node.entry.path,
            node.entry.is_dir(),
        ));
        append_manifest_nodes(tree, project_root, &node.children);
    }
}

fn manifest_line(project_root: &Path, path: &Path, is_dir: bool) -> String {
    let rel_opt = path
        .strip_prefix(project_root)
        .ok()
        .map(|rel| rel.to_string_lossy().replace('\\', "/"));
    #[cfg(windows)]
    let rel_opt = rel_opt.or_else(|| {
        let path_s = path.to_string_lossy();
        let root_s = project_root.to_string_lossy();
        let path_clean = path_s.strip_prefix(r"\\?\").unwrap_or(&path_s);
        let root_clean = root_s.strip_prefix(r"\\?\").unwrap_or(&root_s);
        Path::new(path_clean)
            .strip_prefix(Path::new(root_clean))
            .ok()
            .map(|rel| rel.to_string_lossy().replace('\\', "/"))
    });

    let mut rel = rel_opt.unwrap_or_else(|| path.to_string_lossy().replace('\\', "/"));
    if is_dir {
        rel.push('/');
    }
    rel
}

fn root_fallback(project_root: &Path, entries: Vec<VisibleEntry>) -> RepoMap {
    let tree = capped_root_manifest(project_root, entries);
    RepoMap {
        size_bytes: tree.len(),
        tree,
        depth: 0,
        fell_back: true,
    }
}

fn inaccessible_repo_map() -> RepoMap {
    let tree = "/codebase\n(empty or inaccessible)".to_string();
    RepoMap {
        size_bytes: tree.len(),
        tree,
        depth: 0,
        fell_back: true,
    }
}

fn capped_root_manifest(project_root: &Path, entries: Vec<VisibleEntry>) -> String {
    let suffix = "... (scope snapshot truncated) ...";
    let mut tree = String::from("/codebase");
    let mut truncated = false;

    for entry in entries {
        let line = format!(
            "\n{}",
            manifest_line(project_root, &entry.path, entry.is_dir())
        );
        let suffix_len = if truncated { 0 } else { suffix.len() + 1 };
        if tree.len() + line.len() + suffix_len > MAX_TREE_BYTES {
            truncated = true;
            break;
        }
        tree.push_str(&line);
    }

    if truncated {
        tree.push('\n');
        tree.push_str(suffix);
    }
    tree
}

pub fn parse_answer(xml_text: &str, project_root: &Path) -> SearchResult {
    let mut files = Vec::new();
    let resolved_root = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let file_regex = Regex::new(r#"<file\s+path=["']([^"']+)["']>([\s\S]*?)</file>"#).unwrap();
    let range_regex = Regex::new(r"<range>(\d+)-(\d+)</range>").unwrap();

    for captures in file_regex.captures_iter(xml_text) {
        let Some(vpath) = captures.get(1).map(|m| m.as_str()) else {
            continue;
        };
        let body = captures.get(2).map(|m| m.as_str()).unwrap_or_default();
        let Some(rel) = safe_codebase_rel(vpath) else {
            continue;
        };
        if let Some(file) = parse_answer_file(&resolved_root, rel, body, &range_regex) {
            files.push(file);
        }
    }

    SearchResult {
        files,
        ..SearchResult::default()
    }
}

fn parse_answer_file(
    resolved_root: &Path,
    rel: String,
    body: &str,
    range_regex: &Regex,
) -> Option<FileEntry> {
    let full_path = resolved_root.join(&rel);
    let full_path = full_path.canonicalize().ok()?;
    if !full_path.is_file() || !full_path.starts_with(resolved_root) {
        return None;
    }

    let line_count = file_line_count(&full_path).ok()?;
    if line_count == 0 {
        return None;
    }

    let ranges = range_regex
        .captures_iter(body)
        .filter_map(|range| {
            let start = range.get(1)?.as_str().parse::<usize>().ok()?;
            let end = range.get(2)?.as_str().parse::<usize>().ok()?;
            normalize_answer_range(start, end, line_count)
        })
        .collect::<Vec<_>>();
    if ranges.is_empty() {
        return None;
    }

    Some(FileEntry {
        path: rel,
        full_path: full_path.to_string_lossy().into_owned(),
        ranges,
    })
}

fn file_line_count(path: &Path) -> std::io::Result<usize> {
    let mut file = File::open(path)?;
    let mut buffer = [0_u8; 64 * 1024];
    let mut newline_count = 0_usize;
    let mut last_byte = None;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        newline_count += buffer[..read].iter().filter(|byte| **byte == b'\n').count();
        last_byte = Some(buffer[read - 1]);
    }
    let Some(last_byte) = last_byte else {
        return Ok(0);
    };
    if last_byte == b'\n' {
        Ok(newline_count)
    } else {
        Ok(newline_count + 1)
    }
}

fn normalize_answer_range(start: usize, end: usize, line_count: usize) -> Option<(usize, usize)> {
    if start == 0 || end == 0 {
        return None;
    }
    let (start, end) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    if start > line_count {
        return None;
    }
    Some((start, end.min(line_count)))
}

fn safe_codebase_rel(path: &str) -> Option<String> {
    let rel = path
        .strip_prefix("/codebase")
        .unwrap_or(path)
        .trim_start_matches(['/', '\\'])
        .replace('\\', "/");
    if rel.is_empty() {
        return None;
    }
    let rel_path = Path::new(&rel);
    if rel_path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::Prefix(_) | Component::RootDir
        )
    }) {
        return None;
    }
    Some(rel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path_filter::PathFilterConfig;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn parse_answer_filters_path_traversal() {
        let xml = r#"
        Some thoughts first.
        <ANSWER>
          <file path="/codebase/src/main.py">
            <range>10-20</range>
            <range>30-40</range>
          </file>
          <file path="/codebase/tests/test_main.py">
            <range>1-5</range>
          </file>
          <file path="/codebase/../../etc/passwd">
            <range>1-2</range>
          </file>
        </ANSWER>
        "#;
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("src")).unwrap();
        fs::create_dir_all(tmp.path().join("tests")).unwrap();
        fs::write(tmp.path().join("src/main.py"), numbered_lines(40)).unwrap();
        fs::write(tmp.path().join("tests/test_main.py"), numbered_lines(5)).unwrap();

        let result = parse_answer(xml, tmp.path());
        assert_eq!(result.files.len(), 2);
        assert_eq!(result.files[0].path, "src/main.py");
        assert_eq!(result.files[0].ranges, vec![(10, 20), (30, 40)]);
        assert_eq!(result.files[1].path, "tests/test_main.py");
        assert_eq!(result.files[1].ranges, vec![(1, 5)]);
    }

    #[test]
    fn parse_answer_validates_files_and_ranges() {
        let xml = r#"
        <ANSWER>
          <file path="/codebase/src/lib.rs">
            <range>4-2</range>
            <range>3-20</range>
            <range>0-2</range>
            <range>9-10</range>
          </file>
          <file path="/codebase/src/missing.rs">
            <range>1-1</range>
          </file>
          <file path="/codebase/src/empty.rs">
            <range>1-1</range>
          </file>
          <file path="/codebase/src/no_ranges.rs">
          </file>
        </ANSWER>
        "#;
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("src")).unwrap();
        fs::write(tmp.path().join("src/lib.rs"), numbered_lines(6)).unwrap();
        fs::write(tmp.path().join("src/empty.rs"), "").unwrap();
        fs::write(tmp.path().join("src/no_ranges.rs"), numbered_lines(1)).unwrap();

        let result = parse_answer(xml, tmp.path());

        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].path, "src/lib.rs");
        assert_eq!(result.files[0].ranges, vec![(2, 4), (3, 6)]);
    }

    #[test]
    fn get_repo_map_uses_untruncated_manifest() {
        let tmp = TempDir::new().unwrap();
        for i in 0..60 {
            fs::write(tmp.path().join(format!("file_{i:03}.txt")), "").unwrap();
        }

        let result = get_repo_map(tmp.path(), 1, &PathFilterConfig::default());
        assert!(!result.tree.contains("... (lines truncated) ..."));
        assert!(result.tree.contains("file_059.txt"));
        assert_eq!(result.size_bytes, result.tree.len());
    }

    fn numbered_lines(count: usize) -> String {
        (1..=count)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn get_repo_map_falls_back_to_compact_scope_snapshot() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join("src")).unwrap();
        for i in 0..6_000 {
            fs::write(
                tmp.path().join(format!(
                    "very_long_file_name_for_scope_snapshot_padding_{i:05}.txt"
                )),
                "",
            )
            .unwrap();
        }
        fs::write(tmp.path().join("src").join("lib.rs"), "").unwrap();

        let result = get_repo_map(tmp.path(), 4, &PathFilterConfig::default());

        assert!(result.size_bytes <= MAX_TREE_BYTES);
        assert!(result.fell_back);
        assert!(result.tree.starts_with("/codebase"));
        assert!(result.tree.lines().any(|line| line == "src/"));
    }

    #[test]
    fn get_repo_map_keeps_the_last_complete_depth() {
        let tmp = TempDir::new().unwrap();
        let nested = tmp.path().join("src");
        fs::create_dir(&nested).unwrap();
        for index in 0..1_000 {
            fs::write(
                nested.join(format!("long_nested_file_name_{index:04}.rs")),
                "",
            )
            .unwrap();
        }

        let result = get_repo_map(tmp.path(), 4, &PathFilterConfig::default());

        assert_eq!(result.depth, 1);
        assert!(result.fell_back);
        assert!(result.tree.lines().any(|line| line == "src/"));
        assert!(!result.tree.contains("long_nested_file_name"));
        assert!(result.size_bytes <= MAX_TREE_BYTES);
    }

    #[test]
    fn get_repo_map_uses_the_shared_directory_order() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join("zulu-dir")).unwrap();
        fs::create_dir(tmp.path().join("Alpha-dir")).unwrap();
        fs::write(tmp.path().join("zulu.txt"), "").unwrap();
        fs::write(tmp.path().join("Alpha.txt"), "").unwrap();

        let result = get_repo_map(tmp.path(), 1, &PathFilterConfig::default());

        assert_eq!(
            result.tree.lines().collect::<Vec<_>>(),
            vec![
                "/codebase",
                "Alpha-dir/",
                "zulu-dir/",
                "Alpha.txt",
                "zulu.txt"
            ]
        );
    }

    #[test]
    fn get_repo_map_uses_manifest_path_format() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join("src")).unwrap();
        fs::write(tmp.path().join("Cargo.toml"), "").unwrap();
        fs::write(tmp.path().join("src").join("main.rs"), "").unwrap();

        let result = get_repo_map(tmp.path(), 2, &PathFilterConfig::default());
        let lines = result.tree.lines().collect::<Vec<_>>();

        assert_eq!(lines[0], "/codebase");
        assert!(lines.contains(&"Cargo.toml"));
        assert!(lines.contains(&"src/"));
        assert!(lines.contains(&"src/main.rs"));
        assert!(!lines.contains(&"/codebase/src/main.rs"));
    }

    #[test]
    fn get_repo_map_respects_nested_gitignore() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join("pkg")).unwrap();
        fs::write(tmp.path().join("pkg").join(".gitignore"), "hidden.txt\n").unwrap();
        fs::write(tmp.path().join("pkg").join("hidden.txt"), "").unwrap();
        fs::write(tmp.path().join("pkg").join("visible.rs"), "").unwrap();

        let result = get_repo_map(tmp.path(), 2, &PathFilterConfig::default());
        let lines = result.tree.lines().collect::<Vec<_>>();

        assert!(lines.contains(&"pkg/"));
        assert!(lines.contains(&"pkg/visible.rs"));
        assert!(!lines.contains(&"pkg/hidden.txt"));
    }

    #[cfg(unix)]
    #[test]
    fn get_repo_map_applies_directory_ignore_rules_to_symlinks() {
        use std::os::unix::fs::symlink;

        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("project");
        let external = tmp.path().join("external");
        fs::create_dir(&project).unwrap();
        fs::create_dir(&external).unwrap();
        fs::write(external.join("secret.txt"), "secret").unwrap();
        fs::write(project.join(".gitignore"), "linked/\n").unwrap();
        symlink(&external, project.join("linked")).unwrap();

        let result = get_repo_map(&project, 2, &PathFilterConfig::default());

        assert!(!result.tree.contains("linked"));
        assert!(!result.tree.contains("secret.txt"));
    }
}
