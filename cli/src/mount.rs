//! Generic mount infrastructure for AgentFS.
//!
//! This module provides a unified mount API that abstracts over FUSE and NFS backends.
//! The `mount_fs()` function returns a `MountHandle` that automatically unmounts when dropped.
//!
//! # Example
//!
//! ```ignore
//! use agentfs_cli::mount::{mount_fs, MountOpts, MountBackend};
//!
//! let opts = MountOpts::new(PathBuf::from("/mnt/agent"), MountBackend::Fuse);
//! let handle = mount_fs(Arc::new(Mutex::new(my_fs)), opts).await?;
//! // ... use the mounted filesystem ...
//! drop(handle); // auto-unmounts
//! ```

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::nfs::AgentNFS;
pub use crate::parser::MountBackend;
use zerofs_nfsserve::tcp::NFSTcp;

/// Default NFS port to try (use a high port to avoid needing root).
const DEFAULT_NFS_PORT: u32 = 11111;

/// Default timeout for mount to become ready.
const DEFAULT_MOUNT_TIMEOUT: Duration = Duration::from_secs(10);

/// Options for mounting a filesystem.
///
/// This struct provides a unified configuration for both FUSE and NFS backends.
/// Use `MountOpts::new()` to create default options, then customize as needed.
#[derive(Debug, Clone)]
pub struct MountOpts {
    /// The mountpoint path.
    pub mountpoint: PathBuf,
    /// Mount backend to use.
    pub backend: MountBackend,
    /// Filesystem name shown in mount output.
    pub fsname: String,
    /// User ID to report for all files.
    pub uid: Option<u32>,
    /// Group ID to report for all files.
    pub gid: Option<u32>,
    /// Allow other system users to access the mount.
    pub allow_other: bool,
    /// Allow root to access the mount (FUSE only).
    pub allow_root: bool,
    /// Auto unmount when process exits (FUSE only).
    pub auto_unmount: bool,
    /// Use lazy unmount on cleanup.
    pub lazy_unmount: bool,
    /// Timeout for mount to become ready.
    pub timeout: Duration,
}

impl MountOpts {
    /// Create default options for the given mountpoint and backend.
    pub fn new(mountpoint: PathBuf, backend: MountBackend) -> Self {
        Self {
            mountpoint,
            backend,
            fsname: "agentfs".to_string(),
            uid: None,
            gid: None,
            allow_other: false,
            allow_root: false,
            auto_unmount: false,
            lazy_unmount: false,
            timeout: DEFAULT_MOUNT_TIMEOUT,
        }
    }
}

impl Default for MountOpts {
    fn default() -> Self {
        Self::new(PathBuf::new(), MountBackend::default())
    }
}

/// A mounted filesystem handle. Automatically unmounts when dropped.
///
/// This handle represents an active mount and provides RAII-style cleanup.
/// When the handle is dropped, the filesystem is automatically unmounted.
pub struct MountHandle {
    mountpoint: PathBuf,
    backend: MountBackend,
    lazy_unmount: bool,
    inner: MountHandleInner,
}

enum MountHandleInner {
    #[cfg(target_os = "linux")]
    Fuse {
        _thread: std::thread::JoinHandle<anyhow::Result<()>>,
    },
    Nfs {
        shutdown: CancellationToken,
        _server_handle: tokio::task::JoinHandle<()>,
    },
}

impl MountHandle {
    /// Get the mountpoint path.
    pub fn mountpoint(&self) -> &Path {
        &self.mountpoint
    }
}

impl Drop for MountHandle {
    fn drop(&mut self) {
        // Move away from mountpoint before unmounting to avoid EBUSY
        let _ = std::env::set_current_dir("/");

        match &self.inner {
            #[cfg(target_os = "linux")]
            MountHandleInner::Fuse { .. } => {
                if let Err(e) = unmount(&self.mountpoint, self.backend, self.lazy_unmount) {
                    eprintln!(
                        "Warning: Failed to unmount FUSE filesystem at {}: {}",
                        self.mountpoint.display(),
                        e
                    );
                }
            }
            MountHandleInner::Nfs { shutdown, .. } => {
                // Signal the NFS server to shut down
                shutdown.cancel();

                // Unmount the NFS filesystem
                if let Err(e) = unmount(&self.mountpoint, self.backend, self.lazy_unmount) {
                    eprintln!(
                        "Warning: Failed to unmount NFS filesystem at {}: {}",
                        self.mountpoint.display(),
                        e
                    );
                }
            }
        }
    }
}

