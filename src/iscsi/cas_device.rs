//! CAS-backed SCSI block device
//!
//! Implements ScsiBlockDevice trait with CAS backend for direct iSCSI → CAS integration.

use std::collections::HashMap;
use std::io::{BufReader, BufWriter};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use rocksdb::{DB, Options as RocksOptions};

use crate::cas::protocol::{read_frame, write_frame, CasCommand};
use crate::cas::Hash;
use iscsi_target::{IscsiError, ScsiBlockDevice, ScsiResult};

const BLOCK_SIZE: u32 = 4096;  // 4KB blocks - good balance for CAS dedup
const MAX_CACHED_BLOCKS: usize = 1000;  // Auto-flush when cache exceeds 4MB to prevent memory bloat

/// Configuration for CAS SCSI device
#[derive(Debug, Clone)]
pub struct CasScsiDeviceConfig {
    /// CAS server address (e.g., "127.0.0.1:3000")
    pub cas_server_addr: String,
    /// Device capacity in blocks
    pub capacity_blocks: u64,
    /// Path to persistent LBA→hash index
    pub index_path: PathBuf,
    /// SCSI vendor ID (8 chars)
    pub vendor_id: String,
    /// SCSI product ID (16 chars)
    pub product_id: String,
    /// SCSI product revision (4 chars)
    pub product_rev: String,
}

impl Default for CasScsiDeviceConfig {
    fn default() -> Self {
        Self {
            cas_server_addr: "127.0.0.1:3000".to_string(),
            capacity_blocks: 20480, // 10 MB @ 512 bytes
            index_path: PathBuf::from("/var/lib/voe-iscsi/index.json"),
            vendor_id: "VoE     ".to_string(),
            product_id: "CAS Block Device".to_string(),
            product_rev: "1.0 ".to_string(),
        }
    }
}

/// Persistent index of LBA to hash mappings using RocksDB
struct LbaIndex {
    db: Arc<DB>,
    zero_block_hash: Hash,
}

// Special key for storing zero block hash
const ZERO_BLOCK_KEY: &[u8] = b"__ZERO_BLOCK__";

impl LbaIndex {
    fn new(db_path: &PathBuf, zero_block_hash: Hash) -> std::io::Result<Self> {
        // Create parent directory if needed
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut opts = RocksOptions::default();
        opts.create_if_missing(true);
        opts.set_compression_type(rocksdb::DBCompressionType::Lz4);

        let db = DB::open(&opts, db_path)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        let db = Arc::new(db);

        // Store zero block hash
        db.put(ZERO_BLOCK_KEY, &zero_block_hash)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        Ok(Self { db, zero_block_hash })
    }

    fn open(db_path: &PathBuf) -> std::io::Result<Self> {
        let mut opts = RocksOptions::default();
        opts.set_compression_type(rocksdb::DBCompressionType::Lz4);

        let db = DB::open(&opts, db_path)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        let db = Arc::new(db);

        // Load zero block hash
        let zero_block_hash = db.get(ZERO_BLOCK_KEY)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "Missing zero block hash"))?;

        let mut hash = [0u8; 16];
        hash.copy_from_slice(&zero_block_hash);

        Ok(Self { db, zero_block_hash: hash })
    }

    fn get(&self, lba: u64) -> std::io::Result<Option<Hash>> {
        let key = lba.to_le_bytes();
        match self.db.get(&key) {
            Ok(Some(value)) => {
                if value.len() == 16 {
                    let mut hash = [0u8; 16];
                    hash.copy_from_slice(&value);
                    Ok(Some(hash))
                } else {
                    Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid hash size"))
                }
            }
            Ok(None) => Ok(None),
            Err(e) => Err(std::io::Error::new(std::io::ErrorKind::Other, e)),
        }
    }

    fn insert(&self, lba: u64, hash: &Hash) -> std::io::Result<()> {
        let key = lba.to_le_bytes();
        self.db.put(&key, hash)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    }
}

/// Internal state protected by mutex
struct CasScsiDeviceState {
    reader: BufReader<TcpStream>,
    writer: BufWriter<TcpStream>,
    index: LbaIndex,
    /// Write cache: LBA -> (data, dirty flag)
    write_cache: HashMap<u64, Vec<u8>>,
}

