//! Process sandbox derived from a shard manifest.
//!
//! What is **enforced** today:
//!
//! - topics: the bus rejects anything outside `subscribes_to` / `publishes_to`;
//! - environment: the shard starts from an empty environment plus a small
//!   allow-list of OS basics and the variables its manifest requests, so
//!   secrets in the user's shell never leak by accident;
//! - working directory: the workspace root.
//!
//! What is **declared** and exported to the shard (`BROK_FS_READ`,
//! `BROK_FS_WRITE`, `BROK_NETWORK`) but not yet enforced by the OS:
//! file system and network access. OS-level enforcement (Landlock on Linux,
//! Seatbelt on macOS, AppContainer on Windows) is on the roadmap; until then
//! consent at install time is the guard, which is why `brok shard install`
//! always shows the requested capabilities.

use std::ffi::OsString;
use std::path::Path;

use brok_sdk::transport::Endpoint;
use brok_sdk::{ShardManifest, env};

/// Variables every process needs to function, forwarded when present.
const BASE_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LANG",
    "LC_ALL",
    "TERM",
    "TMPDIR",
    "TEMP",
    "TMP",
    "TZ",
    "RUST_LOG",
    "NO_COLOR",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "XDG_RUNTIME_DIR",
    "XDG_CACHE_HOME",
    // Windows
    "SYSTEMROOT",
    "WINDIR",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "PATHEXT",
    "COMSPEC",
    "PROGRAMDATA",
];

/// The environment a shard process starts with.
#[derive(Clone, PartialEq, Eq)]
pub struct SandboxEnv {
    vars: Vec<(OsString, OsString)>,
}

impl std::fmt::Debug for SandboxEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Values may hold secrets: only show names.
        f.debug_list().entries(self.vars.iter().map(|(k, _)| k)).finish()
    }
}

impl SandboxEnv {
    /// Builds the environment for `manifest` from the current process
    /// environment.
    pub fn new(manifest: &ShardManifest, workspace_root: &Path, endpoint: &Endpoint, token: &str) -> Self {
        Self::from_vars(manifest, workspace_root, endpoint, token, std::env::vars_os())
    }

    /// Same as [`SandboxEnv::new`] with an explicit source environment.
    pub fn from_vars(
        manifest: &ShardManifest,
        workspace_root: &Path,
        endpoint: &Endpoint,
        token: &str,
        source: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Self {
        let requested = &manifest.permissions.env;
        let mut vars: Vec<(OsString, OsString)> = source
            .into_iter()
            .filter(|(name, _)| {
                let Some(name) = name.to_str() else { return false };
                let upper = name.to_ascii_uppercase();
                if upper.starts_with(env::RESERVED_PREFIX) {
                    return false;
                }
                BASE_ENV.iter().any(|base| base.eq_ignore_ascii_case(name))
                    || requested.iter().any(|pattern| matches_pattern(&pattern.to_ascii_uppercase(), &upper))
            })
            .collect();

        let join = |paths: &[String]| paths.join(";");
        let permissions = &manifest.permissions;
        vars.extend(
            [
                (env::ENDPOINT, endpoint.as_str().to_owned()),
                (env::TOKEN, token.to_owned()),
                (env::SHARD_NAME, manifest.name.clone()),
                (env::WORKSPACE, workspace_root.to_string_lossy().into_owned()),
                (env::FS_READ, join(&permissions.filesystem.read)),
                (env::FS_WRITE, join(&permissions.filesystem.write)),
                (env::NETWORK, if permissions.network { "1" } else { "0" }.to_owned()),
            ]
            .map(|(k, v)| (OsString::from(k), OsString::from(v))),
        );
        Self { vars }
    }

    /// Applies the environment to a command (clearing everything else).
    pub fn apply(&self, command: &mut tokio::process::Command) {
        command.env_clear().envs(self.vars.iter().map(|(k, v)| (k, v)));
    }

    /// Names of the variables passed through.
    pub fn names(&self) -> impl Iterator<Item = String> + '_ {
        self.vars.iter().map(|(k, _)| k.to_string_lossy().into_owned())
    }
}

/// `*_API_KEY` (suffix), `OPENAI_*` (prefix) or an exact name.
fn matches_pattern(pattern: &str, name: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix('*') {
        name.starts_with(prefix)
    } else if let Some(suffix) = pattern.strip_prefix('*') {
        name.ends_with(suffix)
    } else {
        pattern == name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(env: &[&str]) -> ShardManifest {
        let json = serde_json::json!({
            "name": "demo-shard",
            "version": "0.1.0",
            "entrypoint": "bin/demo",
            "permissions": { "env": env, "network": true, "filesystem": { "read": ["src/", "docs/"] } },
        });
        ShardManifest::from_json(&json.to_string()).unwrap()
    }

    fn source() -> Vec<(OsString, OsString)> {
        [
            ("PATH", "/bin"),
            ("ANTHROPIC_API_KEY", "a"),
            ("OPENAI_BASE_URL", "b"),
            ("AWS_SECRET_ACCESS_KEY", "c"),
            ("BROK_TOKEN", "parent-token"),
            ("GITHUB_TOKEN", "d"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect()
    }

    #[test]
    fn scrubs_everything_not_requested() {
        let env = SandboxEnv::from_vars(
            &manifest(&["*_API_KEY", "OPENAI_*"]),
            Path::new("/ws"),
            &Endpoint::new("/ws/.brokspace/run/brok.sock"),
            "tok",
            source(),
        );
        let names: Vec<String> = env.names().collect();
        for expected in ["PATH", "ANTHROPIC_API_KEY", "OPENAI_BASE_URL", "BROK_TOKEN", "BROK_FS_READ"] {
            assert!(names.contains(&expected.to_owned()), "{expected} missing from {names:?}");
        }
        for leaked in ["AWS_SECRET_ACCESS_KEY", "GITHUB_TOKEN"] {
            assert!(!names.contains(&leaked.to_owned()), "{leaked} leaked");
        }
        let token = env.vars.iter().find(|(k, _)| k == "BROK_TOKEN").unwrap();
        assert_eq!(token.1, "tok", "the parent's BROK_* variables are never inherited");
        let read = env.vars.iter().find(|(k, _)| k == "BROK_FS_READ").unwrap();
        assert_eq!(read.1, "src/;docs/");
        assert!(!format!("{env:?}").contains("tok\""));
    }

    #[test]
    fn patterns() {
        assert!(matches_pattern("*_API_KEY", "OPENAI_API_KEY"));
        assert!(matches_pattern("OPENAI_*", "OPENAI_ORG"));
        assert!(matches_pattern("HTTPS_PROXY", "HTTPS_PROXY"));
        assert!(!matches_pattern("HTTPS_PROXY", "HTTP_PROXY"));
    }
}
