import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync, renameSync, rmSync, openSync, closeSync, copyFileSync, chmodSync, writeSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { cargoEnvironment, workspaceRoot } from './cargo-env.mjs';

// No credentials are copied to a plist, command line or status file. The server
// reads the repository's private .env, just as server:dev does.
const directory = join(workspaceRoot, 'data', 'self-host');
const statusFile = join(directory, 'status.json');
const label = 'com.sanser.self-host';
const domain = `gui/${process.getuid?.()}`;
const plist = join(homedir(), 'Library', 'LaunchAgents', `${label}.plist`);
const binary = join(directory, 'sanser-server');
const installationFile = join(directory, 'installation.json');
const port = 5174;
const origin = `http://127.0.0.1:${port}`;
const action = process.argv[2] ?? 'status';
const version = JSON.parse(readFileSync(join(workspaceRoot, 'package.json'), 'utf8')).version;
mkdirSync(directory, { recursive: true, mode: 0o700 });

function execute(command, args, options = {}) {
  const result = spawnSync(command, args, { stdio: 'inherit', ...options });
  if (result.error || result.status !== 0) throw new Error(`${command} failed (${result.status ?? result.error?.code})`);
}
function xml(value) {
  return value.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;');
}
async function healthy(base, route = 'health') {
  try {
    const response = await fetch(`${base}/api/v2/${route}`, { signal: AbortSignal.timeout(4000), redirect: 'error' });
    const body = await response.json();
    return response.ok && body.protocolVersion === 2 && ['ok', 'ready'].includes(body.status);
  } catch { return false; }
}
function writeStatus(value) {
  writeFileSync(`${statusFile}.tmp`, JSON.stringify(value, null, 2) + '\n', { mode: 0o600 });
  renameSync(`${statusFile}.tmp`, statusFile);
}
function rotateLog(name) {
  const file = join(directory, `${name}.log`);
  if (existsSync(file)) renameSync(file, `${file}.previous`);
  return file;
}

async function run() {
  if (await healthy(origin)) throw new Error(`A server is already using ${origin}; leave it running and stop this duplicate.`);
  if (!existsSync(binary)) throw new Error('Run npm run server:selfhost -- install first.');
  const installedVersion = JSON.parse(readFileSync(installationFile, 'utf8')).version;
  if (!/^\d+\.\d+\.\d+$/.test(installedVersion)) throw new Error('Invalid installed server version; reinstall the service.');
  const children = [];
  let stopped = false;
  let publicUrl = null;
  function stop(code) {
    if (stopped) return;
    stopped = true;
    rmSync(statusFile, { force: true });
    for (const child of children) child.kill('SIGTERM');
    setTimeout(() => {
      for (const child of children) if (child.exitCode === null) child.kill('SIGKILL');
      process.exit(code);
    }, 2000);
  }
  for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => stop(0));
  function launch(name, command, args, env = process.env, capture = false) {
    const fd = openSync(rotateLog(name), 'w', 0o600);
    const child = spawn(command, args, { cwd: workspaceRoot, env, stdio: ['ignore', fd, capture ? 'pipe' : fd] });
    if (capture) {
      let written = 0;
      child.stderr.on('data', (chunk) => {
        if (written < 5 * 1024 * 1024) { writeSync(fd, chunk); written += chunk.length; }
      });
    }
    children.push(child);
    child.on('error', () => { console.error(`${name} could not start; check its installation.`); stop(1); });
    child.once('close', () => closeSync(fd));
    child.on('exit', () => { if (!stopped) { console.error(`${name} stopped; see data/self-host/${name}.log`); stop(1); } });
    return child;
  }
  rmSync(statusFile, { force: true });
  launch('server', binary, [], {
    ...process.env, SERVER_HOST: '127.0.0.1', SERVER_PORT: String(port),
    SANSER_VERSION: installedVersion, SANSER_PROTOCOL_VERSION: '2', NETWORK_MODE: 'auto',
    RUST_LOG: 'sanser_server=info,tower_http=warn',
  });
  let ready = false;
  for (let attempt = 0; attempt < 30 && !stopped; attempt++) {
    if (await healthy(origin, 'readiness')) { ready = true; break; }
    await delay(1000);
  }
  if (!ready || stopped) { stop(1); return; }
  writeStatus({ localUrl: origin, publicUrl, startedAt: new Date().toISOString(), version: installedVersion });
  console.log(`Sanser server ready at ${origin}`);
  // Quick Tunnel is temporary. A named tunnel can later point at the same origin.
  const tunnel = launch('tunnel', 'cloudflared', ['tunnel', '--no-autoupdate', '--url', origin], process.env, true);
  let tail = '';
  tunnel.stderr.on('data', (chunk) => {
    if (stopped) return;
    // Keep only a small rolling buffer; cloudflared stderr can grow indefinitely.
    tail = (tail + chunk.toString()).slice(-8192);
    const found = tail.match(/https:\/\/[a-z0-9-]+\.trycloudflare\.com\b/);
    if (found && found[0] !== 'https://api.trycloudflare.com' && found[0] !== publicUrl) {
      publicUrl = found[0];
      writeStatus({ localUrl: origin, publicUrl, startedAt: new Date().toISOString(), version: installedVersion });
      console.log(`Temporary public server: ${publicUrl}`);
      console.log('Use this same HTTPS URL on both computers. It changes when the tunnel restarts.');
    }
  });
  // Keep the server available while this user session is running and the lid is open.
  if (process.platform === 'darwin') launch('awake', '/usr/bin/caffeinate', ['-i', '-w', String(process.pid)]);
}

