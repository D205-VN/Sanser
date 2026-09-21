import { get } from 'svelte/store';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { ApiError } from '../lib/api';
import type { ConnectionSession, RuntimeStatus } from '../lib/types';

const mocks = vi.hoisted(() => ({
  register: vi.fn(), offline: vi.fn(), sessions: vi.fn(), heartbeat: vi.fn(),
  credentials: vi.fn(), disconnect: vi.fn(), accept: vi.fn(), reject: vi.fn(),
  punch: vi.fn(), launch: vi.fn(), stop: vi.fn(), relay: vi.fn(), stopRelay: vi.fn(), status: vi.fn()
}));
vi.mock('./session', async () => {
  const { writable } = await import('svelte/store');
  return { session: {
    ...writable({ mode: 'cloud', account: { id: 'test-account' } }),
    client: () => ({
      serverUrl: 'https://sanser.example', registerDevice: mocks.register,
      offlineDevice: mocks.offline, hostSessions: mocks.sessions, heartbeatDevice: mocks.heartbeat,
      sessionCredentials: mocks.credentials, disconnectSession: mocks.disconnect,
      acceptSession: mocks.accept, rejectSession: mocks.reject,
      getAccessToken: vi.fn().mockResolvedValue('test-access')
    })
  } };
});
vi.mock('../lib/platform', () => ({
  getLocalRouteAddress: vi.fn().mockResolvedValue('192.0.2.2'),
  launchEngine: mocks.launch, stopEngine: mocks.stop, startRelay: mocks.relay,
  stopRelay: mocks.stopRelay, engineStatus: mocks.status
}));
vi.mock('../lib/p2pSignaling', () => ({ coordinateP2pConnection: mocks.punch }));
import { createHostStore, type HostStore } from './host';

const runtime = { platform: 'macOS', capabilities: {
  hostEngine: { state: 'available' }, nativeDirect: { state: 'available' },
  webRtc: { state: 'unavailable' }, gamepad: { state: 'unavailable' }, crossPlatformHost: true
} } as RuntimeStatus;
const target = { id: 'test-session', hostDeviceId: 'local-host', requesterDeviceId: 'remote-client',
  status: 'accepted', transport: 'native', networkMode: 'auto', requesterReadyAt: 'now',
  qualityProfile: 'balanced', requestedCodec: 'auto'
} as ConnectionSession;

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}
async function settle() { for (let i = 0; i < 20; i += 1) await Promise.resolve(); }
let host: HostStore;
beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  host = createHostStore();
  mocks.register.mockResolvedValue({ id: 'local-host' });
  mocks.offline.mockResolvedValue(undefined);
  mocks.sessions.mockResolvedValue({ items: [] });
  mocks.heartbeat.mockResolvedValue(undefined);
  mocks.credentials.mockResolvedValue({ sessionToken: 'test-token', wireProtocol: 'snv2' });
  mocks.punch.mockResolvedValue({ localPort: 5000, remotePort: 6000, remoteAddress: '192.0.2.1' });
  mocks.launch.mockResolvedValue(undefined);
  mocks.stop.mockResolvedValue(undefined);
  mocks.stopRelay.mockResolvedValue(undefined);
  mocks.disconnect.mockResolvedValue(undefined);
  mocks.status.mockResolvedValue({ running: true });
});
afterEach(async () => { await host.offline(); vi.useRealTimers(); });

it('keeps unsupported Mac hosting offline and exposes a server upgrade reason', async () => {
  mocks.register.mockRejectedValueOnce(new ApiError('Server update required', 422, 'server_upgrade_required', 'test'));
  await expect(host.online(runtime)).rejects.toMatchObject({ code: 'server_upgrade_required' });
  expect(get(host)).toMatchObject({ online: false, busy: false, errorCode: 'server_upgrade_required' });
  expect(mocks.punch).not.toHaveBeenCalled();
});

it('cancels host negotiation on going offline without relay fallback or a late engine launch', async () => {
  let signal: AbortSignal | undefined;
  mocks.punch.mockImplementation((...args: unknown[]) => {
    signal = args[7] as AbortSignal;
    return new Promise((_resolve, reject) => signal?.addEventListener('abort', () => reject(new DOMException('Cancelled', 'AbortError')), { once: true }));
  });
  mocks.sessions.mockResolvedValue({ items: [target] });
  await host.online(runtime);
  await settle();
  expect(signal).toBeDefined();
  await host.offline();
  expect(signal?.aborted).toBe(true);
  expect(mocks.relay).not.toHaveBeenCalled();
  expect(mocks.launch).not.toHaveBeenCalled();
  expect(get(host)).toMatchObject({ online: false, busy: false, error: null });
});

it('ignores credentials returned after host shutdown', async () => {
  const credentials = deferred<{ sessionToken: string; wireProtocol: string }>();
  mocks.credentials.mockReturnValueOnce(credentials.promise);
  mocks.sessions.mockResolvedValue({ items: [target] });
  await host.online(runtime);
  const shutdown = host.offline();
  credentials.resolve({ sessionToken: 'late-token', wireProtocol: 'snv2' });
  await shutdown;
  expect(mocks.punch).not.toHaveBeenCalled();
  expect(mocks.relay).not.toHaveBeenCalled();
  expect(mocks.launch).not.toHaveBeenCalled();
});

