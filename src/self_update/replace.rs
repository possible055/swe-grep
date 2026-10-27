use super::error::UpdateError;
use super::layout::{ManagedSkill, RuntimeLayout, hash_file};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub fn replace_runtime_components(
    layout: &RuntimeLayout,
    candidate_dir: &Path,
    backup_dir: &Path,
) -> Result<(), UpdateError> {
    let candidate_bin = candidate_dir.join("bin");
    let candidate_swe_grep = candidate_bin.join(swe_grep_bin_name());
    let candidate_rg = candidate_bin.join(rg_bin_name());

    if !candidate_swe_grep.is_file() {
        return Err(UpdateError::InvalidArchive(format!(
            "Missing {}",
            candidate_swe_grep.display()
        )));
    }
    if !candidate_rg.is_file() {
        return Err(UpdateError::InvalidArchive(format!(
            "Missing {}",
            candidate_rg.display()
        )));
    }

    let target_swe_grep = layout.bin_dir.join(swe_grep_bin_name());
    fs::create_dir_all(&layout.bin_dir)?;
    let mut replaced = Vec::new();
    let update_result = (|| -> Result<(), UpdateError> {
        let rg_relative = PathBuf::from("bin").join(rg_bin_name());
        copy_atomic(&candidate_rg, &layout.runtime_dir.join(&rg_relative), true)?;
        replaced.push(rg_relative);

        for name in ["SKILL.md", ".env", "include.txt", "exclude.txt"] {
            let source = candidate_dir.join(name);
            if source.is_file() {
                copy_atomic(&source, &layout.runtime_dir.join(name), false)?;
                replaced.push(PathBuf::from(name));
            }
        }

        // Commit the running executable last so auxiliary failures cannot strand a new binary.
        if is_same_file(&layout.current_exe, &target_swe_grep) {
            self_replace::self_replace(&candidate_swe_grep)
                .map_err(|error| UpdateError::SelfReplace(error.to_string()))?;
        } else {
            copy_atomic(&candidate_swe_grep, &target_swe_grep, true)?;
        }
        Ok(())
    })();

    if let Err(update) = update_result {
        // On Windows self-replace can move the old executable before returning an error.
        let main_relative = PathBuf::from("bin").join(swe_grep_bin_name());
        if !target_swe_grep.exists() && backup_dir.join(&main_relative).is_file() {
            replaced.push(main_relative);
        }
        let mut failures = Vec::new();
        for relative in replaced.iter().rev() {
            let original = backup_dir.join(relative);
            let target = layout.runtime_dir.join(relative);
            let result = if original.is_file() {
                copy_atomic(&original, &target, false)
            } else {
                fs::remove_file(&target)
            };
            if let Err(error) = result {
                failures.push(format!("{}: {error}", target.display()));
            }
        }
        return if failures.is_empty() {
            Err(update)
        } else {
            Err(UpdateError::Rollback {
                update: Box::new(update),
                backup: backup_dir.to_path_buf(),
                failures: failures.join("; "),
            })
        };
    }

    Ok(())
}

pub fn sync_managed_skills(layout: &RuntimeLayout) -> Vec<String> {
    let mut updated = Vec::new();
    let runtime_skill = layout.runtime_dir.join("SKILL.md");
    let Ok(new_skill_content) = fs::read(&runtime_skill) else {
        return updated;
    };
    let new_hash = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&new_skill_content);
        format!("{:x}", hasher.finalize())
    };

    for skill in layout.list_managed_skills() {
        if sync_single_skill(&skill, &runtime_skill, &new_hash) {
            updated.push(skill.destination.display().to_string());
        }
    }
    updated
}

fn sync_single_skill(skill: &ManagedSkill, new_skill_path: &Path, new_hash: &str) -> bool {
    let target_skill_md = skill.destination.join("SKILL.md");
    if !target_skill_md.is_file() {
        return false;
    }

    let Ok(current_hash) = hash_file(&target_skill_md) else {
        return false;
    };

    if !current_hash.eq_ignore_ascii_case(&skill.recorded_hash) {
        // User modified SKILL.md, preserve it
        return false;
    }

    if fs::copy(new_skill_path, &target_skill_md).is_err() {
        return false;
    }

    // Update installation record
    let new_entry_content = format!("{}\n{}\n", skill.destination.display(), new_hash);
    fs::write(&skill.entry_file, new_entry_content).is_ok()
}

fn copy_atomic(src: &Path, dst: &Path, executable: bool) -> io::Result<()> {
    let parent = dst.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "destination has no parent directory",
        )
    })?;
    let staged = tempfile::NamedTempFile::new_in(parent)?;
    fs::copy(src, staged.path())?;
    #[cfg(unix)]
    if executable {
        let mut perms = fs::metadata(staged.path())?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(staged.path(), perms)?;
    }
    #[cfg(not(unix))]
    let _ = executable;
    staged.persist(dst).map_err(|error| error.error)?;
    Ok(())
}

