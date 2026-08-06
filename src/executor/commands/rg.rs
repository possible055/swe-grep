use super::super::{BoundedText, ToolExecutor, TruncationProfile};
use crate::rg::{resolve_rg_path, ripgrep_not_found_message};
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::ffi::{OsStr, OsString};
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::Instant;
use walkdir::WalkDir;

#[cfg(unix)]
const RG_ARGUMENT_BUDGET: usize = 64 * 1024;
#[cfg(windows)]
const RG_ARGUMENT_BUDGET: usize = 30_000;
#[cfg(not(any(unix, windows)))]
const RG_ARGUMENT_BUDGET: usize = 64 * 1024;
const PIPE_CHANNEL_CAPACITY: usize = 32;

struct RgPathMatchers {
    include: Option<GlobSet>,
    exclude: Option<GlobSet>,
}

#[derive(Clone, Copy)]
enum StreamKind {
    Stdout,
    Stderr,
}

enum StreamEvent {
    Line(StreamKind, Vec<u8>),
    Done(StreamKind),
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunOutcome {
    Completed,
    OutputLimitReached,
}

impl ToolExecutor {
    pub(super) fn execute_rg(
        &self,
        pattern: &str,
        path: &str,
        include: Option<&[String]>,
        exclude: Option<&[String]>,
        deadline: Option<Instant>,
    ) -> String {
        if pattern.is_empty() {
            return "Error: missing or invalid pattern".to_string();
        }
        if path.is_empty() {
            return "Error: missing or invalid path".to_string();
        }
        if deadline_expired(deadline) {
            return "Error: tool timed out".to_string();
        }

        if let Ok(mut patterns) = self.collected_rg_patterns.lock() {
            patterns.push(pattern.to_string());
        }

        let real_path = self.real(path);
        if !real_path.exists() {
            return format!("Error: path does not exist: {path}");
        }
        if self.path_contains_symlink(&real_path) {
            return format!("Error: symlink paths are not followed: {path}");
        }
        let matchers = match RgPathMatchers::new(include, exclude) {
            Ok(matchers) => matchers,
            Err(error) => return error,
        };
        let rg_path = match resolve_rg_path() {
            Ok(path) => path,
            Err(error) => return error.to_string(),
        };
        let mut stdout = self.bounded_text(TruncationProfile::General);

        let result = if self.path_filter_enabled() {
            self.rg_filtered(
                &rg_path,
                pattern,
                &real_path,
                &matchers,
                deadline,
                &mut stdout,
            )
        } else {
            self.rg_unfiltered(
                &rg_path,
                pattern,
                &real_path,
                include,
                exclude,
                deadline,
                &mut stdout,
            )
        };
        if let Err(error) = result {
            return error;
        }
        if stdout.is_empty() {
            "(no matches)".to_string()
        } else {
            self.remap(&stdout.finish())
        }
    }

    fn rg_filtered(
        &self,
        rg_path: &Path,
        pattern: &str,
        real_path: &Path,
        matchers: &RgPathMatchers,
        deadline: Option<Instant>,
        stdout: &mut BoundedText,
    ) -> Result<(), String> {
        let base_args = filtered_base_args(pattern);
        let base_units = command_units(&base_args);
        let mut chunk = Vec::new();
        let mut chunk_units = base_units;

        if real_path.is_file() {
            if self.is_visible_path(real_path, false) && matchers.is_match(real_path, &self.root) {
                let argument = relative_argument(real_path, &self.root);
                ensure_single_argument_fits(base_units, &argument)?;
                chunk.push(argument);
            }
        } else {
            let walker = WalkDir::new(real_path)
                .follow_links(false)
                .into_iter()
                .filter_entry(|entry| {
                    entry.depth() == 0
                        || self.is_visible_path(entry.path(), entry.file_type().is_dir())
                });
            for result in walker {
                if deadline_expired(deadline) {
                    return Err("Error: tool timed out".to_string());
                }
                let entry = result.map_err(|error| {
                    format!("Error: failed to traverse {}: {error}", real_path.display())
                })?;
                if !entry.file_type().is_file() || !matchers.is_match(entry.path(), &self.root) {
                    continue;
                }
                let argument = relative_argument(entry.path(), &self.root);
                let units = argument_units(&argument);
                ensure_single_argument_fits(base_units, &argument)?;
                if !chunk.is_empty() && chunk_units.saturating_add(units) > RG_ARGUMENT_BUDGET {
                    if self.run_rg_chunk(rg_path, &base_args, &chunk, deadline, stdout)?
                        == RunOutcome::OutputLimitReached
                    {
                        return Ok(());
                    }
                    chunk.clear();
                    chunk_units = base_units;
                }
                chunk_units = chunk_units.saturating_add(units);
                chunk.push(argument);
            }
        }

        if !chunk.is_empty() {
            self.run_rg_chunk(rg_path, &base_args, &chunk, deadline, stdout)?;
        }
        Ok(())
    }

