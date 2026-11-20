export const SANSER_VERSION = '2.3.19' as const;
export const PROTOCOL_VERSION = 2 as const;
export const NATIVE_PROTOCOL = 'Native direct' as const;

export type Page = 'computers' | 'host' | 'session' | 'history' | 'settings' | 'diagnostics' | 'about';
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
  crossPlatformHost?: boolean;
  crossPlatformClient?: boolean;
  multiViewerHost?: boolean;
  hostAudio?: boolean;
  clientAudio?: boolean;
  hostCodecs?: VideoCodec[];
  clientCodecs?: VideoCodec[];
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
  sessionId?: string | null;
  installed: boolean;
  running: boolean;
  videoReceived?: boolean;
  userClosed?: boolean;
  processId: number | null;
  lastError: string | null;
  startupFailure?: 'no-udp' | 'unexpected-peer' | 'no-video' | 'auth-rejected' | 'incomplete-video' | 'decode-failed' | null;
  routeFailure?: 'peer-timeout' | 'relay-lost' | null;
}

export interface SharingPeerVideoStatus {
  viewerId: string;
  ready: boolean;
  running: boolean;
  videoState: 'active' | 'paused' | 'probing';
  stopReason?: number;
  pauses: number;
  backoffMs: number;
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
  /** Stable UDP port used by the host for UPnP/manual forwarding. */
  directUdpPort: number;
  audioEnabled: boolean;
  inputEnabled: boolean;
  clipboardEnabled: boolean;
  /** Host-local black overlay during one-to-one sessions; default off. */
  privacyScreen: boolean;
  /** Also block this computer's own keyboard/mouse (Windows helper only). */
  privacyBlockInput: boolean;
  /** Opt-in: share this computer's adapter MACs so a computer on the same LAN can wake it. */
  wakeOnLan: boolean;
  /** Viewers may record this screen; the host is always told. */
  allowRecording: boolean;
  /** Applications the remote person cannot see or use (see docs/blocked-apps.md). */
  blockedApps: BlockedApp[];
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
  customServer?: boolean;
  networkMode: NetworkMode;
  stream: StreamPreferences;
  host: HostPreferences;
  input: InputPreferences;
  locale: 'vi' | 'en';
  startMinimized: boolean;
  diagnosticsEnabled: boolean;
  pinnedDeviceIds: string[];
  /** Device identities explicitly allowed to request unattended access. */
  trustedDeviceIds: string[];
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
  computerId?: string;
  deviceRole?: 'host' | 'client' | 'unknown';
  crossPlatform?: boolean;
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
  /** Present only when the host opted in to Wake-on-LAN. */
  wakeOnLan?: WakeInterface[];
}

/** One wired or Wi-Fi adapter of a host that can receive a magic packet. */
export interface WakeInterface {
  macAddress: string;
  ipv4Address: string;
  prefixLength: number;
  broadcastAddress: string;
}

export interface WakeResult {
  sameLan: boolean;
  packetsSent: number;
}

/** One connection session in the account access log. Times are ISO strings. */
export interface HistoryEntry {
  id: string;
  requesterDeviceId: string;
  requesterName: string;
  requesterPlatform: string;
  requesterComputerId?: string;
  hostDeviceId: string;
  hostName: string;
  hostPlatform: string;
  hostComputerId?: string;
  state: 'pending' | 'accepted' | 'rejected' | 'disconnected' | 'expired';
  accessMode: 'control' | 'view-only';
  route: 'direct' | 'relay' | null;
  sharedView: boolean;
  startedAt: string;
  connectedAt: string | null;
  endedAt: string | null;
  durationSeconds: number | null;
  endReason: string | null;
}

export interface HistoryQuery {
  cursor?: string;
  limit?: number;
  deviceIds?: string[];
  direction?: 'any' | 'incoming' | 'outgoing';
}

export interface DeviceRegistration {
  computerId?: string;
  deviceRole: 'host' | 'client';
  crossPlatform: boolean;
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
  sharingId?: string;
  accessMode?: 'control' | 'view-only';
  wireProtocol?: 'legacy' | 'snv2';
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
  /** Present when the requester is a guest without an account. */
  guest?: { name: string };
}

