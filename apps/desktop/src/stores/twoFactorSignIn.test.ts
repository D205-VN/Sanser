import { get } from 'svelte/store';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const platform = vi.hoisted(() => ({ secureGet: vi.fn(), secureSet: vi.fn(), secureDelete: vi.fn(), loginDevice: vi.fn() }));
vi.mock('../lib/platform', () => platform);
const origin = 'https://sanser.example';
const account = { id: 'account-1', email: 'test@example.test', displayName: 'Test' };
const tokens = { account, accessToken: 'access', refreshToken: 'refresh' };
const challenge = { error: { code: 'two_factor_required', message: 'Two-step verification is turned on for this account. Update Sanser to the latest version to sign in.', challengeToken: 'sn2_c_challenge', challengeExpiresAt: 2_000_000_000 } };
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });

beforeEach(() => {
  vi.resetModules();
  platform.secureGet.mockResolvedValue(null);
  platform.secureSet.mockResolvedValue(undefined);
  platform.secureDelete.mockResolvedValue(undefined);
  platform.loginDevice.mockResolvedValue({ installationId: '11111111-1111-4111-8111-111111111111', deviceName: 'Mac', platform: 'macos-aarch64' });
});
afterEach(() => { vi.unstubAllGlobals(); vi.restoreAllMocks(); });

describe('two-step sign-in', () => {
  it('asks for a code after the password and signs in with it', async () => {
    const fetchMock = vi.fn<typeof fetch>().mockResolvedValueOnce(json(challenge, 401)).mockResolvedValueOnce(json(tokens));
    vi.stubGlobal('fetch', fetchMock);
    const { session } = await import('./session');
    await session.login(origin, 'test@example.test', 'StrongPass123');
    expect(get(session).mode).toBe('signedOut');
    expect(get(session).twoFactor).toEqual({ expiresAt: 2_000_000_000 });
    expect(get(session).error).toBeNull();
    await session.completeTwoFactor({ code: '123456' });
    expect(get(session).mode).toBe('cloud');
    expect(get(session).twoFactor).toBeNull();
    const [url, init] = fetchMock.mock.calls[1] ?? [];
    expect(url).toBe(`${origin}/api/v2/auth/login/two-factor`);
    expect(JSON.parse(init?.body as string)).toEqual({ challengeToken: 'sn2_c_challenge', code: '123456' });
    expect(new Headers(init?.headers).get('Authorization')).toBeNull();
  });

  it('keeps the step open for a wrong code and starts over when the challenge is gone', async () => {
    const fetchMock = vi.fn<typeof fetch>()
      .mockResolvedValueOnce(json(challenge, 401))
      .mockResolvedValueOnce(json({ error: { code: 'validation_error', message: 'The verification code is incorrect or was already used.' } }, 422))
      .mockResolvedValueOnce(json({ error: { code: 'unauthorized', message: 'authentication is required' } }, 401));
    vi.stubGlobal('fetch', fetchMock);
    const { session } = await import('./session');
    await session.login(origin, 'test@example.test', 'StrongPass123');
    await expect(session.completeTwoFactor({ code: '000000' })).rejects.toThrow('incorrect');
    expect(get(session).twoFactor).not.toBeNull();
    await expect(session.completeTwoFactor({ recoveryCode: 'AAAAA-BBBBB' })).rejects.toThrow();
    expect(get(session).twoFactor).toBeNull();
    expect(get(session).error).toBe('This sign-in attempt expired. Sign in again.');
    await expect(session.completeTwoFactor({ code: '123456' })).rejects.toThrow('Sign in again');
  });

  it('starts a guest session with an in-memory token and no refresh', async () => {
    const pending = { id: 's1', requesterDeviceId: 'g1', hostDeviceId: 'h1', state: 'pending', accessMode: 'view-only', guest: { name: 'Helper' } };
    const fetchMock = vi.fn<typeof fetch>()
      .mockResolvedValueOnce(json({ accessToken: 'guest-token', accessExpiresAt: 1, deviceId: 'g1', hostName: 'Office PC', session: pending }, 201))
      .mockResolvedValueOnce(json({ error: { code: 'unauthorized', message: 'x' } }, 401));
    vi.stubGlobal('fetch', fetchMock);
    const { session } = await import('./session');
    const result = await session.startGuest(origin, (client) => client.redeemGuestCode('123 456 789', 'ABCD2345', 'Helper', {
      platform: 'macos-aarch64', crossPlatform: false, osVersion: 'macos', version: '2.3.16', protocolVersion: 2,
      codecs: ['h264'], nativeTransport: true, webRtc: true, audio: false, gamepad: false
    }));
    expect(result.session.guest).toEqual({ name: 'Helper' });
    expect(get(session).mode).toBe('guest');
    expect(get(session).guest).toEqual({ hostName: 'Office PC', deviceId: 'g1' });
    await expect(session.client()?.getSession('s1')).rejects.toThrow();
    // A 401 never triggers a refresh for a guest; only the one call was made.
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(platform.secureSet).not.toHaveBeenCalled();
    session.endGuest();
    expect(get(session).mode).toBe('signedOut');
    expect(session.client()).toBeNull();
  });
});
