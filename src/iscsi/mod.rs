//! iSCSI target implementation
//!
//! Implements RFC 3720 iSCSI protocol for Windows/Linux block storage access.

pub mod cas_device;
pub mod pdu;
// pub mod session;  // TODO: Update to use BlockStorage trait methods
// pub mod target;  // TODO: Implement iSCSI target

pub use cas_device::{CasScsiDevice, CasScsiDeviceConfig};
// pub use target::{IscsiTarget, IscsiTargetConfig};
