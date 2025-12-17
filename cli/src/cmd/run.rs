//! Run command handler for Linux x86_64.
//!
//! Dispatches to either the overlay sandbox (default) or the experimental
//! ptrace-based sandbox based on command-line flags.

use anyhow::Result;
use std::path::PathBuf;

/// Handle the `run` command, dispatching to the appropriate sandbox implementation.
pub async fn handle_run_command(
    experimental_sandbox: bool,
    strace: bool,
    command: PathBuf,
    args: Vec<String>,
) -> Result<()> {
    if experimental_sandbox {
        crate::sandbox::ptrace::run_cmd(strace, command, args).await;
    } else {
        if strace {
            eprintln!("Warning: --strace is only supported with --experimental-sandbox, ignoring");
        }
        crate::sandbox::overlay::run_cmd(command, args).await?;
    }
    Ok(())
}
