import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const desktopRoot = path.resolve(__dirname, '..');
const unpackedExePath = path.join(
  desktopRoot,
  'dist',
  'win-unpacked',
  'appfs-desktop-shell.exe',
);

if (process.platform === 'win32') {
  await stopPreviousWindowsPackagedPreview();
}

async function stopPreviousWindowsPackagedPreview() {
  if (!fs.existsSync(unpackedExePath)) {
    return;
  }

  const targetPath = path.resolve(unpackedExePath);
  const pids = findProcessesByExecutablePath(targetPath);
  if (pids.length === 0) {
    return;
  }

  console.log(
    `[PrepackClean] Stopping previous packaged preview using ${targetPath}`,
  );

  for (const pid of new Set(pids)) {
    console.log(`[PrepackClean] taskkill /PID ${pid} /T /F`);
    try {
      execFileSync('taskkill.exe', ['/PID', String(pid), '/T', '/F'], {
        stdio: 'inherit',
      });
    } catch {
      // A parent taskkill may have already terminated this child.
    }
  }

  await waitForProcessesToExit(targetPath, 5000);
}

function findProcessesByExecutablePath(executablePath) {
  const target = path.resolve(executablePath).toLowerCase();
  const script = [
    'Get-CimInstance Win32_Process -Filter "Name = \'appfs-desktop-shell.exe\'" |',
    '  Select-Object ProcessId,ExecutablePath,CommandLine |',
    '  ConvertTo-Json -Compress',
  ].join('\n');

  const output = runPowerShell(script).trim();
  if (!output) {
    return [];
  }

  let processes;
  try {
    processes = JSON.parse(output);
  } catch {
    return [];
  }

  const list = Array.isArray(processes) ? processes : [processes];
  return list
    .filter(processInfo => {
      const executable = normalizeProcessPath(processInfo.ExecutablePath);
      const commandLine = String(processInfo.CommandLine ?? '').toLowerCase();
      return executable === target || commandLine.includes(target);
    })
    .map(processInfo => Number(processInfo.ProcessId))
    .filter(Number.isInteger);
}

function normalizeProcessPath(value) {
  if (typeof value !== 'string' || !value.trim()) {
    return '';
  }
  return path.resolve(value).toLowerCase();
}

async function waitForProcessesToExit(executablePath, timeoutMs) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    if (findProcessesByExecutablePath(executablePath).length === 0) {
      return;
    }
    await new Promise(resolve => setTimeout(resolve, 200));
  }

  throw new Error(
    `Timed out waiting for previous packaged preview to exit: ${executablePath}`,
  );
}

function runPowerShell(script) {
  const encoded = Buffer.from(script, 'utf16le').toString('base64');
  return execFileSync(
    'powershell.exe',
    ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-EncodedCommand', encoded],
    {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    },
  );
}
