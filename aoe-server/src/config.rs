//! Configuration file parsing
//!
//! Parses TOML configuration files for the AoE server.

use serde::Deserialize;
use std::path::Path;
use thiserror::Error;

/// Configuration errors
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TOML parse error: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("invalid configuration: {0}")]
    Invalid(String),
}

/// Server configuration
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Server settings
    pub server: ServerConfig,

    /// Target configurations
    #[serde(default)]
    pub target: Vec<TargetConfig>,
}

/// Server settings
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// Network interface to listen on
    pub interface: String,

    /// Log level (trace, debug, info, warn, error)
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

fn default_log_level() -> String {
    "info".to_string()
}

/// Target configuration
#[derive(Debug, Clone, Deserialize)]
pub struct TargetConfig {
    /// Shelf address (0-65534)
    pub shelf: u16,

    /// Slot address (0-254)
    pub slot: u8,

    /// Backend type
    pub backend: BackendType,

    /// File backend settings
    #[serde(default)]
    pub file: Option<FileBackendConfig>,

    /// CAS backend settings
    #[serde(default)]
    pub cas: Option<CasBackendConfig>,

    /// Config string for discovery
    #[serde(default)]
    pub config_string: String,
}

/// Backend type
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BackendType {
    File,
    Cas,
}

/// File backend configuration
#[derive(Debug, Clone, Deserialize)]
pub struct FileBackendConfig {
    /// Path to the file
    pub path: String,

    /// Size in bytes (for creation)
    pub size: Option<u64>,
}

/// CAS backend configuration
#[derive(Debug, Clone, Deserialize)]
pub struct CasBackendConfig {
    /// Block size in bytes
    #[serde(default = "default_block_size")]
    pub block_size: u32,

    /// Total sectors
    pub total_sectors: u64,

    /// Blob store configuration
    pub blob_store: BlobStoreConfig,
}

fn default_block_size() -> u32 {
    4096
}

/// Blob store configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum BlobStoreConfig {
    /// File-based blob store
    File {
        /// Directory path
        path: String,
    },
    // Future: S3, Azure, etc.
}

impl Config {
    /// Load configuration from a file
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    /// Parse configuration from a string
    pub fn parse(content: &str) -> Result<Self, ConfigError> {
        let config: Config = toml::from_str(content)?;
        config.validate()?;
        Ok(config)
    }

    /// Validate the configuration
    fn validate(&self) -> Result<(), ConfigError> {
        // Check for duplicate shelf/slot
        let mut seen = std::collections::HashSet::new();
        for target in &self.target {
            let key = (target.shelf, target.slot);
            if !seen.insert(key) {
                return Err(ConfigError::Invalid(format!(
                    "duplicate target: shelf {} slot {}",
                    target.shelf, target.slot
                )));
            }

            // Validate backend config
            match target.backend {
                BackendType::File => {
                    if target.file.is_none() {
                        return Err(ConfigError::Invalid(format!(
                            "file backend requires [target.file] section for shelf {} slot {}",
                            target.shelf, target.slot
                        )));
                    }
                }
                BackendType::Cas => {
                    if target.cas.is_none() {
                        return Err(ConfigError::Invalid(format!(
                            "cas backend requires [target.cas] section for shelf {} slot {}",
                            target.shelf, target.slot
                        )));
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_config() {
        let config_str = r#"
[server]
interface = "eth0"

[[target]]
shelf = 1
slot = 0
backend = "file"

[target.file]
path = "/data/disk.img"
"#;

        let config = Config::parse(config_str).unwrap();
        assert_eq!(config.server.interface, "eth0");
        assert_eq!(config.target.len(), 1);
        assert_eq!(config.target[0].shelf, 1);
        assert_eq!(config.target[0].slot, 0);
    }

    #[test]
    fn test_parse_cas_config() {
        let config_str = r#"
[server]
interface = "eth0"
log_level = "debug"

[[target]]
shelf = 1
slot = 0
backend = "cas"
config_string = "archive-1"

[target.cas]
block_size = 4096
total_sectors = 2097152

[target.cas.blob_store]
type = "file"
path = "/data/blobs"
"#;

        let config = Config::parse(config_str).unwrap();
        assert_eq!(config.server.log_level, "debug");
        assert_eq!(config.target[0].backend, BackendType::Cas);
        let cas = config.target[0].cas.as_ref().unwrap();
        assert_eq!(cas.total_sectors, 2097152);
    }

    #[test]
    fn test_duplicate_target_error() {
        let config_str = r#"
[server]
interface = "eth0"

[[target]]
shelf = 1
slot = 0
backend = "file"

[target.file]
path = "/data/disk1.img"

[[target]]
shelf = 1
slot = 0
backend = "file"

[target.file]
path = "/data/disk2.img"
"#;

        let result = Config::parse(config_str);
        assert!(matches!(result, Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn test_missing_backend_config_error() {
        let config_str = r#"
[server]
interface = "eth0"

[[target]]
shelf = 1
slot = 0
backend = "file"
"#;

        let result = Config::parse(config_str);
        assert!(matches!(result, Err(ConfigError::Invalid(_))));
    }
}
