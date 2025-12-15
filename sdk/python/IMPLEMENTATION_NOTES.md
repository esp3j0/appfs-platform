# AgentFS Python SDK - Implementation Notes

## Overview

This Python SDK is a complete port of the TypeScript AgentFS SDK, providing the same API and functionality using `pyturso.aio` for async SQLite operations.

## Architecture

### Core Components

1. **AgentFS** (`agentfs/agentfs.py`)
   - Main entry point
   - Manages database connection
   - Provides access to kv, fs, and tools subsystems
   - Supports context manager protocol

2. **KvStore** (`agentfs/kvstore.py`)
   - Simple key-value storage
   - JSON serialization/deserialization
   - Prefix-based listing
   - Automatic timestamps

3. **Filesystem** (`agentfs/filesystem.py`)
   - POSIX-like filesystem operations
   - Inode-based storage
   - Automatic parent directory creation
   - Chunked file storage (4KB chunks)
   - Support for files, directories, and symlinks

4. **ToolCalls** (`agentfs/toolcalls.py`)
   - Tool call tracking and analytics
   - Status tracking (pending, success, error)
   - Performance statistics
   - Duration calculation

### Database Schema

The Python SDK uses the same database schema as the TypeScript SDK:

- `fs_config` - Configuration (chunk_size)
- `fs_inode` - Inode table (files, directories, symlinks)
- `fs_dentry` - Directory entries (names → inodes)
- `fs_data` - File data chunks
- `fs_symlink` - Symlink targets
- `kv_store` - Key-value pairs
- `tool_calls` - Tool call records

## Key Design Decisions

### 1. pyturso.aio Integration

The SDK uses `pyturso.aio` which provides an aiosqlite-like API:

```python
# Connection
db = connect(database_path)
await db

# Execute queries
cursor = await db.execute(sql, parameters)
row = await cursor.fetchone()
rows = await cursor.fetchall()

# Transactions
await db.commit()
await db.rollback()
```

### 2. API Naming Conventions

Python follows PEP 8 naming conventions:
- `writeFile` → `write_file`
- `readFile` → `read_file`
- `deleteFile` → `delete_file`
- `getChunkSize` → `get_chunk_size`

### 3. Type Hints

Full type hints throughout the codebase for better IDE support and type checking:

```python
async def read_file(self, path: str, encoding: Optional[str] = 'utf-8') -> Union[bytes, str]:
    ...
```

### 4. Dataclasses

Using Python dataclasses for structured data:
- `AgentFSOptions`
- `Stats`
- `ToolCall`
- `ToolCallStats`

### 5. Error Handling

Uses Python's native exceptions:
- `FileNotFoundError` for missing files/directories
- `ValueError` for invalid parameters

## Differences from TypeScript SDK

### 1. Encoding Parameter

Python's `read_file` accepts an `encoding` parameter:

```python
# Read as string (default)
content = await fs.read_file('/file.txt')

# Read as bytes
data = await fs.read_file('/image.png', encoding=None)
```

TypeScript version uses options object:

```typescript
const content = await fs.readFile('/file.txt');
const data = await fs.readFile('/image.png', { encoding: undefined });
```

### 2. Context Manager Support

Python SDK supports async context managers:

```python
async with await AgentFS.open(AgentFSOptions(id='agent')) as agentfs:
    await agentfs.kv.set('key', 'value')
    # Automatically closed
```

### 3. Options as Dataclass

Python uses `AgentFSOptions` dataclass instead of plain dict:

```python
# Python
agent = await AgentFS.open(AgentFSOptions(id='agent'))

# TypeScript
const agent = await AgentFS.open({ id: 'agent' });
```

### 4. lastInsertRowid

Python uses `cursor.lastrowid` instead of `result.lastInsertRowid`:

```python
cursor = await self._db.execute(sql, params)
row_id = cursor.lastrowid
```

## Testing

The SDK includes basic tests in `tests/test_basic.py`:

- Opening with ID and path
- Key-value operations
- Filesystem operations
- Tool call tracking
- Context manager support

Run tests with:

```bash
pytest
```

## Examples

Three complete examples demonstrate usage:

1. **filesystem_demo.py** - File and directory operations
2. **kvstore_demo.py** - Key-value storage patterns
3. **toolcalls_demo.py** - Tool call tracking and analytics

## Installation

From PyPI (when published):

```bash
pip install agentfs
```

From source:

```bash
cd sdk/python
pip install -e .
```

With development dependencies:

```bash
pip install -e ".[dev]"
```

## Publishing

Build and publish to PyPI:

```bash
# Build
python -m build

# Upload to PyPI
python -m twine upload dist/*
```

## Future Enhancements

Potential improvements:

1. Add async iterators for large file reads
2. Implement symlink operations
3. Add batch operations for better performance
4. Support for file permissions and ownership
5. Add compression for large files
6. Implement file locking
7. Add migration utilities

## Compatibility

- Python 3.8+
- Requires `pyturso.aio` package
- Compatible with asyncio event loop
- Works on Linux, macOS, and Windows

## Performance Considerations

1. **Chunked Storage**: Files are stored in 4KB chunks by default
2. **Transactions**: All write operations are committed immediately
3. **Indexing**: Proper indexes on parent_ino, name, and timestamps
4. **Connection Pooling**: Single connection per AgentFS instance

## Maintenance

The Python SDK should be kept in sync with the TypeScript SDK:

1. Monitor TypeScript SDK changes
2. Port new features and bug fixes
3. Maintain API compatibility
4. Update tests and examples
5. Keep documentation current

## Support

For issues and questions:
- GitHub Issues: https://github.com/tursodatabase/agentfs/issues
- Documentation: https://github.com/tursodatabase/agentfs/tree/main/sdk/python
