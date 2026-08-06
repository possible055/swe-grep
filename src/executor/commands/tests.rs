use crate::executor::{InstantContextToolCall, ToolExecutionStatus, ToolExecutor};
use crate::path_filter::PathFilterConfig;
use crate::rg::resolve_rg_path;
use serde_json::json;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn rg_available() -> bool {
    let Ok(path) = resolve_rg_path() else {
        return false;
    };
    Command::new(path).arg("--version").output().is_ok()
}

#[test]
fn exec_command_readfile_supports_full_file_and_ranges() {
    let tmp = TempDir::new().unwrap();
    let executor = ToolExecutor::new(tmp.path());
    fs::write(
        tmp.path().join("test.txt"),
        "line1\nline2\nline3\nline4\nline5",
    )
    .unwrap();

    let res = executor.exec_command(&json!({
        "type": "readfile",
        "file": "/codebase/test.txt"
    }));
    assert!(res.contains("1:line1\n2:line2\n3:line3\n4:line4\n5:line5"));

    let res = executor.exec_command(&json!({
        "type": "readfile",
        "file": "/codebase/test.txt",
        "start_line": 2,
        "end_line": 4
    }));
    assert!(res.contains("2:line2\n3:line3\n4:line4"));
    assert!(!res.contains("1:line1"));

    assert!(
        executor
            .exec_command(&json!({"type": "readfile", "file": "/codebase/nonexistent.txt"}))
            .contains("Error: file not found")
    );
}

#[test]
fn exec_command_tree_respects_depth_and_dotfile_filter() {
    let tmp = TempDir::new().unwrap();
    let executor = ToolExecutor::new(tmp.path());
    fs::create_dir(tmp.path().join("dir1")).unwrap();
    fs::write(tmp.path().join("dir1").join("file1.py"), "").unwrap();
    fs::create_dir_all(tmp.path().join("dir2").join("sub")).unwrap();
    fs::write(tmp.path().join("dir2").join("sub").join("file2.py"), "").unwrap();
    fs::write(tmp.path().join("file3.txt"), "").unwrap();
    fs::create_dir(tmp.path().join(".cache")).unwrap();

    let res = executor.exec_command(&json!({
        "type": "tree",
        "path": "/codebase",
        "levels": 2
    }));
    assert!(res.contains("dir1"));
    assert!(res.contains("file1.py"));
    assert!(res.contains("dir2"));
    assert!(res.contains("sub"));
    assert!(res.contains("file3.txt"));
    assert!(!res.contains("file2.py"));
    assert!(!res.contains(".cache"));
}

#[test]
fn exec_command_ls_supports_short_and_long_formats() {
    let tmp = TempDir::new().unwrap();
    let executor = ToolExecutor::new(tmp.path());
    fs::write(tmp.path().join("file1.txt"), "").unwrap();
    fs::create_dir(tmp.path().join("dir1")).unwrap();

    let res = executor.exec_command(&json!({"type": "ls", "path": "/codebase"}));
    assert!(res.contains("dir1\nfile1.txt"));

    let res = executor.exec_command(&json!({
        "type": "ls",
        "path": "/codebase",
        "long_format": true
    }));
    assert!(res.contains("total 2"));
    assert!(res.contains("dir1"));
    assert!(res.contains("file1.txt"));
}

#[test]
fn exec_command_glob_supports_recursive_patterns() {
    let tmp = TempDir::new().unwrap();
    let executor = ToolExecutor::new(tmp.path());
    fs::create_dir(tmp.path().join("dir1")).unwrap();
    fs::write(tmp.path().join("dir1").join("test1.py"), "").unwrap();
    fs::create_dir(tmp.path().join("dir2")).unwrap();
    fs::write(tmp.path().join("dir2").join("test2.py"), "").unwrap();
    fs::write(tmp.path().join("other.txt"), "").unwrap();

    let res = executor.exec_command(&json!({
        "type": "glob",
        "pattern": "**/test*.py",
        "path": "/codebase",
        "type_filter": "all"
    }));
    assert!(res.contains("/codebase/dir1/test1.py"));
    assert!(res.contains("/codebase/dir2/test2.py"));
    assert!(!res.contains("other.txt"));
}

