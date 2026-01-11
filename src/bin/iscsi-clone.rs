//! iSCSI target clone management CLI
//!
//! Commands:
//! - create: Create a new target
//! - clone: Clone an existing target
//! - list: List all targets
//! - info: Show target details
//! - delete: Delete a target
//! - gc: Garbage collect CAS blocks (Phase 3)

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use env_logger::Env;
use std::collections::HashSet;
use std::path::PathBuf;

use aoe_server::iscsi::{CloneManager, TargetRegistry};

#[derive(Parser)]
#[command(name = "iscsi-clone")]
#[command(about = "iSCSI target clone management", long_about = None)]
struct Cli {
    /// Path to registry file
    #[arg(long, default_value = "/var/lib/voe-iscsi/registry.json")]
    registry: PathBuf,

    /// Base directory for target indexes
    #[arg(long, default_value = "/var/lib/voe-iscsi/targets")]
    targets_dir: PathBuf,

    /// CAS server address
    #[arg(long, default_value = "127.0.0.1:3000")]
    cas_server: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new target
    Create {
        /// Target name (will generate IQN)
        name: String,

        /// Target size in megabytes
        #[arg(short, long)]
        size_mb: u64,

        /// Optional description
        #[arg(short, long)]
        description: Option<String>,
    },

    /// Clone an existing target
    Clone {
        /// Source target IQN or name
        source: String,

        /// Destination target name
        dest: String,
    },

    /// List all targets
    List {
        /// Show as tree (organized by parent/child relationships)
        #[arg(short, long)]
        tree: bool,
    },

    /// Show target information
    Info {
        /// Target IQN or name
        target: String,

        /// Show detailed statistics
        #[arg(short, long)]
        stats: bool,
    },

