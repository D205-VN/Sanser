import { beforeEach, expect, it } from 'vitest';
import { get } from 'svelte/store';
import { nativePerformance, metricIsFresh, type NativePerformanceUpdate } from './nativePerformance';

const update = (extra: Partial<NativePerformanceUpdate> = {}): NativePerformanceUpdate => ({
  reportId: 'new', kind: 'client', wireProtocol: 'snv2', route: 'direct', relayTransport: null,
  startedAtMs: 200, ended: false, performance: { metrics: {} }, findings: [], ...extra
});
beforeEach(() => nativePerformance.clear());
it('isolates host/viewer observations and ignores late events from an older native process', () => {
  nativePerformance.update(update());
  nativePerformance.update(update({ reportId: 'host', kind: 'host', startedAtMs: 100 }));
  nativePerformance.update(update({ reportId: 'old', startedAtMs: 100, ended: true }));
  expect(get(nativePerformance).client?.reportId).toBe('new');
  expect(get(nativePerformance).host?.reportId).toBe('host');
  nativePerformance.update(update({ ended: true }));
  nativePerformance.update(update());
  expect(get(nativePerformance).client?.ended).toBe(true);
});
it('treats an actual zero as measured and absent or expired data as unknown', () => {
  const metric = { latestMs: 0, p50Ms: 0, p95Ms: 0, maxMs: 0, samples: 1, updatedAtMs: 100, source: 'snv2-video' };
  expect(metricIsFresh(metric, 200)).toBe(true);
  expect(metricIsFresh(metric, 5_101)).toBe(false);
  expect(metricIsFresh(metric, 50)).toBe(false);
  expect(metricIsFresh(undefined, 200)).toBe(false);
});
