import { readFileSync } from 'node:fs';
import { isIP } from 'node:net';

function readDotEnv(path) {
  try {
    const values = new Map();
    for (const rawLine of readFileSync(path, 'utf8').split(/\r?\n/u)) {
      const line = rawLine.trim();
      if (!line || line.startsWith('#')) continue;
      const separator = line.indexOf('=');
      if (separator < 1) continue;
      const key = line.slice(0, separator).trim();
      let value = line.slice(separator + 1).trim();
      if ((value.startsWith('"') && value.endsWith('"')) || (value.startsWith("'") && value.endsWith("'"))) {
        value = value.slice(1, -1);
      }
      values.set(key, value);
    }
    return values;
  } catch (error) {
    if (error && typeof error === 'object' && 'code' in error && error.code === 'ENOENT') return new Map();
    throw error;
  }
}

function fail(message) {
  process.stderr.write(`release config: ${message}\n`);
  process.exit(1);
}

const envFile = readDotEnv('.env');
const configured = (process.env.VITE_SANSER_SERVER_URL ?? envFile.get('VITE_SANSER_SERVER_URL') ?? '').trim();
const allowLoopback = process.argv.includes('--allow-loopback');

function isPrivateOrReservedHost(rawHostname) {
  const hostname = rawHostname.replace(/^\[|\]$/gu, '').toLowerCase();
  if (
    hostname === 'localhost' ||
    hostname.endsWith('.localhost') ||
    hostname.endsWith('.local') ||
    hostname.endsWith('.lan') ||
    hostname.endsWith('.internal') ||
    hostname.endsWith('.example') ||
    hostname.endsWith('.invalid') ||
    hostname.endsWith('.test')
  ) {
    return true;
  }
  if (isIP(hostname) === 4) {
    const [first, second, third] = hostname.split('.').map(Number);
    return (
      first === 0 ||
      first === 10 ||
      first === 127 ||
      (first === 100 && second >= 64 && second <= 127) ||
      (first === 169 && second === 254) ||
      (first === 172 && second >= 16 && second <= 31) ||
      (first === 192 && second === 0 && (third === 0 || third === 2)) ||
      (first === 192 && second === 168) ||
      (first === 198 && (second === 18 || second === 19)) ||
      (first === 198 && second === 51 && third === 100) ||
      (first === 203 && second === 0 && third === 113) ||
      first >= 224
    );
  }
  if (isIP(hostname) === 6) {
    return (
      hostname === '::' ||
      hostname === '::1' ||
      hostname.startsWith('2001:db8:') ||
      hostname.startsWith('fc') ||
      hostname.startsWith('fd') ||
      hostname.startsWith('fe8') ||
      hostname.startsWith('fe9') ||
      hostname.startsWith('fea') ||
      hostname.startsWith('feb')
    );
  }
  return false;
}

if (!configured) {
  fail('VITE_SANSER_SERVER_URL is required for a release build');
}

let endpoint;
try {
  endpoint = new URL(configured);
} catch {
  fail('VITE_SANSER_SERVER_URL must be an absolute URL');
}

if (endpoint.username || endpoint.password || endpoint.search || endpoint.hash) {
  fail('VITE_SANSER_SERVER_URL must not contain credentials, query parameters, or fragments');
}

const loopback =
  endpoint.hostname === 'localhost' ||
  endpoint.hostname === '127.0.0.1' ||
  endpoint.hostname === '::1' ||
  endpoint.hostname === '[::1]';
if (loopback && !allowLoopback) {
  fail('a release build cannot use a loopback Sanser API URL');
}
if (isPrivateOrReservedHost(endpoint.hostname) && !(allowLoopback && loopback)) {
  fail('a release build requires a publicly reachable Sanser API hostname');
}
if (endpoint.protocol !== 'https:' && !(allowLoopback && loopback && endpoint.protocol === 'http:')) {
  fail('a release build requires a public HTTPS Sanser API URL');
}

console.log(`release config: ok (${loopback ? 'local verification' : 'public HTTPS endpoint'})`);
