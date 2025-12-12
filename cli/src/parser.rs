use crate::cmd::{completions::Shell, MountConfig};
use clap::{Parser, Subcommand};
use clap_complete::{ArgValueCompleter, PathCompleter};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "agentfs")]
#[command(about = "The filesystem for agents", long_about = None)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Manage shell completions
    Completions {
        #[command(subcommand)]
        command: CompletionsCommand,
    },
    /// Initialize a new agent filesystem
    Init {
        /// Agent identifier (if not provided, generates a unique one)
        id: Option<String>,

        /// Overwrite existing file if it exists
        #[arg(long)]
        force: bool,
    },
    /// Filesystem operations
    Fs {
        #[command(subcommand)]
        command: FsCommand,
    },
    /// Run a command in the sandboxed environment (experimental).
    Run {
        /// Mount configuration (format: type=bind,src=<host_path>,dst=<sandbox_path>)
        #[arg(long = "mount", value_name = "MOUNT_SPEC")]
        mounts: Vec<MountConfig>,

        /// Enable strace-like output for system calls
        #[arg(long = "strace")]
        strace: bool,

        /// Command to execute
        command: PathBuf,

        /// Arguments for the command
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Mount an agent filesystem using FUSE
    Mount {
        /// Agent ID or database path
        #[arg(value_name = "ID_OR_PATH")]
        id_or_path: String,

        /// Mount point directory
        #[arg(value_name = "MOUNTPOINT", add = ArgValueCompleter::new(PathCompleter::dir()))]
        mountpoint: PathBuf,

        /// Automatically unmount on exit
        #[arg(short = 'a', long)]
        auto_unmount: bool,

        /// Allow root user to access filesystem
        #[arg(long)]
        allow_root: bool,

        /// Run in foreground (don't daemonize)
        #[arg(short = 'f', long)]
        foreground: bool,

        /// User ID to report for all files (defaults to current user)
        #[arg(long)]
        uid: Option<u32>,

        /// Group ID to report for all files (defaults to current group)
        #[arg(long)]
        gid: Option<u32>,
    },
}

#[derive(Subcommand, Debug)]
pub enum FsCommand {
    /// List files in the filesystem
    Ls {
        /// Agent ID or database path
        id_or_path: String,

        /// Path to list (default: /)
        #[arg(default_value = "/")]
        fs_path: String,
    },
    /// Display file contents
    Cat {
        /// Agent ID or database path
        id_or_path: String,

        /// Path to the file in the filesystem
        file_path: String,
    },
}

#[derive(Subcommand, Debug, Clone, Copy)]
pub enum CompletionsCommand {
    /// Install shell completions to your shell rc file
    Install {
        /// Shell to install completions for (defaults to current shell)
        #[arg(value_enum)]
        shell: Option<Shell>,
    },
    /// Uninstall shell completions from your shell rc file
    Uninstall {
        /// Shell to uninstall completions for (defaults to current shell)
        #[arg(value_enum)]
        shell: Option<Shell>,
    },
    /// Print instructions for manual installation
    Show,
}