export interface CreatedGuestGrant {
  id: string;
  hostDeviceId: string;
  accessCode: string;
  password: string;
  expiresAt: number;
}

export interface GuestGrant {
  id: string;
  hostDeviceId: string;
  state: 'waiting' | 'redeemed';
  createdAt: number;
  expiresAt: number;
  redeemedAt?: number;
  sessionId?: string;
  guestName?: string;
}

export interface GuestDeviceInfo {
  platform: string;
  crossPlatform: boolean;
  osVersion: string;
  version: string;
  protocolVersion: number;
  codecs: string[];
  nativeTransport: boolean;
  webRtc: boolean;
  audio: boolean;
  gamepad: boolean;
  routeAddress?: string;
}

export interface GuestRedemption {
  accessToken: string;
  accessExpiresAt: number;
  deviceId: string;
  hostName: string;
  session: ConnectionSession;
}

export interface NativeSessionCredentials {
  accessMode?: 'control' | 'view-only';
  wireProtocol?: 'legacy' | 'snv2';
  sessionId: string;
  deviceId: string;
  peerDeviceId: string;
  peerRouteAddress: string;
  basePort: number;
  expiresAt: number;
  /** Ephemeral and memory-only. Never persist or include in diagnostics. */
  sessionToken: string;
}

export interface SharingMember {
  sessionId?: string;
  startedAt?: number;
  id: string;
  viewerDeviceId: string;
  state: 'invited' | 'accepted' | 'declined' | 'removed';
  accessMode: 'view-only';
  createdAt: number;
  updatedAt: number;
}

/** Consent/session state; startedAt records the viewer's decoded first frame. */
export interface SharingGroup {
  id: string;
  hostDeviceId: string;
  state: 'preparing' | 'active' | 'stopped';
  networkMode: Exclude<NetworkMode, 'manual'>;
  qualityProfile: Exclude<QualityProfile, 'custom'>;
  maxViewers: number;
  createdAt: number;
  updatedAt: number;
  members: SharingMember[];
}

export interface IceServerConfiguration {
  urls: string[];
  username?: string;
  credential?: string;
}

export interface IceConfiguration {
  iceServers: IceServerConfiguration[];
  networkMode: NetworkMode;
  iceTransportPolicy: 'all' | 'relay';
  expiresAt?: number;
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
  sharingId?: string;
  sharingViewerId?: string;
  ultraLowLatency?: boolean;
  directCheck?: 'passed' | 'relay-selected' | 'timeout' | 'no-direct-route' | 'gather-failed' | 'signaling-failed' | 'failed';
  wireProtocol?: 'legacy' | 'snv2';
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
  audioMuted?: boolean;
  inputEnabled?: boolean;
  clipboardEnabled?: boolean;
  privacyScreen?: boolean;
  privacyBlockInput?: boolean;
  /** Host engine only: macOS bundle IDs or Windows executable names. */
  blockedApps?: string[];
  relativeMouse?: boolean;
  /** Ephemeral server-authorized media credential. Never persist or log this value. */
  sessionToken?: string;
  udpConnect?: string;
  udpBindPort?: number;
  /** Use the authenticated local UDP-to-WSS fallback bridge. */
  relay?: boolean;
}

export interface DiagnosticsExport {
  path: string;
}

export const unavailableCapability = (reason: string, state: CapabilityState = 'unavailable'): Capability => ({
  state,
  reason
});

export type RecordingConsentState = 'none' | 'requested' | 'allowed' | 'denied' | 'stopped';
export type RecordingConsentAction = 'request' | 'stop' | 'allow' | 'deny';
export interface RecordingConsent {
  state: RecordingConsentState;
  viewerName?: string;
  viewerDeviceId?: string;
}

/** Viewer-local recording state reported by the native engine via the shell. */
export interface RecordingStatus {
  capable: boolean;
  state: 'idle' | 'waiting' | 'recording' | 'stopped' | 'error';
  audio: boolean;
  reason: string | null;
  bytes: number;
  durationMs: number;
  parts: number;
  path: string | null;
}

export interface ProblemReport {
  path: string;
  folder: string;
}

/** A host application hidden from capture and not usable remotely. */
export interface BlockedApp {
  /** macOS bundle identifier (com.google.Chrome) or Windows executable name (chrome.exe). */
  id: string;
  name: string;
}