/// CAS-backed SCSI block device
pub struct CasScsiDevice {
    config: CasScsiDeviceConfig,
    state: Arc<Mutex<CasScsiDeviceState>>,
}

impl CasScsiDevice {
    /// Create a new CAS SCSI device
    pub fn new(config: CasScsiDeviceConfig) -> std::io::Result<Self> {
        log::info!("Connecting to CAS server at {}", config.cas_server_addr);
        let stream = TcpStream::connect(&config.cas_server_addr)?;

        let mut reader = BufReader::new(stream.try_clone()?);
        let mut writer = BufWriter::new(stream);

        // Try to open existing index, or create new
        let index = if config.index_path.exists() {
            log::info!("Opening existing RocksDB index at {:?}", config.index_path);
            LbaIndex::open(&config.index_path)?
        } else {
            log::info!("Creating new RocksDB index, initializing zero block");
            // Initialize zero block
            let zero_block = vec![0u8; BLOCK_SIZE as usize];
            let zero_hash = Self::write_to_cas_static(&mut writer, &mut reader, &zero_block)?;
            log::info!("Zero block hash: {}", hex::encode(&zero_hash));
            LbaIndex::new(&config.index_path, zero_hash)?
        };

        let state = CasScsiDeviceState {
            reader,
            writer,
            index,
            write_cache: HashMap::new(),
        };

        Ok(Self {
            config,
            state: Arc::new(Mutex::new(state)),
        })
    }

    /// Write data to CAS and get hash (static version for initialization)
    fn write_to_cas_static(
        writer: &mut BufWriter<TcpStream>,
        reader: &mut BufReader<TcpStream>,
        data: &[u8],
    ) -> std::io::Result<Hash> {
        write_frame(writer, CasCommand::Write, data)?;

        let (cmd, hash_data) = read_frame(reader)?;

        if let CasCommand::Write = cmd {
            if hash_data.len() == 16 {
                let mut hash = [0u8; 16];
                hash.copy_from_slice(&hash_data);
                return Ok(hash);
            }
        }

        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid CAS write response",
        ))
    }

    /// Write data to CAS and get hash
    fn write_to_cas(state: &mut CasScsiDeviceState, data: &[u8]) -> std::io::Result<Hash> {
        write_frame(&mut state.writer, CasCommand::Write, data)?;

        let (cmd, hash_data) = read_frame(&mut state.reader)?;

        if let CasCommand::Write = cmd {
            if hash_data.len() == 16 {
                let mut hash = [0u8; 16];
                hash.copy_from_slice(&hash_data);
                return Ok(hash);
            }
        }

        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid CAS write response",
        ))
    }

    /// Read data from CAS by hash
    fn read_from_cas(state: &mut CasScsiDeviceState, hash: &Hash) -> std::io::Result<Vec<u8>> {
        write_frame(&mut state.writer, CasCommand::Read, hash)?;

        let (cmd, data) = read_frame(&mut state.reader)?;

        match cmd {
            CasCommand::Read => Ok(data),
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid CAS read response",
            )),
        }
    }

}

impl ScsiBlockDevice for CasScsiDevice {
    fn read(&self, lba: u64, blocks: u32, block_size: u32) -> ScsiResult<Vec<u8>> {
        if block_size != BLOCK_SIZE {
            return Err(IscsiError::Scsi(format!("unsupported block size: {}", block_size)));
        }

        let total_size = blocks as usize * BLOCK_SIZE as usize;
        let mut buffer = Vec::with_capacity(total_size);

        let mut state = self.state.lock().unwrap();

        for i in 0..blocks {
            let block_lba = lba + i as u64;

            // Check write cache first
            if let Some(cached_data) = state.write_cache.get(&block_lba) {
                buffer.extend_from_slice(cached_data);
                continue;
            }

            // Get hash for this LBA, or use zero block
            let hash = match state.index.get(block_lba) {
                Ok(Some(h)) => h,
                Ok(None) => state.index.zero_block_hash,
                Err(e) => return Err(IscsiError::Io(e)),
            };

            // Read from CAS
            let data = Self::read_from_cas(&mut state, &hash)
                .map_err(|e| IscsiError::Io(e))?;

            if data.len() != BLOCK_SIZE as usize {
                return Err(IscsiError::Scsi(format!(
                    "CAS returned wrong size: expected {}, got {}",
                    BLOCK_SIZE,
                    data.len()
                )));
            }

            buffer.extend_from_slice(&data);
        }

        Ok(buffer)
    }