try {
  if (action === 'install') {
    if (process.platform !== 'darwin') throw new Error('Automatic service installation currently supports macOS.');
    if (!existsSync(join(workspaceRoot, '.env'))) throw new Error('Configure .env with DATABASE_URL and SESSION_CREDENTIAL_KEY first.');
    execute('cloudflared', ['--version']);
    const env = cargoEnvironment({ ...process.env, CARGO_PROFILE_RELEASE_STRIP: 'none' });
    execute('cargo', ['build', '-p', 'sanser-server', '--release'], { cwd: workspaceRoot, env });
    // Stop only our own service, never an unrelated server or tunnel.
    spawnSync('launchctl', ['bootout', `${domain}/${label}`], { stdio: 'ignore' });
    copyFileSync(join(env.CARGO_TARGET_DIR, 'release', 'sanser-server'), `${binary}.next`);
    chmodSync(`${binary}.next`, 0o700);
    renameSync(`${binary}.next`, binary);
    writeFileSync(installationFile, JSON.stringify({ version }) + '\n', { mode: 0o600 });
    const args = [process.execPath, join(workspaceRoot, 'scripts', 'self-host.mjs'), 'run'];
    const path = `${join(process.execPath, '..')}:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin`;
    mkdirSync(join(homedir(), 'Library', 'LaunchAgents'), { recursive: true });
    writeFileSync(plist, `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>${label}</string>
<key>ProgramArguments</key><array>${args.map((arg) => `<string>${xml(arg)}</string>`).join('')}</array>
<key>WorkingDirectory</key><string>${xml(workspaceRoot)}</string>
<key>EnvironmentVariables</key><dict><key>PATH</key><string>${xml(path)}</string></dict>
<key>RunAtLoad</key><true/><key>KeepAlive</key><true/><key>ThrottleInterval</key><integer>30</integer>
<key>StandardOutPath</key><string>${xml(join(directory, 'service.log'))}</string>
<key>StandardErrorPath</key><string>${xml(join(directory, 'service.log'))}</string>
</dict></plist>\n`, { mode: 0o600 });
    execute('launchctl', ['bootstrap', domain, plist]);
    console.log('Installed. Run npm run server:selfhost -- status to get the public URL.');
  } else if (action === 'stop') {
    execute('launchctl', ['bootout', `${domain}/${label}`]);
    rmSync(plist, { force: true });
    rmSync(statusFile, { force: true });
    console.log('Self-host service stopped and removed from automatic startup.');
  } else if (action === 'run') {
    await run();
  } else if (action === 'status') {
    const status = existsSync(statusFile) ? JSON.parse(readFileSync(statusFile, 'utf8')) : {};
    console.log(JSON.stringify({ ...status, localReady: await healthy(origin, 'readiness'), publicReady: status.publicUrl ? await healthy(status.publicUrl, 'readiness') : false }, null, 2));
  } else {
    throw new Error('Usage: npm run server:selfhost -- install|status|stop|run');
  }
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
