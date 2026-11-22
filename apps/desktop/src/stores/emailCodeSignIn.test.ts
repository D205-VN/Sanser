import { get } from 'svelte/store';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const platform = vi.hoisted(() => ({ secureGet: vi.fn(), secureSet: vi.fn(), secureDelete: vi.fn(), loginDevice: vi.fn() }));
vi.mock('../lib/platform', () => platform);
const origin = 'https://sanser.example';
const account = { id: 'account-1', email: 'test@example.test', displayName: 'Test' };
const tokens = { account, accessToken: 'access', refreshToken: 'refresh' };
const challenge = { error: { code: 'email_verification_required', message: 'A Sanser verification code was sent to your email. Update Sanser to the latest version to enter it.', challengeToken: 'sn2_e_challenge', challengeExpiresAt: 2_000_000_000, emailHint: 'te****@example.test' } };
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });

beforeEach(() => {
  vi.resetModules();
  platform.secureGet.mockResolvedValue(null);
  platform.secureSet.mockResolvedValue(undefined);
  platform.secureDelete.mockResolvedValue(undefined);
  platform.loginDevice.mockResolvedValue({ installationId: '11111111-1111-4111-8111-111111111111', deviceName: 'Mac', platform: 'macos-aarch64' });
});
afterEach(() => { vi.unstubAllGlobals(); vi.restoreAllMocks(); });