it('waits for late registration cleanup before advertising the same host again', async () => {
  const registration = deferred<{ id: string }>();
  mocks.register.mockReturnValueOnce(registration.promise);
  const starting = host.online(runtime);
  await settle();
  expect(mocks.register).toHaveBeenCalledTimes(1);
  const shutdown = host.offline();
  expect(get(host)).toMatchObject({ busy: true, online: false });
  registration.resolve({ id: 'local-host' });
  await Promise.all([starting, shutdown]);
  expect(mocks.offline).toHaveBeenCalledWith('local-host');
  expect(get(host)).toMatchObject({ busy: false, online: false });
  await host.online(runtime);
  expect(get(host).online).toBe(true);
  expect(mocks.register).toHaveBeenCalledTimes(2);
});

it('finishes stopping a late native launch before allowing a new host lifecycle', async () => {
  const launch = deferred<undefined>();
  mocks.launch.mockReturnValueOnce(launch.promise);
  mocks.sessions.mockResolvedValue({ items: [target] });
  await host.online(runtime);
  await settle();
  expect(mocks.launch).toHaveBeenCalledTimes(1);
  const shutdown = host.offline();
  await host.online(runtime);
  expect(mocks.register).toHaveBeenCalledTimes(1);
  expect(get(host).busy).toBe(true);
  launch.resolve(undefined);
  await shutdown;
  expect(get(host)).toMatchObject({ online: false, engineRunning: false, busy: false });
  expect(mocks.stop).toHaveBeenCalledWith('host');
});

it('ignores session polling from an earlier lifecycle even when the device ID is unchanged', async () => {
  await host.online(runtime);
  const response = deferred<{ items: ConnectionSession[] }>();
  mocks.sessions.mockReturnValueOnce(response.promise);
  const poll = host.refreshRequests();
  await host.offline();
  await host.online(runtime);
  response.resolve({ items: [target] });
  await poll;
  expect(get(host).sessions).toEqual([]);
  expect(mocks.credentials).not.toHaveBeenCalled();
});

it('does not restore an error after a late heartbeat fails on an offline host', async () => {
  const heartbeat = deferred<undefined>();
  mocks.heartbeat.mockReturnValueOnce(heartbeat.promise.then(() => { throw new Error('network'); }));
  await host.online(runtime);
  await vi.advanceTimersByTimeAsync(6000);
  await host.offline();
  heartbeat.resolve(undefined);
  await settle();
  expect(get(host).error).toBeNull();
});

it('preserves a stopped-engine error across successful request polling', async () => {
  mocks.sessions.mockResolvedValue({ items: [target] });
  await host.online(runtime);
  await settle();
  expect(get(host).engineRunning).toBe(true);
  mocks.status.mockResolvedValueOnce({ running: false, lastError: 'Screen Recording permission is required' });
  await host.refreshRequests();
  await host.refreshRequests();
  expect(get(host).error).toBe('Screen Recording permission is required');
  expect(get(host).failedP2pSessionId).toBe(target.id);
});

it('lets the host stop a session while route negotiation is pending', async () => {
  let signal: AbortSignal | undefined;
  mocks.punch.mockImplementation((...args: unknown[]) => {
    signal = args[7] as AbortSignal;
    return new Promise((_resolve, reject) => signal?.addEventListener('abort', () => reject(new DOMException('Cancelled', 'AbortError')), { once: true }));
  });
  mocks.sessions.mockResolvedValue({ items: [target] });
  await host.online(runtime);
  await settle();
  await host.disconnect(target.id);
  expect(signal?.aborted).toBe(true);
  expect(mocks.disconnect).toHaveBeenCalledWith(target.id);
  expect(mocks.launch).not.toHaveBeenCalled();
  expect(mocks.relay).not.toHaveBeenCalled();
  expect(get(host)).toMatchObject({ online: true, engineRunning: false, sessions: [], launchingSessionId: null });
});

it('cancels negotiation when the remote client ends the accepted session', async () => {
  let signal: AbortSignal | undefined;
  mocks.punch.mockImplementation((...args: unknown[]) => {
    signal = args[7] as AbortSignal;
    return new Promise((_resolve, reject) => signal?.addEventListener('abort', () => reject(new DOMException('Cancelled', 'AbortError')), { once: true }));
  });
  mocks.sessions.mockResolvedValueOnce({ items: [target] });
  await host.online(runtime);
  await settle();
  await host.refreshRequests();
  expect(signal?.aborted).toBe(true);
  expect(mocks.launch).not.toHaveBeenCalled();
  expect(mocks.relay).not.toHaveBeenCalled();
  expect(get(host)).toMatchObject({ online: true, sessions: [], launchingSessionId: null });
});

it('keeps sharing status visible and allows another stop attempt if native shutdown fails', async () => {
  await host.online(runtime);
  host.setEngineRunning(true);
  mocks.stop.mockRejectedValueOnce(new Error('Native shutdown failed'));
  await expect(host.offline()).rejects.toThrow('Unable to stop screen sharing');
  expect(get(host)).toMatchObject({ online: true, busy: false, engineRunning: true });
  expect(mocks.offline).not.toHaveBeenCalled();
  await host.offline();
  expect(get(host)).toMatchObject({ online: false, engineRunning: false });
});

it('revokes an acceptance completed after going offline without restoring session state', async () => {
  mocks.sessions.mockResolvedValue({ items: [{ ...target, status: 'pending' }] });
  await host.online(runtime);
  const acceptance = deferred<ConnectionSession>();
  mocks.accept.mockReturnValueOnce(acceptance.promise);
  const accepting = host.accept(target.id);
  await host.offline();
  acceptance.resolve(target);
  await accepting;
  expect(mocks.disconnect).toHaveBeenCalledWith(target.id);
  expect(get(host)).toMatchObject({ online: false, sessions: [], actionSessionId: null });
  expect(mocks.launch).not.toHaveBeenCalled();
});
