//! CAS (Content-Addressable Storage) client backend for AoE
//!
//! Maps LBA addresses to content hashes stored in a CAS service.
//! Persists the LBA mapping to disk for durability.

use super::{BlockStorage, DeviceInfo, StorageError};
use crate::cas::protocol::{read_frame, write_frame, CasCommand, CasResponse};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const SECTOR_SIZE: usize = 512;

/// CAS backend configuration
pub struct CasBackendConfig {
    pub cas_server_addr: String,
    pub device_size_bytes: u64,
    pub device_model: String,
    pub device_serial: String,
    pub index_path: PathBuf,
}

impl Default for CasBackendConfig {
    fn default() -> Self {
        Self {
            cas_server_addr: "127.0.0.1:3000".to_string(),
            device_size_bytes: 100 * 1024 * 1024, // 100 MB
            device_model: "CAS Virtual Disk".to_string(),
            device_serial: "CAS001".to_string(),
            index_path: PathBuf::from("/var/lib/aoe-cas/index.json"),
        }
    }
}

/// Persistent index of LBA to hash mappings
#[derive(Debug, Serialize, Deserialize)]
struct LbaIndex {
    /// LBA to hash mappings (only non-zero blocks)
    mappings: HashMap<u64, [u8; 32]>,
    /// Hash of the zero block
    zero_block_hash: [u8; 32],
}

impl LbaIndex {
    fn new(zero_block_hash: [u8; 32]) -> Self {
        Self {
            mappings: HashMap::new(),
            zero_block_hash,
        }
    }

    fn load(path: &Path) -> Result<Self, StorageError> {
        let file = File::open(path).map_err(|e| {
            StorageError::Backend(format!("failed to open index: {}", e))
        })?;
        serde_json::from_reader(file).map_err(|e| {
            StorageError::Backend(format!("failed to parse index: {}", e))
        })
    }

    fn save(&self, path: &Path) -> Result<(), StorageError> {
        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                StorageError::Backend(format!("failed to create index dir: {}", e))
            })?;
        }

        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .map_err(|e| StorageError::Backend(format!("failed to create index: {}", e)))?;

        serde_json::to_writer_pretty(file, self).map_err(|e| {
            StorageError::Backend(format!("failed to write index: {}", e))
        })
    }
}

/// CAS backend state
struct CasBackendState {
    reader: BufReader<TcpStream>,
    writer: BufWriter<TcpStream>,
    index: LbaIndex,
}

/// CAS-backed block storage
pub struct CasBackend {
    config: CasBackendConfig,
    state: Arc<Mutex<CasBackendState>>,
    device_info: DeviceInfo,
}

impl CasBackend {
    /// Create a new CAS backend
    pub fn new(config: CasBackendConfig) -> Result<Self, StorageError> {
        let stream = TcpStream::connect(&config.cas_server_addr).map_err(|e| {
            StorageError::Backend(format!("failed to connect to CAS server: {}", e))
        })?;

        let mut reader = BufReader::new(stream.try_clone().map_err(|e| {
            StorageError::Backend(format!("failed to clone stream: {}", e))
        })?);
        let mut writer = BufWriter::new(stream);

        // Try to load existing index, or create new
        let index = if config.index_path.exists() {
            log::info!("Loading existing index from {:?}", config.index_path);
            LbaIndex::load(&config.index_path)?
        } else {
            log::info!("Creating new index");
            // Initialize zero block
            let zero_sector = vec![0u8; SECTOR_SIZE];
            let mut temp_state = CasBackendState {
                reader,
                writer,
                index: LbaIndex::new([0u8; 32]), // Temporary
            };
            let zero_hash = Self::write_to_cas(&mut temp_state, &zero_sector)?;

            // Extract reader/writer back
            reader = temp_state.reader;
            writer = temp_state.writer;

            log::info!("Initialized zero block hash: {}", hex::encode(&zero_hash));
            LbaIndex::new(zero_hash)
        };

        let state = CasBackendState {
            reader,
            writer,
            index,
        };

        let device_info = DeviceInfo {
            model: config.device_model.clone(),
            serial: config.device_serial.clone(),
            firmware: "1.0".to_string(),
            total_sectors: config.device_size_bytes / SECTOR_SIZE as u64,
            sector_size: SECTOR_SIZE as u32,
            lba48: true,
        };

        Ok(Self {
            config,
            state: Arc::new(Mutex::new(state)),
            device_info,
        })
    }