fn swe_grep_bin_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "swe-grep.exe"
    } else {
        "swe-grep"
    }
}

fn rg_bin_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "rg.exe"
    } else {
        "rg"
    }
}

fn is_same_file(path_a: &Path, path_b: &Path) -> bool {
    if path_a == path_b {
        return true;
    }
    match (fs::canonicalize(path_a), fs::canonicalize(path_b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn failed_runtime_update_restores_all_changed_files() {
        let tmp = TempDir::new().unwrap();
        let runtime = tmp.path().join("runtime");
        let candidate = tmp.path().join("candidate");
        for dir in [&runtime, &candidate] {
            fs::create_dir_all(dir.join("bin")).unwrap();
            for name in [swe_grep_bin_name(), rg_bin_name()] {
                fs::write(
                    dir.join("bin").join(name),
                    dir.file_name().unwrap().as_encoded_bytes(),
                )
                .unwrap();
            }
            fs::write(
                dir.join("SKILL.md"),
                dir.file_name().unwrap().as_encoded_bytes(),
            )
            .unwrap();
            fs::write(
                dir.join(".env"),
                dir.file_name().unwrap().as_encoded_bytes(),
            )
            .unwrap();
        }
        fs::create_dir(runtime.join("exclude.txt")).unwrap();
        fs::write(candidate.join("include.txt"), "new include").unwrap();
        fs::write(candidate.join("exclude.txt"), "new exclude").unwrap();
        let layout = RuntimeLayout {
            application_root: tmp.path().to_path_buf(),
            runtime_dir: runtime.clone(),
            bin_dir: runtime.join("bin"),
            current_exe: std::env::current_exe().unwrap(),
            installations_dir: tmp.path().join("installations"),
        };

        let backup = layout.create_runtime_backup().unwrap();
        let error = replace_runtime_components(&layout, &candidate, &backup).unwrap_err();
        assert!(matches!(error, UpdateError::Io(error) if matches!(
            error.kind(), io::ErrorKind::IsADirectory | io::ErrorKind::PermissionDenied
        )));
        for path in [
            runtime.join("bin").join(swe_grep_bin_name()),
            runtime.join("bin").join(rg_bin_name()),
            runtime.join("SKILL.md"),
            runtime.join(".env"),
        ] {
            assert_eq!(fs::read(path).unwrap(), b"runtime");
        }
        assert!(!runtime.join("include.txt").exists());
        assert!(runtime.join("exclude.txt").is_dir());
    }

    #[test]
    fn managed_skill_sync_updates_records_and_preserves_user_changes() {
        let tmp = TempDir::new().unwrap();
        let runtime = tmp.path().join("runtime");
        let installations = tmp.path().join("installations");
        fs::create_dir_all(&runtime).unwrap();
        fs::create_dir_all(&installations).unwrap();
        fs::write(runtime.join("SKILL.md"), "new skill").unwrap();
        let old_hash = "752eed612c883c85bc85f388e32f56af763eda7561a9f58bcd86cb7449034fb3";
        let new_hash = "a106880ea21403f5c19c7165115b646b389d08fe2743b8083ede4f6d5b3392b6";
        let managed = tmp.path().join("managed");
        let customized = tmp.path().join("customized");
        for (destination, content) in [(&managed, "old skill"), (&customized, "user skill")] {
            fs::create_dir(destination).unwrap();
            fs::write(destination.join("SKILL.md"), content).unwrap();
            fs::write(
                installations.join(destination.file_name().unwrap()),
                format!("{}\n{old_hash}\n", destination.display()),
            )
            .unwrap();
        }
        let layout = RuntimeLayout {
            application_root: tmp.path().to_path_buf(),
            runtime_dir: runtime.clone(),
            bin_dir: runtime.join("bin"),
            current_exe: std::env::current_exe().unwrap(),
            installations_dir: installations.clone(),
        };

        assert_eq!(
            sync_managed_skills(&layout),
            vec![managed.display().to_string()]
        );
        assert_eq!(
            fs::read_to_string(managed.join("SKILL.md")).unwrap(),
            "new skill"
        );
        assert_eq!(
            fs::read_to_string(installations.join("managed")).unwrap(),
            format!("{}\n{new_hash}\n", managed.display())
        );
        assert_eq!(
            fs::read_to_string(customized.join("SKILL.md")).unwrap(),
            "user skill"
        );
        assert_eq!(
            fs::read_to_string(installations.join("customized")).unwrap(),
            format!("{}\n{old_hash}\n", customized.display())
        );
    }
}
