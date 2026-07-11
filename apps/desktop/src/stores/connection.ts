import { get, writable } from 'svelte/store';
import type { ApiClient } from '../lib/api';
import type { ConnectionSession, NativeSessionCredentials, SessionMetrics } from '../lib/types';
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
        const credentialActive =
          current.credentialExpiresAt !== undefined && current.credentialExpiresAt > Math.floor(Date.now() / 1_000);
        const merged = credentialActive
          ? {
              ...refreshed,
              address: current.address,
              port: current.port,
              sessionToken: current.sessionToken,
              credentialExpiresAt: current.credentialExpiresAt
            }
          : refreshed;
        store.update((state) => ({ ...state, session: merged, error: null }));
        return merged;
      } catch (error) {
        const message = error instanceof Error ? error.message : 'Unable to refresh the session';
        store.update((state) => ({ ...state, error: message }));
        return null;
      }
    },
    setEngineRunning(running: boolean): void {
      store.update((state) => ({ ...state, engineRunning: running, error: null }));
    },
    authorizeNative(credentials: NativeSessionCredentials): void {
      store.update((state) => {
        if (!state.session || state.session.id !== credentials.sessionId) return state;
        return {
          ...state,
          session: {
            ...state.session,
            address: credentials.peerRouteAddress,
            port: credentials.basePort,
            sessionToken: credentials.sessionToken,
            credentialExpiresAt: credentials.expiresAt
          },
          error: null
        };
      });
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
