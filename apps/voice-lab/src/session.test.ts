import { afterEach, expect, it, vi } from 'vitest';
import { VoiceSession, connectionInput, type Microphone, type VoiceRoom, type RoomSignal } from './session';
const token = 'header.payload.signature';
function deferred<T>() { let resolve!: (value: T) => void;let reject!: (value: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes;reject = no; });return { promise, resolve, reject }; }
class Mic implements Microphone {
  publish = vi.fn(async (): Promise<void> => undefined);stop = vi.fn();unpublish = vi.fn(async (): Promise<void> => undefined);
}
class Room implements VoiceRoom {
  signal: (event: RoomSignal, members?: number) => void = () => undefined;
  connect = vi.fn(async (_url: string, _token: string): Promise<void> => undefined);
  disconnect = vi.fn(async (): Promise<void> => undefined);
  createMicrophone = vi.fn(async (): Promise<Microphone> => new Mic());
  stopMicrophones = vi.fn();setDeafened = vi.fn();startAudio = vi.fn(async (): Promise<void> => undefined);
  onSignal(fn: typeof this.signal) { this.signal = fn; }
}
afterEach(() => vi.useRealTimers());
it('validates explicit endpoints/tokens without echoing or persisting them', () => {
  expect(connectionInput(' wss://example.test ', token).url).toBe('wss://example.test/');
  expect(connectionInput('ws://127.0.0.1:7880', token).url).toContain(':7880');
  for (const url of ['https://example.test', 'ws://remote.test', 'wss://user:secret@example.test', 'wss://example.test?token=secret', 'wss://example.test/#secret']) {
    expect(() => connectionInput(url, token)).toThrow('invalid-config');
  }
  expect(() => connectionInput('wss://example.test', 'secret')).toThrow('invalid-config');
});
it('joins listen-only and opens/publishes a mic only on explicit unmute', async () => {
  const room = new Room(), session = new VoiceSession(() => room), mic = new Mic();room.createMicrophone.mockResolvedValue(mic);
  await session.join('wss://example.test', token);
  expect(session.state.joined).toBe(true);expect(session.state.microphone).toBe(false);
  expect(room.createMicrophone).not.toHaveBeenCalled();
  await session.setMicrophone(true);expect(mic.publish).toHaveBeenCalledOnce();expect(session.state.microphone).toBe(true);
  session.setDeafened(true);expect(mic.stop).toHaveBeenCalled();expect(session.state.microphone).toBe(false);
  session.setDeafened(false);expect(room.createMicrophone).toHaveBeenCalledOnce();
  session.leave();expect(session.state.joined).toBe(false);expect(room.disconnect).toHaveBeenCalled();
});
it('stops delayed capture after Leave and never publishes it into a replacement room', async () => {
  const old = new Room(), next = new Room(), capture = deferred<Microphone>(), mic = new Mic();
  old.createMicrophone.mockReturnValue(capture.promise);
  const rooms = [old, next], session = new VoiceSession(() => { const room = rooms.shift();if (!room) throw new Error();return room; });
  await session.join('wss://example.test', token);const pending = session.setMicrophone(true);
  session.leave();await session.join('wss://example.test', token);capture.resolve(mic);await pending;
  expect(mic.stop).toHaveBeenCalled();expect(mic.publish).not.toHaveBeenCalled();expect(session.state.joined).toBe(true);
  expect(session.state.microphone).toBe(false);expect(session.state.micPending).toBe(false);session.leave();
});
it('cancels a pending publish immediately and closes it again when the publish finishes', async () => {
  const room = new Room(), session = new VoiceSession(() => room), mic = new Mic(), publish = deferred<void>();
  room.createMicrophone.mockResolvedValue(mic);mic.publish.mockReturnValue(publish.promise);
  await session.join('wss://example.test', token);const pending = session.setMicrophone(true);await Promise.resolve();
  await session.setMicrophone(false);expect(mic.stop).toHaveBeenCalled();expect(session.state.microphone).toBe(false);
  publish.resolve();await pending;expect(mic.unpublish).toHaveBeenCalled();expect(session.state.microphone).toBe(false);session.leave();
});
it('serializes microphone prompts and cancels delayed capture on deafen', async () => {
  const room = new Room(), session = new VoiceSession(() => room), capture = deferred<Microphone>(), mic = new Mic();
  room.createMicrophone.mockReturnValue(capture.promise);await session.join('wss://example.test', token);
  const pending = session.setMicrophone(true);await session.setMicrophone(true);expect(room.createMicrophone).toHaveBeenCalledOnce();
  session.setDeafened(true);capture.resolve(mic);await pending;
  expect(mic.stop).toHaveBeenCalled();expect(mic.publish).not.toHaveBeenCalled();session.leave();
});
it('bounds a hanging mic prompt and stops a late grant without publishing or muting a retry', async () => {
  vi.useFakeTimers();const room = new Room(), session = new VoiceSession(() => room), capture = deferred<Microphone>();
  const stale = new Mic(), current = new Mic();room.createMicrophone.mockReturnValueOnce(capture.promise).mockResolvedValueOnce(current);
  await session.join('wss://example.test', token);
  const pending = session.setMicrophone(true);await vi.advanceTimersByTimeAsync(15_001);await pending;
  expect(session.state.micPending).toBe(false);expect(session.state.microphone).toBe(false);
  expect(session.state.error).toBe('microphone-failed');
  await session.setMicrophone(true);expect(session.state.microphone).toBe(true);
  capture.resolve(stale);await Promise.resolve();await Promise.resolve();
  expect(stale.stop).toHaveBeenCalled();expect(stale.publish).not.toHaveBeenCalled();
  expect(current.stop).not.toHaveBeenCalled();expect(session.state.microphone).toBe(true);session.leave();
});
it('bounds a stalled mic publish, stops capture immediately and does not revive consent on completion', async () => {
  vi.useFakeTimers();const room = new Room(), session = new VoiceSession(() => room), mic = new Mic(), publish = deferred<void>();
  mic.publish.mockReturnValue(publish.promise);room.createMicrophone.mockResolvedValue(mic);
  await session.join('wss://example.test', token);const pending = session.setMicrophone(true);
  await vi.advanceTimersByTimeAsync(15_001);await pending;
  expect(mic.stop).toHaveBeenCalled();expect(session.state.micPending).toBe(false);
  const previous = mic.unpublish.mock.calls.length;
  expect(session.state.microphone).toBe(false);publish.resolve();await Promise.resolve();await Promise.resolve();
  expect(mic.unpublish.mock.calls.length).toBeGreaterThan(previous);
  expect(session.state.microphone).toBe(false);session.leave();
});
it('fails closed on reconnect, ignores old signals, and never restores microphone consent', async () => {
  const room = new Room(), session = new VoiceSession(() => room);await session.join('wss://example.test', token);
  await session.setMicrophone(true);room.signal('reconnecting');
  expect(room.stopMicrophones).toHaveBeenCalled();expect(room.disconnect).toHaveBeenCalled();
  expect(session.state.joined).toBe(false);expect(session.state.error).toBe('connection-lost');
  room.signal('connected');room.signal('members', 4);expect(session.state.members).toBe(0);expect(session.state.microphone).toBe(false);
});
it('closes a late connection after cancellation/timeout without overwriting a later attempt', async () => {
  vi.useFakeTimers();const room = new Room(), next = new Room(), rooms = [room, next], connect = deferred<void>();
  const session = new VoiceSession(() => { const value = rooms.shift();if (!value) throw new Error();return value; });room.connect.mockReturnValue(connect.promise);
  const pending = session.join('wss://example.test', token);await vi.advanceTimersByTimeAsync(15_001);await pending;
  expect(session.state.error).toBe('connection-failed');expect(room.disconnect).toHaveBeenCalled();
  await session.join('wss://example.test', token);expect(session.state.joined).toBe(true);
  const before = room.disconnect.mock.calls.length;connect.resolve();await Promise.resolve();await Promise.resolve();
  expect(room.disconnect.mock.calls.length).toBeGreaterThan(before);expect(session.state.joined).toBe(true);expect(next.disconnect).not.toHaveBeenCalled();session.leave();
});
it('does not display raw connection/mic errors and supports explicit autoplay recovery', async () => {
  const room = new Room(), session = new VoiceSession(() => room);room.startAudio.mockRejectedValue(new Error('private-token'));
  await session.join('wss://example.test', token);expect(session.state.playbackBlocked).toBe(true);
  room.startAudio.mockResolvedValue();await session.resumeAudio();expect(session.state.playbackBlocked).toBe(false);
  room.createMicrophone.mockRejectedValue(new DOMException('private-token', 'NotAllowedError'));
  await session.setMicrophone(true);expect(session.state.error).toBe('microphone-denied');
  expect(JSON.stringify(session.state)).not.toContain('private');session.leave();
  const rejected = new Room(), failed = new VoiceSession(() => rejected);rejected.connect.mockRejectedValue(new Error('private-token'));
  await failed.join('wss://example.test', token);expect(failed.state.error).toBe('connection-failed');expect(JSON.stringify(failed.state)).not.toContain('private');
});
it('refuses oversized rooms, stops ended microphones, and never opens a mic after leave', async () => {
  const room = new Room(), session = new VoiceSession(() => room);await session.join('wss://example.test', token);
  await session.setMicrophone(true);room.signal('mic-ended');expect(session.state.microphone).toBe(false);
  room.signal('capacity');expect(session.state.error).toBe('room-capacity');
  await session.setMicrophone(true);expect(room.createMicrophone).toHaveBeenCalledOnce();
});
