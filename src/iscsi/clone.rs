//! Target cloning operations
//!
//! Handles creation, cloning, and deletion of iSCSI targets using redb database
//! copy for safe cloning.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::registry::{TargetMetadata, TargetRegistry};

/// Lock file name to prevent cloning running targets
const LOCK_FILE_NAME: &str = ".serving.lock";

/// Clone manager for target operations
pub struct CloneManager {
    /// Target registry
    pub registry: TargetRegistry,

    /// Base directory for target indexes (e.g., /var/lib/voe-iscsi/targets)
    pub targets_base_dir: PathBuf,

    /// CAS server address
    pub cas_server: String,
}

impl CloneManager {
    /// Create a new clone manager
    pub fn new(registry_path: PathBuf, targets_base_dir: PathBuf, cas_server: String) -> Result<Self> {
        let registry = TargetRegistry::load_or_create(&registry_path)?;

        // Ensure base directory exists
        fs::create_dir_all(&targets_base_dir)
            .with_context(|| format!("Failed to create targets directory: {:?}", targets_base_dir))?;

        Ok(Self {
            registry,
            targets_base_dir,
            cas_server,
        })
    }

    /// Create a new target
    pub fn create_target(&mut self, name: &str, size_mb: u64, description: Option<String>) -> Result<String> {
        log::info!("Creating target: {} ({} MB)", name, size_mb);

        // Generate IQN
        let iqn = TargetRegistry::generate_iqn(name);

        // Check if target already exists
        if self.registry.get_target(&iqn).is_some() {
            anyhow::bail!("Target already exists with this name: {}", name);
        }

        // Create index directory
        let index_path = self.get_index_path(&iqn);
        fs::create_dir_all(&index_path)
            .with_context(|| format!("Failed to create index directory: {:?}", index_path))?;

        // Create metadata
        let metadata = TargetMetadata {
            iqn: iqn.clone(),
            name: name.to_string(),
            size_mb,
            index_path,
            parent: None,
            children: vec![],
            created_at: TargetRegistry::now(),
            description,
        };

        // Add to registry
        self.registry.add_target(metadata)?;

        log::info!("Created target: {} -> {}", name, iqn);
        Ok(iqn)
    }

    /// Clone a target (source must not be running)
    pub fn clone_target(&mut self, source_iqn: &str, dest_name: &str) -> Result<String> {
        log::info!("Cloning target: {} -> {}", source_iqn, dest_name);

        // Check source exists
        let source = self.registry.get_target(source_iqn)
            .ok_or_else(|| anyhow::anyhow!("Source target not found: {}", source_iqn))?
            .clone();

        // Check source is not running
        if self.is_target_running(source_iqn)? {
            anyhow::bail!("Source target is currently running: {}", source_iqn);
        }

        // Generate destination IQN
        let dest_iqn = TargetRegistry::generate_iqn(dest_name);

        // Check destination doesn't exist
        if self.registry.get_target(&dest_iqn).is_some() {
            anyhow::bail!("Destination target already exists: {}", dest_name);
        }

        // Create destination index directory
        let dest_index_path = self.get_index_path(&dest_iqn);
        fs::create_dir_all(&dest_index_path)
            .with_context(|| format!("Failed to create destination index directory: {:?}", dest_index_path))?;

        // Clone the redb database
        log::info!("Cloning source database: {:?}", source.index_path);
        self.clone_redb_database(&source.index_path, &dest_index_path)?;

        // Create destination metadata
        let dest_metadata = TargetMetadata {
            iqn: dest_iqn.clone(),
            name: dest_name.to_string(),
            size_mb: source.size_mb,
            index_path: dest_index_path,
            parent: Some(source_iqn.to_string()),
            children: vec![],
            created_at: TargetRegistry::now(),
            description: Some(format!("Clone of {}", source.name)),
        };

        // Add to registry (this also updates parent's children list)
        self.registry.add_target(dest_metadata)?;

        log::info!("Cloned target: {} -> {} ({})", source_iqn, dest_name, dest_iqn);
        Ok(dest_iqn)
    }

