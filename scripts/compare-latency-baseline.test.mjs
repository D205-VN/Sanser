import assert from 'node:assert/strict';
import test from 'node:test';
import { summarize, summarizeReport } from './compare-latency-baseline.mjs';

test('uses nearest-rank percentiles and does not turn missing/invalid samples into zero', () => {
  assert.equal(summarize([]), null);
  assert.equal(summarize([undefined, null, -1, NaN, Infinity]), null);
  assert.deepEqual(summarize([199, 10, 15, 12, 13]), { samples: 5, min: 10, p50: 13, p95: 199, max: 199 });
});

test('keeps probe, live wire, app and ACK measurements distinct; excludes maxima from percentiles', () => {
  const result = summarizeReport({ version: '2.2.1', kind: 'client', route: 'direct', sampleCount: 200,
    routeHealth: { probe: { medianMs: 10, samples: 5 } }, maxima: { 'SNCONTROL_TIMING.wireEstimateMs': 999 },
    samples: [{ source: 'SNCONTROL_TIMING', values: { wireEstimateMs: 199, appRttMs: 205 } }] });
  assert.equal(result.probeMedianMs, 10);
  assert.equal(result.measurements.wireRttMs.p50, 199);
  assert.equal(result.measurements.controlAppRttMs.p50, 205);
  assert.equal(result.measurements.echoResidualMs, null);
  assert.equal(result.measurements.inputAckRttMs, null);
  assert.equal(result.sampleWindowTruncated, true);
  assert.equal(result.wireProtocol, null);
});

test('exports only allowlisted metadata and numeric metrics', () => {
  const result = summarizeReport({ version: '2.2.7', sessionId: 'private-session', machine: 'private-machine',
    token: 'private-token', route: 'private-route', wireProtocol: 'snv2',
    samples: [{ source: 'SNINPUT_ACKED', values: { rttMs: 12, token: 777 } }],
    routeHealth: { attemptId: 'private-attempt', probe: { medianMs: 10, samples: 5 }, endpoint: 'private-endpoint' } });
  const text = JSON.stringify(result);
  assert.doesNotMatch(text, /private-|777/);
  assert.equal(result.measurements.inputAckRttMs.p50, 12);
  assert.equal(result.route, null);
  assert.equal(result.wireProtocol, 'snv2');
});
