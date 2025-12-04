//! Target manager
//!
//! Maps shelf/slot addresses to storage backends and handles frame routing.

use crate::protocol::{
    handle_ata_command, AoeCommand, AoeError, AoeFrame, AoePayload,
    ConfigResponse, ResponseData, BROADCAST_SHELF, BROADCAST_SLOT,
    MAX_SECTORS_STANDARD,
};
use crate::storage::BlockStorage;
use std::collections::HashMap;

/// Target address (shelf, slot)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TargetAddr {
    pub shelf: u16,
    pub slot: u8,
}

impl TargetAddr {
    pub fn new(shelf: u16, slot: u8) -> Self {
        Self { shelf, slot }
    }
}

/// A storage target
pub struct Target {
    #[allow(dead_code)]
    pub addr: TargetAddr,
    pub storage: Box<dyn BlockStorage>,
    pub config_string: String,
}

/// Manages multiple storage targets
pub struct TargetManager {
    targets: HashMap<TargetAddr, Target>,
    firmware_version: u16,
}

impl TargetManager {
    /// Create a new target manager
    pub fn new() -> Self {
        Self {
            targets: HashMap::new(),
            firmware_version: 0x4019, // Match vblade's firmware version
        }
    }

    /// Add a target
    pub fn add_target(
        &mut self,
        shelf: u16,
        slot: u8,
        storage: Box<dyn BlockStorage>,
        config_string: String,
    ) {
        let addr = TargetAddr::new(shelf, slot);
        self.targets.insert(
            addr,
            Target {
                addr,
                storage,
                config_string,
            },
        );
        log::info!("Added target at shelf {} slot {}", shelf, slot);
    }

    /// Handle an AoE frame, returning responses for matching targets
    /// Returns (target_address, response_data) pairs
    pub fn handle_frame(&mut self, frame: &AoeFrame) -> Result<Vec<(TargetAddr, ResponseData)>, AoeError> {
        let mut responses = Vec::new();

        // Find matching targets
        let matching: Vec<TargetAddr> = self
            .targets
            .keys()
            .filter(|addr| self.address_matches(frame, addr))
            .copied()
            .collect();

        if matching.is_empty() {
            // No matching targets - don't respond
            return Ok(responses);
        }

        for addr in matching {
            let response = self.handle_target_frame(frame, addr)?;
            responses.push((addr, response));
        }

        Ok(responses)
    }

    /// Check if a frame addresses a specific target
    fn address_matches(&self, frame: &AoeFrame, addr: &TargetAddr) -> bool {
        let shelf_match = frame.header.shelf == addr.shelf
            || frame.header.shelf == BROADCAST_SHELF;
        let slot_match = frame.header.slot == addr.slot
            || frame.header.slot == BROADCAST_SLOT;
        shelf_match && slot_match
    }

    /// Handle a frame for a specific target
    fn handle_target_frame(
        &mut self,
        frame: &AoeFrame,
        addr: TargetAddr,
    ) -> Result<ResponseData, AoeError> {
        match frame.header.command {
            AoeCommand::Ata => self.handle_ata(frame, addr),
            AoeCommand::Config => self.handle_config(frame, addr),
        }
    }

    /// Handle an ATA command
    fn handle_ata(
        &mut self,
        frame: &AoeFrame,
        addr: TargetAddr,
    ) -> Result<ResponseData, AoeError> {
        let target = self
            .targets
            .get_mut(&addr)
            .ok_or(AoeError::DeviceUnavailable)?;

        let (header, data) = match &frame.payload {
            AoePayload::Ata { header, data } => (header, data),
            _ => return Err(AoeError::BadArgument("expected ATA payload".to_string())),
        };

        let response = handle_ata_command(target.storage.as_mut(), header, data);
        Ok(ResponseData::Ata(response))
    }

    /// Handle a Config command
    fn handle_config(
        &self,
        frame: &AoeFrame,
        addr: TargetAddr,
    ) -> Result<ResponseData, AoeError> {
        let target = self.targets.get(&addr).ok_or(AoeError::DeviceUnavailable)?;

        let config_header = match &frame.payload {
            AoePayload::Config(header) => header,
            _ => {
                return Err(AoeError::BadArgument(
                    "expected Config payload".to_string(),
                ))
            }
        };

        // Handle config commands
        let ccmd = config_header.config_command().map_err(|c| {
            AoeError::UnrecognizedCommand(c)
        })?;

        use crate::protocol::ConfigCommand;
        match ccmd {
            ConfigCommand::Read => {
                // Return our config string
                log::debug!("Config Read: responding with config_string='{}'", target.config_string);
                Ok(ResponseData::Config(ConfigResponse {
                    buffer_count: 16, // Match vblade - number of outstanding requests we can handle
                    firmware_version: self.firmware_version,
                    sector_count: MAX_SECTORS_STANDARD,
                    config_string: target.config_string.as_bytes().to_vec(),
                }))
            }
            ConfigCommand::TestExact => {
                // Test if config string matches exactly
                if config_header.config_string == target.config_string.as_bytes() {
                    Ok(ResponseData::Config(ConfigResponse {
                        buffer_count: 16, // Match vblade - number of outstanding requests we can handle
                        firmware_version: self.firmware_version,
                        sector_count: MAX_SECTORS_STANDARD,
                        config_string: target.config_string.as_bytes().to_vec(),
                    }))
                } else {
                    // Don't respond if no match
                    Err(AoeError::DeviceUnavailable)
                }
            }
            ConfigCommand::TestPrefix => {
                // Test if config string is a prefix
                if target
                    .config_string
                    .as_bytes()
                    .starts_with(&config_header.config_string)
                {
                    Ok(ResponseData::Config(ConfigResponse {
                        buffer_count: 16, // Match vblade - number of outstanding requests we can handle
                        firmware_version: self.firmware_version,
                        sector_count: MAX_SECTORS_STANDARD,
                        config_string: target.config_string.as_bytes().to_vec(),
                    }))
                } else {
                    Err(AoeError::DeviceUnavailable)
                }
            }
            ConfigCommand::Set | ConfigCommand::ForceSet => {
                // We don't support changing config string
                Err(AoeError::ConfigStringPresent)
            }
        }
    }

    /// Get number of targets
    pub fn target_count(&self) -> usize {
        self.targets.len()
    }
}

impl Default for TargetManager {
    fn default() -> Self {
        Self::new()
    }
}
