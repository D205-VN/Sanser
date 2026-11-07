import { Room, RoomEvent, Track, RemoteAudioTrack, createLocalAudioTrack, AudioPresets, LogLevel, setLogLevel,
  type LocalAudioTrack, type RemoteTrackPublication } from 'livekit-client';
import type { VoiceRoom, Microphone, RoomSignal } from './session';
import type { StatsSource } from './stats';

// SDK diagnostics can contain room/identity/endpoint context. The lab exports
// only its own numeric projection and never renders raw SDK exceptions.
setLogLevel(LogLevel.silent);
export class LiveKitVoiceRoom implements VoiceRoom {
  private room = new Room({ adaptiveStream: false, dynacast: false,
    publishDefaults: { audioPreset: AudioPresets.speech, dtx: true, stopMicTrackOnMute: true } });
  private closed = false;
  private deafened = false;
  private notify: (signal: RoomSignal, members?: number) => void = () => undefined;
  private wanted = new Set<RemoteTrackPublication>();
  private audio = new Map<RemoteAudioTrack, HTMLAudioElement>();
  private owned = new Set<LocalAudioTrack>();
  private statsKeys = new WeakMap<object, object>();
  private devices?: MediaDevices;
  private recoverPlayback = () => {
    // Output changes may invalidate autoplay/routing. Retry playback only;
    // never restart microphone capture or restore revoked mic consent.
    if (this.closed) return;
    void this.startAudio().catch(() => { if (!this.closed) this.notify('playback-blocked'); });
  };
  constructor(private sink: HTMLElement) {
    this.room.on(RoomEvent.Reconnecting, () => { this.stopMicrophones();this.notify('reconnecting'); });
    this.room.on(RoomEvent.SignalReconnecting, () => { this.stopMicrophones();this.notify('reconnecting'); });
    this.room.on(RoomEvent.Reconnected, () => { if (!this.closed) this.notify('connected'); });
    this.room.on(RoomEvent.Disconnected, () => { if (!this.closed) this.notify('disconnected'); });
    const members = () => {
      if (this.closed) return;
      const size = this.room.remoteParticipants.size + 1;
      this.notify(size > 4 ? 'capacity' : 'members', size);
    };
    this.room.on(RoomEvent.ParticipantConnected, members);this.room.on(RoomEvent.ParticipantDisconnected, members);
    this.room.on(RoomEvent.TrackPublished, (publication) => this.select(publication));
    this.room.on(RoomEvent.TrackUnpublished, (publication) => { this.wanted.delete(publication);this.refreshSubscriptions(); });
    this.room.on(RoomEvent.TrackSubscribed, (track, publication) => {
      if (track instanceof RemoteAudioTrack && this.audio.has(track)) return;
      if (this.closed || !this.wanted.has(publication) || !(track instanceof RemoteAudioTrack) || this.audio.size >= 3) { this.wanted.delete(publication);publication.setSubscribed(false);return; }
      const element = track.attach();element.autoplay = true;element.muted = this.deafened;
      element.setAttribute('playsinline', '');this.audio.set(track, element);this.sink.append(element);
    });
    this.room.on(RoomEvent.TrackUnsubscribed, (track, publication) => {
      if (track instanceof RemoteAudioTrack) { this.audio.get(track)?.remove();this.audio.delete(track);track.detach(); }
      this.wanted.delete(publication);this.refreshSubscriptions();
    });
    this.room.on(RoomEvent.AudioPlaybackStatusChanged, () => { if (!this.closed && !this.room.canPlaybackAudio) this.notify('playback-blocked'); });
  }
  private refreshSubscriptions(): void {
    if (this.closed || this.audio.size >= 3) return;
    for (const participant of this.room.remoteParticipants.values()) for (const publication of participant.trackPublications.values()) this.select(publication);
  }
  private select(publication: RemoteTrackPublication): void {
    if (this.closed || this.wanted.has(publication)) return;
    if (publication.kind !== Track.Kind.Audio || publication.source !== Track.Source.Microphone || this.wanted.size >= 3 || this.audio.size >= 3) { publication.setSubscribed(false);return; }
    this.wanted.add(publication);publication.setSubscribed(true);
  }
  onSignal(callback: (signal: RoomSignal, members?: number) => void): void { this.notify = callback; }
  async connect(url: string, token: string): Promise<void> {
    if (this.closed) throw new Error('closed');
    await this.room.connect(url, token, { autoSubscribe: false });
    if (this.closed) { await this.room.disconnect(true);return; }
    this.devices = navigator.mediaDevices;
    this.devices?.addEventListener('devicechange', this.recoverPlayback);
    const size = this.room.remoteParticipants.size + 1;
    if (size > 4) { this.notify('capacity');return; }
    this.notify('members', size);
    this.refreshSubscriptions();
  }
  async createMicrophone(): Promise<Microphone> {
    if (this.closed) throw new Error('closed');
    if (this.room.localParticipant.permissions?.canPublish === false) throw new DOMException('Microphone publishing unavailable', 'NotAllowedError');
    // This is the only capture call. The controller invokes it only after an
    // explicit Unmute, never Join/connect/reconnect or visibility restoration.
    const track = await createLocalAudioTrack({ echoCancellation: true, noiseSuppression: true, autoGainControl: true, channelCount: 1 });
    this.owned.add(track);
    track.mediaStreamTrack.addEventListener('ended', () => {
      // A late hardware event from a deliberately retired track must not mute
      // the replacement microphone. Ownership ends synchronously on stop.
      if (!this.closed && this.owned.has(track)) this.notify('mic-ended');
    }, { once: true });
    if (this.closed) { track.stop();this.owned.delete(track);throw new Error('closed'); }
    let stopped = false;
    return {
      publish: async () => {
        if (this.closed || stopped || track.mediaStreamTrack.readyState === 'ended') throw new Error('closed');
        await this.room.localParticipant.publishTrack(track, { source: Track.Source.Microphone, audioPreset: AudioPresets.speech, dtx: true });
      },
      stop: () => { stopped = true;this.owned.delete(track);track.stop(); },
      unpublish: async () => { track.stop();try { await this.room.localParticipant.unpublishTrack(track, true); } finally { this.owned.delete(track); } }
    };
  }
  stopMicrophones(): void {
    const tracks = Array.from(this.owned);this.owned.clear();
    for (const track of tracks) {
      track.stop();void this.room.localParticipant.unpublishTrack(track, true).catch(() => undefined);
    }
  }
  setDeafened(value: boolean): void { this.deafened = value;for (const element of this.audio.values()) element.muted = value; }
  async startAudio(): Promise<void> { await this.room.startAudio();if (!this.room.canPlaybackAudio) throw new Error('playback-blocked'); }
  async readStats(): Promise<StatsSource[]> {
    const sources = [
      ...Array.from(this.audio.keys()).map((track) => ({ key: track, direction: 'receive' as const, track })),
      ...Array.from(this.owned).filter((track) => track.mediaStreamTrack.readyState !== 'ended').slice(0, 1).map((track) => ({ key: track, direction: 'send' as const, track }))
    ].slice(0, 4);
    const results = await Promise.allSettled(sources.map(async (source) => {
      const report = await source.track.getRTCStatsReport();
      let key = this.statsKeys.get(source.key);if (!key) { key = {};this.statsKeys.set(source.key, key); }
      return { key, direction: source.direction, records: report ? Array.from(report.values()) as Record<string, unknown>[] : [] };
    }));
    return results.flatMap((result) => result.status === 'fulfilled' ? [result.value] : []);
  }
  async disconnect(): Promise<void> {
    this.closed = true;this.stopMicrophones();this.notify = () => undefined;
    this.devices?.removeEventListener('devicechange', this.recoverPlayback);this.devices = undefined;
    for (const [track, element] of this.audio) { element.remove();track.detach(); }
    this.audio.clear();this.wanted.clear();this.room.removeAllListeners();
    await this.room.disconnect(true);this.owned.clear();
  }
}
