//! AoE server implementation
//!
//! Contains the network listener and target manager.

mod listener;
mod target;

pub use listener::AoeListener;
pub use target::TargetManager;
