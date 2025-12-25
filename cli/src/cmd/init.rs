use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use agentfs_sdk::{agentfs_dir, AgentFS, AgentFSOptions};
use anyhow::{Context, Result as AnyhowResult};

pub async fn init_database(
    id: Option<String>,
    force: bool,
    base: Option<PathBuf>,
) -> AnyhowResult<()> {
    // Generate ID if not provided
    let id = id.unwrap_or_else(|| {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        format!("agent-{}", timestamp)
    });

    // Validate agent ID for safety
    if !AgentFS::validate_agent_id(&id) {
        anyhow::bail!(
            "Invalid agent ID '{}'. Agent IDs must contain only alphanumeric characters, hyphens, and underscores.",
            id
        );
    }

    // Validate base directory if provided
    if let Some(ref base_path) = base {
        if !base_path.exists() {
            anyhow::bail!("Base directory does not exist: {}", base_path.display());
        }
        if !base_path.is_dir() {
            anyhow::bail!("Base path is not a directory: {}", base_path.display());
        }
    }

    // Check if agent already exists
    let db_path = agentfs_dir().join(format!("{}.db", &id));
    if db_path.exists() {
        if force {
            for entry in std::fs::read_dir(agentfs_dir())? {
                let entry = entry?;
                let file_name = entry.file_name();
                if file_name.to_string_lossy().starts_with(&id) {
                    std::fs::remove_file(entry.path())
                        .context("Failed to remove existing database file(s)")?;
                }
            }
        } else {
            anyhow::bail!(
                "Agent '{}' already exists at '{}'. Use --force to overwrite.",
                id,
                db_path.display()
            );
        }
    }

    let mut open_options = AgentFSOptions::with_id(&id);
    if let Some(base_path) = base.as_ref() {
        open_options = open_options.with_base(base_path);
    }

    // Use the SDK to initialize the database - this ensures consistency
    // The SDK will create .agentfs directory and database file
    let _agent = AgentFS::open(open_options)
        .await
        .context("Failed to initialize database")?;

    // If base is provided, initialize the overlay schema using the SDK
    if let Some(base_path) = base {
        eprintln!("Created overlay filesystem: {}", db_path.display());
        eprintln!("Agent ID: {}", id);
        eprintln!("Base: {}", base_path.display());
    } else {
        eprintln!("Created agent filesystem: {}", db_path.display());
        eprintln!("Agent ID: {}", id);
    }

    Ok(())
}
