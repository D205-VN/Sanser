import { get, writable } from 'svelte/store';

export type DiagnosticLevel = 'info' | 'warn' | 'error';
export type DiagnosticValue = string | number | boolean | null;

export interface DiagnosticEvent {
  id: string;
  timestamp: string;
  level: DiagnosticLevel;
  category: 'app' | 'auth' | 'device' | 'network' | 'engine' | 'session';
  message: string;
  details?: Record<string, DiagnosticValue>;
}

export interface ProbeStatistics {
  samples: number;
  medianMs: number;
  minMs: number;
  maxMs: number;
  jitterMs: number;
  lossPercent: number;
  scoreMs: number;
}

export interface DirectSelectionDiagnostics {
  pairId: string;
  localCandidateType: string;
  localCandidateEndpoint: string;
  localCandidateInterface?: { name: string; kind: string } | null;
  remoteCandidateType: string;
  remoteCandidateEndpoint: string;
  verifiedRemoteEndpoint: string;
  probe: ProbeStatistics | null;
  reason: string;
}

export interface DirectRouteHealth {
  attemptId: string;
  probe: ProbeStatistics;
  liveWireRttMs: number | null;
  echoRttMs?: number | null;
  echoHostHoldMs?: number | null;
  echoUpdatedAtMs?: number | null;
  status: string;
  consecutiveElevated: number;
}

export interface NetworkDiagnosticState {
  checkedAt: string | null;
  localPort: number | null;
  fixedHostPort: boolean;
  stunSucceeded: boolean | null;
  portMappingSucceeded: boolean | null;
  ipv6Available: boolean | null;
  publicEndpoint: string | null;
  gatheringDurationMs: number | null;
  candidateCount: number;
  selection: (DirectSelectionDiagnostics & { attemptId: string }) | null;
  routeHealth: DirectRouteHealth | null;
}

interface GatherTelemetry {
  candidates: Record<string, unknown>[];
  localPort: number;
  stunSucceeded: boolean;
  portMappingSucceeded: boolean;
  ipv6Available: boolean;
  publicEndpoint: string | null;
  durationMs: number;
}

const EMPTY_NETWORK_DIAGNOSTICS: NetworkDiagnosticState = {
  checkedAt: null,
  localPort: null,
  fixedHostPort: false,
  stunSucceeded: null,
  portMappingSucceeded: null,
  ipv6Available: null,
  publicEndpoint: null,
  gatheringDurationMs: null,
  candidateCount: 0,
  selection: null,
  routeHealth: null
};

function createNetworkDiagnosticStore() {
  const store = writable<NetworkDiagnosticState>({ ...EMPTY_NETWORK_DIAGNOSTICS });
  return {
    subscribe: store.subscribe,
    updateGather(result: GatherTelemetry, fixedHostPort: boolean): void {
      store.set({
        checkedAt: new Date().toISOString(),
        localPort: result.localPort,
        fixedHostPort,
        stunSucceeded: result.stunSucceeded,
        portMappingSucceeded: result.portMappingSucceeded,
        ipv6Available: result.ipv6Available,
        publicEndpoint: result.publicEndpoint,
        gatheringDurationMs: result.durationMs,
        candidateCount: result.candidates.length,
        selection: null,
        routeHealth: null
      });
    },
    selectRoute(selection: DirectSelectionDiagnostics, attemptId: string): void {
      store.update((state) => ({ ...state, selection: { ...selection, attemptId }, routeHealth: null }));
    },
    updateRouteHealth(health: DirectRouteHealth): void {
      store.update((state) => {
        if (state.selection?.attemptId !== health.attemptId) return state;
        if (health.status === 'elevated-after-media' && state.routeHealth?.status !== health.status) {
          diagnostics.add({ level: 'warn', category: 'network',
            message: 'Direct RTT rose after media started. Check the media path, OS/network queues and router; no automatic route migration was performed.',
            details: { probeMedianMs: health.probe.medianMs, liveWireRttMs: health.liveWireRttMs }
          });
        }
        return { ...state, routeHealth: health };
      });
    },
    clear(): void {
      store.set({ ...EMPTY_NETWORK_DIAGNOSTICS });
    }
  };
}

export const networkDiagnostics = createNetworkDiagnosticStore();

const MAX_EVENTS = 300;
const SENSITIVE_KEY = /(password|token|credential|secret|authorization|private.?key|database.?url)/i;
const BEARER_VALUE = /bearer\s+\S+/gi;

function sanitizeValue(value: DiagnosticValue): DiagnosticValue {
  return typeof value === 'string' ? value.replace(BEARER_VALUE, 'Bearer [REDACTED]').slice(0, 1_024) : value;
}

export function sanitizeDetails(details?: Record<string, DiagnosticValue>): Record<string, DiagnosticValue> | undefined {
  if (!details) return undefined;
  return Object.fromEntries(
    Object.entries(details).map(([key, value]) => [key, SENSITIVE_KEY.test(key) ? '[REDACTED]' : sanitizeValue(value)])
  );
}

function createDiagnosticStore() {
  const store = writable<DiagnosticEvent[]>([]);
  let enabled = true;

  return {
    subscribe: store.subscribe,
    setEnabled(value: boolean): void {
      enabled = value;
    },
    add(event: Omit<DiagnosticEvent, 'id' | 'timestamp'>): void {
      if (!enabled && event.level !== 'error') return;
      const next: DiagnosticEvent = {
        ...event,
        id: crypto.randomUUID(),
        timestamp: new Date().toISOString(),
        message: event.message.slice(0, 2_048),
        details: sanitizeDetails(event.details)
      };
      store.update((events) => [...events, next].slice(-MAX_EVENTS));
    },
    clear(): void {
      store.set([]);
      networkDiagnostics.clear();
    },
    exportJson(): string {
      return JSON.stringify({
        schemaVersion: 1,
        exportedAt: new Date().toISOString(),
        network: get(networkDiagnostics),
        events: get(store)
      }, null, 2);
    }
  };
}

export const diagnostics = createDiagnosticStore();
