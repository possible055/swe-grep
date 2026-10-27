use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("Could not discover a release tag from {0}")]
    ReleaseDiscovery(String),

    #[error("Custom release mirrors require --tag to identify the version being installed")]
    MissingMirrorTag,

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Could not find a valid release asset for target platform '{0}'")]
    NoAssetForPlatform(String),

    #[error("Failed to parse SHA-256 checksum file")]
    InvalidChecksumFile,

    #[error("SHA-256 checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("Invalid release archive: {0}")]
    InvalidArchive(String),

    #[error("Extraction error: {0}")]
    Extraction(String),

    #[error(
        "Cannot update unmanaged installation at {0}. Install swe-grep with the official installer first."
    )]
    UnmanagedInstallation(PathBuf),

    #[error("Update cancelled by user")]
    Cancelled,

    #[error("Self-replace error: {0}")]
    SelfReplace(String),

    #[error("{update}; could not fully restore runtime from {}: {failures}", backup.display())]
    Rollback {
        #[source]
        update: Box<UpdateError>,
        backup: PathBuf,
        failures: String,
    },
}
