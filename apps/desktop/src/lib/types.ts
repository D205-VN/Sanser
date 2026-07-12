export const SANSER_VERSION = '2.0.2' as const;
export const PROTOCOL_VERSION = 2 as const;
export const NATIVE_PROTOCOL = 'Native direct' as const;

export type Page = 'computers' | 'host' | 'session' | 'settings' | 'diagnostics' | 'about';
export type SettingsSection =
  | 'general'
  | 'stream'
  | 'video'
  | 'audio'
  | 'input'
  | 'gamepad'
  | 'network'
  | 'host'
  | 'security'
  | 'diagnostics'
  | 'account'
  | 'about';
export type NetworkMode = 'auto' | 'direct' | 'relay';
export type QualityProfile = 'auto' | 'competitive' | 'balanced' | 'quality' | 'custom';
export type VideoCodec = 'auto' | 'h264' | 'hevc';
export type CapabilityState = 'available' | 'unavailable' | 'planned';
export type EngineKind = 'host' | 'client';

export interface Capability {
  state: CapabilityState;
  reason: string | null;
}

export interface RuntimeCapabilities {
  desktopShell: Capability;
  secureStorage: Capability;
  hostEngine: Capability;
  clientEngine: Capability;
  webRtc: Capability;
  nativeDirect: Capability;
  nativeSnv2: Capability;
  gamepad: Capability;
  clipboard: Capability;
  p2pV2: Capability;
}

export interface EngineStatus {
  kind: EngineKind;
  installed: boolean;
  running: boolean;
  processId: number | null;
  lastError: string | null;
}

export interface RuntimeStatus {
  platform: string;
  version: typeof SANSER_VERSION;
  protocolVersion: typeof PROTOCOL_VERSION;
  capabilities: RuntimeCapabilities;
  engines: EngineStatus[];
}

export interface StreamPreferences {
  profile: QualityProfile;
  codec: VideoCodec;
  resolution: 'auto' | '720p' | '1080p' | '1440p' | '2160p';
  fps: 30 | 60 | 90 | 120;
  bitrateMbps: number;
}

export interface HostPreferences {
  autoOnline: boolean;
  autoAcceptOwnDevices: boolean;
  audioEnabled: boolean;
  inputEnabled: boolean;
  clipboardEnabled: boolean;
}

export interface InputPreferences {
  mouseMode: 'auto' | 'absolute' | 'relative';
  pollingRate: 60 | 90 | 120 | 240;
  releaseShortcut: string;
  gamepadEnabled: boolean;
}

export interface Preferences {
  schemaVersion: 2;
  serverUrl: string;
  networkMode: NetworkMode;
  stream: StreamPreferences;
  host: HostPreferences;
  input: InputPreferences;
  locale: 'vi' | 'en';
  startMinimized: boolean;
  diagnosticsEnabled: boolean;
  pinnedDeviceIds: string[];
}

export interface Account {
  id: string;
  email: string;
  displayName: string | null;
}

export interface AuthTokens {
  accessToken: string;
  refreshToken: string;
  expiresIn?: number;
  accessExpiresAt?: number;
  refreshExpiresAt?: number;
  sessionId?: string;
}

export interface AuthResult extends AuthTokens {
  account: Account;
}

export interface DeviceCapabilities {
  codecs: VideoCodec[];
  nativeTransport: boolean;
  webRtc: boolean;
  audio: boolean;
  gamepad: boolean;
}

export interface Device {
  id: string;
  name: string;
  platform: string;
  gpu: string | null;
  online: boolean;
  streaming: boolean;
  version: string;
  latencyMs: number | null;
  networkQuality: 'excellent' | 'good' | 'fair' | 'poor' | 'unknown';
  route: string | null;
  capabilities: DeviceCapabilities;
  lastSeenAt: string | null;
  pinned: boolean;
}

export interface DeviceRegistration {
  id: string;
  name: string;
  platform: string;
  osVersion: string;
  gpu: string;
  version: typeof SANSER_VERSION;
  protocolVersion: typeof PROTOCOL_VERSION;
  codecs: VideoCodec[];
  nativeTransport: boolean;
  webRtc: boolean;
  audio: boolean;
  gamepad: boolean;
  /** Direct route selected by the native shell, when one has been verified. */
  routeAddress?: string;
}

export interface ConnectionSession {
  id: string;
  requesterDeviceId: string;
  hostDeviceId: string;
  status: 'pending' | 'accepted' | 'connecting' | 'connected' | 'rejected' | 'disconnected' | 'expired' | 'closed' | 'failed';
  transport: 'native' | 'webrtc' | null;
  networkMode: NetworkMode;
  qualityProfile: QualityProfile;
  requestedCodec: VideoCodec;
  createdAt: string;
  updatedAt?: string;
  requesterReadyAt?: string;
  address?: string;
  port?: number;
  /** Ephemeral, memory-only credential returned after media negotiation. */
  sessionToken?: string;
  credentialExpiresAt?: number;
}

export interface NativeSessionCredentials {
  sessionId: string;
  deviceId: string;
  peerDeviceId: string;
  peerRouteAddress: string;
  basePort: number;
  expiresAt: number;
  /** Ephemeral and memory-only. Never persist or include in diagnostics. */
  sessionToken: string;
}

export interface LoginSession {
  id: string;
  deviceName: string;
  platform: string;
  current: boolean;
  createdAt: string;
  lastSeenAt: string;
  expiresAt: string;
}

export interface SessionMetrics {
  fps: number;
  bitrateMbps: number;
  rttMs: number;
  jitterMs: number;
  packetLossPercent: number;
  inputLatencyMs: number;
  encodeLatencyMs: number;
  decodeLatencyMs: number;
  droppedFrames: number;
  codec: VideoCodec;
  transport: 'Native' | 'WebRTC' | 'None';
}

export interface LaunchEngineRequest {
  kind: EngineKind;
  sessionId?: string;
  address?: string;
  port?: number;
  codec: VideoCodec;
  fps: number;
  bitrateKbps: number;
  width: number;
  height: number;
  networkMode: NetworkMode;
  audioEnabled?: boolean;
  inputEnabled?: boolean;
  relativeMouse?: boolean;
  /** Ephemeral server-authorized media credential. Never persist or log this value. */
  sessionToken?: string;
}

export interface DiagnosticsExport {
  path: string;
}

export const unavailableCapability = (reason: string, state: CapabilityState = 'unavailable'): Capability => ({
  state,
  reason
});
