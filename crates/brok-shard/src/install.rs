//! `brok shard install`.
//!
//! 1. fetch the sources (shallow `git clone`, or a local directory),
//! 2. load and validate `shard.json`, check `brok` version compatibility,
//! 3. show the requested capabilities and ask for consent,
//! 4. obtain the executable: checksum-pinned release for this platform,
//!    else the manifest's build command, else a prebuilt entrypoint,
//! 5. atomically move the result into `.brokspace/shards/<name>/` and record
//!    its provenance in `shards.lock`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use brok_sdk::ShardManifest;
use brok_sdk::manifest::{MANIFEST_FILE, current_platform};
use futures::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::registry::{InstalledShard, LockEntry, Registry, ShardSource};

/// Version of `brok` checked against manifests' `brok` requirement.
pub const BROK_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Where to install a shard from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallSource {
    /// A git repository, optionally at a tag or branch.
    Git {
        /// Clone URL.
        url: String,
        /// Tag or branch.
        tag: Option<String>,
    },
    /// A local directory.
    Local(PathBuf),
}

impl InstallSource {
    /// Interprets a CLI argument: an existing directory is a local source,
    /// anything else a git URL.
    pub fn parse(spec: &str, tag: Option<String>) -> Self {
        let path = Path::new(spec);
        if path.is_dir() { Self::Local(path.to_path_buf()) } else { Self::Git { url: spec.to_owned(), tag } }
    }
}

/// Install options.
#[derive(Debug, Clone, Copy, Default)]
pub struct InstallOptions {
    /// Reinstall even if the same or a newer version is installed.
    pub force: bool,
}

/// Asks the user to approve a shard's capabilities.
pub trait Consent {
    /// Returns `true` to proceed. `report` lists capabilities and actions.
    fn approve(&self, manifest: &ShardManifest, report: &[String]) -> bool;
}

/// Approves everything (`--yes`).
#[derive(Debug, Clone, Copy)]
pub struct AutoApprove;

impl Consent for AutoApprove {
    fn approve(&self, _: &ShardManifest, _: &[String]) -> bool {
        true
    }
}

/// How the executable was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMethod {
    /// Downloaded release asset (checksum verified).
    Release,
    /// Built from source.
    Build,
    /// Shipped prebuilt in the repository.
    Prebuilt,
    /// Skill-only shard: nothing to execute.
    SkillsOnly,
}

/// Result of a successful install.
#[derive(Debug, Clone)]
pub struct InstallOutcome {
    /// The installed shard.
    pub shard: InstalledShard,
    /// Previously installed version, if any.
    pub replaced: Option<semver::Version>,
    /// How the executable was obtained.
    pub method: InstallMethod,
}

/// Short random directory name: staging paths must stay well under Windows'
/// path length limit even in deep workspaces.
fn staging_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_owned()
}

