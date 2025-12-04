//! NBD server implementation

use super::protocol::*;
use crate::storage::BlockStorage;
use byteorder::{BigEndian, ReadBytesExt};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

const SECTOR_SIZE: usize = 512;

/// NBD server configuration
pub struct NbdServerConfig {
    pub bind_addr: String,
    pub export_name: String,
}

impl Default for NbdServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:10809".to_string(),
            export_name: "cas-disk".to_string(),
        }
    }
}

/// NBD server
pub struct NbdServer<S: BlockStorage> {
    config: NbdServerConfig,
    storage: Arc<Mutex<S>>,
}

impl<S: BlockStorage + Send + 'static> NbdServer<S> {
    pub fn new(config: NbdServerConfig, storage: S) -> Self {
        Self {
            config,
            storage: Arc::new(Mutex::new(storage)),
        }
    }

    pub fn run(&self) -> io::Result<()> {
        let listener = TcpListener::bind(&self.config.bind_addr)?;
        log::info!("NBD server listening on {}", self.config.bind_addr);
        log::info!("Export name: {}", self.config.export_name);

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
                Err(e) => log::error!("Connection error: {}", e),
            }
        }

        Ok(())
    }
}

/// Handle NBD client connection
fn handle_client<S: BlockStorage>(
    stream: TcpStream,
    storage: Arc<Mutex<S>>,
) -> io::Result<()> {
    let peer_addr = stream.peer_addr()?;
    log::info!("Client connected: {}", peer_addr);

    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = BufWriter::new(stream);

    // Get device info
    let device_info = {
        let storage = storage.lock().unwrap();
        storage.info().clone()
    };

    let size_bytes = device_info.total_sectors * SECTOR_SIZE as u64;
    let flags = NBD_FLAG_HAS_FLAGS | NBD_FLAG_SEND_FLUSH;

    // Send newstyle handshake and negotiate options
    send_newstyle_handshake(&mut reader, &mut writer, size_bytes, flags)?;

    log::info!(
        "Completed handshake: size={} bytes ({} sectors)",
        size_bytes,
        device_info.total_sectors
    );

    // Handle requests
    loop {
        let request = match NbdRequest::read(&mut reader) {
            Ok(req) => req,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                log::info!("Client disconnected: {}", peer_addr);
                break;
            }
            Err(e) => {
                log::error!("Failed to read request: {}", e);
                break;
            }
        };

        let cmd = request.command_type();
        log::debug!(
            "Request: {:?}, handle={:016x}, offset={}, length={}",
            cmd,
            request.handle,
            request.offset,
            request.length
        );

        match cmd {
            Some(NbdCommand::Read) => {
                handle_read(&request, &mut writer, &storage)?;
            }
            Some(NbdCommand::Write) => {
                handle_write(&request, &mut reader, &mut writer, &storage)?;
            }
            Some(NbdCommand::Flush) => {
                handle_flush(&request, &mut writer, &storage)?;
            }
            Some(NbdCommand::Disc) => {
                log::info!("Client requested disconnect: {}", peer_addr);
                break;
            }
            Some(NbdCommand::Trim) => {
                // Send success (trim not implemented)
                let reply = NbdReply::new(request.handle, 0);
                reply.write(&mut writer)?;
                writer.flush()?;
            }
            _ => {
                log::warn!("Unsupported command: {}", request.command);
                let reply = NbdReply::new(request.handle, libc::EINVAL as u32);
                reply.write(&mut writer)?;
                writer.flush()?;
            }
        }
    }

    Ok(())
}

/// Handle NBD read request
fn handle_read<S: BlockStorage, W: Write>(
    request: &NbdRequest,
    writer: &mut W,
    storage: &Arc<Mutex<S>>,
) -> io::Result<()> {
    let lba = request.offset / SECTOR_SIZE as u64;
    let sector_count = (request.length as usize + SECTOR_SIZE - 1) / SECTOR_SIZE;

    if sector_count > 255 {
        let reply = NbdReply::new(request.handle, libc::EINVAL as u32);
        reply.write(writer)?;
        writer.flush()?;
        return Ok(());
    }

    let result = {
        let storage = storage.lock().unwrap();
        storage.read(lba, sector_count as u8)
    };

    let (error, data) = match result {
        Ok(data) => (0, data),
        Err(e) => {
            log::error!("Read error at LBA {}: {}", lba, e);
            (libc::EIO as u32, Vec::new())
        }
    };

    let reply = NbdReply::new(request.handle, error);
    reply.write(writer)?;

    if error == 0 {
        // Only send requested bytes
        writer.write_all(&data[..request.length as usize])?;
    }

    writer.flush()?;
    Ok(())
}

/// Handle NBD write request
fn handle_write<S: BlockStorage, R: Read, W: Write>(
    request: &NbdRequest,
    reader: &mut R,
    writer: &mut W,
    storage: &Arc<Mutex<S>>,
) -> io::Result<()> {
    let lba = request.offset / SECTOR_SIZE as u64;
    let sector_count = (request.length as usize + SECTOR_SIZE - 1) / SECTOR_SIZE;

    if sector_count > 255 {
        // Read and discard data
        let mut discard = vec![0u8; request.length as usize];
        reader.read_exact(&mut discard)?;

        let reply = NbdReply::new(request.handle, libc::EINVAL as u32);
        reply.write(writer)?;
        writer.flush()?;
        return Ok(());
    }

    // Read write data
    let mut data = vec![0u8; sector_count * SECTOR_SIZE];
    reader.read_exact(&mut data[..request.length as usize])?;

    // Pad to sector boundary if needed
    if request.length as usize % SECTOR_SIZE != 0 {
        // Partial sector write - need to read-modify-write
        let last_sector_lba = lba + (sector_count - 1) as u64;

        let last_sector_result = {
            let storage = storage.lock().unwrap();
            storage.read(last_sector_lba, 1)
        };

        if let Ok(last_sector) = last_sector_result {
            let partial_bytes = request.length as usize % SECTOR_SIZE;
            data[(sector_count - 1) * SECTOR_SIZE + partial_bytes..].copy_from_slice(
                &last_sector[partial_bytes..],
            );
        }
    }

    let result = {
        let mut storage = storage.lock().unwrap();
        storage.write(lba, &data)
    };

    let error = match result {
        Ok(_) => 0,
        Err(e) => {
            log::error!("Write error at LBA {}: {}", lba, e);
            libc::EIO as u32
        }
    };

    let reply = NbdReply::new(request.handle, error);
    reply.write(writer)?;
    writer.flush()?;

    Ok(())
}

/// Handle NBD flush request
fn handle_flush<S: BlockStorage, W: Write>(
    request: &NbdRequest,
    writer: &mut W,
    storage: &Arc<Mutex<S>>,
) -> io::Result<()> {
    let result = {
        let mut storage = storage.lock().unwrap();
        (*storage).flush()
    };

    let error = match result {
        Ok(_) => 0,
        Err(e) => {
            log::error!("Flush error: {}", e);
            libc::EIO as u32
        }
    };

    let reply = NbdReply::new(request.handle, error);
    reply.write(writer)?;
    writer.flush()?;

    Ok(())
}
