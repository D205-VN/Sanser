import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiClient, normalizeServerUrl } from './api';
import { PROTOCOL_VERSION, SANSER_VERSION, type DeviceRegistration } from './types';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('normalizeServerUrl', () => {
  it('allows HTTPS and loopback HTTP', () => {
    expect(normalizeServerUrl('https://sanser.example/')).toBe('https://sanser.example');
    expect(normalizeServerUrl('http://127.0.0.1:5174')).toBe('http://127.0.0.1:5174');
  });

  it('rejects insecure remote and credential-bearing URLs', () => {
    expect(() => normalizeServerUrl('http://example.test')).toThrow();
    expect(() => normalizeServerUrl('https://user:password@example.test')).toThrow();
  });

  it('uses the v2 requester and heartbeat contracts', async () => {
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(new Response(null, { status: 204 }))
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify({
            id: 'session-id',
            hostDeviceId: 'host-id',
            state: 'pending',
            networkMode: 'auto',
            qualityProfile: 'balanced',
            createdAt: 1_700_000_000,
            updatedAt: 1_700_000_000
          }),
          { status: 200, headers: { 'content-type': 'application/json' } }
        )
      );
    vi.stubGlobal('fetch', fetchMock);
    const client = new ApiClient('https://sanser.example', () => Promise.resolve('access-token'));

    await client.heartbeatDevice('requester-id', false);
    await client.createSession('host-id', 'requester-id', 'auto', 'balanced', 'hevc');

    const heartbeatInit = fetchMock.mock.calls[0]?.[1];
    const sessionInit = fetchMock.mock.calls[1]?.[1];
    const heartbeatBody = typeof heartbeatInit?.body === 'string' ? heartbeatInit.body : '';
    const sessionBody = typeof sessionInit?.body === 'string' ? sessionInit.body : '';
    expect(JSON.parse(heartbeatBody)).toEqual({ deviceId: 'requester-id', streaming: false });
    expect(JSON.parse(sessionBody)).toEqual({
      hostDeviceId: 'host-id',
      requesterDeviceId: 'requester-id',
      networkMode: 'auto',
      qualityProfile: 'balanced',
      requestedCodec: 'hevc'
    });
  });

  it('marks a device offline through the dedicated presence endpoint', async () => {
    const fetchMock = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(
        JSON.stringify({
          id: 'device-id',
          name: 'This Mac',
          platform: 'macos-arm64',
          online: false,
          streaming: false,
          sanserVersion: '2.0.0',
          codecs: ['h264', 'hevc'],
          nativeTransport: true,
          webrtc: false,
          audio: true,
          gamepad: false
        }),
        { status: 200, headers: { 'content-type': 'application/json' } }
      )
    );
    vi.stubGlobal('fetch', fetchMock);
    const client = new ApiClient('https://sanser.example', () => Promise.resolve('access-token'));

    const device = await client.offlineDevice('device-id');

    const [url, init] = fetchMock.mock.calls[0] ?? [];
    const body = typeof init?.body === 'string' ? init.body : '';
    expect(url).toBe('https://sanser.example/api/v2/devices/offline');
    expect(init?.method).toBe('POST');
    expect(JSON.parse(body)).toEqual({ deviceId: 'device-id' });
    expect(device.online).toBe(false);
  });

  it('keeps native session credentials in memory without logging the token', async () => {
    const nativeToken = 'snv2_ephemeral_token_that_must_not_be_logged';
    const logSpy = vi.spyOn(console, 'log').mockImplementation(() => undefined);
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    const fetchMock = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(
        JSON.stringify({
          sessionId: 'session/id',
          deviceId: 'device id',
          peerDeviceId: 'peer-id',
          peerRouteAddress: '192.0.2.10',
          basePort: 49_000,
          expiresAt: 1_900_000_000,
          sessionToken: nativeToken
        }),
        { status: 200, headers: { 'content-type': 'application/json', 'cache-control': 'no-store' } }
      )
    );
    vi.stubGlobal('fetch', fetchMock);
    const client = new ApiClient('https://sanser.example', () => Promise.resolve('access-token'));

    const credentials = await client.sessionCredentials('session/id', 'device id');

    const [url, init] = fetchMock.mock.calls[0] ?? [];
    expect(url).toBe('https://sanser.example/api/v2/sessions/session%2Fid/credentials?deviceId=device%20id');
    expect(init?.method ?? 'GET').toBe('GET');
    expect(init?.body).toBeUndefined();
    expect(credentials.sessionToken).toBe(nativeToken);
    expect(logSpy).not.toHaveBeenCalled();
    expect(warnSpy).not.toHaveBeenCalled();
    expect(errorSpy).not.toHaveBeenCalled();
  });
});