    /// Write data to CAS and get hash
    fn write_to_cas(state: &mut CasBackendState, data: &[u8]) -> Result<[u8; 32], StorageError> {
        write_frame(&mut state.writer, CasCommand::Write, data).map_err(|e| {
            StorageError::Backend(format!("failed to write to CAS: {}", e))
        })?;

        let (cmd, hash_data) = read_frame(&mut state.reader).map_err(|e| {
            StorageError::Backend(format!("failed to read CAS write response: {}", e))
        })?;

        if let CasCommand::Write = cmd {
            if hash_data.len() == 32 {
                let mut hash = [0u8; 32];
                hash.copy_from_slice(&hash_data);
                return Ok(hash);
            }
        }

        Err(StorageError::Backend(
            "invalid CAS write response".to_string(),
        ))
    }

    /// Read data from CAS by hash
    fn read_from_cas(state: &mut CasBackendState, hash: &[u8; 32]) -> Result<Vec<u8>, StorageError> {
        write_frame(&mut state.writer, CasCommand::Read, hash).map_err(|e| {
            StorageError::Backend(format!("failed to read from CAS: {}", e))
        })?;

        let (cmd, data) = read_frame(&mut state.reader).map_err(|e| {
            StorageError::Backend(format!("failed to read CAS read response: {}", e))
        })?;

        match cmd {
            CasCommand::Read => Ok(data),
            _ => Err(StorageError::Backend(
                "invalid CAS read response".to_string(),
            )),
        }
    }

    /// Save the index to disk
    fn save_index(&self) -> Result<(), StorageError> {
        let state = self.state.lock().unwrap();
        state.index.save(&self.config.index_path)
    }
}

impl CasBackend {
    fn read_sectors(&mut self, lba: u64, count: u8, buffer: &mut [u8]) -> Result<(), StorageError> {
        let expected_size = count as usize * SECTOR_SIZE;
        if buffer.len() < expected_size {
            return Err(StorageError::Backend(
                "buffer too small".to_string(),
            ));
        }

        let mut state = self.state.lock().unwrap();

        for i in 0..count {
            let sector_lba = lba + i as u64;
            let offset = i as usize * SECTOR_SIZE;
            let sector_buf = &mut buffer[offset..offset + SECTOR_SIZE];

            // Get hash for this LBA, or use zero block
            let hash = state
                .index
                .mappings
                .get(&sector_lba)
                .copied()
                .unwrap_or(state.index.zero_block_hash);

            // Read from CAS
            let data = Self::read_from_cas(&mut state, &hash)?;

            if data.len() != SECTOR_SIZE {
                return Err(StorageError::Backend(format!(
                    "CAS returned wrong size: expected {}, got {}",
                    SECTOR_SIZE,
                    data.len()
                )));
            }

            sector_buf.copy_from_slice(&data);
        }

        Ok(())
    }

    fn write_sectors(&mut self, lba: u64, count: u8, data: &[u8]) -> Result<(), StorageError> {
        let expected_size = count as usize * SECTOR_SIZE;
        if data.len() < expected_size {
            return Err(StorageError::Backend(
                "data too small".to_string(),
            ));
        }

        let mut state = self.state.lock().unwrap();

        for i in 0..count {
            let sector_lba = lba + i as u64;
            let offset = i as usize * SECTOR_SIZE;
            let sector_data = &data[offset..offset + SECTOR_SIZE];

            // Write to CAS and get hash
            let hash = Self::write_to_cas(&mut state, sector_data)?;

            // Update LBA mapping
            state.index.mappings.insert(sector_lba, hash);
        }

        // Save index after writes
        drop(state);
        self.save_index()?;

        Ok(())
    }

