//! Core building blocks of Brokspace: workspace discovery, the `.brokspace/`
//! layout, `brok.yml` configuration, the hot/cold partition topology,
//! memories, linked workspaces and local metrics.

#![warn(missing_docs)]

pub mod config;
mod error;
pub mod fsutil;
pub mod git;
pub mod init;
pub mod links;
pub mod memory;
pub mod metrics;
pub mod partition;
pub mod user;
pub mod workspace;

pub use config::BrokConfig;
pub use error::{CoreError, Result};
pub use links::Link;
pub use memory::{Memory, MemoryDirs};
pub use partition::{Partition, PartitionMap, Tier};
pub use workspace::{Workspace, WorkspacePaths};
