import { invoke } from '@tauri-apps/api/core';
import type { ApiClient } from './api';

/** What this network is likely to give a session, measured before connecting. */
export interface NetworkCheckResult {
  /** Median round trip to the Sanser server, or null when it is unreachable. */
  serverMs: number | null;
  /** UDP reaches the internet (a STUN server answered). */
  udp: boolean;
  /** The router opened a port by itself (UPnP, NAT-PMP or PCP). */
  portMapping: boolean;
  publicIpv6: boolean;
  verdict: 'offline' | 'relay' | 'direct';
}

interface GatherResult {
  stunSucceeded: boolean;
  portMappingSucceeded: boolean;
  ipv6Available: boolean;
}

async function serverRoundTrip(serverUrl: string): Promise<number | null> {
  const samples: number[] = [];
  for (let attempt = 0; attempt < 3; attempt += 1) {
    const started = performance.now();
    try {
      const response = await fetch(`${serverUrl.replace(/\/$/, '')}/api/v2/health`, { cache: 'no-store', credentials: 'omit' });
      if (response.ok) samples.push(performance.now() - started);
    } catch { /* counted as a missing sample */ }
  }
  if (samples.length === 0) return null;
  samples.sort((left, right) => left - right);
  return Math.round(samples[Math.floor(samples.length / 2)] ?? samples[0] ?? 0);
}

/** Classifies the measurements. Pure, so the rules are testable. */
export function networkVerdict(serverMs: number | null, udp: boolean): NetworkCheckResult['verdict'] {
  if (serverMs === null) return 'offline';
  return udp ? 'direct' : 'relay';
}

/**
 * Measures the server round trip and whether UDP leaves this network. Uses a
 * throwaway P2P attempt under its own random session id that is always
 * released. Callers still skip it during a session: it takes one of the few
 * concurrent negotiation slots and a moment of router traffic.
 */
export async function checkNetwork(client: ApiClient): Promise<NetworkCheckResult> {
  recent = null;
  const serverMs = await serverRoundTrip(client.serverUrl);
  let gathered: GatherResult = { stunSucceeded: false, portMappingSucceeded: false, ipv6Available: false };
  if (serverMs !== null) {
    let stunServers: string[] = [];
    try {
      const ice = await client.iceConfiguration();
      stunServers = [...new Set(ice.iceServers.flatMap((server) => server.urls).filter((url) => url.toLowerCase().startsWith('stun:')))].slice(0, 3);
    } catch { /* no STUN servers: UDP is reported as unavailable */ }
    if (stunServers.length > 0) {
      const attemptId = crypto.randomUUID();
      try {
        gathered = await invoke<GatherResult>('p2p_gather', { sessionId: crypto.randomUUID(), attemptId, stunServers });
      } catch { /* keep the unavailable defaults */ }
      finally { await invoke('p2p_stop', { attemptId }).catch(() => undefined); }
    }
  }
  return {
    serverMs,
    udp: gathered.stunSucceeded,
    portMapping: gathered.portMappingSucceeded,
    publicIpv6: gathered.ipv6Available,
    verdict: networkVerdict(serverMs, gathered.stunSucceeded)
  };
}

let recent: { at: number; result: NetworkCheckResult } | null = null;

/** The home page check: reuse a recent answer instead of probing on every visit. */
export async function recentNetworkCheck(client: ApiClient, maxAgeMs = 10 * 60_000): Promise<NetworkCheckResult> {
  if (recent && Date.now() - recent.at < maxAgeMs) return recent.result;
  const result = await checkNetwork(client);
  recent = { at: Date.now(), result };
  return result;
}
