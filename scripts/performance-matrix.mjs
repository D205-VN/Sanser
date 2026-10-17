#!/usr/bin/env node
// Read explicitly supplied numeric exports; never opens engines or changes routes.
import { readFileSync, statSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { summarize } from './compare-latency-baseline.mjs';

const platforms = ['macos', 'windows'];
const transports = ['direct-lan', 'direct-wan', 'relay-udp', 'relay-wss'];
const stages = {
  controlAppRtt: [['SNCONTROL_TIMING', 'appRttMs'], ['SNV2_CONTROL_TIMING', 'appRttMs'], ['SNINPUT_RTT', 'rttMs']],
  wireRttEstimate: [['SNCONTROL_TIMING', 'wireEstimateMs']],
  udpEchoRtt: [['SNU1_ECHO', 'rttMs']],
  inputAckRtt: [['SNINPUT_ACK', 'rttMs'], ['SNINPUT_ACKED', 'rttMs'], ['SNV2_INPUT_TIMING', 'avgRttMs']],
  inputSendQueue: [['SNV2_INPUT_TIMING', 'inputSendQueueAvgMs']],
  hostInputQueue: [['SNV2_INPUT_HOST_TIMING', 'hostQueueAvgMs'], ['SNV2_INPUT_TIMING', 'hostQueueAvgMs']],
  hostInput: [['SNINPUT_ACK', 'hostProcessMs'], ['SNV2_INPUT_HOST_TIMING', 'hostAvgWorkMs'], ['SNV2_INPUT_TIMING', 'hostAvgWorkMs']],
  captureWork: [['SNV1_HOST_TIMING', 'captureAvgMs'], ['SNV1_STAGE_PROFILE', 'captureAvgMs']],
  captureWait: [['SNV1_HOST_TIMING', 'captureWaitAvgMs']],
  encodeWork: [['SNV1_HOST_TIMING', 'encodeAvgMs'], ['SNV1_STAGE_PROFILE', 'encodeAvgMs']],
  encodeAndPublish: [['SNV2_HOST_TIMING', 'encodeAndPublishAvgMs']],
  sendTotal: [['SNV1_HOST_TIMING', 'sendAvgMs']],
  pacingWait: [['SNV1_HOST_TIMING', 'udpPacedAvgMs']],
  socketSend: [['SNV1_HOST_TIMING', 'udpSocketAvgMs']],
  arrivalJitter: [['SNV1_CLIENT_STATS', 'jitterMs']],
  decodeWork: [['SNV2_VIDEO_HEALTH', 'decodeWorkAvgMs'], ['SNV1_CLIENT_STATS', 'decodeAvgMs']],
  decodeQueue: [['SNV2_VIDEO_HEALTH', 'decodeQueueAvgMs']],
  frameQueue: [['SNV1_RENDER_STATS', 'avgRenderAgeMs']],
  renderGpu: [['SNV1_RENDER_STATS', 'renderGpuMs']],
  paintWork: [['SNV2_VIDEO_HEALTH', 'paintAvgMs']],
  decodedToPaint: [['SNV2_VIDEO_HEALTH', 'decodedToPaintAvgMs']]
};
const finite = (value) => typeof value === 'number' && Number.isFinite(value) && value >= 0 && value <= 1e12;
const bounded = (value, maximum) => Number.isInteger(value) && value > 0 && value <= maximum ? value : null;
const pick = (value, values) => values.includes(value) ? value : null;
const platform = (value) => ({ darwin: 'macos', macos: 'macos', windows: 'windows', win32: 'windows' })[value] ?? null;

function metric(report, name, aliases) {
  const saved = report.performance?.metrics?.[name];
  if (saved && [saved.p50Ms, saved.p95Ms, saved.maxMs].every(finite) && bounded(saved.samples, 120) &&
      saved.p50Ms <= saved.p95Ms && saved.p95Ms <= saved.maxMs) {
    return { samples: saved.samples, p50Ms: saved.p50Ms, p95Ms: saved.p95Ms, maxMs: saved.maxMs, scope: 'rolling-periodic-observations' };
  }
  const samples = Array.isArray(report.samples) ? report.samples : [];
  for (const [source, key] of aliases) {
    const values = samples.filter((sample) => sample?.source === source).map((sample) => sample.values?.[key]).filter(finite).slice(-120);
    const result = summarize(values);
    if (result) return { samples: result.samples, p50Ms: result.p50, p95Ms: result.p95, maxMs: result.max, scope: 'retained-periodic-samples' };
  }
  return null;
}

export function summarizeCase(entry, reports) {
  if (!platforms.includes(entry.viewer) || !platforms.includes(entry.host) || !['lan', 'wan', 'unknown'].includes(entry.network) ||
      (entry.role !== undefined && !['client', 'host'].includes(entry.role))) throw new Error('Each case requires valid viewer, host, network and optional role');
  const role = entry.role ?? 'client';
  return reports.filter((report) => report?.kind === role && (!entry.sessionId || report.sessionId === entry.sessionId)).map((report) => {
    const local = platform(report.platform);
    if (local && local !== (role === 'client' ? entry.viewer : entry.host)) throw new Error('Case annotation conflicts with the recorded local OS');
    const route = pick(report.route, ['direct', 'relay']);
    const relay = pick(report.relayTransport, ['udp', 'wss']);
    const transport = route === 'direct' ? entry.network === 'unknown' ? null : `direct-${entry.network}` : route === 'relay' && relay ? `relay-${relay}` : null;
    return {
      viewer: entry.viewer, host: entry.host, role, network: entry.network, transport,
      protocol: pick(report.wireProtocol, ['legacy', 'snv2']),
      mode: typeof report.ultraLowLatency === 'boolean' ? report.ultraLowLatency ? 'ultra' : 'standard' : null,
      version: typeof report.version === 'string' && /^\d+\.\d+\.\d+(?:-[\w.-]+)?$/.test(report.version) ? report.version : null,
      annotationCheck: local ? 'local-os-matched-peer-unverified' : 'os-unverified',
      settings: { width: bounded(report.requestedWidth, 16384), height: bounded(report.requestedHeight, 16384),
        fps: bounded(report.requestedFps, 1000), bitrateKbps: bounded(report.requestedBitrateKbps, 1_000_000), codec: pick(report.requestedCodec, ['auto', 'h264', 'hevc']) },
      metrics: Object.fromEntries(Object.entries(stages).map(([name, aliases]) => [name, metric(report, name, aliases)]))
    };
  });
}

export function buildMatrix(rows) {
  const coverage = [];
  for (const viewer of platforms) for (const host of platforms) for (const transport of transports) for (const mode of ['standard', 'ultra']) {
    const matches = rows.filter((row) => row.viewer === viewer && row.host === host && row.transport === transport && row.mode === mode && Object.values(row.metrics).some(Boolean));
    coverage.push({ viewer, host, transport, mode, reports: matches.length, status: matches.length ? 'observations-only' : 'not-observed' });
  }
  return { schemaVersion: 1, coverage, rows, limitations: [
    'Case direction and LAN/WAN are operator annotations; only the recorded local OS is checked. Peer OS/version and route endpoints are not verified.',
    'Observations are not passing benchmarks. Missing metrics stay null and missing cases stay not-observed.',
    'P50/P95 summarize bounded periodic observations; some are interval averages. They are not per-frame or screen-to-screen latency.',
    'Reports remain separate. Percentiles from different runs, settings, routes, protocols and roles are never pooled.',
    'Wire RTT, application RTT, receipt ACK, OS input work and combined encode/publish stages have distinct meanings.'
  ] };
}

function readJson(path) {
  if (statSync(path).size > 4 * 1024 * 1024) throw new Error('Input JSON exceeds 4 MiB');
  return JSON.parse(readFileSync(path, 'utf8'));
}
function main() {
  const args = process.argv.slice(2);
  if (args.length !== 4 || args[0] !== '--manifest' || args[2] !== '--output') throw new Error('Usage: node scripts/performance-matrix.mjs --manifest cases.json --output matrix.json');
  const path = resolve(args[1]), output = resolve(args[3]);
  const manifest = readJson(path);
  if (!Array.isArray(manifest.cases) || manifest.cases.length > 100) throw new Error('Manifest cases must be an array of at most 100 entries');
  const inputs = manifest.cases.map((entry) => {
    if (!entry || typeof entry.file !== 'string' || !entry.file) throw new Error('Each case requires an explicit export file');
    return { entry, path: resolve(dirname(path), entry.file) };
  });
  if (output === path || inputs.some((input) => input.path === output)) throw new Error('Output must not overwrite the manifest or an input export');
  const rows = inputs.flatMap(({entry, path: input}) => {
    const json = readJson(input);
    const reports = Array.isArray(json.connectionReports) ? json.connectionReports : [json];
    if (reports.length > 100) throw new Error('An export contains more than 100 reports');
    return summarizeCase(entry, reports);
  });
  writeFileSync(output, `${JSON.stringify(buildMatrix(rows), null, 2)}\n`, { mode: 0o600, flag: 'wx' });
  console.log(`Performance matrix: ${rows.length} separate numeric reports; no automatic performance certification.`);
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { main(); } catch (error) { console.error(error.message); process.exitCode = 1; }
}
