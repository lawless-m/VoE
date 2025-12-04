//! Content-Addressable Storage (CAS) module
//!
//! Provides a standalone CAS service with a simple TCP protocol.

pub mod protocol;
pub mod storage;
pub mod server;

pub use protocol::{CasCommand, CasResponse};
pub use storage::CasStorage;
pub use server::{CasServer, CasServerConfig};

/// Hash type used for content addressing (SHA-256)
pub type Hash = [u8; 32];