    /// Clone a redb database by copying all key-value pairs
    fn clone_redb_database(&self, source_path: &Path, dest_path: &Path) -> Result<()> {
        use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

        const LBA_TABLE: TableDefinition<u64, &[u8; 16]> = TableDefinition::new("lba_index");
        const META_TABLE: TableDefinition<&str, &[u8; 16]> = TableDefinition::new("meta");

        let source_db = Database::open(source_path)
            .with_context(|| format!("Failed to open source database: {:?}", source_path))?;
        let dest_db = Database::create(dest_path)
            .with_context(|| format!("Failed to create destination database: {:?}", dest_path))?;

        let read_txn = source_db.begin_read()
            .context("Failed to begin read transaction")?;
        let write_txn = dest_db.begin_write()
            .context("Failed to begin write transaction")?;

        // Copy meta table
        {
            let src_table = read_txn.open_table(META_TABLE)
                .context("Failed to open source meta table")?;
            let mut dst_table = write_txn.open_table(META_TABLE)
                .context("Failed to open dest meta table")?;
            for result in src_table.iter().context("Failed to iterate meta table")? {
                let (key, value) = result.context("Failed to read meta entry")?;
                dst_table.insert(key.value(), value.value())
                    .context("Failed to write meta entry")?;
            }
        }

        // Copy LBA table
        let mut count = 0u64;
        {
            let src_table = read_txn.open_table(LBA_TABLE)
                .context("Failed to open source LBA table")?;
            let mut dst_table = write_txn.open_table(LBA_TABLE)
                .context("Failed to open dest LBA table")?;
            for result in src_table.iter().context("Failed to iterate LBA table")? {
                let (key, value) = result.context("Failed to read LBA entry")?;
                dst_table.insert(key.value(), value.value())
                    .context("Failed to write LBA entry")?;
                count += 1;
            }
        }

        write_txn.commit().context("Failed to commit destination database")?;

        log::info!("Successfully cloned database: {} entries from {:?} to {:?}", count, source_path, dest_path);
        Ok(())
    }

    /// Delete a target
    pub fn delete_target(&mut self, iqn: &str, remove_data: bool) -> Result<()> {
        log::info!("Deleting target: {} (remove_data={})", iqn, remove_data);

        // Check target is not running
        if self.is_target_running(iqn)? {
            anyhow::bail!("Target is currently running: {}", iqn);
        }

        // Get metadata
        let metadata = self.registry.get_target(iqn)
            .ok_or_else(|| anyhow::anyhow!("Target not found: {}", iqn))?
            .clone();

        // Warn if target has children
        if !metadata.children.is_empty() {
            log::warn!("Target {} has {} child(ren), but proceeding with deletion",
                iqn, metadata.children.len());
        }

        // Remove from registry
        self.registry.remove_target(iqn)?;

        // Remove data if requested
        if remove_data {
            log::info!("Removing target data: {:?}", metadata.index_path);
            if metadata.index_path.exists() {
                fs::remove_dir_all(&metadata.index_path)
                    .with_context(|| format!("Failed to remove target directory: {:?}", metadata.index_path))?;
            }
        }

        log::info!("Deleted target: {}", iqn);
        Ok(())
    }

    /// Check if a target is currently running (has a lock file with valid PID)
    pub fn is_target_running(&self, iqn: &str) -> Result<bool> {
        let metadata = self.registry.get_target(iqn)
            .ok_or_else(|| anyhow::anyhow!("Target not found: {}", iqn))?;

        let lock_file = metadata.index_path.join(LOCK_FILE_NAME);

        if !lock_file.exists() {
            return Ok(false);
        }

        // Read PID from lock file
        let pid_str = fs::read_to_string(&lock_file)
            .context("Failed to read lock file")?
            .trim()
            .to_string();

        let pid: u32 = pid_str.parse()
            .context("Invalid PID in lock file")?;

        // Check if process is running
        let running = is_process_running(pid);

        if !running {
            // Stale lock file - remove it
            log::warn!("Removing stale lock file: {:?} (PID {} not running)", lock_file, pid);
            fs::remove_file(&lock_file)
                .context("Failed to remove stale lock file")?;
        }

        Ok(running)
    }

    /// Get the index path for a target
    fn get_index_path(&self, iqn: &str) -> PathBuf {
        // Extract the target name from IQN (last component after :)
        let name = iqn.split(':').last().unwrap_or(iqn);
        self.targets_base_dir.join(name).join("index")
    }
}

/// Check if a process with the given PID is running
fn is_process_running(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // Send signal 0 to check if process exists
        let output = Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .output();

        match output {
            Ok(output) => output.status.success(),
            Err(_) => false,
        }
    }

    #[cfg(not(unix))]
    {
        // On Windows, check if process exists using tasklist
        let output = Command::new("tasklist")
            .arg("/FI")
            .arg(format!("PID eq {}", pid))
            .arg("/NH")
            .output();

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout.contains(&pid.to_string())
            }
            Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clone_manager() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let registry_path = temp_dir.path().join("registry.json");
        let targets_dir = temp_dir.path().join("targets");
        let cas_server = "127.0.0.1:3000".to_string();

        let mut manager = CloneManager::new(registry_path, targets_dir, cas_server)?;

        // Create a target
        let iqn = manager.create_target("test-target", 100, Some("Test description".to_string()))?;
        assert!(iqn.starts_with("iqn."));

        // Verify target exists in registry
        assert!(manager.registry.get_target(&iqn).is_some());

        Ok(())
    }

    #[test]
    fn test_process_detection() {
        // Current process should be running
        let current_pid = std::process::id();
        assert!(is_process_running(current_pid));

        // PID 999999 should not exist
        assert!(!is_process_running(999999));
    }
}
