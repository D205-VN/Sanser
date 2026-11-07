import { afterEach, beforeEach, expect, it, vi } from 'vitest';
const fake = vi.hoisted(() => {
  class Room {
    static last: Room;
    handlers = new Map<string, ((...args: unknown[]) => void)[]>();
    remoteParticipants = new Map<string, { trackPublications: Map<string, unknown> }>();
    canPlaybackAudio = true;
    connect = vi.fn(async (): Promise<void> => undefined);
    disconnect = vi.fn(async (): Promise<void> => undefined);
    startAudio = vi.fn(async (): Promise<void> => undefined);
    localParticipant = { publishTrack: vi.fn(async (): Promise<void> => undefined), unpublishTrack: vi.fn(async (): Promise<void> => undefined) };
    constructor(public options: unknown) { Room.last = this; }
    on(event: string, callback: (...args: unknown[]) => void) { this.handlers.set(event, [...(this.handlers.get(event) ?? []), callback]); }
    emit(event: string, ...args: unknown[]) { for (const callback of this.handlers.get(event) ?? []) callback(...args); }
    removeAllListeners() { this.handlers.clear(); }
  }
  class RemoteAudioTrack {
    element = document.createElement('audio');attach = vi.fn(() => this.element);detach = vi.fn(() => []);
    getRTCStatsReport = vi.fn(async () => new Map([['secret-id', { id: 'secret-id', type: 'inbound-rtp', kind: 'audio', packetsReceived: 1 }]]));
  }
  return { Room, RemoteAudioTrack, capture: vi.fn(), setLogLevel: vi.fn() };
});
vi.mock('livekit-client', () => ({
  Room: fake.Room, RemoteAudioTrack: fake.RemoteAudioTrack, createLocalAudioTrack: fake.capture,
  Track: { Kind: { Audio: 'audio', Video: 'video' }, Source: { Microphone: 'microphone' } },
  RoomEvent: { Reconnecting: 'reconnecting', SignalReconnecting: 'signal-reconnecting', Reconnected: 'connected', Disconnected: 'disconnected',
    ParticipantConnected: 'members', ParticipantDisconnected: 'left', TrackPublished: 'published', TrackUnpublished: 'unpublished',
    TrackSubscribed: 'subscribed', TrackUnsubscribed: 'unsubscribed', AudioPlaybackStatusChanged: 'playback' },
  AudioPresets: { speech: { maxBitrate: 20000 } }, LogLevel: { silent: 5 }, setLogLevel: fake.setLogLevel
}));
import { LiveKitVoiceRoom } from './livekit';
const publication = (kind = 'audio', source = 'microphone') => ({ kind, source, setSubscribed: vi.fn() });
function localTrack() {
  const mediaStreamTrack = new EventTarget() as EventTarget & { readyState: string };mediaStreamTrack.readyState = 'live';
  return { mediaStreamTrack, stop: vi.fn(() => { mediaStreamTrack.readyState = 'ended'; }), getRTCStatsReport: vi.fn(async () => new Map()) };
}
beforeEach(() => { fake.capture.mockReset(); });
afterEach(() => { vi.unstubAllGlobals(); });
it('disables auto subscription, never captures on connect, and subscribes only microphone audio', async () => {
  const adapter = new LiveKitVoiceRoom(document.createElement('div')), room = fake.Room.last;
  const mic = publication(), camera = publication('video', 'camera'), system = publication('audio', 'screen_share_audio');
  room.remoteParticipants.set('private-person', { trackPublications: new Map([['mic', mic], ['camera', camera], ['system', system]]) });
  await adapter.connect('wss://example.test', 'private-token');
  expect(room.connect).toHaveBeenCalledWith('wss://example.test', 'private-token', { autoSubscribe: false });
  expect(fake.capture).not.toHaveBeenCalled();expect(mic.setSubscribed).toHaveBeenCalledWith(true);
  expect(camera.setSubscribed).toHaveBeenCalledWith(false);expect(system.setSubscribed).toHaveBeenCalledWith(false);
  expect(fake.setLogLevel).toHaveBeenCalledWith(5);
  Object.assign(room.localParticipant, { permissions: { canPublish: false } });
  await expect(adapter.createMicrophone()).rejects.toBeInstanceOf(DOMException);expect(fake.capture).not.toHaveBeenCalled();
  await adapter.disconnect();
});
it('owns remote elements, mutes playback on deafen, bounds tracks and detaches on disconnect', async () => {
  const sink = document.createElement('div'), adapter = new LiveKitVoiceRoom(sink), room = fake.Room.last;
  for (let i = 0; i < 4; i++) { const pub = publication(), track = new fake.RemoteAudioTrack();room.emit('published', pub);room.emit('subscribed', track, pub); }
  expect(sink.children.length).toBe(3);adapter.setDeafened(true);
  expect(Array.from(sink.querySelectorAll('audio')).every((audio) => audio.muted)).toBe(true);
  await adapter.disconnect();expect(sink.children.length).toBe(0);expect(room.disconnect).toHaveBeenCalledWith(true);
  expect(room.handlers.size).toBe(0);
});
it('uses explicit mono speech capture, closes hardware immediately and rejects post-stop publishing', async () => {
  const adapter = new LiveKitVoiceRoom(document.createElement('div')), room = fake.Room.last, track = localTrack();fake.capture.mockResolvedValue(track);
  const mic = await adapter.createMicrophone();
  expect(fake.capture).toHaveBeenCalledWith({ echoCancellation: true, noiseSuppression: true, autoGainControl: true, channelCount: 1 });
  await mic.publish();expect(room.localParticipant.publishTrack).toHaveBeenCalledOnce();
  adapter.stopMicrophones();expect(track.stop).toHaveBeenCalled();await expect(mic.publish()).rejects.toThrow('closed');
  await mic.unpublish();await adapter.disconnect();
  await expect(adapter.createMicrophone()).rejects.toThrow('closed');expect(fake.capture).toHaveBeenCalledOnce();
});
it('stops capture that resolves after disconnect and surfaces hardware-ended without raw details', async () => {
  const adapter = new LiveKitVoiceRoom(document.createElement('div')), track = localTrack();
  let finish!: (track: ReturnType<typeof localTrack>) => void;
  fake.capture.mockReturnValue(new Promise((resolve) => { finish = resolve; }));
  const capture = adapter.createMicrophone();await adapter.disconnect();finish(track);
  await expect(capture).rejects.toThrow('closed');expect(track.stop).toHaveBeenCalled();
  const second = new LiveKitVoiceRoom(document.createElement('div')), active = localTrack(), events: string[] = [];
  fake.capture.mockResolvedValue(active);second.onSignal((event) => events.push(event));await second.createMicrophone();
  active.mediaStreamTrack.dispatchEvent(new Event('ended'));expect(events).toEqual(['mic-ended']);await second.disconnect();
});
it('stops owned microphones before forwarding a reconnect signal', async () => {
  const adapter = new LiveKitVoiceRoom(document.createElement('div')), track = localTrack();fake.capture.mockResolvedValue(track);
  await adapter.createMicrophone();adapter.onSignal(() => expect(track.stop).toHaveBeenCalled());
  fake.Room.last.emit('reconnecting');await adapter.disconnect();
});
it('ignores a late ended event from a stopped mic after a replacement is captured', async () => {
  const adapter = new LiveKitVoiceRoom(document.createElement('div')), old = localTrack(), next = localTrack();
  const events: string[] = [];adapter.onSignal((event) => events.push(event));
  fake.capture.mockResolvedValueOnce(old).mockResolvedValueOnce(next);
  const previous = await adapter.createMicrophone();previous.stop();
  await adapter.createMicrophone();
  old.mediaStreamTrack.dispatchEvent(new Event('ended'));
  expect(events).toEqual([]);expect(next.stop).not.toHaveBeenCalled();
  next.mediaStreamTrack.dispatchEvent(new Event('ended'));expect(events).toEqual(['mic-ended']);
  await adapter.disconnect();
});
it('retires consent before a device emits ended synchronously during stop', async () => {
  const adapter = new LiveKitVoiceRoom(document.createElement('div')), track = localTrack(), events: string[] = [];
  adapter.onSignal((event) => events.push(event));fake.capture.mockResolvedValue(track);
  await adapter.createMicrophone();
  track.stop.mockImplementation(() => { track.mediaStreamTrack.readyState = 'ended';track.mediaStreamTrack.dispatchEvent(new Event('ended')); });
  adapter.stopMicrophones();expect(events).toEqual([]);
  await adapter.disconnect();
});
it('recovers playback on a device change without opening a microphone and cleans up its listener', async () => {
  const devices = new EventTarget();vi.stubGlobal('navigator', { mediaDevices: devices });
  const adapter = new LiveKitVoiceRoom(document.createElement('div')), room = fake.Room.last;
  const events: string[] = [];adapter.onSignal((event) => events.push(event));
  await adapter.connect('wss://example.test', 'private-token');
  devices.dispatchEvent(new Event('devicechange'));await Promise.resolve();
  expect(room.startAudio).toHaveBeenCalledOnce();expect(fake.capture).not.toHaveBeenCalled();
  room.startAudio.mockRejectedValue(new Error('private device'));
  devices.dispatchEvent(new Event('devicechange'));await Promise.resolve();await Promise.resolve();
  expect(events).toContain('playback-blocked');expect(events).not.toContain('private device');
  await adapter.disconnect();devices.dispatchEvent(new Event('devicechange'));
  expect(room.startAudio).toHaveBeenCalledTimes(2);expect(fake.capture).not.toHaveBeenCalled();
});
it('survives duplicate subscribed events and reselects a mic replacement after a full old slot drains', async () => {
  const sink = document.createElement('div'), adapter = new LiveKitVoiceRoom(sink), room = fake.Room.last;
  const pubs = [publication(), publication(), publication()], tracks = pubs.map(() => new fake.RemoteAudioTrack());
  const roster = new Map<string, unknown>(pubs.map((pub, i) => [String(i), pub]));room.remoteParticipants.set('private', { trackPublications: roster });
  await adapter.connect('wss://example.test', 'private-token');
  pubs.forEach((pub, i) => room.emit('subscribed', tracks[i], pub));
  room.emit('subscribed', tracks[0], pubs[0]);expect(sink.children.length).toBe(3);
  expect(pubs[0]?.setSubscribed).not.toHaveBeenCalledWith(false);
  const next = publication(), replacement = new fake.RemoteAudioTrack();roster.delete('0');roster.set('new', next);
  room.emit('published', next);room.emit('unpublished', pubs[0]);room.emit('subscribed', replacement, next);
  room.emit('unsubscribed', tracks[0], pubs[0]);room.emit('subscribed', replacement, next);
  expect(sink.children.length).toBe(3);expect(sink.contains(replacement.element)).toBe(true);
  await adapter.disconnect();
});
