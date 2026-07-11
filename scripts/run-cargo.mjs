import { spawn } from 'node:child_process';
import { cargoEnvironment, workspaceRoot } from './cargo-env.mjs';

const args = process.argv.slice(2);
if (args.length === 0) {
  process.stderr.write('Usage: node scripts/run-cargo.mjs <cargo arguments...>\n');
  process.exit(2);
}

const cargo = process.platform === 'win32' ? 'cargo.exe' : 'cargo';
const child = spawn(cargo, args, {
  cwd: workspaceRoot,
  env: cargoEnvironment(),
  stdio: 'inherit',
  shell: false,
});

const signalHandlers = new Map();
for (const signal of ['SIGINT', 'SIGTERM']) {
  const handler = () => {
    if (!child.killed) child.kill(signal);
  };
  signalHandlers.set(signal, handler);
  process.once(signal, handler);
}

function removeSignalHandlers() {
  for (const [signal, handler] of signalHandlers) {
    process.removeListener(signal, handler);
  }
}

child.on('error', (error) => {
  process.stderr.write(`Unable to start Cargo: ${error.message}\n`);
  process.exit(1);
});

child.on('exit', (code, signal) => {
  removeSignalHandlers();
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exitCode = code ?? 1;
});
