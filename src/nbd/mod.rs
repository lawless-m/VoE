//! NBD (Network Block Device) server implementation
//!
//! NBD protocol is simpler than iSCSI and works well with our CAS backend.
//! Linux has native NBD support, Windows needs third-party drivers.

pub mod protocol;
pub mod server;

pub use server::{NbdServer, NbdServerConfig};
