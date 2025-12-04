//! AoE (ATA over Ethernet) Server with pluggable storage backends
//!
//! This crate implements an AoE server that presents block devices over Ethernet.
//! It supports multiple storage backends including simple files and content-addressed
//! storage (CAS) with automatic deduplication.

pub mod blob;
pub mod cas;
pub mod config;
pub mod iscsi;
pub mod nbd;
pub mod protocol;
pub mod server;
pub mod storage;

pub use cas::{CasServer, CasServerConfig};
pub use config::Config;
pub use protocol::AoeError;
pub use storage::{BlockStorage, DeviceInfo, StorageError};
