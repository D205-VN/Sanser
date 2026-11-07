import { afterEach, expect, it, vi } from 'vitest';
import { LabAccount, serverUrl, type SharingChoice } from './account';
const group = '11111111-1111-4111-8111-111111111111', host = '22222222-2222-4222-8222-222222222222';
const viewer = '33333333-3333-4333-8333-333333333333';
const choice: SharingChoice = { groupId: group, deviceId: host, label: 'test' };
const auth = { accessToken: 'sn2_a_test', refreshToken: 'sn2_r_test' };
const response = (data: unknown, status = 200) => new Response(JSON.stringify(data), { status });
const accounts: LabAccount[] = [];
function account(request: ReturnType<typeof vi.fn>): LabAccount { const a = new LabAccount(request as typeof fetch);accounts.push(a);return a; }
afterEach(() => { for (const a of accounts.splice(0)) a.logout();vi.restoreAllMocks(); });

it('accepts trusted HTTPS/loopback without embedded credentials or plaintext remote URLs', () => {
  expect(serverUrl('https://sanser.example.test/')).toBe('https://sanser.example.test');
  expect(serverUrl('http://127.0.0.1:5174')).toBe('http://127.0.0.1:5174');
  for (const url of ['http://example.test', 'https://user:password@example.test', 'https://example.test?token=x', 'https://example.test#token']) expect(() => serverUrl(url)).toThrow();
});
it('uses an isolated login and authenticated account routes without persisting tokens', async () => {
  const request = vi.fn().mockResolvedValueOnce(response(auth)).mockResolvedValueOnce(response({ items: [{ id: group, hostDeviceId: host, state: 'active', members: [
    { state: 'invited', viewerDeviceId: viewer }, { state: 'accepted', viewerDeviceId: viewer }, { state: 'removed', viewerDeviceId: host }
  ] }] })).mockResolvedValueOnce(response({ provider: 'livekit-lab', roomRevision: 'r'.repeat(43), url: 'wss://voice.test', token: 'header.payload.signature' })).mockResolvedValue(response({}));
  const store = vi.spyOn(Storage.prototype, 'setItem'), a = account(request);
  await a.login('https://sanser.test', 'test@example.test', 'password');
  expect(a.signedIn).toBe(true);
  const options = request.mock.calls[0]?.[1] as RequestInit;
  expect(JSON.parse(String(options.body))).toMatchObject({ deviceName: 'Sanser Voice Lab', platform: 'browser-voice-lab', installationId: expect.any(String) });
  expect(options.redirect).toBe('error');expect(options.credentials).toBe('omit');
  expect(await a.choices()).toHaveLength(2);
  expect(await a.grant(choice)).toEqual({ url: 'wss://voice.test', token: 'header.payload.signature', roomRevision: 'r'.repeat(43) });
  expect(request.mock.calls[2]?.[0]).toBe(`https://sanser.test/api/v2/sharing/${group}/voice-lab/credentials`);
  expect(request.mock.calls[2]?.[1]).toMatchObject({ headers: { Authorization: 'Bearer sn2_a_test' }, body: JSON.stringify({ deviceId: host }) });
  a.logout();expect(a.signedIn).toBe(false);expect(store).not.toHaveBeenCalled();
  expect(request.mock.calls.at(-1)?.[0]).toBe('https://sanser.test/api/v2/auth/logout');
});
it('ignores a late login after logout even if the server/fetch cannot abort', async () => {
  let resolve!: (value: Response) => void;
  const request = vi.fn(() => new Promise<Response>((r) => { resolve = r; })), a = account(request);
  const joining = a.login('https://sanser.test', 'test@example.test', 'password');
  a.logout();resolve(response(auth));await expect(joining).rejects.toMatchObject({ code: 'cancelled' });expect(a.signedIn).toBe(false);
  await expect(a.grant(choice)).rejects.toMatchObject({ code: 'login' });expect(request).toHaveBeenCalledOnce();
});
it('cancels pending admission and does not expose raw server errors or allow caller-selected rooms', async () => {
  let resolve!: (value: Response) => void;
  const request = vi.fn().mockResolvedValueOnce(response(auth)).mockImplementationOnce(() => new Promise<Response>((r) => { resolve = r; })).mockResolvedValue(response({ error: 'sensitive error' }, 403));
  const a = account(request);await a.login('https://sanser.test', 'test@example.test', 'password');
  const grant = a.grant(choice);a.cancelRequests();resolve(response({ provider: 'livekit-lab', roomRevision: 'r'.repeat(43), url: 'wss://voice.test', token: 'header.payload.signature' }));
  await expect(grant).rejects.toMatchObject({ code: 'cancelled' });
  await expect(a.grant(choice)).rejects.toMatchObject({ code: 'membership', message: 'membership' });
  await expect(a.grant({ ...choice, groupId: '../../admin' })).rejects.toMatchObject({ code: 'membership' });
});
it('times out unresponsive admission without retaining a valid result', async () => {
  vi.useFakeTimers();
  const request = vi.fn().mockResolvedValueOnce(response(auth)).mockImplementation((_url: string, options: RequestInit) => new Promise<Response>((_resolve, reject) => options.signal?.addEventListener('abort', () => reject(new Error('aborted')))));
  const a = account(request);await a.login('https://sanser.test', 'test@example.test', 'password');
  const result = a.grant(choice).catch((error: unknown) => error);
  await vi.advanceTimersByTimeAsync(8000);expect(await result).toMatchObject({ code: 'unavailable' });a.logout();
  await vi.advanceTimersByTimeAsync(8000);vi.useRealTimers();
});
it('rejects an old server response without a room revision instead of joining without the fence', async () => {
  const request = vi.fn().mockResolvedValueOnce(response(auth)).mockResolvedValue(response({ provider: 'livekit-lab', url: 'wss://voice.test', token: 'header.payload.signature' }));
  const a = account(request);await a.login('https://sanser.test', 'test@example.test', 'password');
  await expect(a.grant(choice)).rejects.toMatchObject({ code: 'unavailable' });
});