    // Keep process inputs explicit; this path is the only unfiltered subprocess boundary.
    #[allow(clippy::too_many_arguments)]
    fn rg_unfiltered(
        &self,
        rg_path: &Path,
        pattern: &str,
        real_path: &Path,
        include: Option<&[String]>,
        exclude: Option<&[String]>,
        deadline: Option<Instant>,
        stdout: &mut BoundedText,
    ) -> Result<(), String> {
        let mut args = common_args();
        args.push("--hidden".into());
        args.push("--no-ignore".into());
        if let Some(include) = include {
            for pattern in include {
                args.push("--glob".into());
                args.push(pattern.into());
            }
        }
        if let Some(exclude) = exclude {
            for pattern in exclude {
                args.push("--glob".into());
                args.push(format!("!{pattern}").into());
            }
        }
        args.push("--".into());
        args.push(pattern.into());
        let path_arg = relative_argument(real_path, &self.root);
        ensure_single_argument_fits(command_units(&args), &path_arg)?;
        self.run_rg_chunk(rg_path, &args, &[path_arg], deadline, stdout)?;
        Ok(())
    }

    fn run_rg_chunk(
        &self,
        rg_path: &Path,
        base_args: &[OsString],
        paths: &[OsString],
        deadline: Option<Instant>,
        stdout: &mut BoundedText,
    ) -> Result<RunOutcome, String> {
        if deadline_expired(deadline) {
            return Err("Error: tool timed out".to_string());
        }
        let mut command = Command::new(rg_path);
        command
            .current_dir(&self.root)
            .args(base_args)
            .args(paths)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(ripgrep_not_found_message().to_string());
            }
            Err(error) => return Err(format!("Error: {error}")),
        };

        let child_stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Error: failed to capture rg stdout".to_string());
            }
        };
        let child_stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Error: failed to capture rg stderr".to_string());
            }
        };
        let limits = self.output_limits(TruncationProfile::General);
        let max_line_bytes = limits.line_max_chars.saturating_mul(4).saturating_add(64);
        let (sender, receiver) = mpsc::sync_channel(PIPE_CHANNEL_CAPACITY);
        let stdout_thread = spawn_reader(
            child_stdout,
            StreamKind::Stdout,
            sender.clone(),
            max_line_bytes,
        );
        let stderr_thread = spawn_reader(child_stderr, StreamKind::Stderr, sender, max_line_bytes);
        let mut stderr = self.bounded_text(TruncationProfile::General);

        let mut stdout_done = false;
        let mut stderr_done = false;
        while !stdout_done || !stderr_done {
            let event = match receive_event(&receiver, deadline) {
                Ok(event) => event,
                Err(ReceiveError::TimedOut) => {
                    stop_child(&mut child, receiver, stdout_thread, stderr_thread);
                    return Err("Error: tool timed out".to_string());
                }
                Err(ReceiveError::Disconnected) => {
                    stop_child(&mut child, receiver, stdout_thread, stderr_thread);
                    return Err("Error: rg output reader disconnected".to_string());
                }
            };
            match event {
                StreamEvent::Line(StreamKind::Stdout, line) => {
                    let line = String::from_utf8_lossy(&line);
                    if !stdout.push_line(&virtualize_rg_line(&line)) {
                        stdout.mark_truncated();
                        stop_child(&mut child, receiver, stdout_thread, stderr_thread);
                        return Ok(RunOutcome::OutputLimitReached);
                    }
                }
                StreamEvent::Line(StreamKind::Stderr, line) => {
                    let line = String::from_utf8_lossy(&line);
                    stderr.push_line(&line);
                }
                StreamEvent::Done(StreamKind::Stdout) => stdout_done = true,
                StreamEvent::Done(StreamKind::Stderr) => stderr_done = true,
                StreamEvent::Error(error) => {
                    stop_child(&mut child, receiver, stdout_thread, stderr_thread);
                    return Err(format!("Error: failed to read rg output: {error}"));
                }
            }
        }

        let status = child
            .wait()
            .map_err(|error| format!("Error: failed to wait for rg: {error}"))?;
        join_reader(stdout_thread);
        join_reader(stderr_thread);
        interpret_status(status, stderr)
    }
}

