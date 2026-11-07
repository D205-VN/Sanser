import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { summarizeCase } from './performance-matrix.mjs';
import { summarizeAudio } from './audio-health.mjs';
import { compareRuns } from './performance-ab.mjs';

const entry = { viewer: 'windows', host: 'macos', network: 'lan' };
const conditions = { sameDevices: true, sameNetwork: true, sameAudioRoute: true, sameWorkload: true, participants: 3, microphones: 2 };
const counter = { latest: 140, delta: 0, observedIncrease: 40, observedDurationMs: 40_000, samples: 41, resets: 1, source: 'native-voice' };
const metric = { samples: 40, p50Ms: 10, p95Ms: 30, maxMs: 35 };
const report = { kind: 'client', platform: 'windows', route: 'direct', wireProtocol: 'snv2', ultraLowLatency: true,
  version: '2.3.3', requestedWidth: 1920, requestedHeight: 1080, requestedFps: 60, requestedBitrateKbps: 20000, requestedCodec: 'h264',
  performance: { metrics: { controlAppRtt: metric }, audio: { counters: { voicePcmUnderflows: counter } } } };
const row = (extra = {}) => summarizeCase(entry, [{ ...report, ...extra }])[0];

test('compares separate observations without pooling percentiles or calling counters milliseconds', () => {
  const b = row({ performance: { metrics: { controlAppRtt: { ...metric, p95Ms: 20 } },
    audio: { counters: { voicePcmUnderflows: { ...counter, observedIncrease: 20 } } } } });
  const result = compareRuns(row(), b, conditions);
  assert.equal(result.eligibility, 'operator-matched-observations');
  assert.equal(result.metrics.controlAppRtt.p95ChangeMs, -10);
  assert.equal(result.audioCounters.voicePcmUnderflows.changePerSecond, -0.5);
  assert.equal(result.metrics.wireRttEstimate, null);
  assert.equal(result.audioCounters.systemPcmUnderflows, null);
  assert.match(result.limitations.join(' '), /not a pass verdict/);
});
test('refuses mismatched transports, settings, protocols and unconfirmed physical conditions', () => {
  const result = compareRuns(row(), row({ route: 'relay', relayTransport: 'wss', requestedFps: 30, wireProtocol: 'legacy' }), {});
  assert.equal(result.eligibility, 'conditions-mismatch');
  assert.ok(result.mismatches.includes('transport') && result.mismatches.includes('protocol') && result.mismatches.includes('settings.fps'));
  assert.ok(Object.values(result.metrics).every((v) => v === null));
  assert.ok(Object.values(result.audioCounters).every((v) => v === null));
  assert.equal(compareRuns(row(), row(), { ...conditions, participants: 1, microphones: 2 }).eligibility, 'conditions-mismatch');
  assert.ok(compareRuns(row(), row({ requestedCodec: 'auto' }), conditions).mismatches.includes('settings.codec-unresolved'));
  assert.ok(compareRuns(row(), row({ platform: undefined }), conditions).mismatches.includes('recordedLocalOs'));
});
test('requires sufficient comparable coverage, keeping missing and zero distinct', () => {
  const b = row({ performance: { metrics: { controlAppRtt: { ...metric, samples: 1 } },
    audio: { counters: { voicePcmUnderflows: { ...counter, observedDurationMs: 2000 } } } } });
  const result = compareRuns(row(), b, conditions);
  assert.equal(result.metrics.controlAppRtt, null);
  assert.equal(result.audioCounters.voicePcmUnderflows, null);
  const zero = row({ performance: { metrics: {}, audio: { counters: { voicePcmUnderflows: { ...counter, observedIncrease: 0 } } } } });
  assert.equal(compareRuns(zero, zero, conditions).audioCounters.voicePcmUnderflows.changePerSecond, 0);
});
test('allowlists numeric counters, rejects malformed coverage and strips identities', () => {
  const result = summarizeAudio({ performance: { audio: { counters: {
    voicePcmUnderflows: { ...counter, token: 'private', device: 'private' },
    voiceCaptureQueueDrops: { ...counter, observedDurationMs: 0 },
    voiceDecodedPackets: { ...counter, latest: NaN },
    systemPcmUnderflows: { ...counter, samples: 2 },
    secretCounter: counter
  } } } });
  assert.equal(result.voicePcmUnderflows.eventsPerSecond, 1);
  assert.equal(result.voiceCaptureQueueDrops, null);
  assert.equal(result.voiceDecodedPackets, null);
  assert.equal(result.systemPcmUnderflows, null);
  assert.doesNotMatch(JSON.stringify(result), /private|secret|token|device/);
});
test('old exports have unknown audio evidence and lifetime max is not a window percentile', () => {
  const old = row({ performance: undefined, samples: [{ source: 'SNVOICE_HEALTH', values: { voiceWorkerGapMaxMs: 999, voiceWorkerGapWindowMaxMs: 7 } }] });
  assert.ok(Object.values(old.audioCounters).every((v) => v === null));
  assert.equal(old.metrics.voiceWorkerGap.p50Ms, 7);
});
test('CLI selects exactly one explicit report per arm and never overwrites inputs/output', () => {
  const dir = mkdtempSync(join(tmpdir(), 'sanser-voice-ab-'));
  try {
    const input = join(dir, 'reports.json'), manifest = join(dir, 'ab.json'), output = join(dir, 'result.json');
    writeFileSync(input, JSON.stringify({ connectionReports: [{ ...report, sessionId: 'a' }, { ...report, sessionId: 'b' }] }));
    const spec = { baseline: { ...entry, file: 'reports.json', sessionId: 'a' }, candidate: { ...entry, file: 'reports.json', sessionId: 'b' }, conditions };
    writeFileSync(manifest, JSON.stringify(spec));
    const run = (target) => spawnSync(process.execPath, ['scripts/performance-ab.mjs', '--manifest', manifest, '--output', target], { encoding: 'utf8' });
    assert.equal(run(output).status, 0);
    assert.equal(JSON.parse(readFileSync(output)).eligibility, 'operator-matched-observations');
    assert.notEqual(run(output).status, 0);
    assert.notEqual(run(input).status, 0);
    delete spec.baseline.sessionId;writeFileSync(manifest, JSON.stringify(spec));
    assert.notEqual(run(join(dir, 'ambiguous.json')).status, 0);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});