    fn write(&mut self, lba: u64, data: &[u8], block_size: u32) -> ScsiResult<()> {
        if block_size != BLOCK_SIZE {
            return Err(IscsiError::Scsi(format!("unsupported block size: {}", block_size)));
        }

        let blocks = (data.len() + BLOCK_SIZE as usize - 1) / BLOCK_SIZE as usize;
        let mut state = self.state.lock().unwrap();

        // Store all blocks in write cache - return immediately without CAS I/O!
        for i in 0..blocks {
            let block_lba = lba + i as u64;
            let offset = i * BLOCK_SIZE as usize;
            let end = (offset + BLOCK_SIZE as usize).min(data.len());

            // Prepare block data, padding if necessary
            let mut block_data = vec![0u8; BLOCK_SIZE as usize];
            block_data[..end - offset].copy_from_slice(&data[offset..end]);

            // Store in write cache
            state.write_cache.insert(block_lba, block_data);
        }

        // Auto-flush if cache exceeds threshold
        let cache_size = state.write_cache.len();
        if cache_size >= MAX_CACHED_BLOCKS {
            log::info!("Cache has {} blocks, triggering auto-flush", cache_size);
            drop(state); // Release lock before calling flush()
            return self.flush();
        }

        log::debug!("Write cache now has {} blocks", cache_size);
        Ok(())
    }

    fn capacity(&self) -> u64 {
        self.config.capacity_blocks
    }

    fn block_size(&self) -> u32 {
        BLOCK_SIZE
    }

    fn flush(&mut self) -> ScsiResult<()> {
        let mut state = self.state.lock().unwrap();
        let cached_count = state.write_cache.len();

        if cached_count == 0 {
            return Ok(());
        }

        log::info!("flush() called with {} cached blocks - actually flushing to CAS", cached_count);

        // Flush all cached blocks to CAS
        let cache = std::mem::take(&mut state.write_cache);
        for (lba, block_data) in cache.iter() {
            // Write block to CAS and get hash
            let hash = Self::write_to_cas(&mut state, block_data)
                .map_err(|e| IscsiError::Io(e))?;

            // Update index with hash for this LBA (RocksDB writes immediately)
            state.index.insert(*lba, &hash)
                .map_err(|e| IscsiError::Io(e))?;
        }

        log::info!("Flushed {} blocks to CAS and index", cached_count);
        Ok(())
    }

    fn vendor_id(&self) -> &str {
        &self.config.vendor_id
    }

    fn product_id(&self) -> &str {
        &self.config.product_id
    }

    fn product_rev(&self) -> &str {
        &self.config.product_rev
    }
}

impl Drop for CasScsiDevice {
    fn drop(&mut self) {
        // Flush cache to CAS when device is dropped
        let mut state = self.state.lock().unwrap();
        let cached_count = state.write_cache.len();

        if cached_count == 0 {
            return;
        }

        log::warn!("Device being dropped with {} cached blocks - flushing to CAS", cached_count);

        // Flush all cached blocks to CAS
        let cache = std::mem::take(&mut state.write_cache);
        for (lba, block_data) in cache.iter() {
            match CasScsiDevice::write_to_cas(&mut state, block_data) {
                Ok(hash) => {
                    if let Err(e) = state.index.insert(*lba, &hash) {
                        log::error!("Failed to update index for block {}: {}", lba, e);
                    }
                }
                Err(e) => {
                    log::error!("Failed to flush block {} to CAS: {}", lba, e);
                }
            }
        }

        log::info!("Successfully flushed {} blocks to CAS and index on drop", cached_count);
    }
}
