import { describe, it, expect, afterEach } from 'vitest';
import { get } from 'svelte/store';
import { fecHealth, type FecHealthUpdate } from './fecHealth';
import { diagnostics } from './diagnostics';
const stats = { negotiated: true, overheadPercent: 12, videoBytes: 100000, parityBytes: 9000, parityPackets: 8, recovered: 2, expired: 0, invalid: 0, budgetSkipped: 1, receiverLate: 0, congestionPauses: 0 };
const update: FecHealthUpdate = { kind: 'client', sessionId: 'session', startedAtMs: 20, transport: 'direct', stats };
afterEach(() => diagnostics.clear());
describe('FEC diagnostics', () => {
  it('rejects late telemetry from an older engine and keeps host/viewer records independent', () => {
    fecHealth.update(update);
    fecHealth.update({ ...update, startedAtMs: 10, stats: { ...stats, recovered: 999 } });
    expect(get(fecHealth)[0]?.stats.recovered).toBe(2);
    fecHealth.update({ ...update, kind: 'host', transport: 'udp-relay' });
    expect(get(fecHealth)).toHaveLength(2);
    const exported = JSON.parse(diagnostics.exportJson()) as { fec: FecHealthUpdate[] };
    expect(exported.fec).toHaveLength(2);
  });
  it('bounds retained sessions and clears all numeric snapshots', () => {
    for (let index = 0; index < 20; index++) fecHealth.update({ ...update, sessionId: `session-${index}` });
    expect(get(fecHealth)).toHaveLength(8);
    expect(get(fecHealth)[0]?.sessionId).toBe('session-19');
    diagnostics.clear(); expect(get(fecHealth)).toHaveLength(0);
  });
});
