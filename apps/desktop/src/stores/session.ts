import { get, writable } from 'svelte/store';
import { ApiClient, ApiError } from '../lib/api';
import { secureDelete, secureGet, secureSet } from '../lib/platform';
import type { Account, AuthResult } from '../lib/types';
import { diagnostics } from './diagnostics';

export const SESSION_IDLE_MS = 7 * 24 * 60 * 60 * 1_000;
const ACTIVITY_WRITE_INTERVAL_MS = 60_000;
const CLOCK_SKEW_MS = 5 * 60_000;
export type SessionMode = 'signedOut' | 'cloud';

export interface SessionState {
  ready: boolean;
  busy: boolean;
  mode: SessionMode;
  account: Account | null;
  serverUrl: string;
  error: string | null;
}

interface SavedAuth {
  version: 1;
  serverUrl: string;
  accessToken: string;
  refreshToken: string;
  lastActiveAt: number;
}

let accessToken: string | null = null;
let refreshToken: string | null = null;
let api: ApiClient | null = null;
let savedAuth: SavedAuth | null = null;
let rememberSignIn = false;
let credentialSaved = false;
let lastActivityWrite = 0;
const initial: SessionState = { ready: false, busy: false, mode: 'signedOut', account: null, serverUrl: '', error: null };
let authWrites = Promise.resolve();
let authGeneration = 0;

function isExpired(lastActiveAt: number): boolean {
  const now = Date.now();
  return now - lastActiveAt >= SESSION_IDLE_MS || lastActiveAt > now + CLOCK_SKEW_MS;
}

function currentAuthExpired(): boolean {
  return savedAuth === null || isExpired(savedAuth.lastActiveAt);
}

function readSavedAuth(raw: string, serverUrl: string): SavedAuth | null {
  try {
    const record = JSON.parse(raw) as Partial<SavedAuth> | null;
    if (record?.version !== 1 || record.serverUrl !== serverUrl ||
        typeof record.accessToken !== 'string' || !record.accessToken ||
        typeof record.refreshToken !== 'string' || !record.refreshToken ||
        typeof record.lastActiveAt !== 'number' || !Number.isSafeInteger(record.lastActiveAt) || record.lastActiveAt <= 0) return null;
    return record as SavedAuth;
  } catch { return null; }
}

function queueAuthWrite(operation: () => Promise<void>): Promise<void> {
  const write = authWrites.then(operation);
  authWrites = write.catch(() => undefined);
  return write;
}

async function recordActivity(client: ApiClient): Promise<void> {
  if (api !== client || !savedAuth) return;
  // An expired session must never renew its local inactivity window.
  if (isExpired(savedAuth.lastActiveAt)) return;
  const now = Date.now();
  savedAuth = { ...savedAuth, lastActiveAt: Math.max(now, savedAuth.lastActiveAt) };
  if (!rememberSignIn) return;
  if (now - lastActivityWrite < ACTIVITY_WRITE_INTERVAL_MS) return;
  lastActivityWrite = now;
  const snapshot = savedAuth;
  const generation = authGeneration;
  try {
    await queueAuthWrite(async () => {
      if (generation === authGeneration && api === client) await secureSet('auth_session', JSON.stringify(snapshot));
    });
  } catch {
    diagnostics.add({ level: 'warn', category: 'auth', message: 'Unable to save recent sign-in activity; it will be retried.' });
  }
}

function makeClient(serverUrl: string): ApiClient {
  let renewal: Promise<boolean> | null = null;
  const client: ApiClient = new ApiClient(serverUrl, () => Promise.resolve(api === client ? accessToken : null), 12_000, () => {
    if (renewal) return renewal;
    if (!refreshToken || api !== client || !savedAuth || isExpired(savedAuth.lastActiveAt)) return Promise.resolve(false);
    const generation = authGeneration;
    const token = refreshToken;
    renewal = (async () => {
      try {
        const result = await client.refresh(token);
        if (currentAuthExpired()) return false;
        return await persistAuth(result, client, generation);
      } finally { renewal = null; }
    })();
    return renewal;
  }, () => recordActivity(client));
  return client;
}

async function persistAuth(result: AuthResult, client: ApiClient, generation: number): Promise<boolean> {
  if (generation !== authGeneration || api !== client) return false;
  const record: SavedAuth = {
    version: 1, serverUrl: client.serverUrl, accessToken: result.accessToken,
    refreshToken: result.refreshToken, lastActiveAt: Date.now()
  };
  await queueAuthWrite(async () => {
    if (generation !== authGeneration || api !== client) return;
    if (rememberSignIn) {
      await secureSet('auth_session', JSON.stringify(record));
      credentialSaved = true;
    } else if (credentialSaved) {
      await secureDelete('auth_session');
      credentialSaved = false;
    }
  });
  if (generation !== authGeneration || api !== client) return false;
  accessToken = result.accessToken;
  refreshToken = result.refreshToken;
  savedAuth = record;
  lastActivityWrite = record.lastActiveAt;
  return true;
}

