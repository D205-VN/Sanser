import type {
  Account,
  AuthResult,
  ConnectionSession,
  Device,
  DeviceRegistration,
  LoginSession,
  NativeSessionCredentials,
  NetworkMode,
  QualityProfile
} from './types';

export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly code: string,
    readonly requestId: string | null
  ) {
    super(message);
    this.name = 'ApiError';
  }
}

export function normalizeServerUrl(raw: string): string {
  const input = raw.trim();
  if (!input) throw new Error('Server URL is required');

  let url: URL;
  try {
    url = new URL(input);
  } catch {
    throw new Error('Server URL is invalid');
  }

  const loopback = url.hostname === 'localhost' || url.hostname === '127.0.0.1' || url.hostname === '[::1]';
  if (url.protocol !== 'https:' && !(url.protocol === 'http:' && loopback)) {
    throw new Error('Use HTTPS, or HTTP only for a local server');
  }
  if (url.username || url.password || url.search || url.hash) {
    throw new Error('Server URL cannot include credentials, query parameters, or a fragment');
  }
  url.pathname = url.pathname.replace(/\/+$/, '');
  return url.toString().replace(/\/$/, '');
}

interface ErrorEnvelope {
  error?: { code?: string; message?: string };
  message?: string;
  code?: string;
}

interface PageEnvelope<T> {
  items: T[];
  nextCursor?: string | null;
}

interface RawLoginSession extends Omit<LoginSession, 'createdAt' | 'lastSeenAt' | 'expiresAt'> {
  createdAt: string | number;
  lastSeenAt: string | number;
  expiresAt: string | number;
}

interface RawDevice extends Partial<Device> {
  sanserVersion?: string;
  osVersion?: string;
  codecs?: string[];
  nativeTransport?: boolean;
  webRtc?: boolean;
  webrtc?: boolean;
  audio?: boolean;
  gamepad?: boolean;
  routeAddress?: string | null;
}

interface RawSession extends Omit<Partial<ConnectionSession>, 'createdAt' | 'updatedAt'> {
  state?: ConnectionSession['status'];
  selectedTransport?: string | null;
  createdAt?: string | number;
  updatedAt?: string | number;
  mediaCredential?: string;
}

function dateString(value: string | number | null | undefined): string | null {
  if (value === null || value === undefined) return null;
  if (typeof value === 'number') return new Date(value < 10_000_000_000 ? value * 1_000 : value).toISOString();
  return value;
}

function normalizeDevice(raw: RawDevice): Device {
  const codecs = (raw.capabilities?.codecs ?? raw.codecs ?? ['auto'])
    .filter((item): item is 'auto' | 'h264' | 'hevc' => item === 'auto' || item === 'h264' || item === 'hevc');
  return {
    id: raw.id ?? '',
    name: raw.name ?? 'Unnamed computer',
    platform: raw.platform ?? raw.osVersion ?? 'Unknown',
    gpu: raw.gpu || null,
    online: raw.online === true,
    streaming: raw.streaming === true,
    version: raw.version ?? raw.sanserVersion ?? 'Unknown',
    latencyMs: typeof raw.latencyMs === 'number' ? raw.latencyMs : null,
    networkQuality: raw.networkQuality ?? 'unknown',
    route: raw.route ?? raw.routeAddress ?? null,
    capabilities: {
      codecs: codecs.length > 0 ? codecs : ['auto'],
      nativeTransport: raw.capabilities?.nativeTransport ?? raw.nativeTransport === true,
      webRtc: raw.capabilities?.webRtc ?? (raw.webRtc === true || raw.webrtc === true),
      audio: raw.capabilities?.audio ?? raw.audio === true,
      gamepad: raw.capabilities?.gamepad ?? raw.gamepad === true
    },
    lastSeenAt: dateString(raw.lastSeenAt),
    pinned: raw.pinned === true
  };
}

