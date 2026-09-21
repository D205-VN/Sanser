import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { get } from 'svelte/store';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { ConnectionSession, RuntimeStatus } from '../lib/types';
import { connection } from '../stores/connection';

const mocks = vi.hoisted(() => ({
  ready: vi.fn(), credentials: vi.fn(), disconnect: vi.fn(), getSession: vi.fn(),
  punch: vi.fn(), launch: vi.fn(), stop: vi.fn(), relay: vi.fn(), stopRelay: vi.fn()
}));
vi.mock('../stores/session', async () => {
  const { writable } = await import('svelte/store');
  return { session: { ...writable({ mode: 'cloud' }), client: () => ({ markNativeReady: mocks.ready, sessionCredentials: mocks.credentials, disconnectSession: mocks.disconnect, getSession: mocks.getSession }) } };
});
vi.mock('../stores/presence', async () => {
  const { writable } = await import('svelte/store');
  return { presence: writable({ deviceId: 'local-client', online: true }) };
});
vi.mock('../lib/platform', () => ({ launchEngine: mocks.launch, stopEngine: mocks.stop, startRelay: mocks.relay, stopRelay: mocks.stopRelay, engineStatus: vi.fn() }));
vi.mock('../lib/p2pSignaling', () => ({ coordinateP2pConnection: mocks.punch }));
import ActiveSession from './ActiveSession.svelte';
const runtime = { platform: 'macOS', capabilities: { clientEngine: { state: 'available' }, nativeDirect: { state: 'available' }, desktopShell: { state: 'available' } } } as RuntimeStatus;
const target = { id: 'test-session', hostDeviceId: 'remote-host', requesterDeviceId: 'local-client', status: 'accepted', transport: 'native', networkMode: 'auto', qualityProfile: 'balanced', requestedCodec: 'auto' } as ConnectionSession;

beforeEach(() => {
  vi.resetAllMocks();
  connection.setPreparing(false);
  connection.begin(target);
  mocks.ready.mockResolvedValue({ requesterReadyAt: 'now' });
  mocks.credentials.mockResolvedValue({ sessionId: target.id, peerRouteAddress: '192.0.2.1', basePort: 5000, sessionToken: 'test-only-token', expiresAt: 9999999999, wireProtocol: 'snv2' });
  mocks.disconnect.mockResolvedValue(undefined);
  mocks.stop.mockResolvedValue(undefined);
  mocks.stopRelay.mockResolvedValue(undefined);
  mocks.getSession.mockResolvedValue(target);
});
afterEach(() => { cleanup(); connection.setPreparing(false); connection.clear(); });

it('allows cancelling an in-progress route without falling back to relay or launching an engine', async () => {
  let activeSignal: AbortSignal | undefined;
  mocks.punch.mockImplementation((...args: unknown[]) => {
    activeSignal = args[7] as AbortSignal;
    return new Promise((_resolve, reject) => activeSignal?.addEventListener('abort', () => reject(new DOMException('Cancelled', 'AbortError')), { once: true }));
  });
  render(ActiveSession, { runtime, navigate: vi.fn() });
  await fireEvent.click(screen.getByRole('button', { name: 'Open remote desktop' }));
  await screen.findByRole('heading', { name: 'Connecting directly' });
  const cancel = screen.getByRole('button', { name: 'Cancel connection' });
  expect(cancel).toBeEnabled();
  await fireEvent.click(cancel);
  await waitFor(() => expect(get(connection).session).toBeNull());
  expect(activeSignal?.aborted).toBe(true);
  expect(mocks.relay).not.toHaveBeenCalled();
  expect(mocks.launch).not.toHaveBeenCalled();
  expect(mocks.disconnect).toHaveBeenCalledWith(target.id);
});

it('ignores authorization completing after cancellation', async () => {
  let finishReady: ((value: { requesterReadyAt: string }) => void) | undefined;
  mocks.ready.mockImplementation(() => new Promise((resolve) => { finishReady = resolve; }));
  render(ActiveSession, { runtime, navigate: vi.fn() });
  await fireEvent.click(screen.getByRole('button', { name: 'Open remote desktop' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Cancel connection' }));
  await waitFor(() => expect(get(connection).session).toBeNull());
  expect(get(connection).preparing).toBe(true);
  finishReady?.({ requesterReadyAt: 'late' });
  await waitFor(() => expect(get(connection).preparing).toBe(false));
  expect(mocks.credentials).not.toHaveBeenCalled();
  expect(mocks.launch).not.toHaveBeenCalled();
});

it('does not equate an open native process with verified streaming', () => {
  connection.setEngineRunning(true);
  render(ActiveSession, { runtime, navigate: vi.fn() });
  expect(screen.getByText('Window open')).toBeInTheDocument();
  expect(screen.queryByText('Streaming')).not.toBeInTheDocument();
  expect(screen.getByText(/If the screen stays blank, check connection details/)).toBeInTheDocument();
});
