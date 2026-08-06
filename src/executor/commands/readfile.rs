use super::super::{ToolExecutor, TruncationProfile};
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::time::Instant;

const BINARY_PROBE_BYTES: usize = 8 * 1024;

impl ToolExecutor {
    pub(super) fn execute_readfile(
        &self,
        file: &str,
        start_line: Option<usize>,
        end_line: Option<usize>,
        deadline: Option<Instant>,
    ) -> String {
        if file.is_empty() {
            return "Error: missing or invalid file path".to_string();
        }
        let real_path = self.real(file);
        if self.path_contains_symlink(&real_path) {
            return format!("Error: symlink paths are not followed: {file}");
        }
        let metadata = match real_path.symlink_metadata() {
            Ok(metadata) => metadata,
            Err(_) => return format!("Error: file not found: {file}"),
        };
        if !metadata.is_file() {
            return format!("Error: file not found: {file}");
        }
        if !self.is_visible_path(&real_path, false) {
            return format!("Error: file not visible: {file}");
        }

        let input = match File::open(&real_path) {
            Ok(input) => input,
            Err(error) => return format!("Error: {error}"),
        };
        let mut reader = BufReader::with_capacity(BINARY_PROBE_BYTES, input);
        let is_empty = match reader.fill_buf() {
            Ok(prefix)
                if prefix
                    .iter()
                    .take(BINARY_PROBE_BYTES)
                    .any(|byte| *byte == 0) =>
            {
                return format!("Error: binary file: {file}");
            }
            Ok(prefix) => prefix.is_empty(),
            Err(error) => return format!("Error: {error}"),
        };

        let start = start_line.unwrap_or(1).max(1);
        let end = end_line.unwrap_or(usize::MAX);
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return "Error: tool timed out".to_string();
        }
        if start > end {
            return if is_empty {
                "(empty file)".to_string()
            } else {
                String::new()
            };
        }
        let limits = self.output_limits(TruncationProfile::ReadfileExpanded);
        let max_prefix_bytes = limits.line_max_chars.saturating_mul(4).saturating_add(32);
        let mut output = self.bounded_text(TruncationProfile::ReadfileExpanded);
        let mut line_number = 0_usize;
        let mut saw_line = false;

        loop {
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return "Error: tool timed out".to_string();
            }
            let line = match read_line_prefix(&mut reader, max_prefix_bytes) {
                Ok(Some(line)) => line,
                Ok(None) => break,
                Err(error) => return format!("Error: {error}"),
            };
            saw_line = true;
            line_number += 1;
            if line_number < start {
                continue;
            }
            if line_number > end {
                break;
            }
            let text = String::from_utf8_lossy(&line);
            if !output.push_line(&format!("{line_number}:{text}")) {
                break;
            }
        }

        if !saw_line {
            "(empty file)".to_string()
        } else {
            output.finish()
        }
    }
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