    fn flush(&mut self) -> Result<(), StorageError> {
        // Save index on flush
        self.save_index()
    }

}

impl BlockStorage for CasBackend {
    fn read(&self, lba: u64, count: u8) -> super::StorageResult<Vec<u8>> {
        let size = count as usize * SECTOR_SIZE;
        let mut buffer = vec![0u8; size];

        let mut state = self.state.lock().unwrap();

        for i in 0..count {
            let sector_lba = lba + i as u64;
            let offset = i as usize * SECTOR_SIZE;
            let sector_buf = &mut buffer[offset..offset + SECTOR_SIZE];

            // Get hash for this LBA, or use zero block
            let hash = state
                .index
                .mappings
                .get(&sector_lba)
                .copied()
                .unwrap_or(state.index.zero_block_hash);

            // Read from CAS
            let data = Self::read_from_cas(&mut state, &hash)?;

            if data.len() != SECTOR_SIZE {
                return Err(StorageError::Backend(format!(
                    "CAS returned wrong size: expected {}, got {}",
                    SECTOR_SIZE,
                    data.len()
                )));
            }

            sector_buf.copy_from_slice(&data);
        }

        Ok(buffer)
    }

    fn write(&mut self, lba: u64, data: &[u8]) -> super::StorageResult<()> {
        let count = (data.len() + SECTOR_SIZE - 1) / SECTOR_SIZE;

        if count > 255 {
            return Err(StorageError::InvalidSectorCount(count as u8));
        }

        let mut state = self.state.lock().unwrap();

        for i in 0..count {
            let sector_lba = lba + i as u64;
            let offset = i * SECTOR_SIZE;
            let end = (offset + SECTOR_SIZE).min(data.len());

            // Prepare sector data, padding if necessary
            let mut sector_data = vec![0u8; SECTOR_SIZE];
            sector_data[..end - offset].copy_from_slice(&data[offset..end]);

            // Write to CAS and get hash
            let hash = Self::write_to_cas(&mut state, &sector_data)?;

            // Update LBA mapping
            state.index.mappings.insert(sector_lba, hash);
        }

        // Save index after writes
        drop(state);
        self.save_index()?;

        Ok(())
    }

    fn flush(&mut self) -> super::StorageResult<()> {
        self.save_index()
    }

    fn info(&self) -> &DeviceInfo {
        &self.device_info
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Requires CAS server running
    fn test_cas_backend_persistence() {
        let temp_index = std::env::temp_dir().join("test_cas_index.json");

        // Clean up from previous test
        let _ = std::fs::remove_file(&temp_index);

        let config = CasBackendConfig {
            cas_server_addr: "127.0.0.1:3000".to_string(),
            device_size_bytes: 1024 * 1024,
            device_model: "Test Disk".to_string(),
            device_serial: "TEST001".to_string(),
            index_path: temp_index.clone(),
        };

        // Write some data
        {
            let mut backend = CasBackend::new(config.clone()).unwrap();
            let write_data = b"Hello, persistent CAS!".to_vec();
            let mut padded_write = vec![0u8; SECTOR_SIZE];
            padded_write[..write_data.len()].copy_from_slice(&write_data);

            backend.write_sectors(0, 1, &padded_write).unwrap();
            backend.flush().unwrap();
        }

        // Read it back with a new backend instance
        {
            let mut backend = CasBackend::new(config).unwrap();
            let mut read_buf = vec![0u8; SECTOR_SIZE];
            backend.read_sectors(0, 1, &mut read_buf).unwrap();

            assert_eq!(&read_buf[..22], b"Hello, persistent CAS!");
        }

        // Clean up
        let _ = std::fs::remove_file(&temp_index);
    }
}
