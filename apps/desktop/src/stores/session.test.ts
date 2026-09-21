import { get } from 'svelte/store';
import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest';

const storage = vi.hoisted(() => ({ secureGet: vi.fn(), secureSet: vi.fn(), secureDelete: vi.fn() }));
vi.mock('../lib/platform', () => storage);
const account = { id: 'account-1', email: 'test@example.test', displayName: 'Test' };
const auth = { account, accessToken: 'fresh-access', refreshToken: 'fresh-refresh' };
const origin = 'https://sanser.example';
const DAY = 24 * 60 * 60 * 1000;
const startedAt = Date.UTC(2026, 8, 21);
let now = startedAt;
let secrets: Map<string, string>;
function response(body: unknown, status = 200): Response { return new Response(JSON.stringify(body), { status }); }
function saved(lastActiveAt = now, serverUrl = origin) {
  return JSON.stringify({ version: 1, serverUrl, accessToken: 'old-access', refreshToken: 'old-refresh', lastActiveAt });
}
function path(url: Parameters<typeof fetch>[0]): string {
  return typeof url === 'string' ? url : url instanceof URL ? url.href : url.url;
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}
beforeEach(() => {
  vi.resetModules();
  vi.clearAllMocks();
  now = startedAt;
  vi.spyOn(Date, 'now').mockImplementation(() => now);
  secrets = new Map([['auth_session', saved()]]);
  storage.secureGet.mockImplementation((key: string) => Promise.resolve(secrets.get(key) ?? null));
  storage.secureSet.mockImplementation((key: string, value: string) => { secrets.set(key, value); return Promise.resolve(); });
  storage.secureDelete.mockImplementation((key: string) => { secrets.delete(key); return Promise.resolve(); });
});
afterEach(() => { vi.unstubAllGlobals(); vi.restoreAllMocks(); });

