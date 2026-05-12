const fs = require('fs');
const path = require('path');
const { spawnSync } = require('child_process');

function unique(values) {
  return [...new Set(values.filter(Boolean))];
}

function pathCandidates() {
  const names = process.platform === 'win32' ? ['cmake.exe', 'cmake.cmd', 'cmake.bat'] : ['cmake'];
  const entries = (process.env.PATH || '')
    .split(path.delimiter)
    .filter(Boolean);

  return entries.flatMap((entry) => names.map((name) => path.join(entry, name)));
}

function cmakeCandidates() {
  if (process.platform !== 'win32') {
    return unique([process.env.CMAKE_EXE, 'cmake', ...pathCandidates()]);
  }

  return unique([
    process.env.CMAKE_EXE,
    'cmake',
    ...pathCandidates(),
    'C:\\Program Files\\CMake\\bin\\cmake.exe',
    'C:\\Program Files (x86)\\CMake\\bin\\cmake.exe',
  ]);
}

function canTryCandidate(candidate) {
  if (candidate === 'cmake') return true;
  return fs.existsSync(candidate);
}

const args = process.argv.slice(2);
const candidates = cmakeCandidates().filter(canTryCandidate);

for (const candidate of candidates) {
  const result = spawnSync(candidate, args, {
    stdio: 'inherit',
    shell: false,
  });

  if (result.error) {
    if (result.error.code === 'ENOENT') continue;
    console.error(`Failed to run CMake at ${candidate}: ${result.error.message}`);
    process.exit(1);
  }

  process.exit(result.status ?? 0);
}

console.error('Could not find CMake.');
console.error('Install it with: winget install --id Kitware.CMake -e');
console.error('Or set CMAKE_EXE to the full cmake.exe path.');
process.exit(1);
