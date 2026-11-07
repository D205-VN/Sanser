export type RoomSignal = 'reconnecting' | 'connected' | 'disconnected' | 'playback-blocked' | 'members' | 'capacity' | 'mic-ended';
export interface Microphone {
  publish(): Promise<void>;
  stop(): void;
  unpublish(): Promise<void>;
}
export interface VoiceRoom {
  connect(url: string, token: string): Promise<void>;
  disconnect(): Promise<void>;
  onSignal(callback: (signal: RoomSignal, members?: number) => void): void;
  createMicrophone(): Promise<Microphone>;
  stopMicrophones(): void;
  setDeafened(value: boolean): void;
  startAudio(): Promise<void>;
}
export interface VoiceState {
  phase: 'idle' | 'joining' | 'listening' | 'error';
  joined: boolean;
  microphone: boolean;
  micPending: boolean;
  deafened: boolean;
  playbackBlocked: boolean;
  members: number;
  error: 'invalid-config' | 'connection-failed' | 'connection-lost' | 'microphone-denied' | 'microphone-failed' | 'room-capacity' | null;
}
const initial = (): VoiceState => ({ phase: 'idle', joined: false, microphone: false, micPending: false,
  deafened: false, playbackBlocked: false, members: 0, error: null });

export function connectionInput(endpoint: string, token: string): { url: string; token: string } {
  const url = new URL(endpoint.trim());
  const loopback = ['localhost', '127.0.0.1'].includes(url.hostname);
  if ((url.protocol !== 'wss:' && !(url.protocol === 'ws:' && loopback)) || url.username || url.password || url.search || url.hash ||
    token.length > 8192 || !/^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$/.test(token.trim())) throw new Error('invalid-config');
  return { url: url.href, token: token.trim() };
}
interface Attempt { room: VoiceRoom; live: boolean; revision: number; mic?: Microphone; micBusy: boolean }

