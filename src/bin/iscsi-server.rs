//! iSCSI server binary
//!
//! iSCSI target server backed by CAS storage

use clap::Parser;
use env_logger::Env;
use std::path::PathBuf;
use std::process;

use aoe_server::iscsi::{CasScsiDevice, CasScsiDeviceConfig};
use iscsi_target::IscsiTarget;

#[derive(Parser, Debug)]
#[command(name = "iscsi-server")]
#[command(about = "iSCSI target with CAS backend", long_about = None)]
struct Args {
    /// Bind address (e.g., 0.0.0.0:3260)
    #[arg(short, long, default_value = "0.0.0.0:3260")]
    bind: String,

    /// CAS server address
    #[arg(short, long, default_value = "127.0.0.1:3000")]
    cas_server: String,

    /// Device size in MB
    #[arg(short, long, default_value = "100")]
    size: u64,

    /// LBA index file path
    #[arg(short, long, default_value = "/var/lib/voe-iscsi/index.json")]
    index: PathBuf,

    /// iSCSI target name (IQN)
    #[arg(short, long, default_value = "iqn.2025-12.local.voe:storage.cas-disk")]
    target: String,
}

fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    log::info!("Starting iSCSI target server");
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