/// Deletes the staging directory on every exit path.
struct Staging(PathBuf);

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Installs a shard. `log` receives human readable progress lines.
pub async fn install(
    registry: &Registry,
    source: InstallSource,
    options: InstallOptions,
    consent: &dyn Consent,
    log: &(dyn Fn(&str) + Sync),
) -> Result<InstallOutcome> {
    let staging = Staging(registry.staging_dir().join(staging_id()));
    std::fs::create_dir_all(&staging.0).with_context(|| format!("creating {}", staging.0.display()))?;

    let (src, provenance) = match &source {
        InstallSource::Git { url, tag } => {
            validate_git_url(url)?;
            let checkout = staging.0.join("src");
            log(&format!("cloning {url}{}", tag.as_deref().map(|t| format!(" ({t})")).unwrap_or_default()));
            clone(url, tag.as_deref(), &checkout).await?;
            let rev = git_output(&checkout, &["rev-parse", "HEAD"]).await?;
            (checkout, ShardSource::Git { url: url.clone(), tag: tag.clone(), rev })
        }
        InstallSource::Local(path) => {
            let path = brok_core::workspace::dunce_canonicalize(path)?;
            let provenance = ShardSource::Local { path: path.to_string_lossy().into_owned() };
            (path, provenance)
        }
    };

    let manifest = ShardManifest::load_from_dir(&src)?;
    if let Some(requirement) = &manifest.brok {
        let current = semver::Version::parse(BROK_VERSION)?;
        ensure!(
            requirement.matches(&current),
            "{} {} requires brok {requirement}, this is brok {current}",
            manifest.name,
            manifest.version
        );
    }

    let existing = registry.get(&manifest.name)?;
    if let Some(existing) = &existing
        && existing.manifest.version >= manifest.version
        && !options.force
    {
        bail!(
            "{} {} is already installed (found {}); use --force to reinstall",
            manifest.name,
            existing.manifest.version,
            manifest.version
        );
    }

    let plan = plan(&manifest, &src);
    let mut report = manifest.capability_report();
    report.push(match &plan {
        Plan::Release(asset) => format!("will download:   {}", asset.url),
        Plan::Build(command) => format!("will run:        {} (in {})", command.join(" "), src.display()),
        Plan::Prebuilt(path) => format!("will copy:       {}", path.display()),
        Plan::SkillsOnly => format!("will copy:       skills from {}", src.display()),
        Plan::Missing => "no executable available for this platform".to_owned(),
    });
    if !consent.approve(&manifest, &report) {
        bail!("installation cancelled");
    }

    let prepared = staging.0.join("install");
    std::fs::create_dir_all(&prepared)?;
    std::fs::copy(src.join(MANIFEST_FILE), prepared.join(MANIFEST_FILE))?;
    for dir in &manifest.skills {
        let from = src.join(dir);
        ensure!(from.is_dir(), "skills directory `{dir}` declared in shard.json does not exist");
        brok_core::fsutil::copy_dir(&from, &prepared.join(dir))?;
    }

    let executable = manifest.entrypoint_path(&prepared);
    if let Some(executable) = &executable {
        std::fs::create_dir_all(executable.parent().unwrap_or(&prepared))?;
    }
    let method = match (plan, &executable) {
        (Plan::Release(asset), Some(executable)) => {
            log(&format!("downloading {}", asset.url));
            download_verified(&asset.url, &asset.sha256, executable).await?;
            InstallMethod::Release
        }
        (Plan::Build(command), Some(executable)) => {
            log(&format!("building with `{}`", command.join(" ")));
            run_build(&command, &src).await?;
            let artifact = with_exe_suffix(src.join(&manifest.build.as_ref().expect("build plan").artifact));
            std::fs::copy(&artifact, executable)
                .with_context(|| format!("build did not produce {}", artifact.display()))?;
            InstallMethod::Build
        }
        (Plan::Prebuilt(path), Some(executable)) => {
            std::fs::copy(&path, executable)?;
            InstallMethod::Prebuilt
        }
        (Plan::SkillsOnly, _) => InstallMethod::SkillsOnly,
        _ => bail!(
            "{} has no release for {}, no build command and no prebuilt `{}`",
            manifest.name,
            current_platform(),
            manifest.entrypoint.as_deref().unwrap_or_default()
        ),
    };

    let sha256 = match &executable {
        Some(executable) => {
            make_executable(executable)?;
            hex::encode(Sha256::digest(std::fs::read(executable)?))
        }
        None => content_hash(&prepared)?,
    };
    let entry = LockEntry::new(manifest.version.clone(), provenance, sha256);
    let shard = registry.commit(&prepared, manifest, entry)?;
    Ok(InstallOutcome { shard, replaced: existing.map(|e| e.manifest.version), method })
}

