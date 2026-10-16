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
  it('retains the OS-selected source and warns once when the path was slow before media', () => {
    diagnostics.clear();
    const probe = { samples: 5, medianMs: 183, minMs: 180, maxMs: 190, jitterMs: 2, lossPercent: 0, scoreMs: 187 };
    networkDiagnostics.selectRoute({ pairId: 'slow', localCandidateType: 'host', localCandidateEndpoint: '192.0.2.1:50000',
      localMappingProtocol: 'none', remoteMappingProtocol: 'pcp', probeSourceEndpoint: '192.0.2.2:50000',
      probeSourceInterface: { name: 'feth-test', kind: 'virtual' }, remoteCandidateType: 'portMapped',
      remoteCandidateEndpoint: '198.51.100.2:50000', verifiedRemoteEndpoint: '198.51.100.2:50000', probe, reason: 'measured' }, 'slow-attempt');
    const health = { attemptId: 'slow-attempt', probe, liveWireRttMs: 185, status: 'slow-at-nomination', consecutiveElevated: 0 };
    networkDiagnostics.updateRouteHealth(health); networkDiagnostics.updateRouteHealth(health);
    expect(get(diagnostics)).toHaveLength(1);
    expect(get(diagnostics)[0]?.message).toContain('already slow at nomination');
    const exported = JSON.parse(diagnostics.exportJson()) as { network: { selection: { probeSourceEndpoint: string; remoteMappingProtocol: string } } };
    expect(exported.network.selection.probeSourceEndpoint).toBe('192.0.2.2:50000');
    expect(exported.network.selection.remoteMappingProtocol).toBe('pcp');
    diagnostics.clear();
  });
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
