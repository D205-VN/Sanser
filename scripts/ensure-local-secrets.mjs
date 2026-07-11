import { randomBytes } from 'node:crypto';
import { chmodSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const envPath = resolve(import.meta.dirname, '..', '.env');
const temporaryPath = `${envPath}.tmp`;
let contents = readFileSync(envPath, 'utf8');
const generated = [];

function ensure(name, createValue) {
  const value = createValue();
  const expression = new RegExp(`^${name}=(.*)$`, 'm');
  const match = contents.match(expression);
  if (match && match[1].trim().length > 0) return;
  if (match) contents = contents.replace(expression, `${name}=${value}`);
  else contents = `${contents.trimEnd()}\n${name}=${value}\n`;
  generated.push(name);
}

ensure('SESSION_CREDENTIAL_KEY', () => randomBytes(32).toString('base64'));
ensure('VITE_SANSER_SERVER_URL', () => 'http://127.0.0.1:5174');

writeFileSync(temporaryPath, contents, { encoding: 'utf8', mode: 0o600 });
chmodSync(temporaryPath, 0o600);
renameSync(temporaryPath, envPath);
chmodSync(envPath, 0o600);
process.stdout.write(generated.length > 0 ? `Initialized: ${generated.join(', ')}\n` : 'Local secrets already initialized.\n');
