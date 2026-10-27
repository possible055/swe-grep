pub mod archive;
pub mod client;
pub mod error;
pub mod layout;
pub mod replace;
pub mod version;

use client::UpdateClient;
use error::UpdateError;
use layout::RuntimeLayout;
use replace::{replace_runtime_components, sync_managed_skills};
use std::io::{self, Write};
use std::path::PathBuf;

#[derive(Debug, clap::Args)]
pub struct SelfUpdateArgs {
    /// Only check for updates without downloading or installing
    #[arg(long)]
    pub check: bool,

    /// Force update even if already running the latest version
    #[arg(long, short = 'f')]
    pub force: bool,

    /// Target specific release tag (e.g. v0.1.9)
    #[arg(long)]
    pub tag: Option<String>,

    /// Override the release asset directory for a mirror (requires --tag)
    #[arg(long)]
    pub release_base_url: Option<String>,

    /// Do not prompt for confirmation (auto-confirm)
    #[arg(long, short = 'y')]
    pub yes: bool,
}

pub fn run(args: SelfUpdateArgs) -> i32 {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("Error: failed to initialize async runtime: {err}");
            return 1;
        }
    };

    match rt.block_on(run_async(args)) {
        Ok(()) => 0,
        Err(UpdateError::Cancelled) => {
            println!("Update cancelled.");
            1
        }
        Err(err) => {
            eprintln!("Error: {err}");
            1
        }
    }
}

async fn run_async(args: SelfUpdateArgs) -> Result<(), UpdateError> {
    let current_version = env!("CARGO_PKG_VERSION");
    let layout = RuntimeLayout::discover()?.ok_or_else(|| {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("swe-grep"));
        UpdateError::UnmanagedInstallation(exe)
    })?;

    let client = UpdateClient::new(args.release_base_url.clone())?;
    println!("Checking for latest release...");
    let release_info = client.fetch_latest_release(args.tag.as_deref()).await?;

    let is_newer_version = version::is_newer(&release_info.tag_name, current_version);

    if args.check {
        println!("Current version: v{current_version}");
        println!("Latest version:  {}", release_info.tag_name);
        if is_newer_version {
            println!("\nA newer version is available. Run `swe-grep self-update` to upgrade.");
        } else {
            println!("\nswe-grep is up to date.");
        }
        return Ok(());
    }

    if !is_newer_version && !args.force {
        println!(
            "swe-grep is already up to date (current version: v{current_version}, latest: {}).",
            release_info.tag_name
        );
        println!("Use --force to reinstall anyway.");
        return Ok(());
    }

    println!(
        "Updating swe-grep from v{} to {}...",
        current_version, release_info.tag_name
    );

    if !args.yes && !confirm_prompt()? {
        return Err(UpdateError::Cancelled);
    }

    let temp_dir = tempfile::tempdir().map_err(UpdateError::Io)?;
    let archive_path = temp_dir.path().join(&release_info.archive_name);
    let checksum_path = temp_dir
        .path()
        .join(format!("{}.sha256", release_info.archive_name));

    println!("Downloading {}...", release_info.archive_name);
    client
        .download_with_retry(&release_info.archive_url, &archive_path)
        .await?;

    println!("Downloading checksum...");
    client
        .download_with_retry(&release_info.checksum_url, &checksum_path)
        .await?;

    let checksum_content = std::fs::read_to_string(&checksum_path)?;
    let expected_sha256 = archive::parse_sha256(&checksum_content)?;
    let archive_bytes = std::fs::read(&archive_path)?;
    archive::verify_checksum(&archive_bytes, &expected_sha256)?;
    println!("{}: SHA-256 OK", release_info.archive_name);

    println!("Extracting update package...");
    let extract_dir = temp_dir.path().join("extracted");
    let candidate_dir = archive::extract_archive(&archive_path, &extract_dir)?;

    println!("Preserving user configurations...");
    layout.preserve_configs(&candidate_dir)?;

    println!("Creating backup of existing runtime...");
    let backup_path = layout.create_runtime_backup()?;
    println!("Existing runtime backed up at {}", backup_path.display());

    println!("Replacing runtime components...");
    replace_runtime_components(&layout, &candidate_dir, &backup_path)?;

    let synced_skills = sync_managed_skills(&layout);
    for skill_path in synced_skills {
        println!("Updated managed Skill at {skill_path}");
    }

    println!(
        "\nSuccessfully updated swe-grep to {}!",
        release_info.tag_name
    );
    println!("Runtime: {}", layout.runtime_dir.display());
    Ok(())
}

fn confirm_prompt() -> Result<bool, UpdateError> {
    print!("Proceed with update? [y/N]: ");
    io::stdout().flush().map_err(UpdateError::Io)?;

    let mut input = String::new();
    io::stdin().read_line(&mut input).map_err(UpdateError::Io)?;
    let trimmed = input.trim();
    Ok(trimmed.eq_ignore_ascii_case("y") || trimmed.eq_ignore_ascii_case("yes"))
}
