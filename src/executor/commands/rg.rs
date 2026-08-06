use super::super::helpers::matches_any_pattern;
use super::super::{ToolExecutor, TruncationProfile};
use crate::rg::{resolve_rg_path, ripgrep_not_found_message};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

const RG_FILE_CHUNK_SIZE: usize = 200;

impl ToolExecutor {
    pub(super) fn execute_rg(
        &self,
        pattern: &str,
        path: &str,
        include: Option<&[String]>,
        exclude: Option<&[String]>,
    ) -> String {
        if pattern.is_empty() {
            return "Error: missing or invalid pattern".to_string();
        }
        if path.is_empty() {
            return "Error: missing or invalid path".to_string();
        }

        if let Ok(mut patterns) = self.collected_rg_patterns.lock() {
            patterns.push(pattern.to_string());
        }

        let real_path = self.real(path);
        if !real_path.exists() {
            return format!("Error: path does not exist: {path}");
        }

        if !self.path_filter_enabled() {
            return self.rg_native(pattern, &real_path, include, exclude);
        }

        self.rg_filtered(pattern, &real_path, include, exclude)
    }

    fn rg_filtered(
        &self,
        pattern: &str,
        real_path: &Path,
        include: Option<&[String]>,
        exclude: Option<&[String]>,
    ) -> String {
        let files = self.rg_search_files(real_path, include, exclude);
        if files.is_empty() {
            return "(no matches)".to_string();
        }

        let mut all_stdout = String::new();
        for chunk in files.chunks(RG_FILE_CHUNK_SIZE) {
            let mut args = vec![
                "--no-config".to_string(),
                "--no-ignore".to_string(),
                "--hidden".to_string(),
                "--no-heading".to_string(),
                "--with-filename".to_string(),
                "-n".to_string(),
                "--max-count".to_string(),
                "50".to_string(),
                "--".to_string(),
                pattern.to_string(),
            ];
            args.extend(
                chunk
                    .iter()
                    .map(|file| file.to_string_lossy().into_owned())
                    .collect::<Vec<_>>(),
            );

            let rg_path = match resolve_rg_path() {
                Ok(path) => path,
                Err(error) => return error.to_string(),
            };

            match Command::new(rg_path).args(args).output() {
                Ok(output) => {
                    let code = output.status.code().unwrap_or(-1);
                    if code == 1 {
                        continue;
                    }
                    if code == 0 {
                        all_stdout.push_str(&String::from_utf8_lossy(&output.stdout));
                        continue;
                    }
                    if !output.stderr.is_empty() {
                        return self.truncate_text(
                            &self.remap(&String::from_utf8_lossy(&output.stderr)),
                            TruncationProfile::General,
                        );
                    }
                    return format!("Error: exit status {code}");
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    return ripgrep_not_found_message().to_string();
                }
                Err(err) => return format!("Error: {err}"),
            }
        }

        if all_stdout.is_empty() {
            "(no matches)".to_string()
        } else {
            self.truncate_text(&self.remap(&all_stdout), TruncationProfile::General)
        }
    }

    fn rg_native(
        &self,
        pattern: &str,
        real_path: &Path,
        include: Option<&[String]>,
        exclude: Option<&[String]>,
    ) -> String {
        let mut args = vec![
            "--no-config".to_string(),
            "--no-heading".to_string(),
            "--with-filename".to_string(),
            "-n".to_string(),
            "--max-count".to_string(),
            "50".to_string(),
        ];
        if let Some(include) = include {
            for glob in include {
                args.push("--glob".to_string());
                args.push(glob.clone());
            }
        }
        if let Some(exclude) = exclude {
            for glob in exclude {
                args.push("--glob".to_string());
                args.push(format!("!{glob}"));
            }
        }
        args.push("--".to_string());
        args.push(pattern.to_string());
        args.push(real_path.to_string_lossy().into_owned());

        let rg_path = match resolve_rg_path() {
            Ok(path) => path,
            Err(error) => return error.to_string(),
        };

        match Command::new(rg_path).args(args).output() {
            Ok(output) => {
                let code = output.status.code().unwrap_or(-1);
                if code == 1 {
                    return "(no matches)".to_string();
                }
                if code == 0 {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    return self.truncate_text(&self.remap(&stdout), TruncationProfile::General);
                }
                if !output.stderr.is_empty() {
                    return self.truncate_text(
                        &self.remap(&String::from_utf8_lossy(&output.stderr)),
                        TruncationProfile::General,
                    );
                }
                format!("Error: exit status {code}")
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                ripgrep_not_found_message().to_string()
            }
            Err(err) => format!("Error: {err}"),
        }
    }

    fn rg_search_files(
        &self,
        real_path: &Path,
        include: Option<&[String]>,
        exclude: Option<&[String]>,
    ) -> Vec<PathBuf> {
        if real_path.is_file() {
            return if self.is_visible_for_rg(real_path, include, exclude) {
                vec![real_path.to_path_buf()]
            } else {
                Vec::new()
            };
        }

        let mut files = WalkDir::new(real_path)
            .into_iter()
            .filter_entry(|entry| {
                entry.depth() == 0 || self.is_visible_path(entry.path(), entry.file_type().is_dir())
            })
            .flatten()
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| entry.path().to_path_buf())
            .filter(|path| self.is_visible_for_rg(path, include, exclude))
            .collect::<Vec<_>>();
        files.sort();
        files
    }

    fn is_visible_for_rg(
        &self,
        path: &Path,
        include: Option<&[String]>,
        exclude: Option<&[String]>,
    ) -> bool {
        if !self.is_visible_path(path, false) {
            return false;
        }

        let rel = path
            .strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let name = path.file_name().and_then(OsStr::to_str).unwrap_or_default();

        if let Some(include) = include
            && !include.is_empty()
            && !matches_any_pattern(include, name, &rel)
        {
            return false;
        }
        if let Some(exclude) = exclude
            && matches_any_pattern(exclude, name, &rel)
        {
            return false;
        }
        true
    }
}