// Consent belongs to one explicit attempt. No storage, reconnect mic restore or
// token generation; stale capture/publish completions always stop their track.
export class VoiceSession {
  private value = initial();
  private current?: Attempt;
  private observers = new Set<(value: VoiceState) => void>();
  constructor(private factory: () => VoiceRoom) {}
  get state(): VoiceState { return { ...this.value }; }
  subscribe(fn: (state: VoiceState) => void): () => void {
    this.observers.add(fn);fn(this.state);return () => this.observers.delete(fn);
  }
  private change(next: Partial<VoiceState>): void { this.value = { ...this.value, ...next };for (const fn of this.observers) fn(this.state); }
  private owns(attempt: Attempt): boolean { return attempt.live && this.current === attempt; }
  private releaseMic(attempt: Attempt): void {
    ++attempt.revision;
    attempt.room.stopMicrophones();
    const mic = attempt.mic;attempt.mic = undefined;
    if (mic) { mic.stop();void mic.unpublish().catch(() => undefined); }
  }
  leave(): void {
    const old = this.current;this.current = undefined;
    if (old) {
      old.live = false;this.releaseMic(old);old.room.setDeafened(true);
      void old.room.disconnect().catch(() => undefined);
    }
    this.value = initial();this.change({});
  }
  async join(endpoint: string, token: string): Promise<void> {
    if (this.current) return;
    let input: ReturnType<typeof connectionInput>;
    try { input = connectionInput(endpoint, token); } catch { this.change({ phase: 'error', error: 'invalid-config' });return; }
    let attempt: Attempt;
    try { attempt = { room: this.factory(), live: true, revision: 0, micBusy: false }; }
    catch { this.change({ phase: 'error', error: 'connection-failed' });return; }
    this.current = attempt;this.change({ ...initial(), phase: 'joining' });
    attempt.room.onSignal((signal, members) => {
      if (!this.owns(attempt)) return;
      if (signal === 'disconnected' || signal === 'capacity') {
        const error = signal === 'capacity' ? 'room-capacity' : this.value.joined ? 'connection-lost' : 'connection-failed';
        this.leave();this.change({ phase: 'error', error });
      } else if (signal === 'reconnecting') {
        this.leave();this.change({ phase: 'error', error: 'connection-lost' });
      } else if (signal === 'mic-ended') {
        this.releaseMic(attempt);this.change({ microphone: false, error: 'microphone-failed' });
      } else if (signal === 'playback-blocked') this.change({ playbackBlocked: true });
      else if (signal === 'members' && Number.isInteger(members) && members !== undefined && members >= 1 && members <= 4) this.change({ members });
    });
    let timeout: ReturnType<typeof setTimeout> | undefined;
    const connected = Promise.resolve().then(() => attempt.room.connect(input.url, input.token));
    // Even if timeout/Leave wins, a later SDK connect must close its own room.
    void connected.then(() => { if (!this.owns(attempt)) void attempt.room.disconnect().catch(() => undefined); }, () => undefined);
    try {
      await Promise.race([connected, new Promise<never>((_, reject) => { timeout = setTimeout(() => reject(new Error('timeout')), 15_000); })]);
      if (!this.owns(attempt)) return;
      this.change({ phase: 'listening', joined: true, microphone: false });
      await this.resumeAudio();
    } catch {
      if (this.owns(attempt)) { this.leave();this.change({ phase: 'error', error: 'connection-failed' }); }
    } finally { clearTimeout(timeout); }
  }
  async setMicrophone(enabled: boolean): Promise<void> {
    const attempt = this.current;if (!attempt) return;
    if (!enabled) { this.releaseMic(attempt);this.change({ microphone: false, micPending: attempt.micBusy });return; }
    if (!this.value.joined || this.value.phase !== 'listening' || this.value.deafened || attempt.micBusy || this.value.microphone) return;
    const revision = ++attempt.revision;attempt.micBusy = true;this.change({ micPending: true, error: null });
    let mic: Microphone | undefined;
    let timeout: ReturnType<typeof setTimeout> | undefined;
    const expired = new Promise<never>((_, reject) => { timeout = setTimeout(() => reject(new Error('microphone-timeout')), 15_000); });
    const valid = () => this.owns(attempt) && revision === attempt.revision && this.value.phase === 'listening' && !this.value.deafened;
    try {
      const capture = attempt.room.createMicrophone();
      // The OS prompt cannot be cancelled by JavaScript. A track granted after
      // timeout/Leave belongs to the old consent, and must never be published.
      void capture.then((late) => {
        if (!valid()) { late.stop();void late.unpublish().catch(() => undefined); }
      }, () => undefined);
      mic = await Promise.race([capture, expired]);
      if (!valid()) { mic.stop();void mic.unpublish().catch(() => undefined);return; }
      attempt.mic = mic;
      const publishingMic = mic, published = mic.publish();
      void published.then(() => {
        if (!valid()) { publishingMic.stop();void publishingMic.unpublish().catch(() => undefined); }
      }, () => undefined);
      await Promise.race([published, expired]);
      if (!valid()) { mic.stop();void mic.unpublish().catch(() => undefined);return; }
      this.change({ microphone: true });
    } catch (error) {
      mic?.stop();if (mic) void mic.unpublish().catch(() => undefined);
      if (valid()) {
        this.releaseMic(attempt);
        this.change({ microphone: false, error: error instanceof DOMException && error.name === 'NotAllowedError' ? 'microphone-denied' : 'microphone-failed' });
      }
    } finally {
      clearTimeout(timeout);
      attempt.micBusy = false;
      if (this.owns(attempt)) this.change({ micPending: false });
    }
  }
  setDeafened(value: boolean): void {
    const attempt = this.current;if (!attempt || !this.value.joined) return;
    if (value) this.releaseMic(attempt);
    attempt.room.setDeafened(value);this.change({ deafened: value, ...(value ? { microphone: false } : {}) });
  }
  async resumeAudio(): Promise<void> {
    const attempt = this.current;if (!attempt || !this.value.joined) return;
    try { await attempt.room.startAudio();if (this.owns(attempt)) this.change({ playbackBlocked: false }); }
    catch { if (this.owns(attempt)) this.change({ playbackBlocked: true }); }
  }
}
