pub mod completions;
pub mod fs;
pub mod init;
pub mod mcp_server;
pub mod sync;
pub mod timeline;

#[cfg(target_os = "linux")]
mod mount;
#[cfg(not(target_os = "linux"))]
#[path = "mount_stub.rs"]
mod mount;

mod run;

// Standalone NFS server command (Unix only)
#[cfg(unix)]
pub mod nfs;

pub use mount::{mount, MountArgs};
pub use run::handle_run_command;