/// Unmount a filesystem at the given mountpoint.
///
/// This function handles unmounting for both FUSE and NFS backends.
/// If `lazy` is true, uses lazy unmount which detaches immediately even if busy.
pub fn unmount(mountpoint: &Path, backend: MountBackend, lazy: bool) -> Result<()> {
    match backend {
        MountBackend::Fuse => unmount_fuse(mountpoint, lazy),
        MountBackend::Nfs => unmount_nfs(mountpoint, lazy),
    }
}

/// FUSE unmount implementation using fusermount.
#[cfg(target_os = "linux")]
fn unmount_fuse(mountpoint: &Path, lazy: bool) -> Result<()> {
    const FUSERMOUNT_COMMANDS: &[&str] = &["fusermount3", "fusermount"];
    let args: &[&str] = if lazy { &["-uz"] } else { &["-u"] };

    for cmd in FUSERMOUNT_COMMANDS {
        let result = Command::new(cmd)
            .args(args)
            .arg(mountpoint.as_os_str())
            .status();

        match result {
            Ok(status) if status.success() => return Ok(()),
            Ok(_) => continue,
            Err(_) => continue,
        }
    }

    anyhow::bail!(
        "Failed to unmount {}. You may need to unmount manually with: fusermount -u {}",
        mountpoint.display(),
        mountpoint.display()
    )
}

/// NFS unmount implementation (Linux).
#[cfg(target_os = "linux")]
fn unmount_nfs(mountpoint: &Path, lazy: bool) -> Result<()> {
    let output = if lazy {
        Command::new("umount")
            .arg("-l")
            .arg(mountpoint)
            .output()
            .context("Failed to execute umount")?
    } else {
        Command::new("umount")
            .arg(mountpoint)
            .output()
            .context("Failed to execute umount")?
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !lazy {
            let output2 = Command::new("umount").arg("-l").arg(mountpoint).output()?;
            if output2.status.success() {
                return Ok(());
            }
        }
        anyhow::bail!(
            "Failed to unmount: {}. You may need to manually unmount with: umount -l {}",
            stderr.trim(),
            mountpoint.display()
        );
    }

    Ok(())
}

/// NFS unmount implementation (macOS).
#[cfg(target_os = "macos")]
fn unmount_nfs(mountpoint: &Path, lazy: bool) -> Result<()> {
    let _ = lazy;
    let output = Command::new("/sbin/umount")
        .arg(mountpoint)
        .output()
        .context("Failed to execute umount")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let output2 = Command::new("/sbin/umount")
            .arg("-f")
            .arg(mountpoint)
            .output()?;

        if !output2.status.success() {
            anyhow::bail!(
                "Failed to unmount: {}. You may need to manually unmount with: umount -f {}",
                stderr.trim(),
                mountpoint.display()
            );
        }
    }

    Ok(())
}

/// FUSE unmount is not available on macOS.
#[cfg(target_os = "macos")]
fn unmount_fuse(_mountpoint: &Path, _lazy: bool) -> Result<()> {
    anyhow::bail!("FUSE unmount is not supported on macOS")
}

/// Mount a filesystem with the given options.
///
/// Returns a handle that automatically unmounts when dropped.
/// The filesystem must be wrapped in `Arc<Mutex<dyn FileSystem + Send>>`.
#[cfg(target_os = "linux")]
pub async fn mount_fs(
    fs: Arc<Mutex<dyn agentfs_sdk::FileSystem + Send>>,
    opts: MountOpts,
) -> Result<MountHandle> {
    match opts.backend {
        MountBackend::Fuse => mount_fuse(fs, opts),
        MountBackend::Nfs => mount_nfs(fs, opts).await,
    }
}

/// Mount a filesystem with the given options (macOS version).
#[cfg(target_os = "macos")]
pub async fn mount_fs(
    fs: Arc<Mutex<dyn agentfs_sdk::FileSystem + Send>>,
    opts: MountOpts,
) -> Result<MountHandle> {
    match opts.backend {
        MountBackend::Fuse => {
            anyhow::bail!(
                "FUSE mounting is not supported on macOS.\n\
                 Use --backend nfs (default) instead."
            );
        }
        MountBackend::Nfs => mount_nfs(fs, opts).await,
    }
}