#[test]
fn path_filter_applies_to_all_local_tools() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join(".gitignore"), "ignored/\n*.log\n").unwrap();
    fs::create_dir(tmp.path().join("ignored")).unwrap();
    fs::write(tmp.path().join("ignored").join("keep.txt"), "needle").unwrap();
    fs::write(tmp.path().join("ignored").join("skip.txt"), "needle").unwrap();
    fs::create_dir(tmp.path().join("logs")).unwrap();
    fs::write(tmp.path().join("logs").join("keep.log"), "needle").unwrap();
    fs::write(tmp.path().join("logs").join("drop.log"), "needle").unwrap();
    fs::create_dir(tmp.path().join(".cache")).unwrap();
    fs::write(tmp.path().join(".cache").join("visible.txt"), "needle").unwrap();
    fs::create_dir(tmp.path().join(".secret")).unwrap();
    fs::write(tmp.path().join(".secret").join("hidden.txt"), "needle").unwrap();
    fs::create_dir(tmp.path().join("pkg")).unwrap();
    fs::write(
        tmp.path().join("pkg").join(".gitignore"),
        "hidden.txt\n*.tmp\n",
    )
    .unwrap();
    fs::write(tmp.path().join("pkg").join("hidden.txt"), "needle").unwrap();
    fs::write(tmp.path().join("pkg").join("drop.tmp"), "needle").unwrap();
    fs::write(tmp.path().join("pkg").join("visible.rs"), "needle").unwrap();
    fs::write(tmp.path().join("visible.txt"), "needle").unwrap();

    let config = PathFilterConfig {
        include_patterns: vec![
            "ignored/keep.txt".to_string(),
            "logs/keep.log".to_string(),
            ".cache/visible.txt".to_string(),
        ],
        exclude_patterns: vec!["logs/drop.log".to_string()],
        ..PathFilterConfig::default()
    };
    let executor = ToolExecutor::with_limits_and_filter(tmp.path(), None, None, config);

    let tree = executor.exec_command(&json!({
        "type": "tree",
        "path": "/codebase",
        "levels": 3
    }));
    assert!(tree.contains("visible.txt"));
    assert!(tree.contains("keep.txt"));
    assert!(tree.contains("keep.log"));
    assert!(tree.contains("visible.rs"));
    assert!(!tree.contains("skip.txt"));
    assert!(!tree.contains("drop.log"));
    assert!(!tree.contains("hidden.txt"));
    assert!(!tree.contains("drop.tmp"));

    let glob = executor.exec_command(&json!({
        "type": "glob",
        "pattern": "**/*",
        "path": "/codebase",
        "type_filter": "file"
    }));
    assert!(glob.contains("/codebase/visible.txt"));
    assert!(glob.contains("/codebase/ignored/keep.txt"));
    assert!(glob.contains("/codebase/logs/keep.log"));
    assert!(glob.contains("/codebase/.cache/visible.txt"));
    assert!(glob.contains("/codebase/pkg/visible.rs"));
    assert!(!glob.contains("/codebase/ignored/skip.txt"));
    assert!(!glob.contains("/codebase/logs/drop.log"));
    assert!(!glob.contains("/codebase/pkg/hidden.txt"));
    assert!(!glob.contains("/codebase/pkg/drop.tmp"));

    let included_readfile = executor.exec_command(&json!({
        "type": "readfile",
        "file": "/codebase/ignored/keep.txt"
    }));
    assert!(included_readfile.contains("1:needle"));

    assert_eq!(
        executor.exec_command(&json!({
            "type": "readfile",
            "file": "/codebase/ignored/skip.txt"
        })),
        "Error: file not visible: /codebase/ignored/skip.txt"
    );
    assert_eq!(
        executor.exec_command(&json!({
            "type": "readfile",
            "file": "/codebase/logs/drop.log"
        })),
        "Error: file not visible: /codebase/logs/drop.log"
    );
    assert_eq!(
        executor.exec_command(&json!({
            "type": "readfile",
            "file": "/codebase/.secret/hidden.txt"
        })),
        "Error: file not visible: /codebase/.secret/hidden.txt"
    );
    assert_eq!(
        executor.exec_command(&json!({
            "type": "readfile",
            "file": "/codebase/pkg/hidden.txt"
        })),
        "Error: file not visible: /codebase/pkg/hidden.txt"
    );

    let logs = executor.exec_command(&json!({
        "type": "ls",
        "path": "/codebase/logs"
    }));
    assert!(logs.contains("keep.log"));
    assert!(!logs.contains("drop.log"));

    let pkg = executor.exec_command(&json!({
        "type": "ls",
        "path": "/codebase/pkg",
        "all": true
    }));
    assert!(pkg.contains("visible.rs"));
    assert!(!pkg.contains("hidden.txt"));
    assert!(!pkg.contains("drop.tmp"));

    let root = executor.exec_command(&json!({
        "type": "ls",
        "path": "/codebase"
    }));
    assert!(!root.contains(".cache"));
    assert!(!root.contains(".secret"));

    let root_all = executor.exec_command(&json!({
        "type": "ls",
        "path": "/codebase",
        "all": true
    }));
    assert!(root_all.contains(".cache"));
    assert!(!root_all.contains(".secret"));

    if rg_available() {
        let result = executor.exec_command(&json!({
            "type": "rg",
            "pattern": "needle",
            "path": "/codebase"
        }));
        assert!(result.contains("/codebase/visible.txt"));
        assert!(result.contains("/codebase/ignored/keep.txt"));
        assert!(result.contains("/codebase/logs/keep.log"));
        assert!(result.contains("/codebase/.cache/visible.txt"));
        assert!(result.contains("/codebase/pkg/visible.rs"));
        assert!(!result.contains("/codebase/ignored/skip.txt"));
        assert!(!result.contains("/codebase/logs/drop.log"));
        assert!(!result.contains("/codebase/pkg/hidden.txt"));
        assert!(!result.contains("/codebase/pkg/drop.tmp"));
    }
}