async function clearAuth(): Promise<number> {
  const generation = ++authGeneration;
  accessToken = null;
  refreshToken = null;
  savedAuth = null;
  rememberSignIn = false;
  await queueAuthWrite(async () => {
    // Remove legacy entries as well. Tokens without an activity timestamp cannot
    // prove that the previous sign-in is within the seven-day window.
    if (credentialSaved) {
      await Promise.all(['auth_session', 'access_token', 'refresh_token'].map(secureDelete));
      credentialSaved = false;
    }
  });
  return generation;
}

function createSessionStore() {
  const store = writable<SessionState>(initial);

  async function authenticate(serverUrl: string, request: (client: ApiClient) => Promise<AuthResult>, success: string, remember: boolean): Promise<void> {
    const generation = ++authGeneration;
    store.update((state) => ({ ...state, busy: true, error: null }));
    try {
      const client = makeClient(serverUrl);
      api = client;
      accessToken = null;
      refreshToken = null;
      savedAuth = null;
      rememberSignIn = remember;
      const result = await request(client);
      if (!await persistAuth(result, client, generation)) return;
      store.set({ ready: true, busy: false, mode: 'cloud', account: result.account, serverUrl: client.serverUrl, error: null });
      diagnostics.add({ level: 'info', category: 'auth', message: success });
    } catch (error) {
      if (generation !== authGeneration) return;
      const message = error instanceof Error ? error.message : 'Unable to sign in';
      store.update((state) => ({ ...state, busy: false, error: message }));
      throw error;
    }
  }

  return {
    subscribe: store.subscribe,
    client(): ApiClient | null { return api; },
    isInactive(): boolean { return get(store).mode === 'cloud' && savedAuth !== null && isExpired(savedAuth.lastActiveAt); },
    async initialize(serverUrl: string): Promise<void> {
      const generation = ++authGeneration;
      accessToken = null;
      refreshToken = null;
      savedAuth = null;
      rememberSignIn = false;
      api = null;
      store.set({ ...initial, busy: true, serverUrl });
      if (!serverUrl.trim()) {
        store.set({ ...initial, ready: true, serverUrl });
        return;
      }
      try {
        const client = makeClient(serverUrl);
        api = client;
        await authWrites;
        const raw = await secureGet('auth_session');
        if (generation !== authGeneration || api !== client) return;
        credentialSaved = raw !== null;
        if (!raw) {
          store.set({ ...initial, ready: true, serverUrl });
          return;
        }
        const saved = readSavedAuth(raw, client.serverUrl);
        if (!saved || isExpired(saved.lastActiveAt)) {
          const clearedGeneration = await clearAuth();
          if (clearedGeneration === authGeneration) {
            api = null;
            store.set({ ...initial, ready: true, serverUrl, error: 'Your saved sign-in expired. Please sign in again.' });
          }
          return;
        }
        savedAuth = saved;
        rememberSignIn = true;
        lastActivityWrite = saved.lastActiveAt;
        accessToken = saved.accessToken;
        refreshToken = saved.refreshToken;
        // Refresh on reopening so the server also sees use of the saved session.
        const result = await client.refresh(saved.refreshToken);
        if (isExpired(saved.lastActiveAt)) throw new ApiError('Saved sign-in expired', 401, 'session_expired', null);
        if (!await persistAuth(result, client, generation)) return;
        store.set({ ready: true, busy: false, mode: 'cloud', account: result.account, serverUrl: client.serverUrl, error: null });
      } catch (error) {
        if (generation !== authGeneration) return;
        if (error instanceof ApiError && error.status === 401) {
          const clearedGeneration = await clearAuth();
          if (clearedGeneration !== authGeneration) return;
          api = null;
          store.set({ ...initial, ready: true, serverUrl, error: 'Your session expired. Please sign in again.' });
        } else {
          // Temporary network/keychain failure never erases a saved credential.
          store.set({ ...initial, ready: true, serverUrl, error: 'Unable to restore your session. Check your connection and retry.' });
        }
      }
    },
    login(serverUrl: string, email: string, password: string, remember = false): Promise<void> {
      return authenticate(serverUrl, (client) => client.login(email.trim().toLowerCase(), password), 'Signed in successfully', remember);
    },
    register(serverUrl: string, email: string, password: string, displayName: string, remember = false): Promise<void> {
      return authenticate(serverUrl, (client) => client.register(email.trim().toLowerCase(), password, displayName), 'Account created successfully', remember);
    },
    async logout(): Promise<void> {
      const currentApi = api;
      const currentRefresh = refreshToken;
      const generation = ++authGeneration;
      savedAuth = null;
      try {
        if (currentApi) await currentApi.logout(currentRefresh);
      } catch {
        // Explicit sign-out still clears local credentials during an outage.
      }
      if (generation !== authGeneration) return;
      const clearedGeneration = await clearAuth();
      if (clearedGeneration !== authGeneration) return;
      api = null;
      const current = get(store);
      store.set({ ...initial, ready: true, serverUrl: current.serverUrl });
      diagnostics.add({ level: 'info', category: 'auth', message: 'Signed out' });
    }
  };
}

export const session = createSessionStore();
