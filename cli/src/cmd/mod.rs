pub mod completions;
pub mod fs;
pub mod init;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod mount;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[path = "mount_stub.rs"]
mod mount;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod run;
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
#[path = "run_stub.rs"]
mod run;

pub use mount::{mount, MountArgs};
pub use run::handle_run_command;
