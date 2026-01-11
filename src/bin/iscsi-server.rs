//! iSCSI server binary
//!
//! iSCSI target server backed by CAS storage
//!
//! Supports two modes:
//! 1. Single-target mode (CLI args) - backwards compatible
//! 2. Multi-target mode (TOML config) - new feature

use clap::Parser;
use env_logger::Env;
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use std::process;

use aoe_server::iscsi::{CasScsiDevice, CasScsiDeviceConfig};
use iscsi_target::{IscsiTarget, IscsiServer};

#[derive(Parser, Debug)]
#[command(name = "iscsi-server")]
#[command(about = "iSCSI target with CAS backend", long_about = None)]
struct Args {
    /// Path to TOML configuration file (for multi-target mode)
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Bind address (e.g., 0.0.0.0:3260) [single-target mode]
    #[arg(short, long, default_value = "0.0.0.0:3260")]
    bind: String,

    /// CAS server address [single-target mode]
    #[arg(long, default_value = "127.0.0.1:3000")]
    cas_server: String,

    /// Device size in MB [single-target mode]
    #[arg(short, long, default_value = "100")]
    size: u64,

    /// LBA index database path [single-target mode]
    #[arg(short, long, default_value = "/var/lib/voe-iscsi/index")]
    index: PathBuf,

    /// iSCSI target name (IQN) [single-target mode]
    #[arg(short, long, default_value = "iqn.2025-12.local.voe:storage.cas-disk")]
    target: String,
}

/// TOML configuration for multi-target server
#[derive(Debug, Deserialize)]
struct Config {
    server: ServerConfig,
    targets: Vec<TargetConfig>,
}

#[derive(Debug, Deserialize)]
struct ServerConfig {
    bind: String,
    cas_server: String,
}

#[derive(Debug, Deserialize)]
struct TargetConfig {
    name: String,
    size_mb: u64,
    index_path: PathBuf,
    #[serde(default)]
    alias: Option<String>,
}

fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    if let Some(config_path) = args.config {
        run_multi_target(config_path);
    } else {
        run_single_target(args);
    }
}

/// Run in multi-target mode using TOML configuration
fn run_multi_target(config_path: PathBuf) {
    log::info!("Starting iSCSI server in multi-target mode");
    log::info!("  Config file: {:?}", config_path);

    // Load and parse configuration
    let config_str = match fs::read_to_string(&config_path) {
        Ok(s) => s,
        Err(e) => {
            log::error!("Failed to read config file: {}", e);
            process::exit(1);
        }
    };

    let config: Config = match toml::from_str(&config_str) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to parse config file: {}", e);
            process::exit(1);
        }
    };

    log::info!("  Bind address: {}", config.server.bind);
    log::info!("  CAS server: {}", config.server.cas_server);
    log::info!("  Targets: {}", config.targets.len());

    if config.targets.is_empty() {
        log::error!("No targets defined in configuration");
        process::exit(1);
    }

    // Create multi-target server
    let mut server_builder = IscsiServer::builder()
        .bind_addr(&config.server.bind);

    // Add each target
    for target_config in &config.targets {
        log::info!("  - {} ({} MB)", target_config.name, target_config.size_mb);

        let capacity_blocks = (target_config.size_mb * 1024 * 1024) / 4096;

        let device_config = CasScsiDeviceConfig {
            cas_server_addr: config.server.cas_server.clone(),
            capacity_blocks,
            index_path: target_config.index_path.clone(),
            vendor_id: "VoE     ".to_string(),
            product_id: format!("CAS Disk {:>6}MB", target_config.size_mb),
            product_rev: "1.0 ".to_string(),
        };

        let device = match CasScsiDevice::new(device_config) {
            Ok(device) => device,
            Err(e) => {
                log::error!("Failed to create device for {}: {}", target_config.name, e);
                process::exit(1);
            }
        };

        let alias = target_config.alias.clone();

        server_builder = server_builder.add_target(
            target_config.name.clone(),
            Box::new(device),
            alias,
        );
    }

    let server = match server_builder.build() {
        Ok(s) => s,
        Err(e) => {
            log::error!("Failed to create multi-target server: {}", e);
            process::exit(1);
        }
    };

    log::info!("Multi-target iSCSI server ready, waiting for connections...");

    if let Err(e) = server.run() {
        log::error!("Server error: {}", e);
        process::exit(1);
    }
}

/// Run in single-target mode (backwards compatible with original CLI)
fn run_single_target(args: Args) {
    log::info!("Starting iSCSI server in single-target mode");
    log::info!("  Bind address: {}", args.bind);
    log::info!("  CAS server: {}", args.cas_server);
    log::info!("  Device size: {} MB", args.size);
    log::info!("  Index file: {:?}", args.index);
    log::info!("  Target IQN: {}", args.target);

    // Calculate capacity in blocks (4KB each to match CAS device block size)
    let capacity_blocks = (args.size * 1024 * 1024) / 4096;

    // Create CAS SCSI device
    let device_config = CasScsiDeviceConfig {
        cas_server_addr: args.cas_server,
        capacity_blocks,
        index_path: args.index,
        vendor_id: "VoE     ".to_string(),
        product_id: format!("CAS Disk {:>6}MB", args.size),
        product_rev: "1.0 ".to_string(),
    };

    let device = match CasScsiDevice::new(device_config) {
        Ok(device) => device,
        Err(e) => {
            log::error!("Failed to create CAS SCSI device: {}", e);
            process::exit(1);
        }
    };

    log::info!("CAS SCSI device created successfully");
    log::info!("  Capacity: {} blocks ({} MB)", capacity_blocks, args.size);

    // Create iSCSI target
    let target = match IscsiTarget::builder()
        .bind_addr(&args.bind)
        .target_name(&args.target)
        .build(device)
    {
        Ok(target) => target,
        Err(e) => {
            log::error!("Failed to create iSCSI target: {}", e);
            process::exit(1);
        }
    };

    log::info!("iSCSI target ready, waiting for connections...");

    // Run the target
    if let Err(e) = target.run() {
        log::error!("Target error: {}", e);
        process::exit(1);
    }
}