describe('device registration compatibility', () => {
  const registration: DeviceRegistration = {
    id: 'stable-client-id', name: 'This Mac', platform: 'macos-aarch64',
    osVersion: 'macos-aarch64', gpu: 'Not reported', version: SANSER_VERSION,
    protocolVersion: PROTOCOL_VERSION, deviceRole: 'client', crossPlatform: true,
    codecs: ['h264', 'hevc'], nativeTransport: true, webRtc: false,
    audio: true, gamepad: false, routeAddress: '192.0.2.10'
  };
  const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), {
    status, headers: { 'content-type': 'application/json' }
  });
  const schemaError = (field: string) => json({ error: {
    code: 'invalid_json', message: `invalid JSON payload: unknown field \`${field}\`, expected one of id, name, platform`
  } }, 422);
  const client = () => new ApiClient('https://sanser.example', () => Promise.resolve('access-token'));

  it('preserves explicit roles and capabilities on an updated server', async () => {
    const fetchMock = vi.fn<typeof fetch>().mockResolvedValueOnce(json(registration));
    vi.stubGlobal('fetch', fetchMock);
    const device = await client().registerDevice(registration);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock.mock.calls[0]?.[1]?.body).toBe(JSON.stringify(registration));
    expect(device.deviceRole).toBe('client');
    expect(device.crossPlatform).toBe(true);
  });

  it.each(['deviceRole', 'crossPlatform'])('retries a rejected %s field with the same identity and legacy fields', async (field) => {
    const fetchMock = vi.fn<typeof fetch>()
      .mockResolvedValueOnce(schemaError(field))
      .mockResolvedValueOnce(json({ id: registration.id, platform: registration.platform, online: true }));
    vi.stubGlobal('fetch', fetchMock);
    const device = await client().registerDevice(registration);
    expect(fetchMock).toHaveBeenCalledTimes(2);
    const body = fetchMock.mock.calls[1]?.[1]?.body;
    const expected: Partial<DeviceRegistration> = { ...registration };
    delete expected.deviceRole;
    delete expected.crossPlatform;
    expect(body).toBe(JSON.stringify(expected));
    expect(registration.deviceRole).toBe('client');
    expect(device.online).toBe(true);
    expect(device.crossPlatform).toBe(false);
  });

  it('allows legacy Windows host registration', async () => {
    const host = { ...registration, deviceRole: 'host' as const, platform: 'windows-x86_64' };
    const fetchMock = vi.fn<typeof fetch>().mockResolvedValueOnce(schemaError('deviceRole'))
      .mockResolvedValueOnce(json({ id: host.id, platform: host.platform }));
    vi.stubGlobal('fetch', fetchMock);
    await client().registerDevice(host);
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it.each([
    { deviceRole: 'host' as const, platform: 'macos-aarch64' },
    { deviceRole: 'client' as const, platform: 'windows-x86_64' }
  ])('requires a server update for $platform $deviceRole', async (role) => {
    const fetchMock = vi.fn<typeof fetch>().mockResolvedValueOnce(schemaError('deviceRole'));
    vi.stubGlobal('fetch', fetchMock);
    await expect(client().registerDevice({ ...registration, ...role }))
      .rejects.toMatchObject({ code: 'server_upgrade_required' });
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it.each([
    [401, 'Unauthenticated'], [500, 'Internal error'], [422, 'unknown field `unrelated`']
  ])('does not retry unrelated registration failures (%s)', async (status, message) => {
    const fetchMock = vi.fn<typeof fetch>().mockResolvedValueOnce(json({ error: { message } }, status));
    vi.stubGlobal('fetch', fetchMock);
    await expect(client().registerDevice(registration)).rejects.toMatchObject({ status });
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it('does not retry a network failure', async () => {
    const fetchMock = vi.fn<typeof fetch>().mockRejectedValueOnce(new TypeError('Failed to fetch'));
    vi.stubGlobal('fetch', fetchMock);
    await expect(client().registerDevice(registration)).rejects.toMatchObject({ code: 'network_error' });
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it('reports a failed fallback without retrying indefinitely', async () => {
    const fetchMock = vi.fn<typeof fetch>().mockResolvedValueOnce(schemaError('deviceRole'))
      .mockResolvedValueOnce(json({ error: { message: 'Service unavailable' } }, 503));
    vi.stubGlobal('fetch', fetchMock);
    await expect(client().registerDevice(registration)).rejects.toMatchObject({ status: 503 });
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });
});