impl RgPathMatchers {
    fn new(include: Option<&[String]>, exclude: Option<&[String]>) -> Result<Self, String> {
        Ok(Self {
            include: compile_patterns(include, "include")?,
            exclude: compile_patterns(exclude, "exclude")?,
        })
    }

    fn is_match(&self, path: &Path, root: &Path) -> bool {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let name = path.file_name().and_then(OsStr::to_str).unwrap_or_default();
        if self
            .include
            .as_ref()
            .is_some_and(|include| !include.is_match(&rel) && !include.is_match(name))
        {
            return false;
        }
        !self
            .exclude
            .as_ref()
            .is_some_and(|exclude| exclude.is_match(&rel) || exclude.is_match(name))
    }
}

fn compile_patterns(patterns: Option<&[String]>, label: &str) -> Result<Option<GlobSet>, String> {
    let Some(patterns) = patterns.filter(|patterns| !patterns.is_empty()) else {
        return Ok(None);
    };
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        Glob::new(pattern)
            .map_err(|error| format!("Error: invalid {label} glob '{pattern}': {error}"))?;
        let normalized = pattern.replace('\\', "/");
        let glob = Glob::new(&normalized)
            .map_err(|error| format!("Error: invalid {label} glob '{pattern}': {error}"))?;
        builder.add(glob);
    }
    builder
        .build()
        .map(Some)
        .map_err(|error| format!("Error: invalid {label} glob: {error}"))
}

fn common_args() -> Vec<OsString> {
    [
        "--no-config",
        "--no-heading",
        "--with-filename",
        "-n",
        "--max-count",
        "50",
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

fn filtered_base_args(pattern: &str) -> Vec<OsString> {
    let mut args = common_args();
    args.push("--no-ignore".into());
    args.push("--hidden".into());
    args.push("--".into());
    args.push(pattern.into());
    args
}

fn relative_argument(path: &Path, root: &Path) -> OsString {
    let relative = path.strip_prefix(root).unwrap_or(path);
    if relative.as_os_str().is_empty() {
        OsString::from(".")
    } else {
        relative.as_os_str().to_os_string()
    }
}

fn ensure_single_argument_fits(base_units: usize, argument: &OsStr) -> Result<(), String> {
    if base_units.saturating_add(argument_units(argument)) > RG_ARGUMENT_BUDGET {
        return Err(format!(
            "Error: path exceeds the rg command-line budget: {}",
            PathBuf::from(argument).display()
        ));
    }
    Ok(())
}

fn command_units(arguments: &[OsString]) -> usize {
    arguments
        .iter()
        .map(|argument| argument_units(argument))
        .sum()
}

#[cfg(unix)]
fn argument_units(argument: &OsStr) -> usize {
    use std::os::unix::ffi::OsStrExt;
    argument.as_bytes().len().saturating_add(1)
}

#[cfg(windows)]
fn argument_units(argument: &OsStr) -> usize {
    use std::os::windows::ffi::OsStrExt;
    argument.encode_wide().count().saturating_add(3)
}

#[cfg(not(any(unix, windows)))]
fn argument_units(argument: &OsStr) -> usize {
    argument.to_string_lossy().len().saturating_add(1)
}

fn spawn_reader(
    reader: impl Read + Send + 'static,
    kind: StreamKind,
    sender: SyncSender<StreamEvent>,
    max_line_bytes: usize,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        loop {
            match read_line_prefix(&mut reader, max_line_bytes) {
                Ok(Some(line)) => {
                    if sender.send(StreamEvent::Line(kind, line)).is_err() {
                        return;
                    }
                }
                Ok(None) => {
                    let _ = sender.send(StreamEvent::Done(kind));
                    return;
                }
                Err(error) => {
                    let _ = sender.send(StreamEvent::Error(error.to_string()));
                    return;
                }
            }
        }
    })
}

