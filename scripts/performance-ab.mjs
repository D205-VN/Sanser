#!/usr/bin/env node
// Offline A/B of two explicitly selected reports. Does not launch devices,
// upload credentials, provision servers or certify perceived audio quality.
import { readFileSync, statSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { summarizeCase } from './performance-matrix.mjs';

export function compareRuns(a, b, conditions = {}) {
  const mismatches = [];
  for (const key of ['viewer', 'host', 'role', 'network', 'transport', 'protocol', 'mode']) {
    if (!a[key] || !b[key] || a[key] === 'unknown' || a[key] !== b[key]) mismatches.push(key);
  }
  if (a.annotationCheck !== 'local-os-matched-peer-unverified' || b.annotationCheck !== 'local-os-matched-peer-unverified') mismatches.push('recordedLocalOs');
  for (const key of ['width', 'height', 'fps', 'bitrateKbps', 'codec']) {
    if (a.settings[key] === null || b.settings[key] === null || a.settings[key] !== b.settings[key]) mismatches.push(`settings.${key}`);
  }
  if (a.settings.codec === 'auto' || b.settings.codec === 'auto') mismatches.push('settings.codec-unresolved');
  for (const key of ['sameDevices', 'sameNetwork', 'sameAudioRoute', 'sameWorkload']) {
    if (conditions[key] !== true) mismatches.push(`conditions.${key}`);
  }
  for (const key of ['participants', 'microphones']) {
    const n = conditions[key];
    if (!Number.isInteger(n) || n < (key === 'participants' ? 1 : 0) || n > 4) mismatches.push(`conditions.${key}`);
  }
  if (conditions.microphones > conditions.participants) mismatches.push('conditions.microphones');
  const eligible = mismatches.length === 0;
  const metrics = Object.fromEntries(Object.keys(a.metrics).map((key) => {
    const left = a.metrics[key], right = b.metrics[key];
    if (!eligible || !left || !right || left.scope !== right.scope || Math.min(left.samples, right.samples) < 10) return [key, null];
    return [key, { baselineP50Ms: left.p50Ms, candidateP50Ms: right.p50Ms, p50ChangeMs: right.p50Ms - left.p50Ms,
      baselineP95Ms: left.p95Ms, candidateP95Ms: right.p95Ms, p95ChangeMs: right.p95Ms - left.p95Ms,
      baselineSamples: left.samples, candidateSamples: right.samples, scope: left.scope }];
  }));
  const audio = Object.fromEntries(Object.keys(a.audioCounters).map((key) => {
    const left = a.audioCounters[key], right = b.audioCounters[key];
    if (!eligible || !left || !right || left.source !== right.source ||
      Math.min(left.observedDurationMs, right.observedDurationMs) < 30_000) return [key, null];
    return [key, { baselinePerSecond: left.eventsPerSecond, candidatePerSecond: right.eventsPerSecond,
      changePerSecond: right.eventsPerSecond - left.eventsPerSecond,
      baselineObservedMs: left.observedDurationMs, candidateObservedMs: right.observedDurationMs,
      baselineCounterResets: left.resets, candidateCounterResets: right.resets,
      scope: 'observed-counter-intervals' }];
  }));
  return { schemaVersion: 1, eligibility: eligible ? 'operator-matched-observations' : 'conditions-mismatch', mismatches,
    versions: { baseline: a.version, candidate: b.version }, metrics, audioCounters: audio,
    limitations: [
      'Conditions are operator assertions, not verified hardware, route-endpoint or acoustic measurements.',
      'A null comparison means missing, incompatible or insufficient observations; it never means zero errors.',
      'Counter rates require at least 30 observed seconds in each run and exclude first samples, resets and gaps over 5 seconds.',
      'Timing percentiles require at least 10 periodic observations, may describe interval averages, and are never pooled.',
      'Negative deltas are numeric changes only, not a pass verdict. FEC attempts are not confirmed recoveries; PLC is not a measured network-loss percentage.',
      'Listening tests and real same-device voice/music/video trials remain necessary. This does not run or compare a LiveKit implementation.'
    ] };
}
function readJson(path) {
  if (statSync(path).size > 4 * 1024 * 1024) throw new Error('Input exceeds 4 MiB');
  return JSON.parse(readFileSync(path, 'utf8'));
}
function main() {
  const args = process.argv.slice(2);
  if (args.length !== 4 || args[0] !== '--manifest' || args[2] !== '--output') throw new Error('Usage: node scripts/performance-ab.mjs --manifest ab.json --output result.json');
  const manifestPath = resolve(args[1]), output = resolve(args[3]), manifest = readJson(manifestPath), inputs = [];
  const rows = ['baseline', 'candidate'].map((arm) => {
    const entry = manifest[arm];
    if (!entry || typeof entry.file !== 'string' || !entry.file) throw new Error('Each arm requires an explicit report file');
    const path = resolve(dirname(manifestPath), entry.file); inputs.push(path);
    const json = readJson(path), reports = Array.isArray(json.connectionReports) ? json.connectionReports : [json];
    if (reports.length > 100) throw new Error('Input contains too many reports');
    const rows = summarizeCase(entry, reports);
    if (rows.length !== 1) throw new Error('Select exactly one report per arm using sessionId, role or a single-report export');
    return rows[0];
  });
  if (output === manifestPath || inputs.includes(output)) throw new Error('Output must not overwrite inputs');
  writeFileSync(output, `${JSON.stringify(compareRuns(...rows, manifest.conditions), null, 2)}\n`, { mode: 0o600, flag: 'wx' });
  console.log('A/B numeric observations exported; no automatic quality certification.');
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { main(); } catch (error) { console.error(error.message); process.exitCode = 1; }
}
