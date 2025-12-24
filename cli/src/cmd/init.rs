use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use agentfs_sdk::{agentfs_dir, AgentFS, AgentFSOptions, OverlayFS};
use anyhow::{Context, Result as AnyhowResult};

use crate::parser::SyncConfig;

pub async fn init_database(
    id: Option<String>,
    sync_config_path: Option<PathBuf>,
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
    if !AgentFSOptions::validate_agent_id(&id) {
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

    // Use the SDK to initialize the database - this ensures consistency
    // The SDK will create .agentfs directory and database file
    let sync_config = SyncConfig::parse(sync_config_path)?;
    let options = AgentFSOptions::with_id(&id);
    let mut synced_db = None;
    let agent = if let Some(sync) = sync_config {
        let mut builder = turso::sync::Builder::new_remote(&options.db_path()?, &sync.remote_url);
        if let Some(auth_token) = sync.auth_token {
            builder = builder.with_auth_token(auth_token);
        }
        let db = builder.build().await?;
        let conn = db.connect().await?;
        let agent = AgentFS::open_with(conn)
            .await
            .context("Failed to initialize synced database")?;
        synced_db = Some(db);
        agent
    } else {
        AgentFS::open(options)
            .await
            .context("Failed to initialize database")?
    };

    // If base is provided, initialize the overlay schema using the SDK
    if let Some(base_path) = base {
        let base_path_str = base_path
            .canonicalize()
            .context("Failed to canonicalize base path")?
            .to_string_lossy()
            .to_string();

        // Use SDK's OverlayFS::init_schema to ensure schema consistency
        OverlayFS::init_schema(&agent.get_connection(), &base_path_str)
            .await
            .context("Failed to initialize overlay schema")?;

        if let Some(synced_db) = synced_db {
            synced_db.push().await?;
        }

        eprintln!("Created overlay filesystem: {}", db_path.display());
        eprintln!("Agent ID: {}", id);
        eprintln!("Base: {}", base_path_str);
    } else {
        if let Some(synced_db) = synced_db {
            synced_db.push().await?;
        }

        eprintln!("Created agent filesystem: {}", db_path.display());
        eprintln!("Agent ID: {}", id);
    }

    Ok(())
}
