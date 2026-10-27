use crate::config;
use crate::core::{
    SearchOptions, extract_key, format_search_success, mask_api_key, save_cached_api_key, search,
};
use crate::executor::ripgrep::resolve_rg_path;
use crate::path_filter::PathFilterConfig;
use crate::self_update::{self, SelfUpdateArgs};
use clap::{Args, Parser, Subcommand};
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::io::Write;
use std::path::{Path, PathBuf};

const ENV_FILE_NAME: &str = ".env";
const INCLUDE_FILE_NAME: &str = "include.txt";
const EXCLUDE_FILE_NAME: &str = "exclude.txt";
const SKILL_FILE_NAME: &str = "SKILL.md";

#[derive(Debug, Parser)]
#[command(name = "swe-grep", version)]
#[command(disable_help_subcommand = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    #[command(about = "Execute semantic code search")]
    Search(SearchArgs),
    #[command(
        name = "extract-key",
        about = "Extract Windsurf API key from local database"
    )]
    ExtractKey(ExtractKeyArgs),
    #[command(
        name = "self-update",
        alias = "update",
        about = "Check for updates and upgrade swe-grep in-place"
    )]
    SelfUpdate(SelfUpdateArgs),
}

#[derive(Debug, Args)]
struct SearchArgs {
    #[arg(help = "Natural language search query")]
    query: String,

    #[arg(long, help = "Windsurf API key. Overrides env and config.")]
    api_key: Option<String>,

    #[arg(long, help = "Absolute or relative path to project root.")]
    path: PathBuf,

    #[arg(
        long,
        value_parser = parse_turns,
        help = "Maximum search rounds. Default is from FC_MAX_TURNS or 4."
    )]
    turns: Option<usize>,
}

#[derive(Debug, Args)]
struct ExtractKeyArgs {
    #[arg(long, help = "Path to Windsurf state.vscdb. Default is auto-detect.")]
    db_path: Option<PathBuf>,

    #[arg(long, help = "Save extracted key to swe-tools config.")]
    save: bool,

    #[arg(long, help = "Print the full key instead of a masked key.")]
    show: bool,
}

pub fn run() -> i32 {
    load_skill_env();
    let cli = Cli::parse();
    match cli.command {
        Commands::ExtractKey(args) => run_extract_key(args),
        Commands::Search(args) => {
            for warning in config::invalid_env_warnings() {
                eprintln!("Config warning: {warning}");
            }
            run_search(
                args,
                config::fc_max_turns(),
                config::scope_snapshot_tree_depth(),
                config::fc_max_commands(),
                config::fc_timeout_ms(),
            )
        }
        Commands::SelfUpdate(args) => self_update::run(args),
    }
}

pub fn load_skill_env() {
    if let Some(env_path) = packaged_config_file(ENV_FILE_NAME) {
        load_env_file(&env_path);
    }
}

fn load_env_file(env_path: &Path) {
    let Ok(text) = fs::read_to_string(env_path) else {
        return;
    };

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || !line.contains('=') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let mut value = value.trim().to_string();
        if (value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\''))
        {
            value = value[1..value.len() - 1].to_string();
        }

        if key == "API_KEY" {
            set_env_var("WINDSURF_API_KEY", &value);
        } else {
            set_env_var(key, &value);
        }
    }
}

fn packaged_config_file(file_name: &str) -> Option<PathBuf> {
    let current_exe = env::current_exe().ok();
    packaged_config_file_from(current_exe.as_deref(), file_name)
}

fn packaged_config_file_from(current_exe: Option<&Path>, file_name: &str) -> Option<PathBuf> {
    packaged_skill_dir_from(current_exe).map(|dir| dir.join(file_name))
}

fn packaged_skill_dir_from(current_exe: Option<&Path>) -> Option<PathBuf> {
    let bin_dir = current_exe?.parent()?;
    let skill_dir = bin_dir.parent()?;
    skill_dir
        .join(SKILL_FILE_NAME)
        .is_file()
        .then(|| skill_dir.to_path_buf())
}

fn set_env_var(key: &str, value: &str) {
    // SAFETY: CLI loads .env before Tokio runtime.
    unsafe {
        env::set_var(key, value);
    }
}

fn parse_turns(value: &str) -> Result<usize, String> {
    config::parse_range(value, config::FC_MAX_TURNS_RANGE, "turns")
}

fn run_extract_key(args: ExtractKeyArgs) -> i32 {
    let result = extract_key(args.db_path.as_deref());
    if let Some(error) = result.error {
        eprintln!("Error: {error}");
        if let Some(hint) = result.hint {
            eprintln!("Hint: {hint}");
        }
        return 1;
    }

    let Some(key) = result.api_key else {
        eprintln!("Error: apiKey field is empty");
        return 1;
    };

    if args.save {
        match save_cached_api_key(&key, None) {
            Ok(config_path) => eprintln!("Saved Windsurf API key to {}", config_path.display()),
            Err(error) => {
                eprintln!("Error: {error}");
                return 1;
            }
        }
    }

    println!(
        "Windsurf API Key: {}",
        if args.show {
            key.clone()
        } else {
            mask_api_key(&key)
        }
    );
    if let Some(key_type) = result.key_type {
        eprintln!("Key type: {key_type}");
    }
    eprintln!("Source DB: {}", result.db_path);

    if args.show {
        println!("\nRun the following command to set the env var:");
        println!("  export WINDSURF_API_KEY=\"{key}\"");
    }

    0
}

