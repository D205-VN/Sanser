import { get, writable } from 'svelte/store';
import type { ApiClient } from '../lib/api';
import type { ConnectionSession, NativeSessionCredentials, SessionMetrics } from '../lib/types';
import { diagnostics } from './diagnostics';

export interface ConnectionState {
  session: ConnectionSession | null;
  metrics: SessionMetrics | null;
  engineRunning: boolean;
  preparing: boolean;
  busy: boolean;
  error: string | null;
  nativeStartupRetries: number;
  nativeRetryBlocked: boolean;
}

const initial: ConnectionState = {
  session: null,
  metrics: null,
  engineRunning: false,
  preparing: false,
  busy: false,
  error: null,
  nativeStartupRetries: 0,
  nativeRetryBlocked: false
};

function createConnectionStore() {
  const store = writable<ConnectionState>(initial);
  let refreshError: string | null = null;

  return {
    subscribe: store.subscribe,
    begin(connectionSession: ConnectionSession): void {
      refreshError = null;
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
        const latest = get(store).session;
        if (!latest || latest.id !== current.id) return null;
        const credentialActive =
          latest.credentialExpiresAt !== undefined && latest.credentialExpiresAt > Math.floor(Date.now() / 1_000);
        const merged = credentialActive
          ? {
              ...refreshed,
              address: latest.address,
              port: latest.port,
              sessionToken: latest.sessionToken,
              wireProtocol: latest.wireProtocol,
              credentialExpiresAt: latest.credentialExpiresAt
            }
          : refreshed;
        store.update((state) => ({ ...state, session: merged, error: state.error === refreshError ? null : state.error }));
        refreshError = null;
        return merged;
      } catch (error) {
        if (get(store).session?.id !== current.id) return null;
        const message = error instanceof Error ? error.message : 'Unable to refresh the session';
        store.update((state) => {
          if (state.error !== null && state.error !== refreshError) return state;
          refreshError = message;
          return { ...state, error: message };
        });
        return null;
      }
    },
    setPreparing(preparing: boolean): void {
      store.update((state) => ({ ...state, preparing }));
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
            wireProtocol: credentials.wireProtocol,
            credentialExpiresAt: credentials.expiresAt
          },
          error: null
        };
      });
    },
    setError(message: string | null): void {
      refreshError = null;
      store.update((state) => ({ ...state, busy: false, error: message }));
    },
    takeNativeStartupRetry(): boolean {
      const current = get(store);
      if (current.session?.status !== 'accepted' || current.nativeRetryBlocked || current.nativeStartupRetries >= 1) return false;
      store.update((state) => ({ ...state, nativeStartupRetries: state.nativeStartupRetries + 1 }));
      return true;
    },
    blockNativeRetry(): void {
      store.update((state) => ({ ...state, nativeRetryBlocked: true }));
    },
    resetNativeRetry(): void {
      store.update((state) => ({ ...state, nativeStartupRetries: 0, nativeRetryBlocked: false }));
    },
    setBusy(busy: boolean): void {
      store.update((state) => ({ ...state, busy }));
    },
    setMetrics(metrics: SessionMetrics): void {
      store.update((state) => ({ ...state, metrics }));
    },
    clear(): void {
      refreshError = null;
      store.set({ ...initial, preparing: get(store).preparing });
    }
  };
}

export const connection = createConnectionStore();
