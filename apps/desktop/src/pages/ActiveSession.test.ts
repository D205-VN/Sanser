import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { get } from 'svelte/store';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { ConnectionSession, RuntimeStatus } from '../lib/types';
import { connection } from '../stores/connection';

const mocks = vi.hoisted(() => ({
  ready: vi.fn(), credentials: vi.fn(), disconnect: vi.fn(), getSession: vi.fn(),
  punch: vi.fn(), launch: vi.fn(), stop: vi.fn(), relay: vi.fn(), stopRelay: vi.fn(), status: vi.fn()
}));
vi.mock('../stores/session', async () => {
  const { writable } = await import('svelte/store');
  return { session: { ...writable({ mode: 'cloud' }), client: () => ({ markNativeReady: mocks.ready, sessionCredentials: mocks.credentials, disconnectSession: mocks.disconnect, getSession: mocks.getSession }) } };
});
vi.mock('../stores/presence', async () => {
  const { writable } = await import('svelte/store');
  return { presence: writable({ deviceId: 'local-client', online: true }) };
});
vi.mock('../lib/platform', () => ({ launchEngine: mocks.launch, stopEngine: mocks.stop, startRelay: mocks.relay, stopRelay: mocks.stopRelay, engineStatus: mocks.status }));
vi.mock('../lib/p2pSignaling', async (importOriginal) => ({ ...await importOriginal<typeof import('../lib/p2pSignaling')>(), coordinateP2pConnection: mocks.punch }));
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
  mocks.status.mockResolvedValue({ running: true, lastError: null });
  mocks.punch.mockResolvedValue({ localPort: 5000, remotePort: 6000, remoteAddress: '192.0.2.1' });
  mocks.launch.mockResolvedValue(undefined);
});
afterEach(() => { cleanup(); connection.setPreparing(false); connection.clear(); vi.useRealTimers(); });

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

it('retries a blank startup once, retaining Direct mode and the limit across navigation', async () => {
  vi.useFakeTimers();
  const direct = { ...target, networkMode: 'direct' as const };
  connection.begin(direct);
  mocks.getSession.mockResolvedValue(direct);
  mocks.status.mockResolvedValue({ running: false, lastError: 'No UDP packets received', startupFailure: 'no-udp' });
  render(ActiveSession, { runtime, navigate: vi.fn() });
  await fireEvent.click(screen.getByRole('button', { name: 'Open remote desktop' }));
  await vi.advanceTimersByTimeAsync(0);
  expect(mocks.launch).toHaveBeenCalledTimes(1);
  await vi.advanceTimersByTimeAsync(2_000);
  expect(screen.getByRole('alert')).toHaveTextContent('Retrying once');
  expect(mocks.stopRelay).toHaveBeenCalledWith('client');
  await vi.advanceTimersByTimeAsync(2_000);
  expect(mocks.ready).toHaveBeenCalledTimes(2);
  expect(mocks.credentials).toHaveBeenCalledTimes(2);
  expect(mocks.launch).toHaveBeenCalledTimes(2);
  expect(mocks.launch).toHaveBeenLastCalledWith(expect.objectContaining({ networkMode: 'direct', relay: false }));
  expect(mocks.relay).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(10_000);
  expect(mocks.launch).toHaveBeenCalledTimes(2);
  expect(get(connection).nativeRetryBlocked).toBe(true);
  cleanup();
  render(ActiveSession, { runtime, navigate: vi.fn() });
  await vi.advanceTimersByTimeAsync(6_000);
  expect(mocks.launch).toHaveBeenCalledTimes(2);
  expect(screen.getByRole('alert')).toHaveTextContent('No UDP packets received');
  await fireEvent.click(screen.getByRole('button', { name: 'Retry connection' }));
  await vi.advanceTimersByTimeAsync(0);
  expect(mocks.launch).toHaveBeenCalledTimes(3);
});

it.each(['auth-rejected', 'decode-failed', null])('does not auto-reopen the viewer after %s or a normal close', async (startupFailure) => {
  vi.useFakeTimers();
  connection.setEngineRunning(true);
  mocks.status.mockResolvedValue({ running: false, lastError: 'Viewer stopped', startupFailure });
  render(ActiveSession, { runtime, navigate: vi.fn() });
  await vi.advanceTimersByTimeAsync(10_000);
  expect(mocks.launch).not.toHaveBeenCalled();
  expect(screen.getByRole('alert')).toHaveTextContent('Viewer stopped');
  cleanup();
  render(ActiveSession, { runtime, navigate: vi.fn() });
  await vi.advanceTimersByTimeAsync(4_000);
  expect(mocks.launch).not.toHaveBeenCalled();
  expect(screen.getByRole('alert')).toHaveTextContent('Viewer stopped');
});