fn run_search(
    args: SearchArgs,
    default_turns: usize,
    scope_snapshot_tree_depth: usize,
    max_commands: usize,
    timeout_ms: u64,
) -> i32 {
    if let Err(error) = resolve_rg_path() {
        eprintln!("{error}");
        return 1;
    }

    let project_path = absolute_path(&args.path);
    if !project_path.is_dir() {
        eprintln!(
            "Error: Project path does not exist: {}",
            project_path.display()
        );
        return 1;
    }

    let path_filter = read_path_filter_config();
    let mut options = SearchOptions::new(args.query, project_path);
    options.api_key = args.api_key;
    options.max_turns = args.turns.unwrap_or(default_turns);
    options.max_commands = max_commands;
    options.timeout_ms = timeout_ms;
    options.tree_depth = scope_snapshot_tree_depth;
    options.path_filter = path_filter;

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("Unexpected error: {error}");
            return 1;
        }
    };

    let progress = |message: &str| {
        eprintln!("[fast-context] {message}");
        let _ = std::io::stderr().flush();
    };

    let result = runtime.block_on(async { search(options, Some(&progress)).await });
    if let Some(error) = result.error {
        eprintln!("Search failed: {error}");
        return 1;
    }

    println!("{}", format_search_success(&result));
    0
}

fn read_path_filter_config() -> PathFilterConfig {
    let path_filter_enabled = config::swegrep_path_filter_enabled();
    if path_filter_enabled == Some(false) {
        return PathFilterConfig::disabled();
    }

    let mut warnings = Vec::new();
    let include_patterns =
        read_filter_patterns_from_packaged_config(INCLUDE_FILE_NAME, &mut warnings);
    let exclude_patterns =
        read_filter_patterns_from_packaged_config(EXCLUDE_FILE_NAME, &mut warnings);

    PathFilterConfig {
        enabled: true,
        include_patterns,
        exclude_patterns,
        warnings,
    }
}

fn read_filter_patterns_from_packaged_config(
    file_name: &str,
    warnings: &mut Vec<String>,
) -> Vec<String> {
    let Some(path) = packaged_config_file(file_name) else {
        return Vec::new();
    };
    read_filter_patterns(&path, warnings)
}

fn read_filter_patterns(path: &Path, warnings: &mut Vec<String>) -> Vec<String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == ErrorKind::NotFound => return Vec::new(),
        Err(err) => {
            warnings.push(format!("Could not read {}: {err}", path.display()));
            return Vec::new();
        }
    };

    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(ToOwned::to_owned)
        .collect()
}

fn absolute_path(path: &Path) -> PathBuf {
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    candidate.canonicalize().unwrap_or(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{parse_env_range_value, parse_env_u64_range_value};
    use tempfile::TempDir;

    #[test]
    fn scope_snapshot_tree_depth_env_accepts_expected_range() {
        assert_eq!(parse_env_range_value("0", 0..=8), Some(0));
        assert_eq!(parse_env_range_value("4", 0..=8), Some(4));
        assert_eq!(parse_env_range_value("8", 0..=8), Some(8));
        assert_eq!(parse_env_range_value("9", 0..=8), None);
        assert_eq!(parse_env_range_value("not-a-number", 0..=8), None);
    }

    #[test]
    fn fc_max_commands_env_accepts_expected_range() {
        assert_eq!(parse_env_range_value("1", 1..=8), Some(1));
        assert_eq!(parse_env_range_value("8", 1..=8), Some(8));
        assert_eq!(parse_env_range_value("0", 1..=8), None);
        assert_eq!(parse_env_range_value("9", 1..=8), None);
        assert_eq!(parse_env_range_value("not-a-number", 1..=8), None);
    }

    #[test]
    fn fc_max_turns_env_accepts_expected_range() {
        assert_eq!(parse_env_range_value("4", 4..=6), Some(4));
        assert_eq!(parse_env_range_value("6", 4..=6), Some(6));
        assert_eq!(parse_env_range_value("3", 4..=6), None);
        assert_eq!(parse_env_range_value("7", 4..=6), None);
        assert_eq!(parse_env_range_value("not-a-number", 4..=6), None);
    }

    #[test]
    fn fc_timeout_ms_env_accepts_expected_range() {
        assert_eq!(
            parse_env_u64_range_value("1000", 1_000..=300_000),
            Some(1_000)
        );
        assert_eq!(
            parse_env_u64_range_value("300000", 1_000..=300_000),
            Some(300_000)
        );
        assert_eq!(parse_env_u64_range_value("999", 1_000..=300_000), None);
        assert_eq!(parse_env_u64_range_value("300001", 1_000..=300_000), None);
        assert_eq!(
            parse_env_u64_range_value("not-a-number", 1_000..=300_000),
            None
        );
    }

    #[test]
    fn packaged_skill_dir_requires_packaged_skill_layout() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path().join("swe-grep");
        let bin_dir = skill_dir.join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(skill_dir.join(SKILL_FILE_NAME), "").unwrap();

        let current_exe = bin_dir.join("swe-grep");
        assert_eq!(
            packaged_skill_dir_from(Some(&current_exe)).as_deref(),
            Some(skill_dir.as_path())
        );
    }

    #[test]
    fn packaged_skill_dir_ignores_regular_bin_dirs() {
        let tmp = TempDir::new().unwrap();
        let current_exe = tmp.path().join("bin").join("swe-grep");

        assert_eq!(packaged_skill_dir_from(Some(&current_exe)), None);
    }

    #[test]
    fn packaged_config_file_uses_packaged_skill_dir_only() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path().join("swe-grep");
        let bin_dir = skill_dir.join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(skill_dir.join(SKILL_FILE_NAME), "").unwrap();
        let expected = skill_dir.join(ENV_FILE_NAME);

        assert_eq!(
            packaged_config_file_from(Some(&bin_dir.join("swe-grep")), ENV_FILE_NAME).as_deref(),
            Some(expected.as_path())
        );
    }
}
