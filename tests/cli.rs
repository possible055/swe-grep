use assert_cmd::Command;
use rusqlite::{Connection, params};
use serde_json::json;
use std::fs;
use tempfile::TempDir;

fn write_auth_db(path: &std::path::Path, key: &str) {
    let conn = Connection::open(path).unwrap();
    conn.execute(
        "CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
        params!["windsurfAuthStatus", json!({ "apiKey": key }).to_string()],
    )
    .unwrap();
}

#[test]
fn extract_key_show_prints_full_key_and_source() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("state.vscdb");
    write_auth_db(&db_path, "sk-ws-01-mock");

    let output = Command::cargo_bin("swe-grep")
        .unwrap()
        .args(["extract-key", "--show", "--db-path"])
        .arg(&db_path)
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("Windsurf API Key: sk-ws-01-mock"));
    assert!(stdout.contains("export WINDSURF_API_KEY=\"sk-ws-01-mock\""));
    assert!(stderr.contains("Source DB:"));
}

#[test]
fn search_requires_rg() {
    let tmp = TempDir::new().unwrap();
    let output = Command::cargo_bin("swe-grep")
        .unwrap()
        .args(["search", "dummy_query", "--path"])
        .arg(tmp.path())
        .env("PATH", "")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ripgrep"));
}

#[test]
fn search_accepts_bundled_rg_when_path_is_empty() {
    let tmp = TempDir::new().unwrap();
    let binary_name = format!("swe-grep{}", std::env::consts::EXE_SUFFIX);
    let bundled_rg_name = format!("rg{}", std::env::consts::EXE_SUFFIX);
    let bundled_bin = tmp.path().join(binary_name);
    let bundled_rg = tmp.path().join(bundled_rg_name);
    fs::copy(assert_cmd::cargo::cargo_bin("swe-grep"), &bundled_bin).unwrap();
    fs::write(&bundled_rg, "").unwrap();

    let missing_project = tmp.path().join("missing-project");
    let output = std::process::Command::new(&bundled_bin)
        .args(["search", "dummy_query", "--path"])
        .arg(&missing_project)
        .env("PATH", "")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Project path does not exist"));
    assert!(!stderr.contains("ripgrep"));
}

#[test]
fn search_requires_path() {
    let output = Command::cargo_bin("swe-grep")
        .unwrap()
        .args(["search", "dummy_query"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--path"));
}

#[test]
fn search_rejects_turns_below_minimum() {
    let tmp = TempDir::new().unwrap();
    let output = Command::cargo_bin("swe-grep")
        .unwrap()
        .args(["search", "dummy_query", "--path"])
        .arg(tmp.path())
        .args(["--turns", "3"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("turns must be between 4 and 6"));
}

#[test]
fn search_warns_once_for_an_invalid_environment_value() {
    let tmp = TempDir::new().unwrap();
    let output = Command::cargo_bin("swe-grep")
        .unwrap()
        .args(["search", "dummy_query", "--path"])
        .arg(tmp.path())
        .env("FC_MAX_TURNS", "10")
        .env("PATH", "")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let warning = "Config warning: Invalid FC_MAX_TURNS value \"10\"; using default 4";
    assert_eq!(stderr.matches(warning).count(), 1);
    assert!(stderr.contains("ripgrep"));
}

#[test]
fn self_update_fails_on_unmanaged_installation() {
    let output = Command::cargo_bin("swe-grep")
        .unwrap()
        .args(["self-update"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Cannot update unmanaged installation"));
}

#[test]
fn self_update_help_shows_options() {
    let output = Command::cargo_bin("swe-grep")
        .unwrap()
        .args(["self-update", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--check"));
    assert!(stdout.contains("--force"));
    assert!(stdout.contains("--tag"));
    assert!(stdout.contains("--release-base-url"));
    assert!(stdout.contains("--yes"));
}

#[test]
fn self_update_alias_update_works() {
    let output = Command::cargo_bin("swe-grep")
        .unwrap()
        .args(["update", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--check"));
}