function normalizeConnectionSession(raw: RawSession): ConnectionSession {
  const selected = raw.transport ?? raw.selectedTransport;
  return {
    id: raw.id ?? '',
    requesterDeviceId: raw.requesterDeviceId ?? '',
    hostDeviceId: raw.hostDeviceId ?? '',
    status: raw.status ?? raw.state ?? 'pending',
    transport: selected === 'native' || selected === 'webrtc' ? selected : null,
    networkMode: raw.networkMode ?? 'auto',
    qualityProfile: raw.qualityProfile ?? 'auto',
    requestedCodec:
      raw.requestedCodec === 'h264' || raw.requestedCodec === 'hevc' ? raw.requestedCodec : 'auto',
    createdAt: dateString(raw.createdAt) ?? new Date().toISOString(),
    updatedAt: dateString(raw.updatedAt) ?? undefined,
    requesterReadyAt: dateString(raw.requesterReadyAt) ?? undefined,
    address: raw.address,
    port: raw.port,
    sessionToken: raw.sessionToken ?? raw.mediaCredential
  };
}

type TokenProvider = () => Promise<string | null>;

export class ApiClient {
  private readonly baseUrl: string;

  constructor(
    serverUrl: string,
    private readonly tokenProvider: TokenProvider = () => Promise.resolve(null),
    private readonly timeoutMs = 12_000
  ) {
    this.baseUrl = normalizeServerUrl(serverUrl);
  }

  get serverUrl(): string {
    return this.baseUrl;
  }

  async register(email: string, password: string, displayName?: string): Promise<AuthResult> {
    return this.request('/api/v2/auth/register', {
      method: 'POST',
      body: { email, password, displayName: displayName?.trim() || undefined },
      authenticated: false
    });
  }

  async login(email: string, password: string): Promise<AuthResult> {
    return this.request('/api/v2/auth/login', {
      method: 'POST',
      body: { email, password },
      authenticated: false
    });
  }

  async refresh(refreshToken: string): Promise<AuthResult> {
    return this.request('/api/v2/auth/refresh', {
      method: 'POST',
      body: { refreshToken },
      authenticated: false
    });
  }

  async logout(refreshToken: string | null): Promise<void> {
    await this.request('/api/v2/auth/logout', { method: 'POST', body: { refreshToken } });
  }

  async account(): Promise<Account> {
    return this.request('/api/v2/account');
  }

  async changePassword(currentPassword: string, newPassword: string): Promise<void> {
    await this.request('/api/v2/account/password', {
      method: 'POST',
      body: { currentPassword, newPassword }
    });
  }

  async devices(cursor?: string): Promise<PageEnvelope<Device>> {
    const query = cursor ? `?cursor=${encodeURIComponent(cursor)}` : '';
    const response = await this.request<PageEnvelope<RawDevice> | RawDevice[]>(`/api/v2/devices${query}`);
    return Array.isArray(response)
      ? { items: response.map(normalizeDevice) }
      : { ...response, items: response.items.map(normalizeDevice) };
  }

  async renameDevice(id: string, name: string): Promise<Device> {
    const response = await this.request<RawDevice>(`/api/v2/devices/${encodeURIComponent(id)}`, {
      method: 'PATCH',
      body: { name }
    });
    return normalizeDevice(response);
  }

  async pinDevice(id: string, pinned: boolean): Promise<Device> {
    const response = await this.request<RawDevice>(`/api/v2/devices/${encodeURIComponent(id)}`, {
      method: 'PATCH',
      body: { pinned }
    });
    return normalizeDevice(response);
  }

  async removeDevice(id: string): Promise<void> {
    await this.request(`/api/v2/devices/${encodeURIComponent(id)}`, { method: 'DELETE' });
  }

  async registerDevice(device: DeviceRegistration): Promise<Device> {
    return normalizeDevice(await this.request<RawDevice>('/api/v2/devices/register', { method: 'POST', body: device }));
  }

  async heartbeatDevice(id: string, streaming: boolean, routeAddress?: string): Promise<void> {
    await this.request('/api/v2/devices/heartbeat', {
      method: 'POST',
      body: { deviceId: id, streaming, routeAddress }
    });
  }

  async offlineDevice(id: string): Promise<Device> {
    return normalizeDevice(
      await this.request<RawDevice>('/api/v2/devices/offline', {
        method: 'POST',
        body: { deviceId: id }
      })
    );
  }

  async createSession(
    hostDeviceId: string,
    requesterDeviceId: string,
    networkMode: NetworkMode,
    qualityProfile: QualityProfile,
    requestedCodec: 'auto' | 'h264' | 'hevc'
  ): Promise<ConnectionSession> {
    const response = await this.request<RawSession>('/api/v2/sessions', {
      method: 'POST',
      body: { hostDeviceId, requesterDeviceId, networkMode, qualityProfile, requestedCodec }
    });
    return normalizeConnectionSession(response);
  }