/// SHA-256 over every file of a directory, in path order (skill-only shards).
fn content_hash(dir: &Path) -> Result<String> {
    fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() { collect(&path, out)? } else { out.push(path) }
        }
        Ok(())
    }
    let mut files = Vec::new();
    collect(dir, &mut files)?;
    files.sort();
    let mut hasher = Sha256::new();
    for file in files {
        hasher.update(file.strip_prefix(dir).unwrap_or(&file).to_string_lossy().replace('\\', "/").as_bytes());
        hasher.update(std::fs::read(&file)?);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Installs a locally available executable as the given manifest (used by
/// `brok init` for the bundled system agent).
pub fn install_builtin(registry: &Registry, manifest: ShardManifest, binary: &Path) -> Result<InstalledShard> {
    manifest.validate()?;
    let staging = Staging(registry.staging_dir().join(staging_id()));
    let prepared = staging.0.join("install");
    let executable = manifest.entrypoint_path(&prepared).context("a builtin shard must have an entrypoint")?;
    std::fs::create_dir_all(executable.parent().unwrap_or(&prepared))?;
    std::fs::write(prepared.join(MANIFEST_FILE), manifest.to_json_pretty())?;
    std::fs::copy(binary, &executable).with_context(|| format!("copying {}", binary.display()))?;
    make_executable(&executable)?;
    let sha256 = hex::encode(Sha256::digest(std::fs::read(&executable)?));
    let entry = LockEntry::new(manifest.version.clone(), ShardSource::Builtin, sha256);
    registry.commit(&prepared, manifest, entry)
}

enum Plan<'m> {
    Release(&'m brok_sdk::manifest::ReleaseAsset),
    Build(Vec<String>),
    Prebuilt(PathBuf),
    SkillsOnly,
    Missing,
}

fn plan<'m>(manifest: &'m ShardManifest, src: &Path) -> Plan<'m> {
    let Some(prebuilt) = manifest.entrypoint_path(src) else { return Plan::SkillsOnly };
    if let Some(asset) = manifest.releases.get(&current_platform()) {
        return Plan::Release(asset);
    }
    if let Some(build) = &manifest.build {
        return Plan::Build(build.command.clone());
    }
    if prebuilt.is_file() { Plan::Prebuilt(prebuilt) } else { Plan::Missing }
}

/// Only remote transports that authenticate the server, plus local paths.
fn validate_git_url(url: &str) -> Result<()> {
    ensure!(!url.starts_with('-'), "invalid repository URL `{url}`");
    let allowed = ["https://", "ssh://", "git@", "file://"];
    ensure!(
        allowed.iter().any(|prefix| url.starts_with(prefix)),
        "unsupported repository URL `{url}` (use https://, ssh://, git@host:path or a local directory)"
    );
    Ok(())
}

async fn clone(url: &str, tag: Option<&str>, into: &Path) -> Result<()> {
    let mut command = tokio::process::Command::new("git");
    // Deep workspaces exceed Windows' 260 character limit inside .git.
    command.args(["-c", "core.longpaths=true", "clone", "--depth", "1", "--quiet"]);
    if let Some(tag) = tag {
        ensure!(!tag.starts_with('-'), "invalid tag `{tag}`");
        command.args(["--branch", tag]);
    }
    let output = command.arg("--").arg(url).arg(into).output().await.context("cannot run git")?;
    ensure!(output.status.success(), "git clone failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    Ok(())
}

async fn git_output(dir: &Path, args: &[&str]) -> Result<String> {
    let output = tokio::process::Command::new("git").arg("-C").arg(dir).args(args).output().await?;
    ensure!(output.status.success(), "git {} failed", args.join(" "));
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

async fn run_build(command: &[String], dir: &Path) -> Result<()> {
    let (program, args) = command.split_first().context("empty build command")?;
    let status = tokio::process::Command::new(program)
        .args(args)
        .current_dir(dir)
        .status()
        .await
        .with_context(|| format!("cannot run `{program}`"))?;
    ensure!(status.success(), "build command failed ({status})");
    Ok(())
}

/// Downloads `url` to `target` (through `target.part`), failing unless its
/// SHA-256 matches. `target` is only written once the checksum is good.
pub async fn download_verified(url: &str, expected_sha256: &str, target: &Path) -> Result<()> {
    let response = reqwest::get(url).await?.error_for_status()?;
    let partial = target.with_extension("part");
    let mut file = tokio::fs::File::create(&partial).await?;
    let mut hasher = Sha256::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
    }
    file.sync_all().await?;
    drop(file);
    let actual = hex::encode(hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected_sha256) {
        let _ = tokio::fs::remove_file(&partial).await;
        bail!("checksum mismatch for {url}: expected {expected_sha256}, got {actual}");
    }
    tokio::fs::rename(&partial, target).await?;
    Ok(())
}

fn with_exe_suffix(path: PathBuf) -> PathBuf {
    if cfg!(windows) && path.extension().is_none() { path.with_extension("exe") } else { path }
}

fn make_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_url_policy() {
        assert!(validate_git_url("https://github.com/brokspace/brok-agent").is_ok());
        assert!(validate_git_url("git@github.com:brokspace/brok-agent.git").is_ok());
        assert!(validate_git_url("http://insecure.example/x").is_err());
        assert!(validate_git_url("--upload-pack=evil").is_err());
    }
}
