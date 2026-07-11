import { createHash } from 'node:crypto';
import { mkdirSync, realpathSync } from 'node:fs';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';

const workspaceRoot = resolve(import.meta.dirname, '..');

function cacheDirectory(environment) {
  if (process.platform === 'darwin') {
    return join(homedir(), 'Library', 'Caches', 'Sanser');
  }
  if (process.platform === 'win32') {
    return join(
      environment.LOCALAPPDATA || join(homedir(), 'AppData', 'Local'),
      'Sanser',
    );
  }
  return join(environment.XDG_CACHE_HOME || join(homedir(), '.cache'), 'sanser');
}

function defaultTargetDirectory(environment) {
  const canonicalRoot = realpathSync.native(workspaceRoot);
  const workspaceId = createHash('sha256').update(canonicalRoot).digest('hex').slice(0, 12);
  return join(cacheDirectory(environment), 'cargo-target', workspaceId);
}

export function cargoEnvironment(environment = process.env) {
  const targetDirectory = environment.CARGO_TARGET_DIR || defaultTargetDirectory(environment);
  mkdirSync(targetDirectory, { recursive: true });

  return {
    ...environment,
    CARGO_TARGET_DIR: targetDirectory,
  };
}

export { workspaceRoot };