it('cancels a scheduled startup retry when disconnected', async () => {
  vi.useFakeTimers();
  connection.setEngineRunning(true);
  mocks.status.mockResolvedValue({ running: false, lastError: 'Video incomplete', startupFailure: 'incomplete-video' });
  render(ActiveSession, { runtime, navigate: vi.fn() });
  await vi.advanceTimersByTimeAsync(2_000);
  expect(get(connection).nativeStartupRetries).toBe(1);
  await fireEvent.click(screen.getByRole('button', { name: 'Disconnect' }));
  await vi.advanceTimersByTimeAsync(10_000);
  expect(get(connection).session).toBeNull();
  expect(mocks.ready).not.toHaveBeenCalled();
  expect(mocks.launch).not.toHaveBeenCalled();
});

it('does not retry after the host closes the accepted session', async () => {
  vi.useFakeTimers();
  connection.setEngineRunning(true);
  mocks.status.mockResolvedValue({ running: false, lastError: 'No video', startupFailure: 'no-video' });
  render(ActiveSession, { runtime, navigate: vi.fn() });
  await vi.advanceTimersByTimeAsync(2_000);
  mocks.getSession.mockResolvedValue({ ...target, status: 'disconnected' });
  await vi.advanceTimersByTimeAsync(10_000);
  expect(mocks.launch).not.toHaveBeenCalled();
  expect(mocks.ready).not.toHaveBeenCalled();
});

it('does not restart another retry cycle when recovery authorization fails', async () => {
  vi.useFakeTimers();
  connection.setEngineRunning(true);
  mocks.status.mockResolvedValue({ running: false, lastError: 'No UDP', startupFailure: 'no-udp' });
  mocks.ready.mockRejectedValue(new Error('Host unavailable'));
  render(ActiveSession, { runtime, navigate: vi.fn() });
  await vi.advanceTimersByTimeAsync(20_000);
  expect(mocks.ready).toHaveBeenCalledTimes(1);
  expect(mocks.launch).not.toHaveBeenCalled();
  expect(get(connection).nativeRetryBlocked).toBe(true);
  expect(screen.getByRole('alert')).toHaveTextContent('Host unavailable');
});

it('does not stop a new session relay or change its state when old cleanup finishes late', async () => {
  vi.useFakeTimers();
  connection.setEngineRunning(true);
  mocks.status.mockResolvedValue({ running: false, lastError: 'No UDP', startupFailure: 'no-udp' });
  let finishStop: (() => void) | undefined;
  mocks.stop.mockImplementationOnce(() => new Promise<void>((resolve) => { finishStop = resolve; }));
  render(ActiveSession, { runtime, navigate: vi.fn() });
  await vi.advanceTimersByTimeAsync(2_000);
  expect(mocks.stop).toHaveBeenCalledTimes(1);
  connection.begin({ ...target, id: 'new-session' });
  connection.setEngineRunning(true);
  finishStop?.();
  await vi.advanceTimersByTimeAsync(0);
  expect(mocks.stopRelay).not.toHaveBeenCalled();
  expect(get(connection)).toMatchObject({ engineRunning: true, nativeStartupRetries: 0, error: null });
});

it('releases the connection controls after navigation cancels negotiation', async () => {
  vi.useFakeTimers();
  mocks.punch.mockImplementationOnce((...args: unknown[]) => {
    const signal = args[7] as AbortSignal;
    return new Promise((_resolve, reject) => signal.addEventListener('abort',
      () => reject(new DOMException('Cancelled', 'AbortError')), { once: true }));
  });
  render(ActiveSession, { runtime, navigate: vi.fn() });
  await fireEvent.click(screen.getByRole('button', { name: 'Open remote desktop' }));
  await vi.advanceTimersByTimeAsync(0);
  expect(get(connection).busy).toBe(true);
  cleanup();
  await vi.advanceTimersByTimeAsync(0);
  expect(get(connection)).toMatchObject({ busy: false, preparing: false, engineRunning: false });
  render(ActiveSession, { runtime, navigate: vi.fn() });
  expect(screen.getByRole('button', { name: 'Open remote desktop' })).toBeEnabled();
});
