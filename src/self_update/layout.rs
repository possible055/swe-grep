use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const CONFIG_FILES: &[&str] = &[".env", "include.txt", "exclude.txt"];
const SKILL_FILE_NAME: &str = "SKILL.md";

#[derive(Debug, Clone)]
pub struct RuntimeLayout {
    pub application_root: PathBuf,
    pub runtime_dir: PathBuf,
    pub bin_dir: PathBuf,
    pub current_exe: PathBuf,
    pub installations_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ManagedSkill {
    pub destination: PathBuf,
    pub recorded_hash: String,
    pub entry_file: PathBuf,
}

impl RuntimeLayout {
    pub fn discover() -> io::Result<Option<Self>> {
        let current_exe = env::current_exe()?;
        let canonical_exe = fs::canonicalize(&current_exe).unwrap_or(current_exe);

        let Some(bin_dir) = canonical_exe.parent() else {
            return Ok(None);
        };
        let Some(runtime_dir) = bin_dir.parent() else {
            return Ok(None);
        };

        if !runtime_dir.join(SKILL_FILE_NAME).is_file() {
            return Ok(None);
        }

        let application_root = runtime_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| runtime_dir.to_path_buf());
        let installations_dir = application_root.join("installations");

        Ok(Some(Self {
            application_root,
            runtime_dir: runtime_dir.to_path_buf(),
            bin_dir: bin_dir.to_path_buf(),
            current_exe: canonical_exe,
            installations_dir,
        }))
    }

    pub fn preserve_configs(&self, candidate_dir: &Path) -> io::Result<()> {
        for config in CONFIG_FILES {
            let src = self.runtime_dir.join(config);
            let dst = candidate_dir.join(config);
            if src.is_file() {
                fs::copy(&src, &dst)?;
            }
        }
        Ok(())
    }

    pub fn create_runtime_backup(&self) -> io::Result<PathBuf> {
        let now = chrono_like_timestamp();
        let backup_name = format!("runtime.backup.{now}");
        let mut backup_path = self.application_root.join(&backup_name);
        while backup_path.exists() {
            backup_path = PathBuf::from(format!("{}.new", backup_path.display()));
        }

        copy_dir_recursive(&self.runtime_dir, &backup_path)?;
        Ok(backup_path)
    }

    pub fn list_managed_skills(&self) -> Vec<ManagedSkill> {
        let mut result = Vec::new();
        let Ok(entries) = fs::read_dir(&self.installations_dir) else {
            return result;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if let Ok(content) = fs::read_to_string(&path) {
                let mut lines = content.lines();
                if let (Some(dest), Some(hash)) = (lines.next(), lines.next()) {
                    let dest_path = PathBuf::from(dest.trim());
                    result.push(ManagedSkill {
                        destination: dest_path,
                        recorded_hash: hash.trim().to_ascii_lowercase(),
                        entry_file: path,
                    });
                }
            }
        }
        result
    }
}

pub fn hash_file(path: &Path) -> io::Result<String> {
    let bytes = fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

fn chrono_like_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    let sub = duration.subsec_millis();
    format!("{secs}{sub:03}")
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn preserve_configs_copies_existing_user_files() {
        let tmp = TempDir::new().unwrap();
        let runtime = tmp.path().join("runtime");
        let candidate = tmp.path().join("candidate");
        fs::create_dir_all(&runtime).unwrap();
        fs::create_dir_all(&candidate).unwrap();

        fs::write(runtime.join(".env"), "CUSTOM_KEY=123").unwrap();
        fs::write(runtime.join("include.txt"), "*.rs").unwrap();
        fs::write(candidate.join(".env"), "DEFAULT_KEY=abc").unwrap();

        let layout = RuntimeLayout {
            application_root: tmp.path().to_path_buf(),
            runtime_dir: runtime.clone(),
            bin_dir: runtime.join("bin"),
            current_exe: runtime.join("bin/swe-grep"),
            installations_dir: tmp.path().join("installations"),
        };

        layout.preserve_configs(&candidate).unwrap();

        assert_eq!(
            fs::read_to_string(candidate.join(".env")).unwrap(),
            "CUSTOM_KEY=123"
        );
        assert_eq!(
            fs::read_to_string(candidate.join("include.txt")).unwrap(),
            "*.rs"
        );
    }

    #[test]
    fn create_runtime_backup_creates_independent_copy() {
        let tmp = TempDir::new().unwrap();
        let runtime = tmp.path().join("runtime");
        fs::create_dir_all(runtime.join("bin")).unwrap();
        fs::write(runtime.join("SKILL.md"), "skill content").unwrap();

        let layout = RuntimeLayout {
            application_root: tmp.path().to_path_buf(),
            runtime_dir: runtime.clone(),
            bin_dir: runtime.join("bin"),
            current_exe: runtime.join("bin/swe-grep"),
            installations_dir: tmp.path().join("installations"),
        };

        let backup = layout.create_runtime_backup().unwrap();
        assert!(backup.is_dir());
        assert_eq!(
            fs::read_to_string(backup.join("SKILL.md")).unwrap(),
            "skill content"
        );
    }

    #[test]
    fn list_managed_skills_parses_records() {
        let tmp = TempDir::new().unwrap();
        let installations = tmp.path().join("installations");
        fs::create_dir_all(&installations).unwrap();

        let record_content = "/home/user/.agents/skills/swe-grep\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\n";
        fs::write(installations.join("hash123"), record_content).unwrap();

        let layout = RuntimeLayout {
            application_root: tmp.path().to_path_buf(),
            runtime_dir: tmp.path().join("runtime"),
            bin_dir: tmp.path().join("runtime/bin"),
            current_exe: tmp.path().join("runtime/bin/swe-grep"),
            installations_dir: installations,
        };

        let skills = layout.list_managed_skills();
        assert_eq!(skills.len(), 1);
        assert_eq!(
            skills[0].destination,
            PathBuf::from("/home/user/.agents/skills/swe-grep")
        );
        assert_eq!(
            skills[0].recorded_hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
