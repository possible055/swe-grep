use crate::executor::ripgrep::resolve_rg_path;
use crate::executor::{InstantContextToolCall, ToolExecutionStatus, ToolExecutor};
use crate::path_filter::PathFilterConfig;
use serde_json::json;
use std::fs;
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};
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

    let reversed = executor.exec_command(&json!({
        "type": "readfile",
        "file": "/codebase/test.txt",
        "start_line": 4,
        "end_line": 2
    }));
    assert!(reversed.is_empty());

    assert!(
        executor
            .exec_command(&json!({"type": "readfile", "file": "/codebase/nonexistent.txt"}))
            .contains("Error: file not found")
    );
}

#[test]
fn readfile_handles_empty_terminal_newline_crlf_and_binary() {
    let tmp = TempDir::new().unwrap();
    let executor = ToolExecutor::new(tmp.path());
    fs::write(tmp.path().join("empty.txt"), "").unwrap();
    fs::write(tmp.path().join("terminal.txt"), "one\n").unwrap();
    fs::write(tmp.path().join("crlf.txt"), "one\r\ntwo\r\n").unwrap();
    fs::write(tmp.path().join("binary.bin"), b"text\0binary").unwrap();

    assert_eq!(
        executor.exec_command(&json!({"type": "readfile", "file": "/codebase/empty.txt"})),
        "(empty file)"
    );
    assert_eq!(
        executor.exec_command(&json!({"type": "readfile", "file": "/codebase/terminal.txt"})),
        "1:one"
    );
    assert_eq!(
        executor.exec_command(&json!({"type": "readfile", "file": "/codebase/crlf.txt"})),
        "1:one\n2:two"
    );
    assert_eq!(
        executor.exec_command(&json!({"type": "readfile", "file": "/codebase/binary.bin"})),
        "Error: binary file: /codebase/binary.bin"
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
    assert!(res.contains("directory\t-\tfalse\tdir1"));
    assert!(res.contains("file\t0\tfalse\tfile1.txt"));
}

#[test]
fn directory_outputs_use_stable_directories_first_order() {
    let tmp = TempDir::new().unwrap();
    let executor = ToolExecutor::new(tmp.path());
    fs::create_dir(tmp.path().join("zulu-dir")).unwrap();
    fs::create_dir(tmp.path().join("Alpha-dir")).unwrap();
    fs::write(tmp.path().join("zulu.txt"), "").unwrap();
    fs::write(tmp.path().join("Alpha.txt"), "").unwrap();

    let output = executor.exec_command(&json!({"type": "ls", "path": "/codebase"}));

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec!["Alpha-dir", "zulu-dir", "Alpha.txt", "zulu.txt"]
    );
}

#[cfg(unix)]
#[test]
fn directory_order_uses_the_original_name_as_a_case_tie_breaker() {
    let tmp = TempDir::new().unwrap();
    let executor = ToolExecutor::new(tmp.path());
    fs::write(tmp.path().join("alpha.txt"), "").unwrap();
    fs::write(tmp.path().join("Alpha.txt"), "").unwrap();

    let output = executor.exec_command(&json!({"type": "ls", "path": "/codebase"}));

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec!["Alpha.txt", "alpha.txt"]
    );
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

    let res_backslash = executor.exec_command(&json!({
        "type": "glob",
        "pattern": "**\\test*.py",
        "path": "/codebase",
        "type_filter": "all"
    }));
    assert!(res_backslash.contains("/codebase/dir1/test1.py"));
    assert!(res_backslash.contains("/codebase/dir2/test2.py"));
}

#[test]
fn glob_rejects_invalid_patterns_and_marks_the_hundred_match_limit() {
    let tmp = TempDir::new().unwrap();
    for index in 0..=100 {
        fs::write(tmp.path().join(format!("file_{index:03}.txt")), "").unwrap();
    }
    let executor = ToolExecutor::with_limits(tmp.path(), Some(200), None);

    let invalid = executor.exec_command(&json!({
        "type": "glob",
        "pattern": "[",
        "path": "/codebase"
    }));
    assert!(invalid.starts_with("Error: invalid glob pattern"));

    let output = executor.exec_command(&json!({
        "type": "glob",
        "pattern": "*.txt",
        "path": "/codebase"
    }));
    assert!(output.contains("/codebase/file_099.txt"));
    assert!(!output.contains("/codebase/file_100.txt"));
    assert!(output.ends_with("... (lines truncated) ..."));
}

