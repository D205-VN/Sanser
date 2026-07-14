import { describe, expect, it } from 'vitest';
import type { ConnectionSession } from './types';
import { trustedPendingSession } from './trustedDevice';

const hostDeviceId = '11111111-1111-4111-8111-111111111111';
const trustedMacId = '22222222-2222-4222-8222-222222222222';
const unknownMacId = '33333333-3333-4333-8333-333333333333';

function request(overrides: Partial<ConnectionSession> = {}): ConnectionSession {
  return {
    id: '44444444-4444-4444-8444-444444444444',
    requesterDeviceId: trustedMacId,
    hostDeviceId,
    status: 'pending',
    transport: null,
    networkMode: 'auto',
    qualityProfile: 'balanced',
    requestedCodec: 'auto',
    createdAt: new Date(0).toISOString(),
    ...overrides
  };
}

describe('trustedPendingSession', () => {
  it('selects only a pending request for this host from an explicitly trusted device', () => {
    const selected = trustedPendingSession(
      [request({ requesterDeviceId: unknownMacId }), request()],
      hostDeviceId,
      [trustedMacId]
    );
    expect(selected?.requesterDeviceId).toBe(trustedMacId);
  });

  it('rejects untrusted, already accepted, foreign-host and self requests', () => {
    expect(trustedPendingSession([request()], hostDeviceId, [])).toBeNull();
    expect(trustedPendingSession([request({ status: 'accepted' })], hostDeviceId, [trustedMacId])).toBeNull();
    expect(trustedPendingSession([request({ hostDeviceId: unknownMacId })], hostDeviceId, [trustedMacId])).toBeNull();
    expect(trustedPendingSession(
      [request({ requesterDeviceId: hostDeviceId })],
      hostDeviceId,
      [hostDeviceId]
    )).toBeNull();
  });
});
