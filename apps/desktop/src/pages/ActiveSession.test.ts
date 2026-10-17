import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { get } from 'svelte/store';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { ConnectionSession, RuntimeStatus } from '../lib/types';
import { connection } from '../stores/connection';

const mocks = vi.hoisted(() => ({
  ready: vi.fn(), credentials: vi.fn(), disconnect: vi.fn(), getSession: vi.fn(),
  punch: vi.fn(), release: vi.fn(), launch: vi.fn(), stop: vi.fn(), relay: vi.fn(), stopRelay: vi.fn(), status: vi.fn(), recovery: vi.fn()
}));
vi.mock('../stores/session', async () => {
  const { writable } = await import('svelte/store');
  return { session: { ...writable({ mode: 'cloud' }), client: () => ({ markNativeReady: mocks.ready, sessionCredentials: mocks.credentials, disconnectSession: mocks.disconnect, getSession: mocks.getSession }) } };
});
vi.mock('../stores/presence', async () => {
  const { writable } = await import('svelte/store');
  return { presence: writable({ deviceId: 'local-client', online: true }) };
});
vi.mock('../lib/platform', () => ({ launchEngine: mocks.launch, stopEngine: mocks.stop, startRelay: mocks.relay, stopRelay: mocks.stopRelay, engineStatus: mocks.status, routeRecoveryPlan: mocks.recovery, saveNativePreferences: vi.fn().mockResolvedValue(undefined) }));
vi.mock('../lib/p2pSignaling', async (importOriginal) => ({ ...await importOriginal<typeof import('../lib/p2pSignaling')>(), coordinateP2pConnection: mocks.punch, releaseP2pConnection: mocks.release }));
import ActiveSession from './ActiveSession.svelte';
import { DEFAULT_PREFERENCES, preferences } from '../stores/preferences';
import { nativePerformance, type NativePerformanceUpdate } from '../stores/nativePerformance';
const runtime = { platform: 'macOS', capabilities: { clientEngine: { state: 'available' }, nativeDirect: { state: 'available' }, desktopShell: { state: 'available' } } } as RuntimeStatus;
const target = { id: 'test-session', hostDeviceId: 'remote-host', requesterDeviceId: 'local-client', status: 'accepted', transport: 'native', networkMode: 'auto', qualityProfile: 'balanced', requestedCodec: 'auto' } as ConnectionSession;

beforeEach(async () => {
  vi.resetAllMocks();
  nativePerformance.clear();
  connection.setPreparing(false);
  connection.begin(target);
  mocks.ready.mockResolvedValue({ requesterReadyAt: '2026-10-04T00:00:00.000Z' });
  mocks.credentials.mockResolvedValue({ sessionId: target.id, peerRouteAddress: '192.0.2.1', basePort: 5000, sessionToken: 'test-only-token', expiresAt: 9999999999, wireProtocol: 'snv2' });
  mocks.disconnect.mockResolvedValue(undefined);
  mocks.stop.mockResolvedValue(undefined);
  mocks.stopRelay.mockResolvedValue(undefined);
  mocks.getSession.mockResolvedValue(target);
  mocks.status.mockResolvedValue({ running: true, lastError: null });
  mocks.punch.mockResolvedValue({ localPort: 5000, remotePort: 6000, remoteAddress: '192.0.2.1' });
  mocks.launch.mockResolvedValue(undefined);
  mocks.release.mockResolvedValue(undefined);
  mocks.recovery.mockResolvedValue({ action: 'reconnect', delayMs: 1_500, reason: 'peer-timeout' });
  await preferences.save(structuredClone(DEFAULT_PREFERENCES));
});

it.each([
  { capable: true, enabled: true, audio: true },
  { capable: true, enabled: false, audio: false },
  { capable: false, enabled: true, audio: false }
])('Windows audio receiving follows capability and viewer preference: %j', async ({ capable, enabled, audio }) => {
  const settings = structuredClone(DEFAULT_PREFERENCES); settings.host.audioEnabled = enabled;
  await preferences.save(settings);
  const windows = { ...runtime, platform: 'windows', capabilities: { ...runtime.capabilities, clientAudio: capable } };
  render(ActiveSession, { runtime: windows, navigate: vi.fn() });
  await fireEvent.click(screen.getByRole('button', { name: 'Open remote desktop' }));
  await waitFor(() => expect(mocks.launch).toHaveBeenCalledWith(expect.objectContaining({ wireProtocol: 'snv2', audioEnabled: audio })));
  expect(screen.queryByText('Sound is unavailable for this connection.')).not.toBeInTheDocument();
  if(capable) expect(screen.queryByText('This viewer does not support sound for this connection yet.')).not.toBeInTheDocument();
  else expect(screen.getByText('This viewer does not support sound for this connection yet.')).toBeInTheDocument();
});
afterEach(() => { cleanup(); connection.setPreparing(false); connection.clear(); nativePerformance.clear(); vi.useRealTimers(); });