#[test]
fn exec_command_rg_includes_filename_for_single_file_results() {
    if !rg_available() {
        return;
    }

    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("only.txt"), "needle").unwrap();
    let executor = ToolExecutor::new(tmp.path());

    let result = executor.exec_command(&json!({
        "type": "rg",
        "pattern": "needle",
        "path": "/codebase"
    }));

    assert!(result.contains("/codebase/only.txt:1:needle"));
}

#[test]
fn disabled_path_filter_uses_native_rg_traversal() {
    if !rg_available() {
        return;
    }

    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("visible.txt"), "needle").unwrap();
    let config = PathFilterConfig {
        exclude_patterns: vec!["visible.txt".to_string()],
        ..PathFilterConfig::default()
    };
    let disabled_config = PathFilterConfig {
        enabled: false,
        exclude_patterns: vec!["visible.txt".to_string()],
        ..PathFilterConfig::default()
    };

    let filtered = ToolExecutor::with_limits_and_filter(tmp.path(), None, None, config);
    let native = ToolExecutor::with_limits_and_filter(tmp.path(), None, None, disabled_config);

    assert_eq!(
        filtered.exec_command(&json!({"type": "rg", "pattern": "needle", "path": "/codebase"})),
        "(no matches)"
    );
    assert!(
        native
            .exec_command(&json!({"type": "rg", "pattern": "needle", "path": "/codebase"}))
            .contains("/codebase/visible.txt:1:needle")
    );
}