/// Internal FUSE mount implementation.
#[cfg(target_os = "linux")]
fn mount_fuse(
    fs: Arc<Mutex<dyn agentfs_sdk::FileSystem + Send>>,
    opts: MountOpts,
) -> Result<MountHandle> {
    use crate::fuse::FuseMountOptions;

    let fuse_opts = FuseMountOptions {
        mountpoint: opts.mountpoint.clone(),
        auto_unmount: opts.auto_unmount,
        allow_root: opts.allow_root,
        allow_other: opts.allow_other,
        fsname: opts.fsname.clone(),
        uid: opts.uid,
        gid: opts.gid,
    };

    let mountpoint = opts.mountpoint.clone();
    let timeout = opts.timeout;
    let lazy_unmount = opts.lazy_unmount;

    let fs_adapter = MutexFsAdapter { inner: fs };
    let fs_arc: Arc<dyn agentfs_sdk::FileSystem> = Arc::new(fs_adapter);

    let fuse_handle = std::thread::spawn(move || {
        let rt = crate::get_runtime();
        crate::fuse::mount(fs_arc, fuse_opts, rt)
    });

    if !wait_for_mount(&mountpoint, timeout) {
        anyhow::bail!("FUSE mount did not become ready within {:?}", timeout);
    }

    Ok(MountHandle {
        mountpoint,
        backend: MountBackend::Fuse,
        lazy_unmount,
        inner: MountHandleInner::Fuse {
            _thread: fuse_handle,
        },
    })
}

