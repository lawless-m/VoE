//! NBD server binary
//!
//! Network Block Device server backed by CAS storage

use clap::Parser;
use env_logger::Env;
use std::path::PathBuf;
use std::process;

use aoe_server::nbd::{NbdServer, NbdServerConfig};
use aoe_server::storage::cas_client::{CasBackend, CasBackendConfig};

#[derive(Parser, Debug)]
#[command(name = "nbd-server")]
#[command(about = "NBD server with CAS backend", long_about = None)]
struct Args {
    /// Bind address (e.g., 127.0.0.1:10809)
    #[arg(short, long, default_value = "127.0.0.1:10809")]
    bind: String,

    /// CAS server address
    #[arg(short, long, default_value = "127.0.0.1:3000")]
    cas_server: String,

    /// Device size in MB
    #[arg(short, long, default_value = "100")]
    size: u64,

    /// LBA index file path
    #[arg(short, long, default_value = "/var/lib/nbd-cas/index.json")]
    index: PathBuf,

    /// Export name
    #[arg(short, long, default_value = "cas-disk")]
    export: String,
}

fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    log::info!("Starting NBD server");
    log::info!("  Bind address: {}", args.bind);
    log::info!("  CAS server: {}", args.cas_server);
    log::info!("  Device size: {} MB", args.size);
    log::info!("  Index file: {:?}", args.index);
    log::info!("  Export name: {}", args.export);

    // Create CAS backend
    let cas_config = CasBackendConfig {
        cas_server_addr: args.cas_server,
        device_size_bytes: args.size * 1024 * 1024,
        device_model: format!("NBD CAS Disk {}MB", args.size),
        device_serial: format!("NBD-CAS-{:08x}", rand::random::<u32>()),
        index_path: args.index,
    };

    let backend = match CasBackend::new(cas_config) {
        Ok(backend) => backend,
        Err(e) => {
            log::error!("Failed to create CAS backend: {}", e);
            process::exit(1);
        }
    };

    // Create NBD server
    let nbd_config = NbdServerConfig {
        bind_addr: args.bind,
        export_name: args.export,
    };

    let server = NbdServer::new(nbd_config, backend);

    if let Err(e) = server.run() {
        log::error!("Server error: {}", e);
        process::exit(1);
    }
}
