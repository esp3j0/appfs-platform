//! Sandbox implementations for running commands in isolated environments.
//!
//! This module provides platform-specific sandbox approaches:
//! - `linux`: FUSE + namespace-based sandbox with copy-on-write filesystem
//! - `linux_ptrace`: ptrace-based syscall interception sandbox (experimental)
//! - `darwin`: Kernel-enforced sandbox using sandbox-exec

#[cfg(all(target_os = "linux", feature = "sandbox"))]
pub mod linux;

#[cfg(all(target_os = "linux", feature = "sandbox"))]
pub mod linux_ptrace;

#[cfg(all(target_os = "macos", feature = "sandbox"))]
pub mod darwin;
