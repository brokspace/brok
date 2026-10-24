//! Shards: standalone mini-agents that join the Brokspace event bus.
//!
//! - [`registry`]: what is installed in `.brokspace/shards/`, and from where;
//! - [`install`]: git/local installation with capability consent and
//!   checksum-verified releases;
//! - [`sandbox`]: the scrubbed environment shards run in;
//! - [`supervisor`]: process lifecycle for `brok run`.

#![warn(missing_docs)]

pub mod install;
pub mod registry;
pub mod sandbox;
pub mod supervisor;

pub use install::{AutoApprove, Consent, InstallMethod, InstallOptions, InstallOutcome, InstallSource, install};
pub use registry::{InstalledShard, LockEntry, Registry, ShardSource};
pub use sandbox::SandboxEnv;
pub use supervisor::{LaunchSpec, supervise};

/// Name of the default system shard.
pub const SYSTEM_AGENT: &str = "brok-agent";

/// Finds an executable named `name` next to the running `brok` binary or
/// on `PATH`.
pub fn find_executable(name: &str) -> Option<std::path::PathBuf> {
    let file = if cfg!(windows) { format!("{name}.exe") } else { name.to_owned() };
    let beside = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join(&file)));
    beside
        .into_iter()
        .chain(
            std::env::var_os("PATH")
                .into_iter()
                .flat_map(|path| std::env::split_paths(&path).map(|dir| dir.join(&file)).collect::<Vec<_>>()),
        )
        .find(|candidate| candidate.is_file())
}
