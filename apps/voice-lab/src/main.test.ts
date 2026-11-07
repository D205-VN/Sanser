import html from '../index.html?raw';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
const fake = vi.hoisted(() => {
  class Room {
    static last: Room;
    connect = vi.fn(async (): Promise<void> => undefined);disconnect = vi.fn(async (): Promise<void> => undefined);
    stopMicrophones = vi.fn();setDeafened = vi.fn();startAudio = vi.fn(async (): Promise<void> => undefined);
    createMicrophone = vi.fn(async () => ({ publish: async () => undefined, stop: vi.fn(), unpublish: async () => undefined }));
    onSignal = vi.fn();readStats = vi.fn(async () => []);
    constructor(_sink: HTMLElement) { Room.last = this; }
  }
  return { Room };
});
vi.mock('./livekit', () => ({ LiveKitVoiceRoom: fake.Room }));
let destroy: (() => void) | undefined;
let request: ReturnType<typeof vi.fn>;
const group = '11111111-1111-4111-8111-111111111111', device = '22222222-2222-4222-8222-222222222222';
function response(value: unknown, status = 200): Response { return new Response(JSON.stringify(value), { status }); }
function button(id: string): HTMLButtonElement {
  const node = document.getElementById(id);if (!(node instanceof HTMLButtonElement)) throw new Error();return node;
}
function input(id: string): HTMLInputElement {
  const node = document.getElementById(id);if (!(node instanceof HTMLInputElement)) throw new Error();return node;
}
beforeEach(async () => {
  vi.useFakeTimers();vi.resetModules();
  request = vi.fn().mockResolvedValue(response({}));vi.stubGlobal('fetch', request);
  const body = html.match(/<body>([\s\S]*)<\/body>/)?.[1];if (!body) throw new Error();document.body.innerHTML = body;
  destroy = (await import('./main')).destroy;
});
afterEach(() => { destroy?.();vi.clearAllTimers();vi.useRealTimers();vi.unstubAllGlobals(); });
async function join() {
  input('endpoint').value = 'wss://example.test';input('token').value = 'header.payload.signature';
  document.getElementById('connect-form')?.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
  await vi.advanceTimersByTimeAsync(0);
}
it('keeps microphone disabled before join, clears the secret field and joins listening only', async () => {
  expect(button('mic').disabled).toBe(true);expect(button('leave').disabled).toBe(true);
  const persisted = vi.spyOn(Storage.prototype, 'setItem');await join();
  expect(input('token').value).toBe('');expect(button('mic').disabled).toBe(false);
  expect(fake.Room.last.createMicrophone).not.toHaveBeenCalled();expect(persisted).not.toHaveBeenCalled();
  expect(document.getElementById('room-state')?.textContent).toContain('Đang nghe');
});
it('opens mic explicitly, leaves on hidden tab and does not restore on visibility return', async () => {
  await join();button('mic').click();await vi.advanceTimersByTimeAsync(0);
  expect(button('mic').getAttribute('aria-pressed')).toBe('true');
  const visibility = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden');document.dispatchEvent(new Event('visibilitychange'));
  expect(fake.Room.last.stopMicrophones).toHaveBeenCalled();expect(fake.Room.last.disconnect).toHaveBeenCalled();
  expect(button('mic').disabled).toBe(true);visibility.mockReturnValue('visible');document.dispatchEvent(new Event('visibilitychange'));
  expect(fake.Room.last.createMicrophone).toHaveBeenCalledOnce();
});
it('destroys old controls/timers so reloading the prototype cannot create a second call', async () => {
  await join();const room = fake.Room.last;destroy?.();button('mic').click();
  document.getElementById('connect-form')?.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
  await vi.advanceTimersByTimeAsync(2000);expect(room.createMicrophone).not.toHaveBeenCalled();expect(room.readStats).not.toHaveBeenCalled();
});
async function login() {
  request.mockResolvedValueOnce(response({ accessToken: 'sn2_a_test', refreshToken: 'sn2_r_test' }))
    .mockResolvedValueOnce(response({ items: [{ id: group, hostDeviceId: device, state: 'active', members: [] }] }));
  input('sanser-server').value = 'https://sanser.test';input('email').value = 'test@example.test';input('password').value = 'password';
  document.getElementById('account-form')?.dispatchEvent(new Event('submit', { cancelable: true }));
  expect(input('password').value).toBe('');await vi.advanceTimersByTimeAsync(0);
}
function accountJoin() { document.getElementById('sharing-form')?.dispatchEvent(new Event('submit', { cancelable: true })); }
it('joins from the authenticated sharing role without pasted JWT or implicit mic consent', async () => {
  await login();expect(button('account-join').disabled).toBe(false);
  request.mockResolvedValueOnce(response({ provider: 'livekit-lab', roomRevision: 'r'.repeat(43), url: 'wss://voice.test', token: 'header.payload.signature' }));
  accountJoin();await vi.advanceTimersByTimeAsync(0);
  expect(fake.Room.last.connect).toHaveBeenCalledWith('wss://voice.test/', 'header.payload.signature');
  expect(fake.Room.last.createMicrophone).not.toHaveBeenCalled();expect(input('token').value).toBe('');
  expect(request.mock.calls[2]?.[1]).toMatchObject({ body: JSON.stringify({ deviceId: device }) });
});
it('leaving while a credential request is pending cannot open a late room', async () => {
  await login();let resolve!: (value: Response) => void;
  request.mockImplementationOnce(() => new Promise<Response>((r) => { resolve = r; }));
  const previous = fake.Room.last;accountJoin();button('leave').click();
  resolve(response({ provider: 'livekit-lab', roomRevision: 'r'.repeat(43), url: 'wss://voice.test', token: 'header.payload.signature' }));
  await vi.advanceTimersByTimeAsync(0);expect(fake.Room.last).toBe(previous);expect(button('mic').disabled).toBe(true);
});
it('stops mic and leaves when a cooperative membership check is refused', async () => {
  await login();request.mockResolvedValueOnce(response({ provider: 'livekit-lab', roomRevision: 'r'.repeat(43), url: 'wss://voice.test', token: 'header.payload.signature' }));
  accountJoin();await vi.advanceTimersByTimeAsync(0);button('mic').click();await vi.advanceTimersByTimeAsync(0);
  expect(button('mic').getAttribute('aria-pressed')).toBe('true');
  request.mockResolvedValueOnce(response({}, 403));await vi.advanceTimersByTimeAsync(10000);
  expect(fake.Room.last.stopMicrophones).toHaveBeenCalled();expect(button('mic').disabled).toBe(true);
  expect(document.getElementById('account-status')?.textContent).toContain('không còn được phép');
});
it('forgets the browser login on page exit and requests revocation without creating another call', async () => {
  await login();window.dispatchEvent(new Event('pagehide'));await vi.advanceTimersByTimeAsync(0);
  expect(request.mock.calls.at(-1)?.[0]).toBe('https://sanser.test/api/v2/auth/logout');
  expect(request.mock.calls.at(-1)?.[1]).toMatchObject({ keepalive: true, body: JSON.stringify({ refreshToken: 'sn2_r_test' }) });
  expect(document.getElementById('sharing-form')?.hidden).toBe(true);
  expect(document.getElementById('account-form')?.hidden).toBe(false);expect(button('mic').disabled).toBe(true);
});
it('an old membership response cannot terminate a replacement manual-token call', async () => {
  await login();request.mockResolvedValueOnce(response({ provider: 'livekit-lab', roomRevision: 'r'.repeat(43), url: 'wss://voice.test', token: 'header.payload.signature' }));
  accountJoin();await vi.advanceTimersByTimeAsync(0);
  let resolve!: (value: Response) => void;
  request.mockImplementationOnce(() => new Promise<Response>((r) => { resolve = r; }));
  await vi.advanceTimersByTimeAsync(10000);
  const signal = fake.Room.last.onSignal.mock.calls[0]?.[0] as ((value: string) => void) | undefined;
  signal?.('disconnected');await join();const replacement = fake.Room.last;
  resolve(response({}, 403));await vi.advanceTimersByTimeAsync(0);
  expect(button('mic').disabled).toBe(false);expect(replacement.disconnect).not.toHaveBeenCalled();
});
it('leaves and stops mic on room revision change without switching rooms or restoring consent', async () => {
  await login();request.mockResolvedValueOnce(response({ provider: 'livekit-lab', roomRevision: 'r'.repeat(43), url: 'wss://voice.test', token: 'header.payload.signature' }));
  accountJoin();await vi.advanceTimersByTimeAsync(0);button('mic').click();await vi.advanceTimersByTimeAsync(0);
  const old = fake.Room.last;
  request.mockResolvedValueOnce(response({ provider: 'livekit-lab', roomRevision: 's'.repeat(43), url: 'wss://voice.test', token: 'header.new.signature' }));
  await vi.advanceTimersByTimeAsync(10000);
  expect(old.stopMicrophones).toHaveBeenCalled();expect(old.disconnect).toHaveBeenCalledOnce();
  expect(button('mic').disabled).toBe(true);expect(old.connect).toHaveBeenCalledOnce();
  expect(document.getElementById('account-status')?.textContent).toContain('phòng mới');
});
it('does not interrupt voice when the room revision is unchanged even if the signed token is refreshed', async () => {
  await login();request.mockResolvedValueOnce(response({ provider: 'livekit-lab', roomRevision: 'r'.repeat(43), url: 'wss://voice.test', token: 'header.payload.signature' }));
  accountJoin();await vi.advanceTimersByTimeAsync(0);const old = fake.Room.last;
  request.mockResolvedValueOnce(response({ provider: 'livekit-lab', roomRevision: 'r'.repeat(43), url: 'wss://voice.test', token: 'header.new.signature' }));
  await vi.advanceTimersByTimeAsync(10000);
  expect(old.disconnect).not.toHaveBeenCalled();expect(button('mic').disabled).toBe(false);
});
