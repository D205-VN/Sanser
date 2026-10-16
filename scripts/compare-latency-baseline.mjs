#!/usr/bin/env node
// Read-only comparison of committed source and saved numeric connection reports.
// It neither launches engines nor changes the installed app, routes or settings.
import { execFileSync } from 'node:child_process';
import { readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const metrics = {
  wireRttMs: ['SNCONTROL_TIMING', 'wireEstimateMs'],
  echoResidualMs: ['SNU1_ECHO', 'residualMs'],
  controlAppRttMs: ['SNCONTROL_TIMING', 'appRttMs'],
  inputAckRttMs: ['SNINPUT_ACKED', 'rttMs'],
  macSendQueueMs: ['SNCONTROL_TIMING', 'macSendQueueMs'],
  hostReceiveQueueMs: ['SNCONTROL_TIMING', 'hostReceiveQueueMs'],
  hostControlWorkMs: ['SNCONTROL_TIMING', 'hostControlWorkMs'],
  macReceiveQueueMs: ['SNCONTROL_TIMING', 'macReceiveQueueMs']
};
const finite = (value) => typeof value === 'number' && Number.isFinite(value) && value >= 0;
const pick = (value, allowed) => allowed.includes(value) ? value : null;

export function summarize(values) {
  const sorted = values.filter(finite).sort((a, b) => a - b);
  if (!sorted.length) return null;
  const at = (fraction) => sorted[Math.max(0, Math.ceil(sorted.length * fraction) - 1)];
  return { samples: sorted.length, min: sorted[0], p50: at(.5), p95: at(.95), max: sorted.at(-1) };
}

export function summarizeReport(report) {
  // Explicit allowlist: never copy machine names, session IDs, raw log text,
  // endpoints, account details or credentials into the comparison artifact.
  const samples = Array.isArray(report.samples) ? report.samples : [];
  const measurements = Object.fromEntries(Object.entries(metrics).map(([name, [source, key]]) => [
    name, summarize(samples.filter((sample) => sample?.source === source).map((sample) => sample.values?.[key]))
  ]));
  return {
    version: typeof report.version === 'string' && /^\d+\.\d+\.\d+(?:-[\w.-]+)?$/.test(report.version) ? report.version : null,
    kind: pick(report.kind, ['client', 'host', 'local-server']),
    route: pick(report.route, ['direct', 'relay']),
    relayTransport: pick(report.relayTransport, ['udp', 'wss']),
    wireProtocol: pick(report.wireProtocol, ['legacy', 'snv2']),
    ultraLowLatency: typeof report.ultraLowLatency === 'boolean' ? report.ultraLowLatency : null,
    requestedFps: finite(report.requestedFps) ? report.requestedFps : null,
    requestedBitrateKbps: finite(report.requestedBitrateKbps) ? report.requestedBitrateKbps : null,
    startedAtMs: finite(report.startedAtMs) ? report.startedAtMs : null,
    probeMedianMs: finite(report.routeHealth?.probe?.medianMs) ? report.routeHealth.probe.medianMs : null,
    probeSamples: finite(report.routeHealth?.probe?.samples) ? report.routeHealth.probe.samples : null,
    storedSamples: samples.length,
    totalSampleCount: finite(report.sampleCount) ? report.sampleCount : null,
    sampleWindowTruncated: finite(report.sampleCount) && report.sampleCount > samples.length,
    measurements
  };
}

function git(...args) {
  return execFileSync('git', args, { cwd: root, encoding: 'utf8', maxBuffer: 4 * 1024 * 1024 }).trim();
}

export function auditSource(baseline, current) {
  const from = git('rev-parse', '--verify', `${baseline}^{commit}`);
  const to = git('rev-parse', '--verify', `${current}^{commit}`);
  const paths = ['native/client-macos/src', 'native/host-windows/src', 'native/common', 'native/protocol'];
  const tree = (revision) => new Map(git('ls-tree', '-r', revision, '--', ...paths).split('\n').filter(Boolean).map((line) => {
    const [metadata, path] = line.split('\t');
    return [path, metadata.split(' ')[2]];
  }).filter(([path]) => /\.(?:cpp|h|hpp|mm|m|metal)$/.test(path) && !/(?:_test\.|sanser_version\.h$)/.test(path)));
  const before = tree(from), after = tree(to);
  const files = [...new Set([...before.keys(), ...after.keys()])].sort().map((path) => ({
    path, baselineBlob: before.get(path) ?? null, currentBlob: after.get(path) ?? null,
    unchanged: before.has(path) && before.get(path) === after.get(path)
  }));
  return {
    baselineCommit: from, currentCommit: to,
    scope: 'Mac client / Windows host legacy native implementation; excludes version headers, build/link configuration, SNV2 backend and runtime route/settings',
    identical: files.length > 0 && files.every((file) => file.unchanged), files
  };
}

function readReports(path) {
  const paths = statSync(path).isDirectory()
    ? readdirSync(path).filter((name) => /^connection-.*\.json$/.test(name)).map((name) => join(path, name)) : [path];
  return paths.flatMap((file) => {
    if (statSync(file).size > 4 * 1024 * 1024) throw new Error('Report exceeds 4 MiB');
    const parsed = JSON.parse(readFileSync(file, 'utf8'));
    // Accept either an automatic report or an exported Diagnostics document.
    const reports = Array.isArray(parsed.connectionReports) ? parsed.connectionReports : [parsed];
    return reports.filter((report) => report && typeof report === 'object' && Array.isArray(report.samples)).map(summarizeReport);
  });
}

function main() {
  const args = process.argv.slice(2);
  const options = { baseline: 'v2.2.1', current: 'HEAD' };
  for (let i = 0; i < args.length; i += 2) {
    const key = args[i]?.replace(/^--/, '');
    if (!['baseline', 'current', 'reports', 'output'].includes(key) || !args[i + 1])
      throw new Error('Usage: node scripts/compare-latency-baseline.mjs [--baseline v2.2.1] [--current HEAD] [--reports <directory-or-json>] [--output <json>]');
    options[key] = args[i + 1];
  }
  const reports = options.reports ? readReports(resolve(options.reports)).sort((a, b) => (a.startedAtMs ?? 0) - (b.startedAtMs ?? 0)) : [];
  const result = {
    schemaVersion: 1, source: auditSource(options.baseline, options.current), reports,
    limitations: [
      'Source parity is not proof of identical binaries, compiler settings, routes or real-world latency.',
      'P50/P95 describe only retained samples; automatic reports keep at most 120 mixed-source samples, not the entire session.',
      'Probe RTT, wire estimate, echo and input ACK are separate measurements; do not substitute a probe for missing live RTT.',
      'Direct alone does not identify a LAN, STUN/public or VPN/overlay path. Older reports omit selected endpoints and protocol.',
      'A missing measurement is null, never zero. No causal performance regression or successful latency fix is inferred automatically.'
    ]
  };
  const json = JSON.stringify(result, null, 2) + '\n';
  if (options.output) writeFileSync(resolve(options.output), json, { mode: 0o600 });
  else process.stdout.write(json);
  console.error(`Legacy implementation: ${result.source.identical ? 'unchanged' : 'changed'} (${result.source.files.length} files); saved reports: ${reports.length}`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { main(); } catch (error) { console.error(error.message); process.exitCode = 1; }
}
