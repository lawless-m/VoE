//! Target registry for managing iSCSI target metadata
//!
//! The registry tracks all configured iSCSI targets, their properties,
//! and clone relationships. It's stored as JSON at /var/lib/voe-iscsi/registry.json.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Default registry path
pub const DEFAULT_REGISTRY_PATH: &str = "/var/lib/voe-iscsi/registry.json";

/// Target registry managing all iSCSI targets
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetRegistry {
    /// Path to the registry file
    #[serde(skip)]
    pub registry_path: PathBuf,

    /// Map of target name (IQN) to metadata
    pub targets: HashMap<String, TargetMetadata>,
}

/// Metadata for a single iSCSI target
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetMetadata {
    /// Target IQN (e.g., "iqn.2025-12.local.voe:storage.debian-static")
    pub iqn: String,

    /// Friendly name for the target
    pub name: String,

    /// Target size in megabytes
    pub size_mb: u64,

    /// Path to the sled index database
    pub index_path: PathBuf,

    /// Parent target IQN if this is a clone
    pub parent: Option<String>,

    /// Child target IQNs (clones of this target)
    pub children: Vec<String>,

    /// Creation timestamp (Unix epoch seconds)
    pub created_at: u64,

    /// Optional description
    pub description: Option<String>,
}

impl TargetRegistry {
    /// Create a new empty registry
    pub fn new(registry_path: PathBuf) -> Self {
        Self {
            registry_path,
            targets: HashMap::new(),
        }
    }

