import { writable } from 'svelte/store';

export interface NativeMetric {
  latestMs: number;
  p50Ms: number;
  p95Ms: number;
  maxMs: number;
  samples: number;
  updatedAtMs: number;
  source: string;
}
export interface NativePerformanceUpdate {
  reportId: string;
  kind: 'client' | 'host' | 'localServer';
  wireProtocol: 'legacy' | 'snv2';
  route: 'direct' | 'relay';
  relayTransport: 'udp' | 'wss' | null;
  startedAtMs: number;
  ended: boolean;
  performance: { metrics: Record<string, NativeMetric> };
  findings: string[];
}

const store = writable<Partial<Record<'client' | 'host', NativePerformanceUpdate>>>({});
export const nativePerformance = {
  subscribe: store.subscribe,
  update(next: NativePerformanceUpdate): void {
    const kind = next.kind;
    if (kind !== 'client' && kind !== 'host') return;
    store.update((current) => {
      const previous = current[kind];
      // A finishing previous process must not overwrite a newer launch. Host
      // and client are independent when sharing and viewing simultaneously.
      if (previous && previous.reportId !== next.reportId && previous.startedAtMs >= next.startedAtMs) return current;
      if (previous?.reportId === next.reportId && previous.ended && !next.ended) return current;
      return { ...current, [kind]: next };
    });
  },
  clear(): void { store.set({}); }
};

export function metricIsFresh(metric: NativeMetric | undefined, now: number): metric is NativeMetric {
  return !!metric && now >= metric.updatedAtMs && now - metric.updatedAtMs <= 5_000;
}
