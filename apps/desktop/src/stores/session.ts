import { get, writable } from 'svelte/store';
import { ApiClient, ApiError } from '../lib/api';
import { secureDelete, secureGet, secureSet } from '../lib/platform';
import type { Account, AuthResult } from '../lib/types';
import { diagnostics } from './diagnostics';

export type SessionMode = 'signedOut' | 'cloud';

export interface SessionState {
  ready: boolean;
  busy: boolean;
  mode: SessionMode;
  account: Account | null;
  serverUrl: string;
  error: string | null;
}

let accessToken: string | null = null;
let refreshToken: string | null = null;
let api: ApiClient | null = null;

const initial: SessionState = {
  ready: false,
  busy: false,
  mode: 'signedOut',
  account: null,
  serverUrl: '',
  error: null
};

function makeClient(serverUrl: string): ApiClient {
  return new ApiClient(serverUrl, () => Promise.resolve(accessToken));
}

async function persistAuth(result: AuthResult): Promise<void> {
  accessToken = result.accessToken;
  refreshToken = result.refreshToken;
  await Promise.all([secureSet('access_token', result.accessToken), secureSet('refresh_token', result.refreshToken)]);
}

async function clearAuth(): Promise<void> {
  accessToken = null;
  refreshToken = null;
  await Promise.all([secureDelete('access_token'), secureDelete('refresh_token')]);
}

function createSessionStore() {
  const store = writable<SessionState>(initial);

  return {
    subscribe: store.subscribe,
    client(): ApiClient | null {
      return api;
    },
    async initialize(serverUrl: string): Promise<void> {
      store.set({ ...initial, busy: true, serverUrl });
      if (!serverUrl.trim()) {
        api = null;
        store.set({ ...initial, ready: true, serverUrl });
        return;
      }
      try {
        api = makeClient(serverUrl);
      } catch (error) {
        api = null;
        store.set({
          ...initial,
          ready: true,
          serverUrl,
          error: error instanceof Error ? error.message : 'The configured server URL is invalid'
        });
        return;
      }
      [accessToken, refreshToken] = await Promise.all([secureGet('access_token'), secureGet('refresh_token')]);

      if (!accessToken) {
        store.set({ ...initial, ready: true, serverUrl });
        return;
      }

      try {
        const account = await api.account();
        store.set({ ready: true, busy: false, mode: 'cloud', account, serverUrl, error: null });
      } catch (error) {
        if (error instanceof ApiError && error.status === 401 && refreshToken) {
          try {
            const refreshed = await api.refresh(refreshToken);
            await persistAuth(refreshed);
            store.set({ ready: true, busy: false, mode: 'cloud', account: refreshed.account, serverUrl, error: null });
            return;
          } catch {
            // Revoked/expired refresh credentials are removed below.
          }
        }
        await clearAuth();
        store.set({ ...initial, ready: true, serverUrl });
      }
    },
    async login(serverUrl: string, email: string, password: string): Promise<void> {
      store.update((state) => ({ ...state, busy: true, error: null }));
      try {
        api = makeClient(serverUrl);
        const result = await api.login(email.trim().toLowerCase(), password);
        await persistAuth(result);
        store.set({ ready: true, busy: false, mode: 'cloud', account: result.account, serverUrl, error: null });
        diagnostics.add({ level: 'info', category: 'auth', message: 'Signed in successfully' });
      } catch (error) {
        const message = error instanceof Error ? error.message : 'Unable to sign in';
        store.update((state) => ({ ...state, busy: false, error: message }));
        throw error;
      }
    },
    async register(serverUrl: string, email: string, password: string, displayName: string): Promise<void> {
      store.update((state) => ({ ...state, busy: true, error: null }));
      try {
        api = makeClient(serverUrl);
        const result = await api.register(email.trim().toLowerCase(), password, displayName);
        await persistAuth(result);
        store.set({ ready: true, busy: false, mode: 'cloud', account: result.account, serverUrl, error: null });
        diagnostics.add({ level: 'info', category: 'auth', message: 'Account created successfully' });
      } catch (error) {
        const message = error instanceof Error ? error.message : 'Unable to create account';
        store.update((state) => ({ ...state, busy: false, error: message }));
        throw error;
      }
    },
    async logout(): Promise<void> {
      const currentApi = api;
      const currentRefresh = refreshToken;
      try {
        if (currentApi) await currentApi.logout(currentRefresh);
      } catch {
        // Local credentials are still cleared when the server is unavailable.
      }
      await clearAuth();
      api = null;
      const current = get(store);
      store.set({ ...initial, ready: true, serverUrl: current.serverUrl });
      diagnostics.add({ level: 'info', category: 'auth', message: 'Signed out' });
    }
  };
}

export const session = createSessionStore();