describe('email code sign-in', () => {
  it('asks a new computer for the emailed code after the password and signs in with it', async () => {
    const fetchMock = vi.fn<typeof fetch>().mockResolvedValueOnce(json(challenge, 401)).mockResolvedValueOnce(json(tokens));
    vi.stubGlobal('fetch', fetchMock);
    const { session } = await import('./session');
    await session.login(origin, 'test@example.test', 'StrongPass123');
    expect(get(session).mode).toBe('signedOut');
    expect(get(session).emailCode).toEqual({ expiresAt: 2_000_000_000, emailHint: 'te****@example.test', purpose: 'login' });
    expect(get(session).error).toBeNull();
    const [, loginInit] = fetchMock.mock.calls[0] ?? [];
    expect(JSON.parse(loginInit?.body as string)).toMatchObject({ installationId: '11111111-1111-4111-8111-111111111111' });
    await session.completeEmailCode('123 456');
    expect(get(session).mode).toBe('cloud');
    expect(get(session).emailCode).toBeNull();
    const [url, init] = fetchMock.mock.calls[1] ?? [];
    expect(url).toBe(`${origin}/api/v2/auth/email/verify`);
    expect(JSON.parse(init?.body as string)).toEqual({ challengeToken: 'sn2_e_challenge', code: '123456' });
    expect(new Headers(init?.headers).get('Authorization')).toBeNull();
  });

  it('confirms the email before a new account exists', async () => {
    const fetchMock = vi.fn<typeof fetch>().mockResolvedValueOnce(json(challenge, 401)).mockResolvedValueOnce(json(tokens, 201));
    vi.stubGlobal('fetch', fetchMock);
    const { session } = await import('./session');
    await session.register(origin, 'test@example.test', 'StrongPass123', 'Test');
    expect(get(session).mode).toBe('signedOut');
    expect(get(session).emailCode?.purpose).toBe('register');
    await session.completeEmailCode('654321');
    expect(get(session).mode).toBe('cloud');
    expect(fetchMock.mock.calls[0]?.[0]).toBe(`${origin}/api/v2/auth/register`);
    expect(fetchMock.mock.calls[1]?.[0]).toBe(`${origin}/api/v2/auth/email/verify`);
  });

  it('resets a forgotten password with the emailed code and signs in', async () => {
    const fetchMock = vi.fn<typeof fetch>()
      .mockResolvedValueOnce(json({ challengeToken: 'sn2_e_reset', challengeExpiresAt: 2_000_000_000, emailHint: 'te****@example.test' }, 202))
      .mockResolvedValueOnce(json(tokens));
    vi.stubGlobal('fetch', fetchMock);
    const { session } = await import('./session');
    await session.forgotPassword(origin, ' Test@Example.test ');
    expect(get(session).emailCode).toEqual({ expiresAt: 2_000_000_000, emailHint: 'te****@example.test', purpose: 'reset' });
    const [forgotUrl, forgotInit] = fetchMock.mock.calls[0] ?? [];
    expect(forgotUrl).toBe(`${origin}/api/v2/auth/password/forgot`);
    expect(JSON.parse(forgotInit?.body as string)).toMatchObject({ email: 'test@example.test', installationId: '11111111-1111-4111-8111-111111111111' });
    await session.completePasswordReset('123-456', 'A-new-passphrase1');
    expect(get(session).mode).toBe('cloud');
    expect(get(session).emailCode).toBeNull();
    const [url, init] = fetchMock.mock.calls[1] ?? [];
    expect(url).toBe(`${origin}/api/v2/auth/password/reset`);
    expect(JSON.parse(init?.body as string)).toEqual({ challengeToken: 'sn2_e_reset', code: '123456', newPassword: 'A-new-passphrase1' });
  });

  it('returns to sign-in when a reset code expired', async () => {
    const fetchMock = vi.fn<typeof fetch>()
      .mockResolvedValueOnce(json({ challengeToken: 'sn2_e_reset', challengeExpiresAt: 2_000_000_000, emailHint: 'te****@example.test' }, 202))
      .mockResolvedValueOnce(json({ error: { code: 'unauthorized', message: 'Authentication is required' } }, 401));
    vi.stubGlobal('fetch', fetchMock);
    const { session } = await import('./session');
    await session.forgotPassword(origin, 'test@example.test');
    await expect(session.completePasswordReset('123456', 'A-new-passphrase1')).rejects.toThrow();
    expect(get(session).emailCode).toBeNull();
    expect(get(session).error).toBe('This code expired. Ask for a new reset code.');
    expect(get(session).mode).toBe('signedOut');
  });

  it('resends a code and updates its expiry', async () => {
    const fetchMock = vi.fn<typeof fetch>()
      .mockResolvedValueOnce(json(challenge, 401))
      .mockResolvedValueOnce(json({ challengeExpiresAt: 2_000_000_600, emailHint: 'te****@example.test' }));
    vi.stubGlobal('fetch', fetchMock);
    const { session } = await import('./session');
    await session.login(origin, 'test@example.test', 'StrongPass123');
    await session.resendEmailCode();
    expect(get(session).emailCode?.expiresAt).toBe(2_000_000_600);
    const [url, init] = fetchMock.mock.calls[1] ?? [];
    expect(url).toBe(`${origin}/api/v2/auth/email/resend`);
    expect(JSON.parse(init?.body as string)).toEqual({ challengeToken: 'sn2_e_challenge' });
  });

  it('keeps the step open for a wrong code and starts over when the code expired', async () => {
    const fetchMock = vi.fn<typeof fetch>()
      .mockResolvedValueOnce(json(challenge, 401))
      .mockResolvedValueOnce(json({ error: { code: 'validation_error', message: 'The code is incorrect. Check the latest Sanser email.' } }, 422))
      .mockResolvedValueOnce(json({ error: { code: 'unauthorized', message: 'authentication is required' } }, 401));
    vi.stubGlobal('fetch', fetchMock);
    const { session } = await import('./session');
    await session.login(origin, 'test@example.test', 'StrongPass123');
    await expect(session.completeEmailCode('000000')).rejects.toThrow('incorrect');
    expect(get(session).emailCode).not.toBeNull();
    await expect(session.completeEmailCode('111111')).rejects.toThrow();
    expect(get(session).emailCode).toBeNull();
    expect(get(session).error).toBe('This code expired. Sign in again.');
    await expect(session.completeEmailCode('123456')).rejects.toThrow('Sign in again');
  });

  it('signs a known computer in with the password alone', async () => {
    const fetchMock = vi.fn<typeof fetch>().mockResolvedValueOnce(json(tokens));
    vi.stubGlobal('fetch', fetchMock);
    const { session } = await import('./session');
    await session.login(origin, 'test@example.test', 'StrongPass123');
    expect(get(session).mode).toBe('cloud');
    expect(get(session).emailCode).toBeNull();
    expect(fetchMock).toHaveBeenCalledTimes(1);
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
