use super::error::UpdateError;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

pub fn parse_sha256(content: &str) -> Result<String, UpdateError> {
    for word in content.split_whitespace() {
        let clean = word.trim();
        if clean.len() == 64 && clean.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(clean.to_ascii_lowercase());
        }
    }
    Err(UpdateError::InvalidChecksumFile)
}

pub fn verify_checksum(bytes: &[u8], expected_digest: &str) -> Result<(), UpdateError> {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let result = hasher.finalize();
    let actual_digest = format!("{result:x}");

    if actual_digest.eq_ignore_ascii_case(expected_digest.trim()) {
        Ok(())
    } else {
        Err(UpdateError::ChecksumMismatch {
            expected: expected_digest.trim().to_ascii_lowercase(),
            actual: actual_digest,
        })
    }
}

pub fn extract_archive(archive_path: &Path, extract_to: &Path) -> Result<PathBuf, UpdateError> {
    fs::create_dir_all(extract_to)?;

    let path_str = archive_path.to_string_lossy();
    if path_str.ends_with(".tar.gz") || path_str.ends_with(".tgz") {
        extract_tar_gz(archive_path, extract_to)?;
    } else if path_str.ends_with(".zip") {
        extract_zip(archive_path, extract_to)?;
    } else {
        return Err(UpdateError::InvalidArchive(format!(
            "Unsupported archive format: {}",
            archive_path.display()
        )));
    }

    let candidate = extract_to.join("swe-grep");
    if candidate.join("SKILL.md").is_file() {
        return Ok(candidate);
    }
    if extract_to.join("SKILL.md").is_file() {
        return Ok(extract_to.to_path_buf());
    }

    Err(UpdateError::InvalidArchive(
        "Release archive does not contain a valid swe-grep package structure (missing SKILL.md)"
            .to_string(),
    ))
}

fn extract_tar_gz(archive_path: &Path, extract_to: &Path) -> Result<(), UpdateError> {
    let file = fs::File::open(archive_path)?;
    let tar = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(tar);
    archive
        .unpack(extract_to)
        .map_err(|e| UpdateError::Extraction(e.to_string()))?;
    Ok(())
}

fn extract_zip(archive_path: &Path, extract_to: &Path) -> Result<(), UpdateError> {
    let file = fs::File::open(archive_path)?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| UpdateError::Extraction(e.to_string()))?;
    archive
        .extract(extract_to)
        .map_err(|e| UpdateError::Extraction(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn parses_sha256_formats() {
        let digest = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let format1 = format!("{digest}  swe-grep-linux.tar.gz");
        let format2 = format!("{digest} *swe-grep-windows.zip");
        let format3 = format!("{digest}\n");

        assert_eq!(parse_sha256(&format1).unwrap(), digest);
        assert_eq!(parse_sha256(&format2).unwrap(), digest);
        assert_eq!(parse_sha256(&format3).unwrap(), digest);
        assert!(matches!(
            parse_sha256("invalid content").unwrap_err(),
            UpdateError::InvalidChecksumFile
        ));
    }

    #[test]
    fn verifies_checksum_correctly() {
        let data = b"hello swe-grep";
        let expected = "e5f47c93e348ee8b782ed9e5096ecc3410db31a3162aba9aa5fbdbb515301e06";
        verify_checksum(data, expected).unwrap();
        let invalid = "0".repeat(64);
        assert!(matches!(
            verify_checksum(data, &invalid).unwrap_err(),
            UpdateError::ChecksumMismatch { expected: wanted, actual }
                if wanted == invalid && actual == expected
        ));
    }

    #[test]
    fn release_archives_extract_the_packaged_runtime() {
        let tmp = TempDir::new().unwrap();
        for name in ["release.tar.gz", "release.zip"] {
            let suffix = if name.ends_with(".zip") { ".exe" } else { "" };
            let entries = [
                ("SKILL.md".to_string(), b"skill".as_slice()),
                (format!("bin/swe-grep{suffix}"), b"binary".as_slice()),
                (format!("bin/rg{suffix}"), b"ripgrep".as_slice()),
            ];
            let archive_path = tmp.path().join(name);
            let file = fs::File::create(&archive_path).unwrap();
            if name.ends_with(".zip") {
                let mut archive = zip::ZipWriter::new(file);
                for (path, content) in &entries {
                    archive
                        .start_file(
                            format!("swe-grep/{path}"),
                            zip::write::SimpleFileOptions::default(),
                        )
                        .unwrap();
                    archive.write_all(content).unwrap();
                }
                archive.finish().unwrap();
            } else {
                let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
                let mut archive = tar::Builder::new(encoder);
                for (path, content) in &entries {
                    let mut header = tar::Header::new_gnu();
                    header.set_size(content.len() as u64);
                    header.set_mode(0o755);
                    archive
                        .append_data(&mut header, format!("swe-grep/{path}"), *content)
                        .unwrap();
                }
                archive.into_inner().unwrap().finish().unwrap();
            }

            let extract_to = tmp.path().join(format!("extracted-{name}"));
            let runtime = extract_archive(&archive_path, &extract_to).unwrap();
            assert_eq!(runtime, extract_to.join("swe-grep"));
            for (path, content) in entries {
                assert_eq!(fs::read(runtime.join(path)).unwrap(), content);
            }
        }
    }
}