  async hostSessions(hostDeviceId: string): Promise<PageEnvelope<ConnectionSession>> {
    const response = await this.request<PageEnvelope<RawSession>>(
      `/api/v2/sessions?hostDeviceId=${encodeURIComponent(hostDeviceId)}&state=active`
    );
    return { ...response, items: response.items.map(normalizeConnectionSession) };
  }

  async getSession(id: string): Promise<ConnectionSession> {
    return normalizeConnectionSession(await this.request<RawSession>(`/api/v2/sessions/${encodeURIComponent(id)}`));
  }

  async acceptSession(id: string): Promise<ConnectionSession> {
    return normalizeConnectionSession(
      await this.request<RawSession>(`/api/v2/sessions/${encodeURIComponent(id)}/accept`, {
        method: 'POST'
      })
    );
  }

  async rejectSession(id: string): Promise<void> {
    await this.request(`/api/v2/sessions/${encodeURIComponent(id)}/reject`, { method: 'POST' });
  }

  async sessionCredentials(id: string, deviceId: string): Promise<NativeSessionCredentials> {
    return this.request<NativeSessionCredentials>(
      `/api/v2/sessions/${encodeURIComponent(id)}/credentials?deviceId=${encodeURIComponent(deviceId)}`
    );
  }

  async markNativeReady(id: string, deviceId: string): Promise<ConnectionSession> {
    return normalizeConnectionSession(
      await this.request<RawSession>(`/api/v2/sessions/${encodeURIComponent(id)}/native-ready`, {
        method: 'POST',
        body: { deviceId }
      })
    );
  }

  async disconnectSession(id: string): Promise<void> {
    await this.request(`/api/v2/sessions/${encodeURIComponent(id)}/disconnect`, { method: 'POST' });
  }

  async loginSessions(): Promise<LoginSession[]> {
    const response = await this.request<RawLoginSession[] | PageEnvelope<RawLoginSession>>('/api/v2/account/sessions');
    const items = Array.isArray(response) ? response : response.items;
    return items.map((item) => ({
      ...item,
      createdAt: dateString(item.createdAt) ?? '',
      lastSeenAt: dateString(item.lastSeenAt) ?? '',
      expiresAt: dateString(item.expiresAt) ?? ''
    }));
  }

  async revokeLoginSession(id: string): Promise<void> {
    await this.request(`/api/v2/account/sessions/${encodeURIComponent(id)}`, { method: 'DELETE' });
  }

  private async request<T = unknown>(
    path: string,
    options: {
      method?: 'GET' | 'POST' | 'PATCH' | 'DELETE';
      body?: unknown;
      authenticated?: boolean;
    } = {}
  ): Promise<T> {
    const requestId = crypto.randomUUID();
    const token = options.authenticated === false ? null : await this.tokenProvider();
    const controller = new AbortController();
    const timeout = window.setTimeout(() => controller.abort(), this.timeoutMs);

    try {
      const response = await fetch(`${this.baseUrl}${path}`, {
        method: options.method ?? 'GET',
        headers: {
          Accept: 'application/json',
          'Content-Type': 'application/json',
          'X-Request-ID': requestId,
          ...(token ? { Authorization: `Bearer ${token}` } : {})
        },
        body: options.body === undefined ? undefined : JSON.stringify(options.body),
        signal: controller.signal,
        cache: 'no-store'
      });

      const responseRequestId = response.headers.get('x-request-id');
      if (response.status === 204) return undefined as T;

      const payload = (await response.json().catch(() => ({}))) as ErrorEnvelope & { data?: T };
      if (!response.ok) {
        const error = payload.error;
        throw new ApiError(
          error?.message ?? payload.message ?? `Request failed (${String(response.status)})`,
          response.status,
          error?.code ?? payload.code ?? 'request_failed',
          responseRequestId
        );
      }
      return (payload.data === undefined ? payload : payload.data) as T;
    } catch (error) {
      if (error instanceof ApiError) throw error;
      if (error instanceof DOMException && error.name === 'AbortError') {
        throw new ApiError('The server did not respond in time', 0, 'timeout', requestId);
      }
      throw new ApiError('Unable to reach the server', 0, 'network_error', requestId);
    } finally {
      window.clearTimeout(timeout);
    }
  }
}
