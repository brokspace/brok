//! Per-user state shared by every workspace.
//!
//! - **Model cache**: embedding models are downloaded once per user, not per
//!   project (`$BROK_MODELS_DIR`, else the OS cache directory).
//! - **Defaults**: the LLM chosen with `brok setup llm` becomes the default
//!   for every new workspace (`$BROK_CONFIG_DIR/config.yml`, else the OS
//!   config directory).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::LlmConfig;
use crate::error::{CoreError, Result};

/// Overrides the model cache directory.
pub const MODELS_DIR_ENV: &str = "BROK_MODELS_DIR";
/// Overrides the user configuration directory.
pub const CONFIG_DIR_ENV: &str = "BROK_CONFIG_DIR";

/// Shared embedding model cache (`~/.cache/brokspace/models` on Linux,
/// `~/Library/Caches/brokspace/models` on macOS, `%LOCALAPPDATA%\brokspace\models`
/// on Windows).
pub fn models_dir() -> Option<PathBuf> {
    std::env::var_os(MODELS_DIR_ENV)
        .map(PathBuf::from)
        .or_else(|| dirs::cache_dir().map(|dir| dir.join("brokspace").join("models")))
}

/// Directory of the user configuration.
pub fn config_dir() -> Option<PathBuf> {
    std::env::var_os(CONFIG_DIR_ENV).map(PathBuf::from).or_else(|| dirs::config_dir().map(|dir| dir.join("brokspace")))
}

/// Defaults applied to new workspaces.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UserConfig {
    /// LLM routing chosen with `brok setup llm`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm: Option<LlmConfig>,
}

impl UserConfig {
    /// Path of the user configuration file.
    pub fn path() -> Option<PathBuf> {
        config_dir().map(|dir| dir.join("config.yml"))
    }

    /// Loads the user configuration; a missing or unreadable file yields the
    /// defaults (with a warning for the latter).
    pub fn load() -> Self {
        Self::path().map(|path| Self::load_from(&path)).unwrap_or_default()
    }

    /// Loads from an explicit file.
    pub fn load_from(path: &std::path::Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else { return Self::default() };
        serde_yaml_ng::from_str(&text).unwrap_or_else(|e| {
            tracing::warn!(path = %path.display(), error = %e, "ignoring unreadable user configuration");
            Self::default()
        })
    }

    /// Persists the user configuration.
    pub fn save(&self) -> Result<PathBuf> {
        let path = Self::path().ok_or_else(|| CoreError::Config("cannot locate the user config directory".into()))?;
        self.save_to(&path)?;
        Ok(path)
    }

    /// Persists to an explicit file.
    pub fn save_to(&self, path: &std::path::Path) -> Result<()> {
        let yaml = serde_yaml_ng::to_string(self).map_err(|e| CoreError::Config(e.to_string()))?;
        let text = format!("# Brokspace defaults for new workspaces (written by `brok setup llm`).\n{yaml}");
        crate::fsutil::write_atomic(path, text.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.yml");
        assert_eq!(UserConfig::load_from(&path), UserConfig::default());
        let config = UserConfig { llm: Some(LlmConfig::default()) };
        config.save_to(&path).unwrap();
        assert_eq!(UserConfig::load_from(&path), config);
        std::fs::write(&path, "nope: [").unwrap();
        assert_eq!(UserConfig::load_from(&path), UserConfig::default(), "corrupt files fall back to defaults");
    }
}
