import { writable } from 'svelte/store';

export interface FecStats {
  negotiated: boolean;
  overheadPercent: number;
  videoBytes: number;
  parityBytes: number;
  parityPackets: number;
  recovered: number;
  expired: number;
  invalid: number;
  budgetSkipped: number;
  receiverLate: number;
  congestionPauses: number;
}
export interface FecHealthUpdate {
  kind: 'host' | 'client';
  sessionId: string;
  startedAtMs: number;
  transport: 'direct' | 'udp-relay';
  stats: FecStats;
}
export type FecHealth = FecHealthUpdate & { receivedAtMs: number };

function createFecHealth() {
  const store = writable<FecHealth[]>([]);
  return {
    subscribe: store.subscribe,
    update(update: FecHealthUpdate): void {
      store.update((entries) => {
        const current = entries.find((entry) => entry.sessionId === update.sessionId && entry.kind === update.kind);
        if (current && current.startedAtMs > update.startedAtMs) return entries;
        return [
        { ...update, receivedAtMs: Date.now() },
        ...entries.filter((entry) => entry.sessionId !== update.sessionId || entry.kind !== update.kind)
        ].slice(0, 8);
      });
    },
    clear(): void { store.set([]); }
  };
}
export const fecHealth = createFecHealth();
