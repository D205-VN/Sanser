import { get } from 'svelte/store';
import { expect, it, vi } from 'vitest';
import { ApiClient } from '../lib/api';
import type { ConnectionSession } from '../lib/types';
import { connection } from './connection';

it('ignores a stale request error after the connection is cleared', async () => {
  const client = new ApiClient('https://sanser.example');
  let rejectRequest: ((error: Error) => void) | undefined;
  vi.spyOn(client, 'getSession').mockImplementation(() => new Promise((_resolve, reject) => { rejectRequest = reject; }));
  connection.begin({ id: 'previous-session' } as ConnectionSession);
  const request = connection.refresh(client);
  connection.clear();
  rejectRequest?.(new Error('late failure'));
  await request;
  expect(get(connection).error).toBeNull();
  expect(get(connection).session).toBeNull();
});

it('retains the negotiated wire protocol when refreshing an authorized session', async () => {
  const client = new ApiClient('https://sanser.example');
  const target = { id: 'authorized-session', status: 'accepted' } as ConnectionSession;
  vi.spyOn(client, 'getSession').mockResolvedValue(target);
  connection.begin(target);
  connection.authorizeNative({ sessionId: target.id, deviceId: 'client', peerDeviceId: 'host', peerRouteAddress: '192.0.2.1', basePort: 5000, expiresAt: Math.floor(Date.now() / 1000) + 60, sessionToken: 'ephemeral', wireProtocol: 'snv2' });
  await connection.refresh(client);
  expect(get(connection).session?.wireProtocol).toBe('snv2');
  connection.clear();
});

it('keeps preparation locked until cancelled work has finished cleaning up', () => {
  connection.begin({ id: 'cancelled-session' } as ConnectionSession);
  connection.setPreparing(true);
  connection.clear();
  expect(get(connection).session).toBeNull();
  expect(get(connection).preparing).toBe(true);
  connection.setPreparing(false);
  expect(get(connection).preparing).toBe(false);
});

it('keeps native errors visible through successful polling and transient API failures', async () => {
  const client = new ApiClient('https://sanser.example');
  const target = { id: 'failed-video', status: 'accepted' } as ConnectionSession;
  const poll = vi.spyOn(client, 'getSession').mockResolvedValue(target);
  connection.begin(target);
  connection.setError('No video received');
  await connection.refresh(client);
  expect(get(connection).error).toBe('No video received');
  // Even identical text must not transfer ownership to the polling error.
  poll.mockRejectedValueOnce(new Error('No video received'));
  await connection.refresh(client);
  expect(get(connection).error).toBe('No video received');
  await connection.refresh(client);
  expect(get(connection).error).toBe('No video received');
  connection.clear();
});

it('does not overwrite fresh native credentials when an older poll completes', async () => {
  const client = new ApiClient('https://sanser.example');
  const target = { id: 'credentials-rotated', status: 'accepted' } as ConnectionSession;
  let finishPoll: ((value: ConnectionSession) => void) | undefined;
  vi.spyOn(client, 'getSession').mockImplementation(() => new Promise((resolve) => { finishPoll = resolve; }));
  connection.begin(target);
  const poll = connection.refresh(client);
  connection.authorizeNative({ sessionId: target.id, deviceId: 'client', peerDeviceId: 'host', peerRouteAddress: '192.0.2.2', basePort: 6000, expiresAt: Math.floor(Date.now() / 1000) + 60, sessionToken: 'fresh-test-credential', wireProtocol: 'snv2' });
  finishPoll?.(target);
  await poll;
  expect(get(connection).session).toMatchObject({ sessionToken: 'fresh-test-credential', port: 6000, wireProtocol: 'snv2' });
  connection.clear();
});

it('clears a polling error when polling recovers', async () => {
  const client = new ApiClient('https://sanser.example');
  const target = { id: 'polling-session', status: 'accepted' } as ConnectionSession;
  vi.spyOn(client, 'getSession').mockRejectedValueOnce(new Error('Network unavailable')).mockResolvedValue(target);
  connection.begin(target);
  await connection.refresh(client);
  expect(get(connection).error).toBe('Network unavailable');
  await connection.refresh(client);
  expect(get(connection).error).toBeNull();
  connection.clear();
});

it('retains the startup retry limit until a manual retry or a new session', () => {
  connection.begin({ id: 'retry-budget', status: 'accepted' } as ConnectionSession);
  expect(connection.takeNativeStartupRetry()).toBe(true);
  connection.setEngineRunning(true);
  connection.setEngineRunning(false);
  expect(connection.takeNativeStartupRetry()).toBe(false);
  connection.blockNativeRetry();
  expect(get(connection).nativeRetryBlocked).toBe(true);
  connection.resetNativeRetry();
  expect(connection.takeNativeStartupRetry()).toBe(true);
  connection.begin({ id: 'next-session', status: 'accepted' } as ConnectionSession);
  expect(connection.takeNativeStartupRetry()).toBe(true);
  connection.clear();
  expect(connection.takeNativeStartupRetry()).toBe(false);
});