it('explains video-only group sharing and never opts a group viewer into audio', async () => {
  const viewer = { ...target, sharingId: 'group', wireProtocol: 'snv2' as const, accessMode: 'view-only' as const };
  connection.begin(viewer); mocks.getSession.mockResolvedValue(viewer);
  render(ActiveSession, { runtime: { ...runtime, capabilities: { ...runtime.capabilities, clientAudio: true } }, navigate: vi.fn() });
  expect(screen.getByText('Multi-computer sharing currently sends video only. Sound is not included.')).toBeInTheDocument();
  expect(screen.queryByText('Sound is unavailable for this connection.')).not.toBeInTheDocument();
  await fireEvent.click(screen.getByRole('button', { name: 'Open remote desktop' }));
  await waitFor(() => expect(mocks.launch).toHaveBeenCalledWith(expect.objectContaining({ audioEnabled: false, inputEnabled: false })));
});

it('does not report unavailable audio for the legacy Mac to Windows connection', async () => {
  const legacy = { ...target, wireProtocol: 'legacy' as const };
  connection.begin(legacy); mocks.getSession.mockResolvedValue(legacy);
  mocks.credentials.mockResolvedValue({ sessionId: target.id, peerRouteAddress: '192.0.2.1', basePort: 5000, sessionToken: 'test-only-token', expiresAt: 9999999999, wireProtocol: 'legacy' });
  render(ActiveSession, { runtime, navigate: vi.fn() });
  expect(screen.queryByText(/does not support sound/)).not.toBeInTheDocument();
  await fireEvent.click(screen.getByRole('button', { name: 'Open remote desktop' }));
  await waitFor(() => expect(mocks.launch).toHaveBeenCalledWith(expect.objectContaining({ audioEnabled: true })));
});

it('keeps a view-only session input-disabled even if credential metadata omits the restriction', async () => {
  const viewer = { ...target, accessMode: 'view-only' as const };
  connection.begin(viewer);
  mocks.getSession.mockResolvedValue(viewer);
  mocks.credentials.mockResolvedValue({ sessionId: target.id, peerRouteAddress: '192.0.2.1',
    basePort: 5000, sessionToken: 'test-only-token', expiresAt: 9999999999, wireProtocol: 'legacy', accessMode: 'control' });
  render(ActiveSession, { runtime, navigate: vi.fn() });
  await fireEvent.click(screen.getByRole('button', { name: 'Open remote desktop' }));
  await waitFor(() => expect(mocks.launch).toHaveBeenCalledWith(expect.objectContaining({
    inputEnabled: false, relativeMouse: false
  })));
  expect(get(connection).session?.accessMode).toBe('view-only');
});

