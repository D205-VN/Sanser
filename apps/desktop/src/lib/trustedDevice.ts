import type { ConnectionSession } from './types';

/**
 * Selects one unattended-access request that this host is explicitly allowed
 * to accept. Account ownership and online state are revalidated by the server
 * when the accept endpoint is called; this local allowlist narrows automatic
 * acceptance to device identities the user trusted on this Windows machine.
 */
export function trustedPendingSession(
  sessions: readonly ConnectionSession[],
  hostDeviceId: string,
  trustedDeviceIds: readonly string[]
): ConnectionSession | null {
  const trusted = new Set(trustedDeviceIds);
  return sessions.find((item) =>
    item.status === 'pending' &&
    item.hostDeviceId === hostDeviceId &&
    item.requesterDeviceId !== hostDeviceId &&
    trusted.has(item.requesterDeviceId)
  ) ?? null;
}
