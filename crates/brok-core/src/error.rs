use std::path::{Path, PathBuf};

/// Result alias for this crate.
pub type Result<T, E = CoreError> = std::result::Result<T, E>;

/// Errors raised by workspace and configuration handling.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// No `.brokspace/brok.yml` was found walking up from the start path.
    #[error("no Brokspace workspace found in {0} or any parent directory (run `brok init`)")]
    NotAWorkspace(PathBuf),
    /// The workspace is already initialized.
    #[error("{0} is already a Brokspace workspace (use --force to overwrite the configuration)")]
    AlreadyInitialized(PathBuf),
    /// `brok.yml` is invalid.
    #[error("invalid configuration: {0}")]
    Config(String),
    /// A git command failed.
    #[error("git {args}: {message}")]
    Git {
        /// Arguments passed to git.
        args: String,
        /// stderr or spawn error.
        message: String,
    },
    /// File system error with the offending path.
    #[error("{path}: {source}")]
    Io {
        /// Path involved.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
}

impl CoreError {
    pub(crate) fn io(path: &Path, source: std::io::Error) -> Self {
        Self::Io { path: path.to_path_buf(), source }
    }
}
