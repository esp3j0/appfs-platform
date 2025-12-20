# Testing AgentFS

## FUSE

On macOS, install prerequisites first:

```bash
./scripts/install-deps.sh
brew install pkg-config libgit2 cmake
```

First, build the `agentfs` executable and install it locally including the `mount.fuse.agentfs` helper:

```bash
cd cli
cargo build --release
cp target/release/agentfs /usr/local/bin
cp scripts/mount.fuse.agentfs /sbin
```

Then, clone the xfstests repo:

```bash
git clone git://git.kernel.org/pub/scm/fs/xfs/xfstests-dev.git
```

Configure the filesystem under test:

```bash
cat local.config
export FSTYP=fuse
export FUSE_SUBTYP=.agentfs
export TEST_DEV=<database file>
export TEST_DIR=<mount directory>
```

Then, run xfstests:

```bash
sudo ./check -g quick generic/
```