#[cfg(unix)]
#[test]
fn local_tools_display_symlinks_without_following_them() {
    use std::os::unix::fs::symlink;

    let tmp = TempDir::new().unwrap();
    let project = tmp.path().join("project");
    let external = tmp.path().join("external");
    fs::create_dir(&project).unwrap();
    fs::create_dir(&external).unwrap();
    fs::write(external.join("secret.txt"), "secret").unwrap();
    symlink(&external, project.join("linked")).unwrap();
    let executor = ToolExecutor::new(&project);

    let tree = executor.exec_command(&json!({
        "type": "tree",
        "path": "/codebase",
        "levels": 3
    }));
    assert!(tree.contains("linked"));
    assert!(!tree.contains("secret.txt"));

    let glob = executor.exec_command(&json!({
        "type": "glob",
        "pattern": "**/*",
        "path": "/codebase",
        "type_filter": "all"
    }));
    assert!(glob.contains("/codebase/linked"));
    assert!(!glob.contains("secret.txt"));

    for command in [
        json!({"type": "tree", "path": "/codebase/linked", "levels": 2}),
        json!({"type": "ls", "path": "/codebase/linked"}),
        json!({"type": "glob", "pattern": "**/*", "path": "/codebase/linked"}),
    ] {
        assert_eq!(
            executor.exec_command(&command),
            "Error: symlink paths are not followed: /codebase/linked"
        );
    }
    assert!(
        executor
            .exec_command(&json!({"type": "readfile", "file": "/codebase/linked/secret.txt"}))
            .starts_with("Error:")
    );
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
    fs::write(tmp.path().join(".gitignore"), "ignored.txt\n").unwrap();
    fs::write(tmp.path().join("visible.txt"), "needle").unwrap();
    fs::write(tmp.path().join("ignored.txt"), "needle").unwrap();
    fs::write(tmp.path().join(".hidden.txt"), "needle").unwrap();
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
    let result =
        native.exec_command(&json!({"type": "rg", "pattern": "needle", "path": "/codebase"}));
    assert!(result.contains("/codebase/visible.txt:1:needle"));
    assert!(result.contains("/codebase/ignored.txt:1:needle"));
    assert!(result.contains("/codebase/.hidden.txt:1:needle"));

    let explicit = native.exec_command(&json!({
        "type": "rg",
        "pattern": "needle",
        "path": "/codebase",
        "exclude": ["ignored.txt", ".hidden.txt"]
    }));
    assert!(explicit.contains("/codebase/visible.txt:1:needle"));
    assert!(!explicit.contains("/codebase/ignored.txt"));
    assert!(!explicit.contains("/codebase/.hidden.txt"));
}

#[test]
fn rg_reports_invalid_regex_as_an_error() {
    if !rg_available() {
        return;
    }
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("file.txt"), "needle").unwrap();
    let executor = ToolExecutor::new(tmp.path());

    let output = executor.exec_command(&json!({
        "type": "rg",
        "pattern": "(",
        "path": "/codebase"
    }));

    assert!(output.starts_with("Error:"));
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

#[test]
fn restricted_exec_receivers_share_one_deadline() {
    let mut senders = Vec::new();
    let mut receivers = Vec::new();
    for _ in 0..3 {
        let (sender, receiver) = mpsc::channel();
        senders.push(sender);
        receivers.push(receiver);
    }
    let started = Instant::now();
    let deadline = started + Duration::from_millis(100);

    let outputs = super::receive_outputs(receivers, deadline, 100);

    assert!(started.elapsed() < Duration::from_millis(250));
    assert!(
        outputs
            .iter()
            .all(|(_, status, _, _)| *status == ToolExecutionStatus::TimedOut)
    );
    drop(senders);
}
