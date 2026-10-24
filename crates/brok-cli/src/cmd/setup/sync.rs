//! `brok setup sync`: where the index is mirrored.
//!
//! Interactive by default; every prompt also has a flag so the command can
//! run in CI. Credentials are never written to `brok.yml`: only the names of
//! the environment variables that hold them.

use std::path::Path;

use anyhow::{Context, Result, bail};
use brok_core::config::{RemoteStore, SyncBackend};
use brok_core::git;
use clap::ValueEnum;
use dialoguer::{Input, Select};

use crate::ui;

/// Arguments of `brok setup sync`.
#[derive(Debug, Default, clap::Args)]
pub struct Args {
    /// Backend (asked interactively when omitted).
    #[arg(long, value_enum)]
    backend: Option<Backend>,
    /// Bucket name (object storage).
    #[arg(long)]
    bucket: Option<String>,
    /// Key prefix inside the bucket.
    #[arg(long)]
    prefix: Option<String>,
    /// Region (S3); use `auto` for Cloudflare R2.
    #[arg(long)]
    region: Option<String>,
    /// Custom endpoint (R2: https://<account>.r2.cloudflarestorage.com, MinIO: http://host:9000).
    #[arg(long)]
    endpoint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Backend {
    None,
    S3,
    R2,
    Minio,
    GitLfs,
}

impl From<Backend> for SyncBackend {
    fn from(b: Backend) -> Self {
        match b {
            Backend::None => Self::None,
            Backend::S3 => Self::S3,
            Backend::R2 => Self::R2,
            Backend::Minio => Self::Minio,
            Backend::GitLfs => Self::GitLfs,
        }
    }
}

pub async fn run(cwd: &Path, args: Args) -> Result<()> {
    let mut workspace = crate::cmd::workspace(cwd)?;
    let interactive = ui::interactive();
    let backend = match args.backend {
        Some(backend) => backend,
        None if interactive => {
            let choices = [
                "Local only",
                "AWS S3",
                "Cloudflare R2",
                "MinIO / S3-compatible",
                "Git LFS (version the index with the repo)",
            ];
            let picked = Select::new()
                .with_prompt("Where should the index be mirrored?")
                .items(choices)
                .default(0)
                .interact()?;
            [Backend::None, Backend::S3, Backend::R2, Backend::Minio, Backend::GitLfs][picked]
        }
        None => bail!("not a terminal: pass --backend"),
    };

    let mut config = workspace.config().clone();
    config.sync.backend = backend.into();
    config.sync.remote = None;

    match backend {
        Backend::None => {}
        Backend::GitLfs => setup_lfs(workspace.root(), &workspace.paths().brokspace)?,
        Backend::S3 | Backend::R2 | Backend::Minio => {
            let ask = |prompt: &str, given: Option<String>, default: Option<&str>| -> Result<String> {
                if let Some(value) = given {
                    return Ok(value);
                }
                if !interactive {
                    return default.map(str::to_owned).with_context(|| format!("missing --{}", prompt.to_lowercase()));
                }
                let mut input = Input::<String>::new().with_prompt(prompt);
                if let Some(default) = default {
                    input = input.default(default.to_owned());
                }
                Ok(input.interact_text()?)
            };
            let (default_region, default_endpoint) = match backend {
                Backend::R2 => (Some("auto"), Some("https://<account-id>.r2.cloudflarestorage.com")),
                Backend::Minio => (Some("us-east-1"), Some("http://localhost:9000")),
                _ => (Some("us-east-1"), None),
            };
            let bucket = ask("Bucket", args.bucket, None)?;
            let prefix = ask("Prefix", args.prefix, Some("brokspace"))?;
            let region = ask("Region", args.region, default_region)?;
            let endpoint = match default_endpoint {
                Some(default) => Some(ask("Endpoint", args.endpoint, Some(default))?),
                None => args.endpoint,
            };
            config.sync.remote = Some(RemoteStore {
                bucket,
                prefix,
                region: Some(region),
                endpoint,
                access_key_env: "AWS_ACCESS_KEY_ID".into(),
                secret_key_env: "AWS_SECRET_ACCESS_KEY".into(),
            });
        }
    }

    workspace.save_config(config)?;
    ui::success(&format!("Saved sync settings to {}", workspace.paths().config_file.display()));
    if matches!(backend, Backend::S3 | Backend::R2 | Backend::Minio) {
        ui::hint("Credentials are read from AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY at sync time.");
        ui::hint("Note: the sync engine itself (`brok sync`) is on the roadmap; this records the target.");
    }
    Ok(())
}

/// Tracks the index with Git LFS and stops ignoring it.
fn setup_lfs(root: &Path, brokspace: &Path) -> Result<()> {
    if !git::is_repository(root) {
        bail!("{} is not a git repository", root.display());
    }
    git::lfs_track(root, ".brokspace/databases/**").context("is git-lfs installed? (https://git-lfs.com)")?;
    let gitignore = brokspace.join(".gitignore");
    if let Ok(content) = std::fs::read_to_string(&gitignore) {
        let kept: Vec<&str> = content.lines().filter(|line| line.trim() != "databases/").collect();
        brok_core::fsutil::write_atomic(&gitignore, format!("{}\n", kept.join("\n")).as_bytes())?;
    }
    ui::success("Tracking .brokspace/databases/** with Git LFS (commit .gitattributes)");
    Ok(())
}
