import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { basename, resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
const artifactsDirectory = resolve(process.argv[2] || resolve(root, 'artifacts'));
const packageMetadata = JSON.parse(readFileSync(resolve(root, 'package.json'), 'utf8'));
const version = String(process.env.SANSER_VERSION || packageMetadata.version || '').trim();
const repository = String(process.env.RELEASE_REPOSITORY || process.env.GITHUB_REPOSITORY || '').trim();
const tag = String(process.env.RELEASE_TAG || process.env.GITHUB_REF_NAME || `v${version}`).trim();

function fail(message) {
  process.stderr.write(`updater manifest: ${message}\n`);
  process.exit(1);
}

if (!/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/u.test(version)) {
  fail('SANSER_VERSION must be a valid semantic version');
}
if (!/^[0-9A-Za-z_.-]+\/[0-9A-Za-z_.-]+$/u.test(repository)) {
  fail('RELEASE_REPOSITORY must use the owner/repository form');
}
if (!/^v?[0-9A-Za-z_.+-]+$/u.test(tag)) {
  fail('RELEASE_TAG contains unsupported characters');
}

const candidates = [
  ['windows-x86_64', `Sanser-Windows-${version}-x64-setup.exe`],
  ['darwin-aarch64', `Sanser-macOS-${version}-arm64.app.tar.gz`],
  ['darwin-x86_64', `Sanser-macOS-${version}-x64.app.tar.gz`]
];
const platforms = {};

for (const [target, filename] of candidates) {
  const bundlePath = resolve(artifactsDirectory, filename);
  const signaturePath = `${bundlePath}.sig`;
  const hasBundle = existsSync(bundlePath);
  const hasSignature = existsSync(signaturePath);
  if (hasBundle !== hasSignature) {
    fail(`${filename} and its .sig file must be published together`);
  }
  if (!hasBundle) continue;

  const signature = readFileSync(signaturePath, 'utf8').trim();
  if (!signature) fail(`${basename(signaturePath)} is empty`);
  platforms[target] = {
    signature,
    url: `https://github.com/${repository}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(filename)}`
  };
}

if (Object.keys(platforms).length === 0) {
  fail('no signed updater bundles were found');
}

mkdirSync(artifactsDirectory, { recursive: true });
const manifest = {
  version,
  notes: `Sanser ${version}`,
  pub_date: new Date().toISOString(),
  platforms
};
writeFileSync(
  resolve(artifactsDirectory, 'updater-latest.json'),
  `${JSON.stringify(manifest, null, 2)}\n`,
  { mode: 0o644 }
);
process.stdout.write(`updater manifest: generated for ${Object.keys(platforms).join(', ')}\n`);
