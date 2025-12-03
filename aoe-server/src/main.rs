//! AoE Server - ATA over Ethernet server with pluggable storage backends
//!
//! Usage:
//!   aoe-server [OPTIONS] <CONFIG>
//!
//! Example:
//!   aoe-server /etc/aoe-server.toml

use aoe_server::blob::FileBlobStore;
use aoe_server::config::{BackendType, BlobStoreConfig, Config};
use aoe_server::server::{AoeListener, TargetManager};
use aoe_server::storage::{CasBackend, FileBackend};
use aoe_server::BlockStorage;
use anyhow::{Context, Result};
use std::env;
use std::path::Path;

fn main() -> Result<()> {
    // Parse command line arguments
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <CONFIG>", args[0]);
        eprintln!();
        eprintln!("Arguments:");
        eprintln!("  CONFIG    Path to configuration file (TOML)");
        eprintln!();
        eprintln!("Environment:");
        eprintln!("  RUST_LOG  Log level (trace, debug, info, warn, error)");
        std::process::exit(1);
    }

    let config_path = &args[1];

    // Load configuration
    let config = Config::load(config_path)
        .with_context(|| format!("failed to load config from {}", config_path))?;

    // Initialize logging
    env_logger::Builder::new()
        .filter_level(parse_log_level(&config.server.log_level))
        .init();

    log::info!("AoE Server v{}", env!("CARGO_PKG_VERSION"));
    log::info!("Loaded configuration from {}", config_path);

    // Create target manager
    let mut targets = TargetManager::new();

    // Initialize backends
    for target_config in &config.target {
        log::info!(
            "Initializing target shelf {} slot {}",
            target_config.shelf,
            target_config.slot
        );

        let storage: Box<dyn aoe_server::BlockStorage> = match target_config.backend {
            BackendType::File => {
                let file_config = target_config
                    .file
                    .as_ref()
                    .expect("file config validated");

                let backend = if let Some(size) = file_config.size {
                    FileBackend::open_or_create(&file_config.path, size)
                        .with_context(|| {
                            format!("failed to create file backend at {}", file_config.path)
                        })?
                } else {
                    FileBackend::open(&file_config.path).with_context(|| {
                        format!("failed to open file backend at {}", file_config.path)
                    })?
                };

                log::info!(
                    "  File backend: {} ({} sectors)",
                    file_config.path,
                    backend.info().total_sectors
                );

                Box::new(backend)
            }
            BackendType::Cas => {
                let cas_config = target_config
                    .cas
                    .as_ref()
                    .expect("cas config validated");

                // Create blob store
                let blob_store: Box<dyn aoe_server::blob::BlobStore> =
                    match &cas_config.blob_store {
                        BlobStoreConfig::File { path } => {
                            std::fs::create_dir_all(path).with_context(|| {
                                format!("failed to create blob store directory: {}", path)
                            })?;
                            Box::new(FileBlobStore::new(path).with_context(|| {
                                format!("failed to create file blob store at {}", path)
                            })?)
                        }
                    };

                // Determine snapshot file path (alongside blob store)
                let snapshot_path = match &cas_config.blob_store {
                    BlobStoreConfig::File { path } => {
                        Path::new(path).parent().unwrap_or(Path::new(".")).join("snapshots.json")
                    }
                };

                let backend = CasBackend::new(
                    blob_store,
                    cas_config.total_sectors,
                    &snapshot_path,
                )
                .with_context(|| {
                    format!(
                        "failed to create CAS backend for shelf {} slot {}",
                        target_config.shelf, target_config.slot
                    )
                })?;

                log::info!(
                    "  CAS backend: {} ({} sectors, snapshots at {})",
                    match &cas_config.blob_store {
                        BlobStoreConfig::File { path } => path.as_str(),
                    },
                    cas_config.total_sectors,
                    snapshot_path.display()
                );

                Box::new(backend)
            }
        };

        targets.add_target(
            target_config.shelf,
            target_config.slot,
            storage,
            target_config.config_string.clone(),
        );
    }

    log::info!(
        "Configured {} target(s) on interface {}",
        targets.target_count(),
        config.server.interface
    );

    // Create and run listener
    let mut listener = AoeListener::new(&config.server.interface, targets)
        .context("failed to create AoE listener")?;

    log::info!("Starting AoE server...");
    listener.run().context("server error")?;

    Ok(())
}

/// Parse log level string
fn parse_log_level(level: &str) -> log::LevelFilter {
    match level.to_lowercase().as_str() {
        "trace" => log::LevelFilter::Trace,
        "debug" => log::LevelFilter::Debug,
        "info" => log::LevelFilter::Info,
        "warn" | "warning" => log::LevelFilter::Warn,
        "error" => log::LevelFilter::Error,
        "off" => log::LevelFilter::Off,
        _ => {
            eprintln!("Unknown log level '{}', defaulting to 'info'", level);
            log::LevelFilter::Info
        }
    }
}
