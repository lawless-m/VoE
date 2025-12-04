//! CAS server binary
//!
//! Standalone content-addressable storage service.

use clap::Parser;
use env_logger::Env;
use std::process;
use aoe_server::cas::{CasServer, CasServerConfig};

#[derive(Parser, Debug)]
#[command(name = "cas-server")]
#[command(about = "Content-Addressable Storage server", long_about = None)]
struct Args {
    /// Bind address (e.g., 127.0.0.1:3000)
    #[arg(short, long, default_value = "127.0.0.1:3000")]
    bind: String,

    /// Storage directory path
    #[arg(short, long, default_value = "/var/lib/cas")]
    storage: String,
}

fn main() {
    // Initialize logging
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    let config = CasServerConfig {
        bind_addr: args.bind,
        storage_path: args.storage,
    };

    log::info!("Starting CAS server");
    log::info!("  Bind address: {}", config.bind_addr);
    log::info!("  Storage path: {}", config.storage_path);

    let server = match CasServer::new(config) {
        Ok(server) => server,
        Err(e) => {
            log::error!("Failed to create server: {}", e);
            process::exit(1);
        }
    };

    if let Err(e) = server.run() {
        log::error!("Server error: {}", e);
        process::exit(1);
    }
}
