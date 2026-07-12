import { existsSync, mkdirSync, readdirSync, rmSync, copyFileSync, chmodSync, realpathSync, readFileSync } from 'node:fs';
import { arch, env, platform } from 'node:process';
import { resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { homedir } from 'node:os';
import { createHash } from 'node:crypto';

const root = resolve(import.meta.dirname, '..');
const binaries = resolve(root, 'apps/desktop/src-tauri/binaries');
const productVersion = JSON.parse(readFileSync(resolve(root, 'package.json'), 'utf8')).version;
if (typeof productVersion !== 'string' || !/^\d+\.\d+\.\d+$/.test(productVersion)) {
  throw new Error('package.json contains an invalid Sanser version');
}

function nativeCacheRoot() {
  if (env.SANSER_NATIVE_BUILD_DIR) return resolve(env.SANSER_NATIVE_BUILD_DIR);
  const workspaceId = createHash('sha256')
    .update(realpathSync.native(root))
    .digest('hex')
    .slice(0, 12);
  if (platform === 'darwin') {
    return resolve(homedir(), 'Library', 'Caches', 'Sanser', 'native-build', workspaceId);
  }
  if (platform === 'win32') {
    return resolve(
      env.LOCALAPPDATA || resolve(homedir(), 'AppData', 'Local'),
      'Sanser',
      'native-build',
      workspaceId
    );
  }
  return resolve(
    env.XDG_CACHE_HOME || resolve(homedir(), '.cache'),
    'sanser',
    'native-build',
    workspaceId
  );
}

const buildRoot = nativeCacheRoot();

function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, stdio: 'inherit', shell: false });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} exited with status ${result.status ?? 'unknown'}`);
  }
}

function cleanStaged(prefix) {
  mkdirSync(binaries, { recursive: true });
  for (const entry of readdirSync(binaries)) {
    if (entry.startsWith(prefix)) rmSync(resolve(binaries, entry), { force: true });
  }
}

function stageMacOS() {
  const hostTarget = arch === 'arm64' ? 'aarch64-apple-darwin' : 'x86_64-apple-darwin';
  const target = env.SANSER_TARGET_TRIPLE || hostTarget;
  if (!['aarch64-apple-darwin', 'x86_64-apple-darwin'].includes(target)) {
    throw new Error(`Unsupported macOS target: ${target}`);
  }
  const cmakeArch = target === 'aarch64-apple-darwin' ? 'arm64' : 'x86_64';
  const build = resolve(buildRoot, `client-macos-${cmakeArch}`);
  run('cmake', [
    '-S', 'native/client-macos',
    '-B', build,
    '-DCMAKE_BUILD_TYPE=Release',
    '-DBUILD_TESTING=OFF',
    `-DSANSER_APP_VERSION=${productVersion}`,
    `-DCMAKE_OSX_ARCHITECTURES=${cmakeArch}`,
  ]);
  run('cmake', ['--build', build, '--config', 'Release', '--parallel']);
  const source = resolve(build, 'sanser-client-macos');
  if (!existsSync(source)) throw new Error('macOS native client was not produced');
  cleanStaged('sanser-client-macos-');
  const destination = resolve(binaries, `sanser-client-macos-${target}`);
  copyFileSync(source, destination);
  chmodSync(destination, 0o755);
}

function stageWindows() {
  if (arch !== 'x64') throw new Error(`Unsupported Windows architecture: ${arch}`);
  const target = 'x86_64-pc-windows-msvc';
  const build = resolve(buildRoot, 'host-windows-x64');
  run('cmake', [
    '-S', 'native/host-windows',
    '-B', build,
    '-A', 'x64',
    '-DBUILD_TESTING=OFF',
    `-DSANSER_APP_VERSION=${productVersion}`,
  ]);
  run('cmake', ['--build', build, '--config', 'Release', '--parallel']);
  const source = resolve(build, 'Release/sanser-host-windows.exe');
  if (!existsSync(source)) throw new Error('Windows native host was not produced');
  cleanStaged('sanser-host-windows-');
  copyFileSync(source, resolve(binaries, `sanser-host-windows-${target}.exe`));
}

if (platform === 'darwin') stageMacOS();
else if (platform === 'win32') stageWindows();
else process.stdout.write(`No native desktop sidecar is required on ${platform}.\n`);
