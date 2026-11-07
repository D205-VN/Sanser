/** Isolated trial authentication. No credentials in storage, URLs or exports. */
export interface SharingChoice { groupId: string; deviceId: string; label: string; }
export interface VoiceGrant { url: string; token: string; roomRevision: string; }
export class LabAccountError extends Error {
  constructor(readonly code: 'config' | 'login' | 'unavailable' | 'membership' | 'cancelled') {
    super(code);this.name = 'LabAccountError';
  }
}
export function serverUrl(value: string): string {
  let url: URL;
  try { url = new URL(value.trim()); } catch { throw new LabAccountError('config'); }
  const local = ['localhost', '127.0.0.1'].includes(url.hostname);
  if (!(url.protocol === 'https:' || (local && url.protocol === 'http:')) || url.username || url.password || url.search || url.hash)
    throw new LabAccountError('config');
  return url.toString().replace(/\/+$/, '');
}
const uuid = /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/i;
function record(value: unknown): value is Record<string, unknown> { return value !== null && typeof value === 'object'; }

export class LabAccount {
  private base = '';
  private access?: string;
  private refresh?: string;
  private generation = 0;
  private pending = new Set<AbortController>();
  // A separate installation prevents this trial login replacing the native app.
  private readonly installation = crypto.randomUUID();
  constructor(private readonly request: typeof fetch = fetch) {}
  get signedIn(): boolean { return !!this.access; }
  cancelRequests(): void { ++this.generation;for (const c of this.pending) c.abort();this.pending.clear(); }

  private async json(path: string, body?: unknown, authorization = this.access, base = this.base): Promise<unknown> {
    const generation = this.generation, controller = new AbortController();this.pending.add(controller);
    const timeout = setTimeout(() => controller.abort(), 8000);
    try {
      const response = await this.request(`${base}${path}`, {
        method: body === undefined ? 'GET' : 'POST', redirect: 'error', cache: 'no-store', credentials: 'omit',
        headers: { ...(authorization ? { Authorization: `Bearer ${authorization}` } : {}), ...(body === undefined ? {} : { 'Content-Type': 'application/json' }) },
        ...(body === undefined ? {} : { body: JSON.stringify(body) }), signal: controller.signal
      });
      if (!response.ok) throw new LabAccountError(response.status === 401 ? 'login' : response.status === 503 ? 'unavailable' : 'membership');
      const data: unknown = await response.json();
      if (generation !== this.generation) throw new LabAccountError('cancelled');
      if (controller.signal.aborted) throw new LabAccountError('unavailable');
      return data;
    } catch (error) {
      if (generation !== this.generation) throw new LabAccountError('cancelled');
      throw error instanceof LabAccountError ? error : new LabAccountError('unavailable');
    } finally { clearTimeout(timeout);this.pending.delete(controller); }
  }

  async login(base: string, email: string, password: string): Promise<void> {
    this.logout();this.base = serverUrl(base);
    const generation = this.generation;
    const data = await this.json('/api/v2/auth/login', { email, password, deviceName: 'Sanser Voice Lab', platform: 'browser-voice-lab', installationId: this.installation });
    if (generation !== this.generation) throw new LabAccountError('cancelled');
    if (!record(data) || typeof data.accessToken !== 'string' || !data.accessToken.startsWith('sn2_a_') || typeof data.refreshToken !== 'string' || !data.refreshToken.startsWith('sn2_r_'))
      throw new LabAccountError('login');
    this.access = data.accessToken;this.refresh = data.refreshToken;
  }

  async choices(): Promise<SharingChoice[]> {
    if (!this.access) throw new LabAccountError('login');
    const data = await this.json('/api/v2/sharing');
    if (!record(data) || !Array.isArray(data.items)) throw new LabAccountError('membership');
    const choices: SharingChoice[] = [];
    for (const group of data.items.slice(0, 20)) {
      if (!record(group) || typeof group.id !== 'string' || !uuid.test(group.id) || !['preparing', 'active'].includes(String(group.state)) || typeof group.hostDeviceId !== 'string' || !uuid.test(group.hostDeviceId)) continue;
      choices.push({ groupId: group.id, deviceId: group.hostDeviceId, label: `Host ${group.hostDeviceId.slice(0, 8)} · phòng ${group.id.slice(0, 8)}` });
      if (!Array.isArray(group.members)) continue;
      for (const member of group.members.slice(0, 3)) {
        if (!record(member) || member.state !== 'accepted' || typeof member.viewerDeviceId !== 'string' || !uuid.test(member.viewerDeviceId)) continue;
        choices.push({ groupId: group.id, deviceId: member.viewerDeviceId, label: `Viewer ${member.viewerDeviceId.slice(0, 8)} · phòng ${group.id.slice(0, 8)}` });
      }
    }
    return choices;
  }

  async grant(choice: SharingChoice): Promise<VoiceGrant> {
    if (!this.access) throw new LabAccountError('login');
    if (!uuid.test(choice.groupId) || !uuid.test(choice.deviceId)) throw new LabAccountError('membership');
    const data = await this.json(`/api/v2/sharing/${choice.groupId}/voice-lab/credentials`, { deviceId: choice.deviceId });
    if (!record(data) || data.provider !== 'livekit-lab' || typeof data.url !== 'string' || typeof data.token !== 'string' || data.token.length > 8192 || typeof data.roomRevision !== 'string' || !/^[A-Za-z0-9_-]{43}$/.test(data.roomRevision))
      throw new LabAccountError('unavailable');
    return { url: data.url, token: data.token, roomRevision: data.roomRevision };
  }

  logout(): void {
    const access = this.access, refresh = this.refresh, base = this.base;
    this.cancelRequests();this.access = this.refresh = undefined;
    if (access && refresh) {
      // Best effort revocation of this isolated login; cannot revoke LiveKit tokens.
      const controller = new AbortController(), timeout = setTimeout(() => controller.abort(), 8000);
      void this.request(`${base}/api/v2/auth/logout`, { method: 'POST', redirect: 'error', cache: 'no-store', credentials: 'omit', keepalive: true, signal: controller.signal,
        headers: { Authorization: `Bearer ${access}`, 'Content-Type': 'application/json' }, body: JSON.stringify({ refreshToken: refresh })
      }).catch(() => undefined).finally(() => clearTimeout(timeout));
    }
  }
}