    /// Delete a target
    Delete {
        /// Target IQN or name
        target: String,

        /// Remove target data (default: keep data)
        #[arg(short, long)]
        purge: bool,

        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Garbage collect CAS blocks (Phase 3)
    Gc {
        /// Target IQN or name
        target: String,

        /// Dry run - show what would be deleted without actually deleting
        #[arg(short = 'n', long)]
        dry_run: bool,
    },
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let cli = Cli::parse();

    match &cli.command {
        Commands::Create { name, size_mb, description } => {
            cmd_create(&cli, name, *size_mb, description.clone())
        }
        Commands::Clone { source, dest } => {
            cmd_clone(&cli, source, dest)
        }
        Commands::List { tree } => {
            cmd_list(&cli, *tree)
        }
        Commands::Info { target, stats } => {
            cmd_info(&cli, target, *stats)
        }
        Commands::Delete { target, purge, yes } => {
            cmd_delete(&cli, target, *purge, *yes)
        }
        Commands::Gc { target, dry_run } => {
            cmd_gc(&cli, target, *dry_run)
        }
    }
}

fn cmd_create(cli: &Cli, name: &str, size_mb: u64, description: Option<String>) -> Result<()> {
    let mut manager = CloneManager::new(cli.registry.clone(), cli.targets_dir.clone(), cli.cas_server.clone())?;

    println!("Creating target: {} ({} MB)", name, size_mb);

    let iqn = manager.create_target(name, size_mb, description)?;

    println!("✓ Created target:");
    println!("  Name: {}", name);
    println!("  IQN:  {}", iqn);
    println!("  Size: {} MB", size_mb);

    Ok(())
}

fn cmd_clone(cli: &Cli, source: &str, dest: &str) -> Result<()> {
    let mut manager = CloneManager::new(cli.registry.clone(), cli.targets_dir.clone(), cli.cas_server.clone())?;

    // Resolve source IQN
    let source_iqn = resolve_target_iqn(&manager.registry, source)?;

    println!("Cloning target:");
    println!("  Source: {}", source_iqn);
    println!("  Dest:   {}", dest);

    let dest_iqn = manager.clone_target(&source_iqn, dest)?;

    println!("✓ Cloned target:");
    println!("  Destination name: {}", dest);
    println!("  Destination IQN:  {}", dest_iqn);

    Ok(())
}

fn cmd_list(cli: &Cli, tree: bool) -> Result<()> {
    let registry = TargetRegistry::load_or_create(&cli.registry)?;

    if registry.targets.is_empty() {
        println!("No targets configured.");
        return Ok(());
    }

    if tree {
        // Show as tree organized by parent/child relationships
        println!("Target Clone Tree:\n");

        let roots = registry.get_root_targets();
        for root in roots {
            if let Some(tree) = registry.get_clone_tree(&root.iqn) {
                tree.print(0);
                println!();
            }
        }
    } else {
        // Simple list
        println!("Configured Targets:\n");

        let targets = registry.list_targets();
        for target in targets {
            println!("{}", target.name);
            println!("  IQN:     {}", target.iqn);
            println!("  Size:    {} MB", target.size_mb);
            println!("  Index:   {:?}", target.index_path);

            if let Some(ref parent) = target.parent {
                println!("  Parent:  {}", parent);
            }

            if !target.children.is_empty() {
                println!("  Children: {}", target.children.len());
            }

            println!();
        }
    }

    Ok(())
}

fn cmd_info(cli: &Cli, target: &str, stats: bool) -> Result<()> {
    let manager = CloneManager::new(cli.registry.clone(), cli.targets_dir.clone(), cli.cas_server.clone())?;

    let iqn = resolve_target_iqn(&manager.registry, target)?;
    let metadata = manager.registry.get_target(&iqn)
        .ok_or_else(|| anyhow::anyhow!("Target not found: {}", iqn))?;

    println!("Target Information:");
    println!("  Name:        {}", metadata.name);
    println!("  IQN:         {}", metadata.iqn);
    println!("  Size:        {} MB", metadata.size_mb);
    println!("  Index Path:  {:?}", metadata.index_path);
    println!("  Created:     {}", format_timestamp(metadata.created_at));

    if let Some(ref desc) = metadata.description {
        println!("  Description: {}", desc);
    }

    if let Some(ref parent) = metadata.parent {
        println!("  Parent:      {}", parent);
    }

    if !metadata.children.is_empty() {
        println!("  Children:    {}", metadata.children.len());
        for child in &metadata.children {
            println!("    - {}", child);
        }
    }

    // Check if running
    let running = manager.is_target_running(&iqn)?;
    println!("  Running:     {}", if running { "YES" } else { "NO" });

    if stats {
        println!("\nStatistics:");

        // Count blocks in index
        if metadata.index_path.exists() {
            match sled::open(&metadata.index_path) {
                Ok(db) => {
                    println!("  Index entries: {}", db.len());
                }
                Err(e) => {
                    println!("  Index entries: Error - {}", e);
                }
            }
        } else {
            println!("  Index entries: N/A (not created yet)");
        }
    }

    Ok(())
}

fn cmd_delete(cli: &Cli, target: &str, purge: bool, yes: bool) -> Result<()> {
    let mut manager = CloneManager::new(cli.registry.clone(), cli.targets_dir.clone(), cli.cas_server.clone())?;

    let iqn = resolve_target_iqn(&manager.registry, target)?;
    let metadata = manager.registry.get_target(&iqn)
        .ok_or_else(|| anyhow::anyhow!("Target not found: {}", iqn))?
        .clone();

    // Show what will be deleted
    println!("Target to delete:");
    println!("  Name: {}", metadata.name);
    println!("  IQN:  {}", iqn);

    if !metadata.children.is_empty() {
        println!("  ⚠ Warning: This target has {} child(ren)", metadata.children.len());
    }

    if purge {
        println!("  ⚠ Warning: Target data will be PERMANENTLY removed");
    } else {
        println!("  Note: Target data will be kept (use --purge to remove)");
    }

    // Confirm deletion
    if !yes {
        print!("\nProceed with deletion? [y/N]: ");
        use std::io::{self, Write};
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if input.trim().to_lowercase() != "y" {
            println!("Deletion cancelled.");
            return Ok(());
        }
    }

    manager.delete_target(&iqn, purge)?;

    println!("✓ Target deleted: {}", metadata.name);

    Ok(())
}

fn cmd_gc(cli: &Cli, target: &str, dry_run: bool) -> Result<()> {
    use std::net::TcpStream;
    use std::io::{BufReader, BufWriter};
    use aoe_server::cas::protocol::{write_frame, read_frame, CasCommand};
    use aoe_server::cas::Hash;

    let manager = CloneManager::new(cli.registry.clone(), cli.targets_dir.clone(), cli.cas_server.clone())?;

    // Resolve target IQN
    let target_iqn = resolve_target_iqn(&manager.registry, target)?;
    let target_metadata = manager.registry.get_target(&target_iqn)
        .ok_or_else(|| anyhow::anyhow!("Target not found: {}", target_iqn))?;

    println!("Garbage collecting CAS blocks for target: {}", target_metadata.name);
    println!("Target IQN: {}", target_iqn);

    // Check if target is running
    if manager.is_target_running(&target_iqn)? {
        anyhow::bail!("Target is currently running: {}. Stop it first.", target_iqn);
    }

    // Step 1: Collect all hashes from the target
    println!("\nStep 1: Collecting hashes from target...");
    let target_hashes = collect_target_hashes(&target_metadata.index_path)?;
    println!("  Found {} unique hashes in target", target_hashes.len());

    // Step 2: Collect all hashes from all OTHER targets
    println!("\nStep 2: Collecting hashes from all other targets...");
    let mut other_hashes = HashSet::new();
    let mut other_count = 0;

    for (other_iqn, other_metadata) in &manager.registry.targets {
        if other_iqn == &target_iqn {
            continue; // Skip the target we're GC'ing
        }

        if !other_metadata.index_path.exists() {
            println!("  Skipping {} (index not created yet)", other_metadata.name);
            continue;
        }

        println!("  Scanning {}...", other_metadata.name);
        let hashes = collect_target_hashes(&other_metadata.index_path)?;
        other_hashes.extend(hashes);
        other_count += 1;
    }

    println!("  Found {} unique hashes across {} other targets", other_hashes.len(), other_count);

    // Step 3: Compute difference (blocks unique to this target)
    println!("\nStep 3: Finding blocks unique to target...");
    let unique_hashes: Vec<Hash> = target_hashes
        .difference(&other_hashes)
        .cloned()
        .collect();

    println!("  Found {} blocks unique to target", unique_hashes.len());

    if unique_hashes.is_empty() {
        println!("\nNo unique blocks to delete. All blocks are shared with other targets.");
        return Ok(());
    }

    // Calculate approximate space (assuming 4KB blocks)
    let approx_space_mb = (unique_hashes.len() * 4096) / (1024 * 1024);
    println!("  Approximate space to reclaim: {} MB", approx_space_mb);

    if dry_run {
        println!("\n✓ Dry run complete. Use without --dry-run to actually delete blocks.");
        return Ok(());
    }

    // Step 4: Delete unique blocks from CAS
    println!("\nStep 4: Deleting unique blocks from CAS...");
    println!("  Connecting to CAS server at {}", cli.cas_server);

    let stream = TcpStream::connect(&cli.cas_server)
        .with_context(|| format!("Failed to connect to CAS server: {}", cli.cas_server))?;

    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = BufWriter::new(stream);

    let mut deleted_count = 0;
    let mut not_found_count = 0;
    let mut error_count = 0;

    for (i, hash) in unique_hashes.iter().enumerate() {
        if i % 100 == 0 {
            println!("  Progress: {}/{} blocks...", i, unique_hashes.len());
        }

        // Send DELETE command
        if let Err(e) = write_frame(&mut writer, CasCommand::Delete, hash) {
            log::warn!("Failed to send DELETE for hash {}: {}", hex::encode(hash), e);
            error_count += 1;
            continue;
        }

        // Read response
        match read_frame(&mut reader) {
            Ok((CasCommand::Delete, response_data)) => {
                if response_data.len() == 1 {
                    let deleted = response_data[0] != 0;
                    if deleted {
                        deleted_count += 1;
                    } else {
                        not_found_count += 1;
                    }
                } else {
                    log::warn!("Invalid DELETE response for hash {}", hex::encode(hash));
                    error_count += 1;
                }
            }
            Ok((cmd, _)) => {
                log::warn!("Unexpected response command: {:?}", cmd);
                error_count += 1;
            }
            Err(e) => {
                log::warn!("Failed to read DELETE response for hash {}: {}", hex::encode(hash), e);
                error_count += 1;
            }
        }
    }

    println!("\n✓ Garbage collection complete:");
    println!("  Deleted: {} blocks", deleted_count);
    println!("  Not found: {} blocks", not_found_count);
    println!("  Errors: {} blocks", error_count);
    println!("  Approximate space reclaimed: {} MB", (deleted_count * 4096) / (1024 * 1024));

    Ok(())
}

/// Collect all unique hashes from a target's sled database
fn collect_target_hashes(index_path: &std::path::Path) -> Result<HashSet<aoe_server::cas::Hash>> {
    if !index_path.exists() {
        anyhow::bail!("Index path does not exist: {:?}", index_path);
    }

    let db = sled::open(index_path)
        .with_context(|| format!("Failed to open database: {:?}", index_path))?;

    let mut hashes = HashSet::new();
    let zero_block_key = b"__ZERO_BLOCK__";

    for entry_result in db.iter() {
        let (key, value) = entry_result.context("Failed to read entry from database")?;

        // Skip the zero block hash key (it's a special metadata key)
        if key.as_ref() == zero_block_key {
            continue;
        }

        // Each value is a 16-byte hash
        if value.len() == 16 {
            let mut hash = [0u8; 16];
            hash.copy_from_slice(&value);
            hashes.insert(hash);
        } else {
            log::warn!("Skipping entry with invalid hash size: {} bytes", value.len());
        }
    }

    Ok(hashes)
}

/// Resolve a target name or IQN to a full IQN
fn resolve_target_iqn(registry: &TargetRegistry, target: &str) -> Result<String> {
    // If it looks like an IQN, use it directly
    if target.starts_with("iqn.") {
        if registry.get_target(target).is_none() {
            anyhow::bail!("Target not found: {}", target);
        }
        return Ok(target.to_string());
    }

    // Otherwise, treat it as a name and search
    for metadata in registry.targets.values() {
        if metadata.name == target {
            return Ok(metadata.iqn.clone());
        }
    }

    anyhow::bail!("Target not found: {}", target);
}

/// Format Unix timestamp as human-readable date
fn format_timestamp(timestamp: u64) -> String {
    let datetime = chrono::DateTime::from_timestamp(timestamp as i64, 0)
        .unwrap_or_else(|| chrono::DateTime::UNIX_EPOCH);
    datetime.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}