/// Internal NFS mount implementation.
async fn mount_nfs(
    fs: Arc<Mutex<dyn agentfs_sdk::FileSystem + Send>>,
    opts: MountOpts,
) -> Result<MountHandle> {
    let nfs = AgentNFS::new(fs);

    let port = find_available_port(DEFAULT_NFS_PORT)?;

    let bind_addr: std::net::SocketAddr = format!("127.0.0.1:{}", port)
        .parse()
        .context("Invalid bind address")?;
    let listener = zerofs_nfsserve::tcp::NFSTcpListener::bind(bind_addr, nfs)
        .await
        .context("Failed to bind NFS server")?;

    let shutdown = CancellationToken::new();
    let shutdown_clone = shutdown.clone();
    let server_handle = tokio::spawn(async move {
        if let Err(e) = listener.handle_with_shutdown(shutdown_clone).await {
            eprintln!("NFS server error: {}", e);
        }
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    nfs_mount(port, &opts.mountpoint)?;

    Ok(MountHandle {
        mountpoint: opts.mountpoint,
        backend: MountBackend::Nfs,
        lazy_unmount: opts.lazy_unmount,
        inner: MountHandleInner::Nfs {
            shutdown,
            _server_handle: server_handle,
        },
    })
}

/// Adapter to use `Arc<Mutex<dyn FileSystem>>` as `Arc<dyn FileSystem>`.
struct MutexFsAdapter {
    inner: Arc<Mutex<dyn agentfs_sdk::FileSystem + Send>>,
}

#[async_trait::async_trait]
impl agentfs_sdk::FileSystem for MutexFsAdapter {
    async fn stat(
        &self,
        path: &str,
    ) -> std::result::Result<Option<agentfs_sdk::Stats>, agentfs_sdk::error::Error> {
        self.inner.lock().await.stat(path).await
    }

    async fn lstat(
        &self,
        path: &str,
    ) -> std::result::Result<Option<agentfs_sdk::Stats>, agentfs_sdk::error::Error> {
        self.inner.lock().await.lstat(path).await
    }

    async fn read_file(
        &self,
        path: &str,
    ) -> std::result::Result<Option<Vec<u8>>, agentfs_sdk::error::Error> {
        self.inner.lock().await.read_file(path).await
    }

    async fn readdir(
        &self,
        path: &str,
    ) -> std::result::Result<Option<Vec<String>>, agentfs_sdk::error::Error> {
        self.inner.lock().await.readdir(path).await
    }

    async fn readdir_plus(
        &self,
        path: &str,
    ) -> std::result::Result<Option<Vec<agentfs_sdk::DirEntry>>, agentfs_sdk::error::Error> {
        self.inner.lock().await.readdir_plus(path).await
    }

    async fn readlink(
        &self,
        path: &str,
    ) -> std::result::Result<Option<String>, agentfs_sdk::error::Error> {
        self.inner.lock().await.readlink(path).await
    }

    async fn open(
        &self,
        path: &str,
    ) -> std::result::Result<agentfs_sdk::BoxedFile, agentfs_sdk::error::Error> {
        self.inner.lock().await.open(path).await
    }

    async fn create_file(
        &self,
        path: &str,
        mode: u32,
        uid: u32,
        gid: u32,
    ) -> std::result::Result<(agentfs_sdk::Stats, agentfs_sdk::BoxedFile), agentfs_sdk::error::Error>
    {
        self.inner
            .lock()
            .await
            .create_file(path, mode, uid, gid)
            .await
    }

    async fn mkdir(
        &self,
        path: &str,
        uid: u32,
        gid: u32,
    ) -> std::result::Result<(), agentfs_sdk::error::Error> {
        self.inner.lock().await.mkdir(path, uid, gid).await
    }

    async fn mknod(
        &self,
        path: &str,
        mode: u32,
        rdev: u64,
        uid: u32,
        gid: u32,
    ) -> std::result::Result<(), agentfs_sdk::error::Error> {
        self.inner
            .lock()
            .await
            .mknod(path, mode, rdev, uid, gid)
            .await
    }

    async fn remove(&self, path: &str) -> std::result::Result<(), agentfs_sdk::error::Error> {
        self.inner.lock().await.remove(path).await
    }

    async fn rename(
        &self,
        from: &str,
        to: &str,
    ) -> std::result::Result<(), agentfs_sdk::error::Error> {
        self.inner.lock().await.rename(from, to).await
    }

    async fn symlink(
        &self,
        target: &str,
        link_path: &str,
        uid: u32,
        gid: u32,
    ) -> std::result::Result<(), agentfs_sdk::error::Error> {
        self.inner
            .lock()
            .await
            .symlink(target, link_path, uid, gid)
            .await
    }

    async fn link(
        &self,
        old_path: &str,
        new_path: &str,
    ) -> std::result::Result<(), agentfs_sdk::error::Error> {
        self.inner.lock().await.link(old_path, new_path).await
    }

    async fn chmod(
        &self,
        path: &str,
        mode: u32,
    ) -> std::result::Result<(), agentfs_sdk::error::Error> {
        self.inner.lock().await.chmod(path, mode).await
    }

    async fn chown(
        &self,
        path: &str,
        uid: Option<u32>,
        gid: Option<u32>,
    ) -> std::result::Result<(), agentfs_sdk::error::Error> {
        self.inner.lock().await.chown(path, uid, gid).await
    }

    async fn statfs(
        &self,
    ) -> std::result::Result<agentfs_sdk::FilesystemStats, agentfs_sdk::error::Error> {
        self.inner.lock().await.statfs().await
    }
}

/// Find an available TCP port starting from the given port.
fn find_available_port(start_port: u32) -> Result<u32> {
    for port in start_port..start_port + 100 {
        if std::net::TcpListener::bind(format!("127.0.0.1:{}", port)).is_ok() {
            return Ok(port);
        }
    }
    anyhow::bail!(
        "Could not find an available port in range {}-{}",
        start_port,
        start_port + 100
    );
}

/// Mount the NFS filesystem (Linux version).
#[cfg(target_os = "linux")]
fn nfs_mount(port: u32, mountpoint: &Path) -> Result<()> {
    let output = Command::new("mount")
        .args([
            "-t",
            "nfs",
            "-o",
            &format!(
                "vers=3,tcp,port={},mountport={},nolock,soft,timeo=10,retrans=2",
                port, port
            ),
            "127.0.0.1:/",
            mountpoint.to_str().unwrap(),
        ])
        .output()
        .context("Failed to execute mount command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "Failed to mount NFS: {}. Make sure NFS client tools are installed.",
            stderr.trim()
        );
    }

    Ok(())
}

/// Mount the NFS filesystem (macOS version).
#[cfg(target_os = "macos")]
fn nfs_mount(port: u32, mountpoint: &Path) -> Result<()> {
    let output = Command::new("/sbin/mount_nfs")
        .args([
            "-o",
            &format!(
                "locallocks,vers=3,tcp,port={},mountport={},soft,timeo=10,retrans=2",
                port, port
            ),
            "127.0.0.1:/",
            mountpoint.to_str().unwrap(),
        ])
        .output()
        .context("Failed to execute mount_nfs")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to mount NFS: {}", stderr.trim());
    }

    Ok(())
}

/// Wait for a path to become a mountpoint.
pub fn wait_for_mount(path: &Path, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    let interval = Duration::from_millis(50);

    while start.elapsed() < timeout {
        if is_mountpoint(path) {
            return true;
        }
        std::thread::sleep(interval);
    }
    false
}

/// Check if a path is a mountpoint by comparing device IDs with parent.
pub fn is_mountpoint(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let path_meta = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(_) => return false,
        };

        let parent = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("/"),
        };

        let parent_meta = match std::fs::metadata(parent) {
            Ok(m) => m,
            Err(_) => return false,
        };

        path_meta.dev() != parent_meta.dev()
    }

    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}
