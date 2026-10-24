//! Core building blocks of Brokspace: workspace discovery, the `.brokspace/`
//! layout, `brok.yml` configuration and the hot/cold partition topology.

#![warn(missing_docs)]

pub mod config;
mod error;
pub mod fsutil;
pub mod git;
pub mod init;
pub mod partition;
pub mod user;
pub mod workspace;

pub use config::BrokConfig;
pub use error::{CoreError, Result};
pub use partition::{Partition, PartitionMap, Tier};
pub use workspace::{Workspace, WorkspacePaths};