fn read_line_prefix(
    reader: &mut impl BufRead,
    max_prefix_bytes: usize,
) -> io::Result<Option<Vec<u8>>> {
    let mut prefix = Vec::new();
    let mut saw_bytes = false;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            if !saw_bytes {
                return Ok(None);
            }
            break;
        }
        saw_bytes = true;
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(buffer.len(), |index| index + 1);
        let content_end = newline.unwrap_or(buffer.len());
        if prefix.len() < max_prefix_bytes {
            let remaining = max_prefix_bytes - prefix.len();
            prefix.extend_from_slice(&buffer[..content_end.min(remaining)]);
        }
        reader.consume(consumed);
        if newline.is_some() {
            break;
        }
    }
    if prefix.last() == Some(&b'\r') {
        prefix.pop();
    }
    Ok(Some(prefix))
}

enum ReceiveError {
    TimedOut,
    Disconnected,
}

fn receive_event(
    receiver: &Receiver<StreamEvent>,
    deadline: Option<Instant>,
) -> Result<StreamEvent, ReceiveError> {
    match deadline {
        Some(deadline) => {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(ReceiveError::TimedOut);
            }
            receiver
                .recv_timeout(remaining)
                .map_err(|error| match error {
                    RecvTimeoutError::Timeout => ReceiveError::TimedOut,
                    RecvTimeoutError::Disconnected => ReceiveError::Disconnected,
                })
        }
        None => receiver.recv().map_err(|_| ReceiveError::Disconnected),
    }
}

fn interpret_status(status: ExitStatus, stderr: BoundedText) -> Result<RunOutcome, String> {
    match status.code() {
        Some(0 | 1) => Ok(RunOutcome::Completed),
        Some(code) => {
            let message = stderr.finish();
            if message.is_empty() {
                Err(format!("Error: rg exited with status {code}"))
            } else {
                Err(format!("Error: {message}"))
            }
        }
        None => Err("Error: rg terminated without an exit status".to_string()),
    }
}

fn stop_child(
    child: &mut Child,
    receiver: Receiver<StreamEvent>,
    stdout_thread: JoinHandle<()>,
    stderr_thread: JoinHandle<()>,
) {
    let _ = child.kill();
    let _ = child.wait();
    drop(receiver);
    join_reader(stdout_thread);
    join_reader(stderr_thread);
}

fn join_reader(handle: JoinHandle<()>) {
    let _ = handle.join();
}

fn deadline_expired(deadline: Option<Instant>) -> bool {
    deadline.is_some_and(|deadline| Instant::now() >= deadline)
}

fn virtualize_rg_line(line: &str) -> String {
    let Some((path, remainder)) = line.split_once(':') else {
        return line.to_string();
    };
    let path = path
        .strip_prefix("./")
        .or_else(|| path.strip_prefix(".\\"))
        .unwrap_or(path)
        .replace('\\', "/");
    format!("/codebase/{path}:{remainder}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn argument_budget_counts_all_arguments() {
        let args = vec![OsString::from("--glob"), OsString::from("*.rs")];
        assert!(command_units(&args) >= argument_units(OsStr::new("--glob")));
    }

    #[test]
    fn invalid_path_glob_is_rejected() {
        let patterns = vec!["bad\\".to_string()];
        let error = RgPathMatchers::new(Some(&patterns), None)
            .err()
            .expect("invalid glob should fail");
        assert!(error.starts_with("Error: invalid include glob"));
    }

    #[test]
    fn relative_rg_paths_are_mapped_to_the_virtual_root() {
        assert_eq!(
            virtualize_rg_line("./src/lib.rs:4:match"),
            "/codebase/src/lib.rs:4:match"
        );
        assert_eq!(
            virtualize_rg_line(".\\src\\lib.rs:4:match"),
            "/codebase/src/lib.rs:4:match"
        );
    }

    #[cfg(unix)]
    #[test]
    fn child_is_killed_and_reaped_at_the_deadline() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let fake_rg = tmp.path().join("fake-rg");
        fs::write(&fake_rg, "#!/bin/sh\nexec sleep 5\n").unwrap();
        let mut permissions = fs::metadata(&fake_rg).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake_rg, permissions).unwrap();
        let executor = ToolExecutor::new(tmp.path());
        let mut output = executor.bounded_text(TruncationProfile::General);
        let started = Instant::now();

        let result = executor.run_rg_chunk(
            &fake_rg,
            &[],
            &[],
            Some(started + Duration::from_millis(50)),
            &mut output,
        );

        assert_eq!(result, Err("Error: tool timed out".to_string()));
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
