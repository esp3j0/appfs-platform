import { execSync } from 'node:child_process';
import path from 'node:path';
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const platformRoot = path.resolve(__dirname, '..', '..');

function runCommand(command, cwd) {
  console.log(`[PrepareAssets] Running: "${command}" in ${cwd}`);
  try {
    execSync(command, { cwd, stdio: 'inherit' });
  } catch (err) {
    console.error(`[PrepareAssets] Command failed: "${command}"`);
    process.exit(1);
  }
}

// 1. Build dashboard frontend
const dashboardDir = path.join(platformRoot, 'dashboard');
console.log('\n--- Building Dashboard Frontend ---');
runCommand('npm run build', dashboardDir);

// 2. Build dashboard server
const serverDir = path.join(platformRoot, 'dashboard', 'server');
console.log('\n--- Building Dashboard Server ---');
runCommand('npm run build', serverDir);

// 3. Clean and copy dashboard frontend dist to desktop/dashboard/dist
const destDashboardDist = path.join(platformRoot, 'desktop', 'dashboard', 'dist');
console.log(`\n[PrepareAssets] Copying frontend assets to local ${destDashboardDist}`);
if (fs.existsSync(destDashboardDist)) {
  fs.rmSync(destDashboardDist, { recursive: true, force: true });
}
fs.mkdirSync(destDashboardDist, { recursive: true });
fs.cpSync(path.join(dashboardDir, 'dist'), destDashboardDist, { recursive: true, force: true });

// 4. Clean and copy dashboard server dist to desktop/dashboard/server/dist
const destServerDist = path.join(platformRoot, 'desktop', 'dashboard', 'server', 'dist');
console.log(`[PrepareAssets] Copying backend assets to local ${destServerDist}`);
if (fs.existsSync(destServerDist)) {
  fs.rmSync(destServerDist, { recursive: true, force: true });
}
fs.mkdirSync(destServerDist, { recursive: true });
fs.cpSync(path.join(serverDir, 'dist'), destServerDist, { recursive: true, force: true });

// Also copy server package.json to local desktop/dashboard/server/package.json so dependencies resolution inside ASAR works
const destServerPkg = path.join(platformRoot, 'desktop', 'dashboard', 'server', 'package.json');
fs.copyFileSync(path.join(serverDir, 'package.json'), destServerPkg);

// 5. Create desktop/bin folder
const desktopBinDir = path.join(platformRoot, 'desktop', 'bin');
if (!fs.existsSync(desktopBinDir)) {
  fs.mkdirSync(desktopBinDir, { recursive: true });
}

// 6. Compile Rust binaries
console.log('\n--- Compiling Rust Binaries (Release Mode) ---');
try {
  runCommand('cargo build --manifest-path appfs/cli/Cargo.toml --release', platformRoot);
} catch (err) {
  console.warn('[PrepareAssets] Warning: Automatic compilation of CLI failed. We will check standard paths for precompiled binaries.');
}

try {
  runCommand('cargo build --manifest-path appfs-agent/rust/Cargo.toml --release -p rusty-claude-cli --features debug-dump', platformRoot);
} catch (err) {
  console.warn('[PrepareAssets] Warning: Automatic compilation of Agent failed. We will check standard paths for precompiled binaries.');
}

// 7. Resolve and copy Rust binaries
console.log('\n--- Harvesting Rust Binaries ---');
const isWindows = process.platform === 'win32';
const cliBinName = isWindows ? 'agentfs.exe' : 'agentfs';
const agentBinName = isWindows ? 'claw.exe' : 'claw';

// Potential target locations for CLI binary
const cliPaths = [
  path.join(platformRoot, 'appfs', 'target', 'release', cliBinName),
  path.join(platformRoot, 'appfs', 'target', 'debug', cliBinName),
  path.join(platformRoot, 'appfs', 'cli', 'target', 'release', cliBinName),
  path.join(platformRoot, 'appfs', 'cli', 'target', 'debug', cliBinName),
];

// Potential target locations for Agent binary
const agentPaths = [
  path.join(platformRoot, 'appfs-agent', 'target', 'release', agentBinName),
  path.join(platformRoot, 'appfs-agent', 'target', 'debug', agentBinName),
  path.join(platformRoot, 'appfs-agent', 'rust', 'target', 'release', agentBinName),
  path.join(platformRoot, 'appfs-agent', 'rust', 'target', 'debug', agentBinName),
];

function copyBinary(srcPaths, destName) {
  const destPath = path.join(desktopBinDir, destName);
  for (const src of srcPaths) {
    if (fs.existsSync(src)) {
      console.log(`[PrepareAssets] Found ${destName} at ${src}. Copying to ${destPath}`);
      fs.copyFileSync(src, destPath);
      return true;
    }
  }
  console.error(`[PrepareAssets] [Error] Binary "${destName}" not found in any standard target paths.`);
  return false;
}

const foundCli = copyBinary(cliPaths, cliBinName);
const foundAgent = copyBinary(agentPaths, agentBinName);

if (!foundCli || !foundAgent) {
  console.error('\n[PrepareAssets] [Error] Packaging failed: Required Rust binaries are missing from the workspace!');
  console.error('Please build them manually if the automatic cargo compilation failed:');
  console.error('  - Build CLI:   cargo build --manifest-path appfs/cli/Cargo.toml --release');
  console.error('  - Build Agent: cargo build --manifest-path appfs-agent/rust/Cargo.toml --release -p rusty-claude-cli --features debug-dump');
  process.exit(1);
}

console.log('\n[PrepareAssets] Asset preparation completed successfully!\n');
