import { existsSync, readFileSync, readdirSync, rmSync, statSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const skippedDirectories = new Set([
  '.git',
  'node_modules',
  'target',
  'build',
  'dist',
  // Never treat application/user data as disposable source conflicts.
  'data',
  'captures',
  'recordings'
]);
const generatedDirectories = [join('apps', 'desktop', 'src-tauri', 'gen')];
const dryRun = process.argv.includes('--dry-run');
let removedFiles = 0;
let removedBytes = 0;
let removedDirectories = 0;
const keptDivergent = [];

const conflictSuffix = / [2-9]\d*(?=(\.[^/]+)?$)/;

function isInside(path, directories) {
  const localPath = relative(root, path);
  return directories.some(
    (directory) => localPath === directory || localPath.startsWith(`${directory}${sep}`)
  );
}

function filesAreIdentical(left, right) {
  const leftStat = statSync(left);
  const rightStat = statSync(right);
  return leftStat.size === rightStat.size && readFileSync(left).equals(readFileSync(right));
}

function clean(directory) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (entry.isDirectory() && skippedDirectories.has(entry.name)) continue;
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      clean(path);
      if (conflictSuffix.test(entry.name)) {
        const canonical = join(directory, entry.name.replace(conflictSuffix, ''));
        if (existsSync(canonical) && statSync(canonical).isDirectory() && readdirSync(path).length === 0) {
          if (!dryRun) rmSync(path, { recursive: false });
          removedDirectories += 1;
        }
      }
      continue;
    }
    if (!entry.isFile() || !conflictSuffix.test(entry.name)) continue;
    const canonical = join(directory, entry.name.replace(conflictSuffix, ''));
    if (!existsSync(canonical) || !statSync(canonical).isFile()) continue;

    // Divergent hand-written source may contain unsaved work. Only generated
    // files or byte-identical copies are safe to remove automatically.
    if (!isInside(path, generatedDirectories) && !filesAreIdentical(path, canonical)) {
      keptDivergent.push(relative(root, path));
      continue;
    }

    removedBytes += statSync(path).size;
    if (!dryRun) rmSync(path, { force: true });
    removedFiles += 1;
  }
}

clean(root);
process.stdout.write(
  `${dryRun ? 'Would remove' : 'Removed'} ${removedFiles} safe conflict files (${removedBytes} bytes) and ${removedDirectories} empty conflict directories.\n`
);
if (keptDivergent.length > 0) {
  process.stdout.write(
    `Kept ${keptDivergent.length} divergent source conflict files for review:\n${keptDivergent
      .map((path) => `- ${path}`)
      .join('\n')}\n`
  );
}
