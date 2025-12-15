# AgentFS Python SDK - Quick Start

## Installation

```bash
pip install agentfs
```

Or install from source:

```bash
cd sdk/python
pip install -e .
```

## Basic Usage

```python
import asyncio
from agentfs import AgentFS, AgentFSOptions

async def main():
    # Open an agent filesystem
    agent = await AgentFS.open(AgentFSOptions(id='my-agent'))

    # Key-Value Store
    await agent.kv.set('config', {'debug': True})
    config = await agent.kv.get('config')

    # Filesystem
    await agent.fs.write_file('/notes.txt', 'Hello!')
    content = await agent.fs.read_file('/notes.txt')

    # Tool Calls
    call_id = await agent.tools.start('search', {'query': 'Python'})
    await agent.tools.success(call_id, {'results': []})

    await agent.close()

asyncio.run(main())
```

## Running Examples

```bash
cd sdk/python

# Key-Value Store example
python examples/kvstore_demo.py

# Filesystem example
python examples/filesystem_demo.py

# Tool Calls tracking example
python examples/toolcalls_demo.py
```

## Development

### Install development dependencies

```bash
pip install -e ".[dev]"
```

### Run tests

```bash
pytest
```

### Format code

```bash
black agentfs tests
```

### Type checking

```bash
mypy agentfs
```

## Key Differences from TypeScript SDK

1. **Async/Await**: All methods are async and must be awaited
2. **Import style**: `from agentfs import AgentFS, AgentFSOptions`
3. **Options**: Use `AgentFSOptions` dataclass instead of plain dict
4. **Encoding**: `read_file` accepts `encoding` parameter (default: 'utf-8', set to None for bytes)
5. **Context manager**: Supports `async with` for automatic cleanup

## API Compatibility

The Python SDK provides the same API surface as the TypeScript SDK:

| TypeScript | Python |
|------------|--------|
| `AgentFS.open({ id: 'agent' })` | `await AgentFS.open(AgentFSOptions(id='agent'))` |
| `agentfs.kv.set(key, value)` | `await agentfs.kv.set(key, value)` |
| `agentfs.fs.writeFile(path, content)` | `await agentfs.fs.write_file(path, content)` |
| `agentfs.tools.record(...)` | `await agentfs.tools.record(...)` |

## More Information

See [README.md](README.md) for complete documentation.