#[test]
fn disabled_path_filter_allows_readfile_and_ls_entries() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("visible.txt"), "needle").unwrap();
    let config = PathFilterConfig {
        enabled: false,
        exclude_patterns: vec!["visible.txt".to_string()],
        ..PathFilterConfig::default()
    };
    let executor = ToolExecutor::with_limits_and_filter(tmp.path(), None, None, config);

    assert!(
        executor
            .exec_command(&json!({"type": "readfile", "file": "/codebase/visible.txt"}))
            .contains("1:needle")
    );
    assert!(
        executor
            .exec_command(&json!({"type": "ls", "path": "/codebase"}))
            .contains("visible.txt")
    );
}

#[test]
fn exec_command_supports_all_subcommands() {
    let tmp = TempDir::new().unwrap();
    let executor = ToolExecutor::new(tmp.path());
    fs::create_dir(tmp.path().join("src")).unwrap();
    fs::write(tmp.path().join("test.txt"), "hello world").unwrap();
    fs::write(tmp.path().join("src").join("main.rs"), "fn main() {}").unwrap();

    assert!(
        executor
            .exec_command(&json!({"type": "readfile", "file": "/codebase/test.txt"}))
            .contains("1:hello world")
    );
    if rg_available() {
        assert!(
            executor
                .exec_command(&json!({"type": "rg", "pattern": "hello", "path": "/codebase"}))
                .contains("/codebase/test.txt:1:hello world")
        );
    }
    assert!(
        executor
            .exec_command(&json!({"type": "tree", "path": "/codebase", "levels": 1}))
            .contains("src")
    );
    assert!(
        executor
            .exec_command(&json!({"type": "ls", "path": "/codebase"}))
            .contains("test.txt")
    );
    assert!(
            executor
                .exec_command(&json!({"type": "glob", "pattern": "**/*.rs", "path": "/codebase", "type_filter": "file"}))
                .contains("src/main.rs")
        );
}

#[test]
fn exec_restricted_exec_step_returns_pending_and_terminal_updates() {
    let tmp = TempDir::new().unwrap();
    let executor = ToolExecutor::new(tmp.path());
    fs::write(tmp.path().join("test.txt"), "hello world").unwrap();

    let calls = vec![
        InstantContextToolCall {
            id: "call-1".to_string(),
            name: "command1".to_string(),
            args: json!({"type": "readfile", "file": "/codebase/test.txt"}),
        },
        InstantContextToolCall {
            id: "call-2".to_string(),
            name: "command2".to_string(),
            args: json!({"type": "unknown"}),
        },
    ];

    let updates = executor.exec_restricted_exec_step("step-1", &calls, 1_000);

    assert_eq!(updates.len(), 4);
    assert_eq!(updates[0].step_id, "step-1");
    assert_eq!(updates[0].tool_call_id, "call-1");
    assert_eq!(updates[0].status, ToolExecutionStatus::Pending);
    assert_eq!(updates[1].tool_call_id, "call-2");
    assert_eq!(updates[1].status, ToolExecutionStatus::Pending);
    assert_eq!(updates[2].tool_call_id, "call-1");
    assert_eq!(updates[2].status, ToolExecutionStatus::Completed);
    assert!(updates[2].output.contains("1:hello world"));
    assert_eq!(updates[3].tool_call_id, "call-2");
    assert_eq!(updates[3].status, ToolExecutionStatus::Error);
}

#[test]
fn exec_restricted_exec_step_marks_timed_out_when_budget_is_exceeded() {
    let tmp = TempDir::new().unwrap();
    let executor = ToolExecutor::new(tmp.path());
    fs::write(tmp.path().join("test.txt"), "hello world").unwrap();

    let calls = vec![InstantContextToolCall {
        id: "call-1".to_string(),
        name: "command1".to_string(),
        args: json!({"type": "readfile", "file": "/codebase/test.txt"}),
    }];

    let updates = executor.exec_restricted_exec_step("step-1", &calls, 0);

    assert_eq!(updates[1].status, ToolExecutionStatus::TimedOut);
}
