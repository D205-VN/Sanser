import { expect, it } from 'vitest';
import { VoiceMeasurements, type StatsSource } from './stats';
const source = {};
function received(extra: Record<string, unknown> = {}, key = source): StatsSource {
  return { key, direction: 'receive', records: [{ id: 'private-stream', type: 'inbound-rtp', kind: 'audio', jitter: .003,
    packetsReceived: 100, packetsLost: 2, concealedSamples: 960, jitterBufferDelay: 100, jitterBufferEmittedCount: 1000, ...extra }] };
}
function data(value: VoiceMeasurements) { return value.export() as { metrics: Record<string, { p50Ms: number; p95Ms: number; samples: number } | null>;
  samples: { metrics: Record<string, number | undefined>; deltas: Record<string, number | undefined>; baselines: number; receivers: number }[] }; }
it('keeps first observations unknown, computes bounded deltas, and converts seconds to milliseconds', () => {
  const measure = new VoiceMeasurements();measure.observe([received()], 1000, false);
  expect(data(measure).samples[0]?.deltas.receivedPackets).toBeUndefined();
  measure.observe([received({ jitter: .005, packetsReceived: 150, packetsLost: 3, concealedSamples: 1920, jitterBufferDelay: 102, jitterBufferEmittedCount: 2000 })], 2000, false);
  const sample = data(measure).samples[1];expect(sample?.deltas.receivedPackets).toBe(50);
  expect(sample?.deltas.lostPackets).toBe(1);expect(sample?.metrics.jitterBufferResidenceAvgMs).toBe(2);
  expect(data(measure).metrics.receiverJitterMaxMs?.p95Ms).toBe(5);
  expect(data(measure).metrics.senderRemoteRttMaxMs).toBe(null);
});
it('does not confuse SFU sender RTT with receiver jitter and drops private RTC context', () => {
  const measure = new VoiceMeasurements();measure.observe([received(), { key: {}, direction: 'send', records: [
    { type: 'remote-inbound-rtp', kind: 'audio', roundTripTime: .015, address: 'private-address', token: 'private-token' },
    { type: 'candidate-pair', currentRoundTripTime: 99, localAddress: 'private-address' },
    { type: 'inbound-rtp', kind: 'video', id: 'private-video', packetsReceived: 999 }
  ] }], 1000, true);
  expect(data(measure).metrics.senderRemoteRttMaxMs?.p50Ms).toBe(15);
  expect(JSON.stringify(measure.export())).not.toMatch(/private-|address|token":/);
});
it('rebases counter decreases, new tracks and long gaps, ignores duplicates and nonfinite values', () => {
  const measure = new VoiceMeasurements();measure.observe([received()], 1000, false);
  measure.observe([received({ packetsReceived: 1, jitter: NaN })], 2000, false);
  expect(data(measure).samples[1]?.deltas.receivedPackets).toBeUndefined();
  measure.observe([received({ packetsReceived: 2 })], 8000, false);
  expect(data(measure).samples[2]?.deltas.receivedPackets).toBeUndefined();
  measure.observe([received({ packetsReceived: 3 })], 8000, false);expect(data(measure).samples.length).toBe(3);
  measure.observe([received({}, {})], 9000, false);expect(data(measure).samples[3]?.baselines).toBe(1);
});
it('bounds sources/retention, preserves actual zeros, and does not expose mutable sample objects', () => {
  const measure = new VoiceMeasurements();
  for (let n = 0; n < 150; n++) measure.observe(Array.from({ length: 8 }, () => received({ jitter: 0 }, source)), n * 1000, false);
  const result = data(measure);expect(result.samples.length).toBe(120);expect(result.samples[0]?.receivers).toBe(1);
  expect(result.metrics.receiverJitterMaxMs?.p50Ms).toBe(0);
  result.samples.length = 0;expect(data(measure).samples.length).toBe(120);
});
