import { spawn } from 'node:child_process';
import { cargoEnvironment, workspaceRoot } from './cargo-env.mjs';

const [requestedCommand, ...args] = process.argv.slice(2);
if (!requestedCommand) {
  process.stderr.write('Usage: node scripts/run-with-cargo-env.mjs <command> [arguments...]\n');
  process.exit(2);
}

const windowsCommands = new Map([
  ['npm', 'npm.cmd'],
  ['npx', 'npx.cmd'],
  ['pnpm', 'pnpm.cmd'],
  ['yarn', 'yarn.cmd'],
]);
const command = process.platform === 'win32'
  ? (windowsCommands.get(requestedCommand) ?? requestedCommand)
  : requestedCommand;

const child = spawn(command, args, {
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
  process.stderr.write(`Unable to start ${requestedCommand}: ${error.message}\n`);
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
