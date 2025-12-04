//! CAS TCP server
//!
//! Accepts client connections and handles CAS protocol requests.

use super::protocol::{read_frame, write_response, CasCommand, CasResponse};
use super::storage::CasStorage;
use std::io;
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

/// CAS server configuration
pub struct CasServerConfig {
    pub bind_addr: String,
    pub storage_path: String,
}

impl Default for CasServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:3000".to_string(),
            storage_path: "/var/lib/cas".to_string(),
        }
    }
}

/// CAS TCP server
pub struct CasServer {
    config: CasServerConfig,
    storage: Arc<Mutex<CasStorage>>,
}

impl CasServer {
    /// Create a new CAS server
    pub fn new(config: CasServerConfig) -> io::Result<Self> {
        let storage = CasStorage::new(&config.storage_path)?;
        Ok(Self {
            config,
            storage: Arc::new(Mutex::new(storage)),
        })
    }

    /// Run the server
    pub fn run(&self) -> io::Result<()> {
        let listener = TcpListener::bind(&self.config.bind_addr)?;
        log::info!("CAS server listening on {}", self.config.bind_addr);

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let storage = Arc::clone(&self.storage);
                    thread::spawn(move || {
                        if let Err(e) = handle_client(stream, storage) {
                            log::warn!("Client handler error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    log::error!("Connection error: {}", e);
                }
            }
        }

        Ok(())
    }
}

/// Handle a client connection
fn handle_client(mut stream: TcpStream, storage: Arc<Mutex<CasStorage>>) -> io::Result<()> {
    let peer = stream.peer_addr()?;
    log::info!("New connection from {}", peer);

    loop {
        // Read frame
        let (command, data) = match read_frame(&mut stream) {
            Ok(frame) => frame,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                log::info!("Client {} disconnected", peer);
                return Ok(());
            }
            Err(e) => {
                log::warn!("Error reading frame from {}: {}", peer, e);
                return Err(e);
            }
        };

        // Process command
        let response = match command {
            CasCommand::Write => {
                let storage = storage.lock().unwrap();
                match storage.write(&data) {
                    Ok(hash) => CasResponse::Hash(hash),
                    Err(e) => CasResponse::Error(format!("write failed: {}", e)),
                }
            }
            CasCommand::Read => {
                if data.len() != 32 {
                    CasResponse::Error("invalid hash length".to_string())
                } else {
                    let mut hash = [0u8; 32];
                    hash.copy_from_slice(&data);
                    let storage = storage.lock().unwrap();
                    match storage.read(&hash) {
                        Ok(content) => CasResponse::Data(content),
                        Err(e) => CasResponse::Error(format!("read failed: {}", e)),
                    }
                }
            }
            CasCommand::Exists => {
                if data.len() != 32 {
                    CasResponse::Error("invalid hash length".to_string())
                } else {
                    let mut hash = [0u8; 32];
                    hash.copy_from_slice(&data);
                    let storage = storage.lock().unwrap();
                    CasResponse::Exists(storage.exists(&hash))
                }
            }
            CasCommand::Ping => CasResponse::Pong,
        };

        // Send response
        if let Err(e) = write_response(&mut stream, &response) {
            log::warn!("Error writing response to {}: {}", peer, e);
            return Err(e);
        }
    }
}
