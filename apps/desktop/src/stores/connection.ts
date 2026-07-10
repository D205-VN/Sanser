import { get, writable } from 'svelte/store';
import type { ApiClient } from '../lib/api';
import type { ConnectionSession, SessionMetrics } from '../lib/types';
import { diagnostics } from './diagnostics';

export interface ConnectionState {
  session: ConnectionSession | null;
  metrics: SessionMetrics | null;
  engineRunning: boolean;
  busy: boolean;
  error: string | null;
}

const initial: ConnectionState = {
  session: null,
  metrics: null,
  engineRunning: false,
  busy: false,
  error: null
};

function createConnectionStore() {
  const store = writable<ConnectionState>(initial);

  return {
    subscribe: store.subscribe,
    begin(connectionSession: ConnectionSession): void {
      store.set({ ...initial, session: connectionSession });
      diagnostics.add({
        level: 'info',
        category: 'session',
        message: 'Connection request created',
        details: { sessionId: connectionSession.id, networkMode: connectionSession.networkMode }
      });
    },
    async refresh(client: ApiClient): Promise<ConnectionSession | null> {
      const current = get(store).session;
      if (!current) return null;
      try {
        const refreshed = await client.getSession(current.id);
        store.update((state) => ({ ...state, session: refreshed, error: null }));
        return refreshed;
      } catch (error) {
        const message = error instanceof Error ? error.message : 'Unable to refresh the session';
        store.update((state) => ({ ...state, error: message }));
        return null;
      }
    },
    setEngineRunning(running: boolean): void {
      store.update((state) => ({ ...state, engineRunning: running, error: null }));
    },
    setError(message: string): void {
      store.update((state) => ({ ...state, busy: false, error: message }));
    },
    setBusy(busy: boolean): void {
      store.update((state) => ({ ...state, busy }));
    },
    setMetrics(metrics: SessionMetrics): void {
      store.update((state) => ({ ...state, metrics }));
    },
    clear(): void {
      store.set(initial);
    }
  };
}

export const connection = createConnectionStore();