it('allows cancelling an in-progress route without falling back to relay or launching an engine', async () => {
  let activeSignal: AbortSignal | undefined;
  mocks.punch.mockImplementation((...args: unknown[]) => {
    activeSignal = args[7] as AbortSignal;
    return new Promise((_resolve, reject) => activeSignal?.addEventListener('abort', () => reject(new DOMException('Cancelled', 'AbortError')), { once: true }));
  });
  render(ActiveSession, { runtime, navigate: vi.fn() });
  await fireEvent.click(screen.getByRole('button', { name: 'Open remote desktop' }));
  await screen.findByRole('heading', { name: 'Connecting directly' });
  expect(mocks.punch.mock.calls[0]?.[8]).toBe('2026-10-04T00:00:00.000Z');
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
  expect(mocks.stopRelay).toHaveBeenCalledWith('client', target.id);
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

it('reconnects an explicitly lost Direct route using a new generation and credentials', async () => {
  vi.useFakeTimers();
  const direct = { ...target, networkMode: 'direct' as const };
  connection.begin(direct); mocks.getSession.mockResolvedValue(direct);
  mocks.ready.mockResolvedValueOnce({ requesterReadyAt: '2026-10-04T00:00:00.000Z' })
    .mockResolvedValueOnce({ requesterReadyAt: '2026-10-04T00:00:01.000Z' });
  render(ActiveSession, { runtime, navigate: vi.fn() });
  await fireEvent.click(screen.getByRole('button', { name: 'Open remote desktop' }));
  await vi.advanceTimersByTimeAsync(0);
  mocks.status.mockResolvedValueOnce({ running: false, routeFailure: 'peer-timeout', lastError: 'Peer timeout' });
  await vi.advanceTimersByTimeAsync(2_000);
  expect(get(connection)).toMatchObject({ engineRunning: false, nativeRecoveryPending: true, nativeRouteRecoveries: 1 });
  expect(screen.getByRole('button', { name: 'Cancel reconnection' })).toBeEnabled();
  expect(mocks.recovery).toHaveBeenCalledWith(expect.objectContaining({ cause: 'peer-timeout', automaticAttempts: 0 }));
  expect(mocks.ready).toHaveBeenCalledTimes(1);
  await vi.advanceTimersByTimeAsync(2_000);
  expect(mocks.ready).toHaveBeenCalledTimes(2);
  expect(mocks.credentials).toHaveBeenCalledTimes(2);
  expect(mocks.punch.mock.calls.map((call: unknown[]) => call[8])).toEqual(['2026-10-04T00:00:00.000Z', '2026-10-04T00:00:01.000Z']);
  expect(mocks.launch).toHaveBeenLastCalledWith(expect.objectContaining({ networkMode: 'direct', relay: false }));
  expect(mocks.relay).not.toHaveBeenCalled();
  expect(mocks.stop.mock.calls.every((call) => call[0] === 'client')).toBe(true);
});

it('ignores a native status belonging to a different session', async () => {
  vi.useFakeTimers();
  connection.setEngineRunning(true);
  mocks.status.mockResolvedValue({ running: false, sessionId: 'different-session', routeFailure: 'peer-timeout', lastError: 'Old route lost' });
  render(ActiveSession, { runtime, navigate: vi.fn() });
  await vi.advanceTimersByTimeAsync(6_000);
  expect(get(connection)).toMatchObject({ engineRunning: true, nativeRecoveryPending: false, nativeRouteRecoveries: 0 });
  expect(mocks.recovery).not.toHaveBeenCalled();
  expect(mocks.stop).not.toHaveBeenCalled();
  expect(mocks.stopRelay).not.toHaveBeenCalled();
});

it('preserves the recovery budget and delay across navigation and stops after two automatic reconnects', async () => {
  vi.useFakeTimers();
  connection.setEngineRunning(true);
  mocks.status.mockResolvedValue({ running: false, routeFailure: 'peer-timeout', lastError: 'Route lost' });
  mocks.recovery.mockImplementation((request: { automaticAttempts: number }) => Promise.resolve({ action: 'reconnect', delayMs: request.automaticAttempts === 0 ? 1_500 : 5_000, reason: 'peer-timeout' }));
  render(ActiveSession, { runtime, navigate: vi.fn() });
  await vi.advanceTimersByTimeAsync(2_000);
  expect(get(connection).nativeRouteRecoveries).toBe(1);
  cleanup(); render(ActiveSession, { runtime, navigate: vi.fn() });
  await vi.advanceTimersByTimeAsync(0);
  expect(mocks.launch).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(30_000);
  expect(mocks.launch).toHaveBeenCalledTimes(2);
  expect(get(connection)).toMatchObject({ nativeRetryBlocked: true, nativeRouteRecoveries: 2 });
  expect(mocks.recovery).toHaveBeenCalledTimes(2);
});

function snapshot(extra: Partial<NativePerformanceUpdate> = {}): NativePerformanceUpdate {
  const metric = { latestMs: 200, p50Ms: 200, p95Ms: 200, maxMs: 200, samples: 5, updatedAtMs: Date.now(), source: 'legacy-control' };
  return { reportId: 'current', sessionId: target.id, kind: 'client', wireProtocol: 'legacy', route: 'direct', relayTransport: null,
    startedAtMs: get(connection).nativeLaunchStartedAtMs ?? Date.now(), ended: false,
    performance: { metrics: { wireRttEstimate: metric } }, findings: [], routeHealth: { attemptId: 'route', probe: { samples: 5, medianMs: 14, minMs: 12, maxMs: 16, jitterMs: 1, lossPercent: 0, scoreMs: 16 },
      liveWireRttMs: 200, status: 'elevated-after-media', consecutiveElevated: 5 }, ...extra };
}

it('requests fresh candidates when a current launch has sustained verified wire delay', async () => {
  vi.useFakeTimers(); render(ActiveSession, { runtime, navigate: vi.fn() });
  await fireEvent.click(screen.getByRole('button', { name: 'Open remote desktop' }));
  await vi.advanceTimersByTimeAsync(20_000);
  nativePerformance.update(snapshot());
  await vi.advanceTimersByTimeAsync(2_000);
  expect(mocks.recovery).toHaveBeenCalledWith(expect.objectContaining({ cause: 'sustained-wire-delay', wireRttMs: 200, probeMedianMs: 14, elevatedSamples: 5 }));
  await vi.advanceTimersByTimeAsync(2_000);
  expect(mocks.launch).toHaveBeenCalledTimes(2);
});

it.each(['application-only', 'slow-at-start', 'stale', 'old-session', 'old-launch', 'ended'] as const)('keeps the existing route for %s measurements', async (scenario) => {
  vi.useFakeTimers(); render(ActiveSession, { runtime, navigate: vi.fn() });
  await fireEvent.click(screen.getByRole('button', { name: 'Open remote desktop' }));
  await vi.advanceTimersByTimeAsync(20_000);
  const update = snapshot();
  const wire = update.performance.metrics.wireRttEstimate;
  if (!wire) throw new Error('Missing test wire measurement');
  if (scenario === 'application-only') update.performance.metrics = { controlAppRtt: wire };
  if (scenario === 'slow-at-start' && update.routeHealth) update.routeHealth.status = 'slow-at-nomination';
  if (scenario === 'old-session') update.sessionId = 'other-session';
  if (scenario === 'old-launch') update.startedAtMs -= 1;
  if (scenario === 'ended') update.ended = true;
  if (scenario === 'stale') {
    wire.updatedAtMs -= 10_000;
    mocks.recovery.mockResolvedValue({ action: 'observe', delayMs: 0, reason: 'stale-evidence' });
  }
  nativePerformance.update(update); await vi.advanceTimersByTimeAsync(8_000);
  expect(mocks.stop).not.toHaveBeenCalled(); expect(mocks.launch).toHaveBeenCalledTimes(1);
});

it('reconnects only the interrupted group viewer without enabling input or sound', async () => {
  vi.useFakeTimers();
  const viewer = { ...target, sharingId: 'group', accessMode: 'view-only' as const, wireProtocol: 'snv2' as const };
  connection.begin(viewer); mocks.getSession.mockResolvedValue(viewer);
  render(ActiveSession, { runtime, navigate: vi.fn() });
  await fireEvent.click(screen.getByRole('button', { name: 'Open remote desktop' }));
  mocks.status.mockResolvedValueOnce({ running: false, routeFailure: 'relay-lost', lastError: 'Relay lost' });
  await vi.advanceTimersByTimeAsync(4_000);
  expect(mocks.launch).toHaveBeenLastCalledWith(expect.objectContaining({ kind: 'client', inputEnabled: false, audioEnabled: false }));
  expect(mocks.launch).toHaveBeenCalledTimes(2);
  expect(mocks.stop).toHaveBeenCalledWith('client', target.id); expect(mocks.stop).not.toHaveBeenCalledWith('host');
});

it('does not reopen after Disconnect while the recovery policy is pending', async () => {
  vi.useFakeTimers(); connection.setEngineRunning(true);
  mocks.status.mockResolvedValue({ running: false, routeFailure: 'peer-timeout', lastError: 'Route lost' });
  let finish: ((value: unknown) => void) | undefined;
  mocks.recovery.mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
  render(ActiveSession, { runtime, navigate: vi.fn() }); await vi.advanceTimersByTimeAsync(2_000);
  await fireEvent.click(screen.getByRole('button', { name: 'Disconnect' }));
  finish?.({ action: 'reconnect', delayMs: 1_500, reason: 'peer-timeout' });
  await vi.advanceTimersByTimeAsync(10_000);
  expect(get(connection).session).toBeNull(); expect(mocks.ready).not.toHaveBeenCalled(); expect(mocks.launch).not.toHaveBeenCalled();
});

it('stops automatic recovery when fresh authorization fails', async () => {
  vi.useFakeTimers(); connection.setEngineRunning(true);
  mocks.status.mockResolvedValue({ running: false, routeFailure: 'peer-timeout', lastError: 'Route lost' });
  mocks.ready.mockRejectedValue(new Error('Host is offline'));
  render(ActiveSession, { runtime, navigate: vi.fn() }); await vi.advanceTimersByTimeAsync(30_000);
  expect(mocks.ready).toHaveBeenCalledTimes(1); expect(mocks.launch).not.toHaveBeenCalled();
  expect(get(connection).nativeRetryBlocked).toBe(true);
});

it('keeps a late recovery cleanup from stopping the relay of a new session', async () => {
  vi.useFakeTimers(); connection.setEngineRunning(true);
  mocks.status.mockResolvedValue({ running: false, routeFailure: 'peer-timeout', lastError: 'Route lost' });
  let finish: (() => void) | undefined; mocks.stop.mockImplementationOnce(() => new Promise<void>((resolve) => { finish = resolve; }));
  render(ActiveSession, { runtime, navigate: vi.fn() }); await vi.advanceTimersByTimeAsync(2_000);
  connection.begin({ ...target, id: 'new-session' }); connection.setEngineRunning(true);
  finish?.(); await vi.advanceTimersByTimeAsync(0);
  expect(mocks.stopRelay).not.toHaveBeenCalled();
  expect(get(connection)).toMatchObject({ engineRunning: true, nativeRouteRecoveries: 0, nativeRecoveryPending: false });
});

it('lets the user explicitly recheck routes while the window is running', async () => {
  render(ActiveSession, { runtime, navigate: vi.fn() });
  await fireEvent.click(screen.getByRole('button', { name: 'Open remote desktop' }));
  await fireEvent.click(await screen.findByRole('button', { name: 'Reconnect' }));
  await waitFor(() => expect(mocks.launch).toHaveBeenCalledTimes(2));
  expect(mocks.recovery).not.toHaveBeenCalled();
  expect(get(connection).nativeRouteRecoveries).toBe(0);
});

it('releases a selected port when a late P2P result arrives after cancellation', async () => {
  let finish: ((value: unknown) => void) | undefined;
  mocks.punch.mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
  render(ActiveSession, { runtime, navigate: vi.fn() });
  await fireEvent.click(screen.getByRole('button', { name: 'Open remote desktop' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Cancel connection' }));
  const selected = { attemptId: 'late', localPort: 5000, remotePort: 6000, remoteAddress: '192.0.2.1' };
  finish?.(selected);
  await waitFor(() => expect(mocks.release).toHaveBeenCalledWith(selected));
  expect(mocks.launch).not.toHaveBeenCalled();
});

it('finishes owned route cleanup and releases busy state when navigation interrupts it', async () => {
  vi.useFakeTimers(); connection.setEngineRunning(true);
  mocks.status.mockResolvedValueOnce({ running: false, routeFailure: 'peer-timeout', lastError: 'Route lost' });
  let finish: (() => void) | undefined; mocks.stop.mockImplementationOnce(() => new Promise<void>((resolve) => { finish = resolve; }));
  render(ActiveSession, { runtime, navigate: vi.fn() }); await vi.advanceTimersByTimeAsync(2_000);
  expect(get(connection).busy).toBe(true);
  cleanup(); finish?.(); await vi.advanceTimersByTimeAsync(0);
  expect(mocks.stopRelay).toHaveBeenCalledWith('client', target.id);
  expect(get(connection)).toMatchObject({ busy: false, engineRunning: false, nativeRecoveryPending: true, nativeRouteRecoveries: 1 });
  render(ActiveSession, { runtime, navigate: vi.fn() }); await vi.advanceTimersByTimeAsync(2_000);
  expect(mocks.launch).toHaveBeenCalledTimes(1);
});

it.each(['auth-rejected', 'decode-failed'] as const)('does not let a later relay failure hide %s and trigger reopening', async (startupFailure) => {
  vi.useFakeTimers(); connection.setEngineRunning(true);
  mocks.status.mockResolvedValue({ running: false, routeFailure: 'relay-lost', startupFailure, lastError: 'Native startup failed' });
  render(ActiveSession, { runtime, navigate: vi.fn() }); await vi.advanceTimersByTimeAsync(20_000);
  expect(mocks.recovery).not.toHaveBeenCalled(); expect(mocks.launch).not.toHaveBeenCalled();
  expect(get(connection).nativeRetryBlocked).toBe(true);
});

it('scopes cleanup by session when an old launch finishes after switching sessions', async () => {
  let finish: (() => void) | undefined;
  mocks.launch.mockImplementationOnce(() => new Promise<void>((resolve) => { finish = resolve; }));
  render(ActiveSession, { runtime, navigate: vi.fn() });
  await fireEvent.click(screen.getByRole('button', { name: 'Open remote desktop' }));
  await waitFor(() => expect(mocks.launch).toHaveBeenCalledTimes(1));
  connection.begin({ ...target, id: 'new-session' }); connection.setEngineRunning(true);
  finish?.();
  await waitFor(() => expect(mocks.stop).toHaveBeenCalledWith('client', target.id));
  expect(mocks.stopRelay).toHaveBeenCalledWith('client', target.id);
  expect(get(connection).session?.id).toBe('new-session');
  expect(get(connection).engineRunning).toBe(true);
});
