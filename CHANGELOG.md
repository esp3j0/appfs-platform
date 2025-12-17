# Changelog

## [0.3.1] - 2025-12-17

- This is the exact same version as 0.3.0, but had to bump version number
  to work around a previous accidental publish on PyPI.

## [0.3.0] - 2025-12-17

### Added

- Python SDK for AgentFS.
- Web browser support for TypeScript SDK.
- Dynamic CLI completions with `completion` command (install/uninstall).

### Changed

- TypeScript SDK: Remove `ready()` method from the API.
- TypeScript SDK: Improve `KvStore.get()` API.
- TypeScript SDK: Improve `FileSystem.readFile()` compatibility.
- TypeScript SDK: Use `RETURNING` clause instead of `lastInsertRowid`.
- TypeScript SDK: Switch to proper Turso dependency versioning.
- Python SDK: Use `RETURNING` clause instead of `lastInsertRowid`.
- Rust SDK: Use `RETURNING` clause instead of `lastInsertRowid`.

### Fixed

- CLI `cat` command bug.
- macFUSE: pass full path to open dynamic libs.

### Documentation

- Add FAQ entry for Docker Sandbox.

## [0.2.3] - 2025-12-10

### Added

- macFUSE support

## [0.2.2] - 2025-12-08

### Added

- Linux/arm64 support.

### Documentation

- Improved FUSE module documentation.

## [0.2.1] - 2025-12-04

### Fixed

- Fix `_Unwind_RaiseException` symbol lookup error on Fedora by linking to `libgcc_s.so` dynamically.
- Eliminate dependency to libfuse by using the `fuser` crate pure Rust FUSE implementation.

## [0.2.0] - 2025-12-04

### Added

- AgentFS FUSE module for mounting agent filesystems.
- TypeScript SDK: Support for custom agent filesystem path.

### Changed

- Switch to fixed-size chunks in AgentFS specification.
- TypeScript SDK: Switch to fixed-size inode chunks.
- Rust SDK: Switch to fixed-size inode chunks.
- Switch AgentFS SDK to use identifier-based API.

## [0.1.2] - 2025-11-14

### Added

- Enable Darwin/x86-64 builds for the CLI.

## [0.1.1] - 2025-11-14

### Added

- Example using OpenAI Agents SDK and AgentFS.
- Example using Claude Agent SDK and AgentFS.

### Fixed

- CLI `ls` command now recursively lists all files.

## [0.1.0] - 2025-11-13

### Added

- Initial release of AgentFS CLI.
- TypeScript SDK with async factory method (`AgentFS.open()`).
- Sandbox command for running agents in isolated environments.
- Passthrough VFS for transparent filesystem access.
- Symlink syscall support in sandbox.
- Cross-platform builds (Linux, macOS).
- Example agent implementations.

[0.3.0]: https://github.com/tursodatabase/agentfs/compare/v0.2.3...v0.3.0
[0.2.3]: https://github.com/tursodatabase/agentfs/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/tursodatabase/agentfs/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/tursodatabase/agentfs/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/tursodatabase/agentfs/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/tursodatabase/agentfs/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/tursodatabase/agentfs/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/tursodatabase/agentfs/releases/tag/v0.1.0