    /// Load registry from disk, or create a new one if it doesn't exist
    pub fn load_or_create<P: AsRef<Path>>(registry_path: P) -> Result<Self> {
        let path = registry_path.as_ref();

        if path.exists() {
            Self::load(path)
        } else {
            log::info!("Creating new registry at {:?}", path);

            // Ensure parent directory exists
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create registry directory: {:?}", parent))?;
            }

            let registry = Self::new(path.to_path_buf());
            registry.save()?;
            Ok(registry)
        }
    }

    /// Load registry from disk
    pub fn load<P: AsRef<Path>>(registry_path: P) -> Result<Self> {
        let path = registry_path.as_ref();
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read registry from {:?}", path))?;

        let mut registry: TargetRegistry = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse registry JSON from {:?}", path))?;

        registry.registry_path = path.to_path_buf();

        log::debug!("Loaded registry with {} target(s)", registry.targets.len());
        Ok(registry)
    }

    /// Save registry to disk
    pub fn save(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(&self)
            .context("Failed to serialize registry to JSON")?;

        fs::write(&self.registry_path, json)
            .with_context(|| format!("Failed to write registry to {:?}", self.registry_path))?;

        log::debug!("Saved registry with {} target(s)", self.targets.len());
        Ok(())
    }

    /// Add a new target to the registry
    pub fn add_target(&mut self, metadata: TargetMetadata) -> Result<()> {
        if self.targets.contains_key(&metadata.iqn) {
            anyhow::bail!("Target already exists: {}", metadata.iqn);
        }

        // If this is a clone, update parent's children list
        if let Some(ref parent_iqn) = metadata.parent {
            let parent = self.targets.get_mut(parent_iqn)
                .ok_or_else(|| anyhow::anyhow!("Parent target not found: {}", parent_iqn))?;

            if !parent.children.contains(&metadata.iqn) {
                parent.children.push(metadata.iqn.clone());
            }
        }

        log::info!("Adding target to registry: {}", metadata.iqn);
        self.targets.insert(metadata.iqn.clone(), metadata);
        self.save()?;

        Ok(())
    }

    /// Remove a target from the registry
    pub fn remove_target(&mut self, iqn: &str) -> Result<TargetMetadata> {
        let metadata = self.targets.remove(iqn)
            .ok_or_else(|| anyhow::anyhow!("Target not found: {}", iqn))?;

        // Remove from parent's children list
        if let Some(ref parent_iqn) = metadata.parent {
            if let Some(parent) = self.targets.get_mut(parent_iqn) {
                parent.children.retain(|child| child != iqn);
            }
        }

        // Warn if this target has children
        if !metadata.children.is_empty() {
            log::warn!("Removing target {} which has {} child(ren): {:?}",
                iqn, metadata.children.len(), metadata.children);
        }

        log::info!("Removed target from registry: {}", iqn);
        self.save()?;

        Ok(metadata)
    }

    /// Get target metadata by IQN
    pub fn get_target(&self, iqn: &str) -> Option<&TargetMetadata> {
        self.targets.get(iqn)
    }

    /// Get mutable target metadata by IQN
    pub fn get_target_mut(&mut self, iqn: &str) -> Option<&mut TargetMetadata> {
        self.targets.get_mut(iqn)
    }

    /// List all targets
    pub fn list_targets(&self) -> Vec<&TargetMetadata> {
        let mut targets: Vec<_> = self.targets.values().collect();
        targets.sort_by_key(|t| &t.iqn);
        targets
    }

    /// Get all root targets (no parent)
    pub fn get_root_targets(&self) -> Vec<&TargetMetadata> {
        let mut roots: Vec<_> = self.targets.values()
            .filter(|t| t.parent.is_none())
            .collect();
        roots.sort_by_key(|t| &t.iqn);
        roots
    }

    /// Get all children of a target
    pub fn get_children(&self, iqn: &str) -> Vec<&TargetMetadata> {
        self.targets.get(iqn)
            .map(|target| {
                target.children.iter()
                    .filter_map(|child_iqn| self.targets.get(child_iqn))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get the full clone tree starting from a root target
    pub fn get_clone_tree(&self, iqn: &str) -> Option<CloneTree> {
        let target = self.targets.get(iqn)?;
        Some(self.build_clone_tree(target))
    }

    fn build_clone_tree(&self, target: &TargetMetadata) -> CloneTree {
        let children = target.children.iter()
            .filter_map(|child_iqn| self.targets.get(child_iqn))
            .map(|child| self.build_clone_tree(child))
            .collect();

        CloneTree {
            iqn: target.iqn.clone(),
            name: target.name.clone(),
            size_mb: target.size_mb,
            children,
        }
    }

    /// Generate an IQN for a target name
    pub fn generate_iqn(name: &str) -> String {
        // Generate IQN in format: iqn.YYYY-MM.local.voe:storage.name
        let now = std::time::SystemTime::now();
        let datetime = chrono::DateTime::<chrono::Utc>::from(now);
        let date_str = datetime.format("%Y-%m").to_string();

        // Sanitize name: lowercase, replace non-alphanumeric with dash
        let sanitized = name.to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect::<String>();

        format!("iqn.{}.local.voe:storage.{}", date_str, sanitized)
    }

    /// Get current Unix timestamp
    pub fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("System time before Unix epoch")
            .as_secs()
    }
}

/// Clone tree visualization structure
#[derive(Debug, Clone)]
pub struct CloneTree {
    pub iqn: String,
    pub name: String,
    pub size_mb: u64,
    pub children: Vec<CloneTree>,
}

impl CloneTree {
    /// Print the clone tree with indentation
    pub fn print(&self, indent: usize) {
        let prefix = "  ".repeat(indent);
        println!("{}{} ({} MB)", prefix, self.name, self.size_mb);
        println!("{}  IQN: {}", prefix, self.iqn);

        if !self.children.is_empty() {
            println!("{}  Children:", prefix);
            for child in &self.children {
                child.print(indent + 2);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_iqn() {
        let iqn = TargetRegistry::generate_iqn("debian-static");
        assert!(iqn.starts_with("iqn."));
        assert!(iqn.contains(".local.voe:storage.debian-static"));
    }

    #[test]
    fn test_iqn_sanitization() {
        let iqn = TargetRegistry::generate_iqn("My Test/Target");
        assert!(iqn.contains("my-test-target"));
    }

    #[test]
    fn test_registry_operations() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let registry_path = temp_dir.path().join("registry.json");

        let mut registry = TargetRegistry::load_or_create(&registry_path)?;

        // Add a target
        let metadata = TargetMetadata {
            iqn: "iqn.2025-12.local.voe:storage.test".to_string(),
            name: "test".to_string(),
            size_mb: 100,
            index_path: PathBuf::from("/tmp/test/index"),
            parent: None,
            children: vec![],
            created_at: TargetRegistry::now(),
            description: Some("Test target".to_string()),
        };

        registry.add_target(metadata.clone())?;
        assert_eq!(registry.targets.len(), 1);

        // Reload and verify
        let registry2 = TargetRegistry::load(&registry_path)?;
        assert_eq!(registry2.targets.len(), 1);
        assert!(registry2.get_target(&metadata.iqn).is_some());

        Ok(())
    }
}
