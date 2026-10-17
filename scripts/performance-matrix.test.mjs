import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { buildMatrix, summarizeCase } from './performance-matrix.mjs';

const entry = { viewer: 'windows', host: 'macos', network: 'wan' };
const report = { kind: 'client', platform: 'windows', route: 'direct', wireProtocol: 'snv2', ultraLowLatency: true, version: '2.2.13',
  samples: [{ source: 'SNV2_CONTROL_TIMING', values: { appRttMs: 12 } }, { source: 'SNV2_INPUT_HOST_TIMING', values: { hostQueueAvgMs: 30, hostAvgWorkMs: 6 } }] };

test('all four directions have explicit missing cases rather than fabricated passing scores', () => {
  const result = buildMatrix([]);
  assert.equal(result.coverage.length, 32);
  assert.ok(result.coverage.every((row) => row.status === 'not-observed'));
  assert.equal(new Set(result.coverage.map((row) => `${row.viewer}-${row.host}`)).size, 4);
});
test('keeps app RTT, wire RTT, input queue/work and missing stages distinct', () => {
  const [row] = summarizeCase(entry, [report]);
  assert.equal(row.metrics.controlAppRtt.p50Ms, 12);
  assert.equal(row.metrics.wireRttEstimate, null);
  assert.equal(row.metrics.inputAckRtt, null);
  assert.equal(row.metrics.captureWork, null);
  assert.equal(row.metrics.hostInputQueue.p50Ms, 30);
  assert.equal(row.metrics.hostInput.p50Ms, 6);
  assert.equal(row.transport, 'direct-wan');
  assert.equal(buildMatrix([row]).coverage.filter((item) => item.status === 'observations-only').length, 1);
});
test('prefers bounded schema-2 observations without pooling per-report percentiles', () => {
  const saved = { ...report, performance: { metrics: { controlAppRtt: { samples: 120, p50Ms: 15, p95Ms: 200, maxMs: 250 } } } };
  const rows = summarizeCase(entry, [saved, report]);
  assert.equal(rows[0].metrics.controlAppRtt.p95Ms, 200);
  assert.equal(rows[0].metrics.controlAppRtt.scope, 'rolling-periodic-observations');
  assert.equal(rows[1].metrics.controlAppRtt.p95Ms, 12);
  assert.equal(buildMatrix(rows).rows.length, 2);
});
test('supports retained Legacy observations without substituting maxima or probes', () => {
  const [row] = summarizeCase({ viewer: 'macos', host: 'windows', network: 'unknown' }, [{ ...report, platform: 'darwin', wireProtocol: 'legacy',
    maxima: { wireRttEstimate: 999 }, routeHealth: { probe: { medianMs: 1 } },
    samples: [{ source: 'SNCONTROL_TIMING', values: { wireEstimateMs: 199, appRttMs: 205 } }, { source: 'SNINPUT_ACKED', values: { rttMs: 210 } }] }]);
  assert.equal(row.transport, null);
  assert.equal(row.metrics.wireRttEstimate.p50Ms, 199);
  assert.equal(row.metrics.inputAckRtt.p50Ms, 210);
});
test('checks local OS and permits host reports only with an explicit role', () => {
  assert.throws(() => summarizeCase({ ...entry, viewer: 'macos' }, [report]), /recorded local OS/);
  assert.throws(() => summarizeCase({ ...entry, network: 'guessed' }, [report]), /valid viewer/);
  assert.deepEqual(summarizeCase(entry, [{ ...report, kind: 'host' }]), []);
  assert.equal(summarizeCase({ ...entry, role: 'host' }, [{ ...report, kind: 'host', platform: 'darwin' }]).length, 1);
});
test('does not export names, endpoints, session IDs, tokens, arbitrary fields or invalid numbers', () => {
  const [row] = summarizeCase(entry, [{ ...report, machine: 'private-machine', sessionId: 'private-session', endpoint: 'private-endpoint',
    token: 'private-token', performance: { metrics: { secret: { p50Ms: 123 }, controlAppRtt: { samples: 1, p50Ms: NaN, p95Ms: 0, maxMs: Infinity } } },
    samples: [...report.samples, { source: 'private-source', values: { secret: 999 } }] }]);
  assert.doesNotMatch(JSON.stringify(row), /private-|secret|999/);
  assert.equal(row.metrics.controlAppRtt.p50Ms, 12);
});
test('CLI reads only explicit exports and refuses to overwrite its inputs', () => {
  const directory = mkdtempSync(join(tmpdir(), 'sanser-matrix-'));
  try {
    const input = join(directory, 'export.json'), manifest = join(directory, 'cases.json'), output = join(directory, 'matrix.json');
    writeFileSync(input, JSON.stringify({ connectionReports: [report] }));
    writeFileSync(manifest, JSON.stringify({ cases: [{ ...entry, file: 'export.json' }] }));
    const run = (target) => spawnSync(process.execPath, ['scripts/performance-matrix.mjs', '--manifest', manifest, '--output', target], { encoding: 'utf8' });
    assert.equal(run(output).status, 0);
    assert.equal(JSON.parse(readFileSync(output)).coverage.length, 32);
    assert.notEqual(run(output).status, 0);
    assert.notEqual(run(input).status, 0);
    assert.equal(JSON.parse(readFileSync(input)).connectionReports.length, 1);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
