import { existsSync, readdirSync, rmSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const skippedDirectories = new Set(['.git', 'node_modules', 'target', 'build', 'dist']);
let removedFiles = 0;
let removedBytes = 0;
let removedDirectories = 0;

const conflictSuffix = / [2-9]\d*(?=(\.[^/]+)?$)/;

function clean(directory) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (entry.isDirectory() && skippedDirectories.has(entry.name)) continue;
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      clean(path);
      if (conflictSuffix.test(entry.name)) {
        const canonical = join(directory, entry.name.replace(conflictSuffix, ''));
        if (existsSync(canonical) && statSync(canonical).isDirectory() && readdirSync(path).length === 0) {
          rmSync(path, { recursive: false });
          removedDirectories += 1;
        }
      }
      continue;
    }
    if (!entry.isFile() || !conflictSuffix.test(entry.name)) continue;
    const canonical = join(directory, entry.name.replace(conflictSuffix, ''));
    if (!existsSync(canonical) || !statSync(canonical).isFile()) continue;
    removedBytes += statSync(path).size;
    rmSync(path, { force: true });
    removedFiles += 1;
  }
}

clean(root);
process.stdout.write(
  `Removed ${removedFiles} conflict files (${removedBytes} bytes) and ${removedDirectories} empty conflict directories.\n`
);
