import type { ApiClient } from './api';
import { serverRoundTrip } from './networkCheck';
import { machineInfo } from './platform';
import type { HostMeasurement } from './streamAdvisor';

/** How long each side moves data. Short, so a session barely notices. */
export const TRANSFER_SECONDS = 2.5;
/** Under the server's per-probe budget of 32 chunks each way. */
const MAX_CHUNKS = 30;
const CHUNK_BYTES = 256 * 1024;
/** The host polls every 2 s; give it time to measure and answer. */
const HOST_ANSWER_MS = 14_000;

/**
 * Moves chunks with two requests in flight until time or the chunk budget
 * runs out. Returns Mb/s, or null when nothing got through.
 */
export async function timedTransfer(move: () => Promise<number>, seconds = TRANSFER_SECONDS, parallel = 2, now = () => performance.now()): Promise<number | null> {
  const started = now();
  let bytes = 0;
  let chunks = 0;
  let failed = false;
  const worker = async () => {
    while (!failed && chunks < MAX_CHUNKS && now() - started < seconds * 1000) {
      chunks += 1;
      try { bytes += await move(); } catch { failed = true; }
    }
  };
  await Promise.all(Array.from({ length: parallel }, worker));
  const elapsed = (now() - started) / 1000;
  if (bytes === 0 || elapsed <= 0) return null;
  return Math.round((bytes * 8) / elapsed / 1e5) / 10;
}

function noise(size: number): Blob {
  const bytes = new Uint8Array(size);
  // getRandomValues fills at most 64 KiB at a time.
  for (let offset = 0; offset < size; offset += 65_536) crypto.getRandomValues(bytes.subarray(offset, Math.min(size, offset + 65_536)));
  return new Blob([bytes], { type: 'application/octet-stream' });
}

/**
 * Host side: reports this computer and, unless a session is running, its
 * round trip and upload speed to the server.
 */
export async function answerProbe(client: ApiClient, probeId: string, busy: boolean): Promise<void> {
  const info = await machineInfo().catch(() => ({}));
  const ratio = window.devicePixelRatio || 1;
  const measurement: HostMeasurement = {
    cpuCores: navigator.hardwareConcurrency || undefined,
    displayWidth: Math.round(window.screen.width * ratio),
    displayHeight: Math.round(window.screen.height * ratio),
    ...info,
    busy
  };
  if (!busy) {
    measurement.serverMs = await serverRoundTrip(client.serverUrl);
    const chunk = noise(CHUNK_BYTES);
    measurement.uploadMbps = await timedTransfer(() => client.uploadProbeChunk(probeId, chunk));
  }
  await client.finishAdvisorProbe(probeId, measurement);
}

export interface LiveMeasurement {
  /** The host answered in time; when false only this side was measured. */
  hostAnswered: boolean;
  host?: HostMeasurement;
  serverMs: number | null;
  downloadMbps: number | null;
}

const pause = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));

/**
 * Viewer side: opens a probe for the host, measures this computer's round
 * trip and download speed meanwhile, then waits for the host's answer.
 */
export async function measureLive(client: ApiClient, hostDeviceId: string): Promise<LiveMeasurement> {
  const started = Date.now();
  const { probeId } = await client.createAdvisorProbe(hostDeviceId);
  const serverMs = await serverRoundTrip(client.serverUrl);
  const downloadMbps = await timedTransfer(() => client.downloadProbeChunk(probeId));
  while (Date.now() - started < HOST_ANSWER_MS) {
    const state = await client.advisorProbe(probeId).catch(() => null);
    if (state?.hostAnswered) return { hostAnswered: true, host: state.host, serverMs, downloadMbps };
    if (!state) break;
    await pause(500);
  }
  return { hostAnswered: false, serverMs, downloadMbps };
}
