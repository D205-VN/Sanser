import { describe, expect, it } from 'vitest';
import { formatAccessCode, normalizeAccessCode } from './guest';
import { trustedPendingSession } from './trustedDevice';
import type { ConnectionSession } from './types';

describe('guest helpers', () => {
  it('normalizes and formats access codes', () => {
    expect(normalizeAccessCode('123 456-789')).toBe('123456789');
    expect(normalizeAccessCode('12345678')).toBeNull();
    expect(normalizeAccessCode('12345678a')).toBeNull();
    expect(formatAccessCode('123456789')).toBe('123 456 789');
  });

  it('never auto-accepts a guest, even if its device id were trusted', () => {
    const request: ConnectionSession = {
      id: 's', requesterDeviceId: 'guest-device', hostDeviceId: 'host', status: 'pending', transport: null,
      networkMode: 'auto', qualityProfile: 'auto', requestedCodec: 'auto', createdAt: new Date(0).toISOString(),
      guest: { name: 'Helper' }
    };
    expect(trustedPendingSession([request], 'host', ['guest-device'])).toBeNull();
    expect(trustedPendingSession([{ ...request, guest: undefined }], 'host', ['guest-device'])).not.toBeNull();
  });
});
