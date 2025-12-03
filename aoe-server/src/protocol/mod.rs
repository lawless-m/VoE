//! AoE protocol handling
//!
//! This module implements parsing and building of AoE frames,
//! as well as ATA command handling.

mod ata;
mod build;
mod parse;
mod types;

pub use ata::{handle_ata_command, AtaResponse};
pub use build::{build_response, ConfigResponse, ResponseData};
pub use parse::{parse_frame, ParseError};
pub use types::*;

use thiserror::Error;

/// AoE protocol errors
#[derive(Debug, Error)]
pub enum AoeError {
    #[error("parse error: {0}")]
    Parse(#[from] ParseError),

    #[error("unrecognized command code: {0}")]
    UnrecognizedCommand(u8),

    #[error("bad argument: {0}")]
    BadArgument(String),

    #[error("device unavailable")]
    DeviceUnavailable,

    #[error("config string present")]
    ConfigStringPresent,

    #[error("unsupported version: {0}")]
    UnsupportedVersion(u8),

    #[error("target reserved")]
    TargetReserved,

    #[error("storage error: {0}")]
    Storage(#[from] crate::storage::StorageError),
}

impl AoeError {
    /// Convert to AoE error code for response
    pub fn to_error_code(&self) -> u8 {
        match self {
            AoeError::Parse(_) => 2,
            AoeError::UnrecognizedCommand(_) => 1,
            AoeError::BadArgument(_) => 2,
            AoeError::DeviceUnavailable => 3,
            AoeError::ConfigStringPresent => 4,
            AoeError::UnsupportedVersion(_) => 5,
            AoeError::TargetReserved => 6,
            AoeError::Storage(_) => 3,
        }
    }
}
