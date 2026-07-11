import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiClient, normalizeServerUrl } from './api';

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
    await client.createSession('host-id', 'requester-id', 'auto', 'balanced');

    const heartbeatInit = fetchMock.mock.calls[0]?.[1];
    const sessionInit = fetchMock.mock.calls[1]?.[1];
    const heartbeatBody = typeof heartbeatInit?.body === 'string' ? heartbeatInit.body : '';
    const sessionBody = typeof sessionInit?.body === 'string' ? sessionInit.body : '';
    expect(JSON.parse(heartbeatBody)).toEqual({ deviceId: 'requester-id', streaming: false });
    expect(JSON.parse(sessionBody)).toEqual({
      hostDeviceId: 'host-id',
      requesterDeviceId: 'requester-id',
      networkMode: 'auto',
      qualityProfile: 'balanced'
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