describe('saved sign-in', () => {
  it('preserves credentials during an outage and restores the account on retry', async () => {
    const fetchMock = vi.fn<typeof fetch>().mockRejectedValueOnce(new TypeError('offline')).mockResolvedValueOnce(response(auth));
    vi.stubGlobal('fetch', fetchMock);
    const { session } = await import('./session');
    await session.initialize(origin);
    expect(get(session).mode).toBe('signedOut');
    expect(get(session).error).toContain('retry');
    expect(storage.secureDelete).not.toHaveBeenCalled();
    await session.initialize(origin);
    expect(get(session).account).toEqual(account);
  });

  it('renews saved tokens on reopen and persists them together with server and activity time', async () => {
    const fetchMock = vi.fn<typeof fetch>().mockResolvedValue(response(auth));
    vi.stubGlobal('fetch', fetchMock);
    const { session } = await import('./session');
    await session.initialize(origin);
    expect(get(session).mode).toBe('cloud');
    expect(JSON.parse(secrets.get('auth_session') ?? '{}')).toEqual({ version: 1, serverUrl: origin, accessToken: auth.accessToken, refreshToken: auth.refreshToken, lastActiveAt: now });
    expect(fetchMock.mock.calls[0]?.[1]?.body).toBe(JSON.stringify({ refreshToken: 'old-refresh' }));
  });

  it('does not erase refresh credentials when the service is temporarily unavailable', async () => {
    vi.stubGlobal('fetch', vi.fn<typeof fetch>().mockResolvedValue(response({}, 503)));
    const { session } = await import('./session');
    await session.initialize(origin);
    expect(storage.secureDelete).not.toHaveBeenCalled();
    expect(get(session).error).toContain('retry');
  });

  it('clears credentials when the refresh token is rejected', async () => {
    vi.stubGlobal('fetch', vi.fn<typeof fetch>().mockResolvedValue(response({}, 401)));
    const { session } = await import('./session');
    await session.initialize(origin);
    expect(secrets.has('auth_session')).toBe(false);
    expect(get(session).error).toContain('expired');
  });

  it('shares one renewal across simultaneous unauthorized requests', async () => {
    const refreshing = deferred<Response>();
    let refreshCount = 0;
    vi.stubGlobal('fetch', vi.fn<typeof fetch>().mockImplementation((url, options) => {
      if (path(url).endsWith('/auth/refresh')) {
        refreshCount += 1;
        return refreshCount === 1 ? Promise.resolve(response({ ...auth, accessToken: 'restored-access' })) : refreshing.promise;
      }
      return Promise.resolve(new Headers(options?.headers).get('Authorization') === 'Bearer fresh-access' ? response({ items: [] }) : response({}, 401));
    }));
    const { session } = await import('./session');
    await session.initialize(origin);
    const first = session.client()?.devices();
    const second = session.client()?.devices();
    await vi.waitFor(() => expect(refreshCount).toBe(2));
    refreshing.resolve(response(auth));
    await Promise.all([first, second]);
    expect(refreshCount).toBe(2); // One startup renewal and one shared retry.
  });

  it('does not restore tokens when a renewal completes after logout', async () => {
    const refreshing = deferred<Response>();
    let refreshCount = 0;
    vi.stubGlobal('fetch', vi.fn<typeof fetch>().mockImplementation((url) => {
      if (path(url).endsWith('/auth/refresh')) return ++refreshCount === 1 ? Promise.resolve(response(auth)) : refreshing.promise;
      if (path(url).endsWith('/auth/logout')) return Promise.resolve(new Response(null, { status: 204 }));
      return Promise.resolve(response({}, 401));
    }));
    const { session } = await import('./session');
    await session.initialize(origin);
    const request = session.client()?.devices().catch(() => undefined);
    await vi.waitFor(() => expect(refreshCount).toBe(2));
    await session.logout();
    refreshing.resolve(response(auth));
    await request;
    expect(secrets.has('auth_session')).toBe(false);
    expect(get(session).mode).toBe('signedOut');
    expect(session.client()).toBeNull();
  });

  it.each([7 * DAY, 8 * DAY])('requires sign-in after %i ms without use before contacting the server', async (elapsed) => {
    now += elapsed;
    const fetchMock = vi.fn<typeof fetch>();
    vi.stubGlobal('fetch', fetchMock);
    const { session } = await import('./session');
    await session.initialize(origin);
    expect(get(session).mode).toBe('signedOut');
    expect(get(session).error).toContain('expired');
    expect(secrets.has('auth_session')).toBe(false);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('automatically restores just before seven days and renews the window on each return', async () => {
    vi.stubGlobal('fetch', vi.fn<typeof fetch>().mockImplementation(() => Promise.resolve(response(auth))));
    now += 7 * DAY - 1;
    const first = (await import('./session')).session;
    await first.initialize(origin);
    expect(get(first).mode).toBe('cloud');
    now += 6 * DAY;
    vi.resetModules();
    const reopened = (await import('./session')).session;
    await reopened.initialize(origin);
    expect(get(reopened).mode).toBe('cloud');
    expect((JSON.parse(secrets.get('auth_session') ?? '{}') as { lastActiveAt: number }).lastActiveAt).toBe(now);
  });

  it('successful authenticated activity keeps an open app signed in; idle time eventually expires it', async () => {
    vi.stubGlobal('fetch', vi.fn<typeof fetch>().mockImplementation((url) => Promise.resolve(response(path(url).endsWith('/auth/refresh') ? auth : account))));
    const { session } = await import('./session');
    await session.initialize(origin);
    now += 6 * DAY;
    await session.client()?.account();
    expect((JSON.parse(secrets.get('auth_session') ?? '{}') as { lastActiveAt: number }).lastActiveAt).toBe(now);
    now += 6 * DAY;
    expect(session.isInactive()).toBe(false);
    now += DAY;
    expect(session.isInactive()).toBe(true);
  });

  it('failed network requests do not extend the seven-day window', async () => {
    vi.stubGlobal('fetch', vi.fn<typeof fetch>().mockResolvedValueOnce(response(auth)).mockRejectedValue(new TypeError('offline')));
    const { session } = await import('./session');
    await session.initialize(origin);
    now += 6 * DAY;
    await session.client()?.account().catch(() => undefined);
    now += DAY;
    expect(session.isInactive()).toBe(true);
    expect((JSON.parse(secrets.get('auth_session') ?? '{}') as { lastActiveAt: number }).lastActiveAt).toBe(startedAt);
  });

  it.each(['{invalid', saved(startedAt, 'https://another.example'), saved(startedAt + 2 * DAY)])('rejects corrupt, mismatched-server or future-dated saved sign-ins', async (record) => {
    secrets.set('auth_session', record);
    const fetchMock = vi.fn<typeof fetch>();
    vi.stubGlobal('fetch', fetchMock);
    const { session } = await import('./session');
    await session.initialize(origin);
    expect(get(session).mode).toBe('signedOut');
    expect(fetchMock).not.toHaveBeenCalled();
    expect(secrets.has('auth_session')).toBe(false);
  });

  it('does not revive a saved account when startup completes after logout', async () => {
    const refreshing = deferred<Response>();
    vi.stubGlobal('fetch', vi.fn<typeof fetch>().mockImplementation((url) => path(url).endsWith('/auth/refresh') ? refreshing.promise : Promise.resolve(new Response(null, { status: 204 }))));
    const { session } = await import('./session');
    const starting = session.initialize(origin);
    await vi.waitFor(() => expect(storage.secureGet).toHaveBeenCalled());
    await session.logout();
    refreshing.resolve(response(auth));
    await starting;
    expect(get(session).mode).toBe('signedOut');
    expect(secrets.has('auth_session')).toBe(false);
  });

  it('requires one fresh sign-in for legacy records without an inactivity timestamp', async () => {
    secrets = new Map([['access_token', 'legacy-access'], ['refresh_token', 'legacy-refresh']]);
    const fetchMock = vi.fn<typeof fetch>();
    vi.stubGlobal('fetch', fetchMock);
    const { session } = await import('./session');
    await session.initialize(origin);
    expect(get(session).mode).toBe('signedOut');
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('saves login credentials without the password and restores after a module restart', async () => {
    secrets.clear();
    vi.stubGlobal('fetch', vi.fn<typeof fetch>().mockImplementation(() => Promise.resolve(response(auth))));
    const { session } = await import('./session');
    await session.login(origin, account.email, 'Test-only-Password123', true);
    expect(secrets.get('auth_session')).not.toContain('Test-only-Password123');
    vi.resetModules();
    const reopened = (await import('./session')).session;
    await reopened.initialize(origin);
    expect(get(reopened).account).toEqual(account);
  });

  it('does not report a successful login when secure persistence fails', async () => {
    storage.secureSet.mockRejectedValueOnce(new Error('Keychain locked'));
    vi.stubGlobal('fetch', vi.fn<typeof fetch>().mockResolvedValue(response(auth)));
    const { session } = await import('./session');
    await expect(session.login(origin, account.email, 'Test-only-Password123', true)).rejects.toThrow('Keychain locked');
    expect(get(session).mode).toBe('signedOut');
    expect(get(session).busy).toBe(false);
  });

  it('keeps an unchecked login only in memory and asks for sign-in after reopening', async () => {
    secrets.clear();
    const fetchMock = vi.fn<typeof fetch>().mockImplementation(() => Promise.resolve(response(auth)));
    vi.stubGlobal('fetch', fetchMock);
    const { session } = await import('./session');
    await session.initialize(origin);
    await session.login(origin, account.email, 'Test-only-Password123');
    expect(get(session).mode).toBe('cloud');
    now += DAY;
    await session.client()?.account();
    expect(storage.secureSet).not.toHaveBeenCalled();
    expect(secrets.has('auth_session')).toBe(false);
    vi.resetModules();
    const reopened = (await import('./session')).session;
    await reopened.initialize(origin);
    expect(get(reopened).mode).toBe('signedOut');
  });

  it('forgets an earlier saved login when the user signs in without remembering', async () => {
    const fetchMock = vi.fn<typeof fetch>().mockRejectedValueOnce(new TypeError('offline')).mockResolvedValueOnce(response(auth));
    vi.stubGlobal('fetch', fetchMock);
    const { session } = await import('./session');
    await session.initialize(origin);
    expect(secrets.has('auth_session')).toBe(true);
    await session.login(origin, account.email, 'Test-only-Password123', false);
    expect(get(session).mode).toBe('cloud');
    expect(secrets.has('auth_session')).toBe(false);
    expect(storage.secureSet).not.toHaveBeenCalled();
  });
});
