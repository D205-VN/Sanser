import { describe, expect, it } from 'vitest';
import { get } from 'svelte/store';
import { diagnostics, networkDiagnostics, sanitizeDetails } from './diagnostics';

describe('sanitizeDetails', () => {
  it('redacts credentials and bearer values', () => {
    expect(sanitizeDetails({ accessToken: 'abc', note: 'Bearer top-secret', rtt: 5 })).toEqual({
      accessToken: '[REDACTED]',
      note: 'Bearer [REDACTED]',
      rtt: 5
    });
  });
});


describe('Direct route diagnostics', () => {
  it('ignores health from previous attempts and warns once per elevated episode', () => {
    diagnostics.clear();
    const probe = { samples: 5, medianMs: 13, minMs: 11, maxMs: 16, jitterMs: 1, lossPercent: 0, scoreMs: 15 };
    networkDiagnostics.selectRoute({ pairId: 'a', localCandidateType: 'host', localCandidateEndpoint: '192.0.2.1:50000',
      remoteCandidateType: 'serverReflexive', remoteCandidateEndpoint: '198.51.100.2:50000',
      verifiedRemoteEndpoint: '198.51.100.2:50001', probe, reason: 'measured' }, 'new-attempt');
    const health = { attemptId: 'old-attempt', probe, liveWireRttMs: 200, status: 'elevated-after-media', consecutiveElevated: 3 };
    networkDiagnostics.updateRouteHealth(health);
    expect(get(networkDiagnostics).routeHealth).toBeNull();
    expect(get(diagnostics)).toHaveLength(0);
    health.attemptId = 'new-attempt';
    networkDiagnostics.updateRouteHealth(health);
    networkDiagnostics.updateRouteHealth(health);
    expect(get(networkDiagnostics).routeHealth?.liveWireRttMs).toBe(200);
    expect(get(diagnostics)).toHaveLength(1);
    diagnostics.clear();
    expect(get(networkDiagnostics).selection).toBeNull();
  });
});
