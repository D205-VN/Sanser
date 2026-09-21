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
