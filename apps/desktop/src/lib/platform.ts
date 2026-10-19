import { invoke } from '@tauri-apps/api/core';
import type { DiagnosticsExport, EngineKind, EngineStatus, LaunchEngineRequest, Preferences, RuntimeStatus, SharingPeerVideoStatus } from './types';
import { PROTOCOL_VERSION, SANSER_VERSION, unavailableCapability } from './types';

export function isTauriRuntime(): boolean {
  return '__TAURI_INTERNALS__' in window;
}

export interface WindowsUacStatus { available: boolean; enabled: boolean; ready: boolean }
export async function windowsUacStatus(): Promise<WindowsUacStatus> {
  return isTauriRuntime() ? invoke<WindowsUacStatus>('windows_uac_status') : { available: false, enabled: false, ready: false };
}
export async function configureWindowsUac(enabled: boolean): Promise<WindowsUacStatus> {
  if (!isTauriRuntime()) throw new Error('UAC support requires the packaged Windows app');
  return invoke<WindowsUacStatus>('configure_windows_uac', { enabled });
}

export interface LoginDevice {
  installationId: string;
  deviceName: string;
  platform: string;
}

export async function loginDevice(): Promise<LoginDevice> {
  if (isTauriRuntime()) return invoke<LoginDevice>('get_login_device');
  const key = 'sanser.login-installation.v1';
  let installationId = localStorage.getItem(key);
  if (!installationId || !/^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/i.test(installationId)) {
    installationId = crypto.randomUUID();
    localStorage.setItem(key, installationId);
  }
  return { installationId, deviceName: 'Sanser browser', platform: 'browser-development' };
}

export async function runtimeStatus(): Promise<RuntimeStatus> {
  if (isTauriRuntime()) return invoke<RuntimeStatus>('get_runtime_status');

  const unavailable = unavailableCapability('Requires the packaged Sanser desktop app');
  return {
    platform: 'browser-development',
    version: SANSER_VERSION,
    protocolVersion: PROTOCOL_VERSION,
    capabilities: {
      desktopShell: unavailable,
      secureStorage: unavailable,
      hostEngine: unavailable,
      clientEngine: unavailable,
      webRtc: unavailableCapability('Native WebRTC backend is not installed', 'planned'),
      nativeDirect: unavailableCapability('Authenticated native direct engine is not installed'),
      nativeSnv2: unavailableCapability('SNV2 engine is not installed'),
      gamepad: unavailableCapability('Native gamepad input is not installed', 'planned'),
      clipboard: unavailableCapability('Clipboard sharing is not installed', 'planned'),
      p2pV2: unavailableCapability('P2P transport is not installed')
    },
    engines: [
      { kind: 'host', installed: false, running: false, processId: null, lastError: null },
      { kind: 'client', installed: false, running: false, processId: null, lastError: null }
    ]
  };
}

export async function loadNativePreferences(): Promise<Preferences | null> {
  return isTauriRuntime() ? invoke<Preferences | null>('load_preferences') : null;
}

export async function saveNativePreferences(preferences: Preferences): Promise<void> {
  if (isTauriRuntime()) await invoke('save_preferences', { preferences });
}

const allowedSecretKeys = new Set(['auth_session', 'access_token', 'refresh_token', 'device_identity']);

function assertSecretKey(key: string): void {
  if (!allowedSecretKeys.has(key)) throw new Error('Unsupported secure storage key');
}

export async function secureGet(key: string): Promise<string | null> {
  assertSecretKey(key);
  if (isTauriRuntime()) return invoke<string | null>('secure_get', { key });
  return sessionStorage.getItem(`sanser:${key}`);
}

export async function secureSet(key: string, value: string): Promise<void> {
  assertSecretKey(key);
  if (isTauriRuntime()) {
    await invoke('secure_set', { key, value });
    return;
  }
  sessionStorage.setItem(`sanser:${key}`, value);
}

export async function secureDelete(key: string): Promise<void> {
  assertSecretKey(key);
  if (isTauriRuntime()) {
    await invoke('secure_delete', { key });
    return;
  }
  sessionStorage.removeItem(`sanser:${key}`);
}

export async function launchEngine(request: LaunchEngineRequest): Promise<void> {
  if (!isTauriRuntime()) throw new Error('Native engine requires the Sanser desktop app');
  await invoke('launch_engine', { request });
}

export async function addSharingPeer(request: LaunchEngineRequest): Promise<void> {
  if (!isTauriRuntime()) throw new Error('Native sharing requires the Sanser desktop app');
  await invoke('sharing_engine_add_peer', { request });
}

export async function removeSharingPeer(sharingId: string, viewerId: string): Promise<void> {
  if (!isTauriRuntime()) throw new Error('Native sharing requires the Sanser desktop app');
  await invoke('sharing_engine_remove_peer', { sharingId, viewerId });
}

export async function sharingPeerStatus(sharingId: string): Promise<SharingPeerVideoStatus[]> {
  return isTauriRuntime() ? invoke<SharingPeerVideoStatus[]>('sharing_engine_peer_status', { sharingId }) : [];
}

export async function stopEngine(kind: EngineKind, sessionId?: string): Promise<void> {
  if (!isTauriRuntime()) throw new Error('Native engine requires the Sanser desktop app');
  await invoke('stop_engine', { kind, sessionId });
}

export async function setViewerAudio(sessionId: string, muted: boolean): Promise<void> {
  if (!isTauriRuntime()) throw new Error('Native audio requires the Sanser desktop app');
  await invoke('set_viewer_audio', { sessionId, muted });
}

export async function engineStatus(kind: EngineKind): Promise<EngineStatus> {
  if (isTauriRuntime()) return invoke<EngineStatus>('get_engine_status', { kind });
  return { kind, installed: false, running: false, processId: null, lastError: null };
}

export interface RouteRecoveryEvidence {
  cause: 'peer-timeout' | 'relay-lost' | 'control-unresponsive' | 'sustained-wire-delay' | 'application-delay';
  automaticAttempts: number;
  stableMs: number;
  evidenceAgeMs: number;
  controlSilenceMs?: number;
  probeSamples?: number;
  probeMedianMs?: number;
  wireRttMs?: number;
  elevatedSamples?: number;
}
export interface RouteRecoveryPlan {
  action: 'observe' | 'reconnect' | 'manual';
  delayMs: number;
  reason: string;
}
export async function routeRecoveryPlan(request: RouteRecoveryEvidence): Promise<RouteRecoveryPlan> {
  return isTauriRuntime() ? invoke<RouteRecoveryPlan>('route_recovery_plan', { request })
    : { action: 'observe', delayMs: 0, reason: 'desktop-required' };
}

export async function getLocalRouteAddress(serverUrl: string): Promise<string | null> {
  if (!isTauriRuntime()) return null;
  return invoke<string | null>('get_local_route_address', { serverUrl });
}

export interface RelayStartRequest {
  kind: Extract<EngineKind, 'host' | 'client'>;
  serverUrl: string;
  sessionId: string;
  deviceId: string;
  peerDeviceId: string;
  accessToken: string;
  sessionCredential: string;
  preferredEnginePort?: number;
  sharingId?: string;
}

export interface RelayStartResult {
  transport?: 'udp' | 'wss';
  enginePort: number;
  proxyPort: number;
}

export async function startRelay(request: RelayStartRequest): Promise<RelayStartResult> {
  if (!isTauriRuntime()) throw new Error('Media relay requires the Sanser desktop app');
  return invoke<RelayStartResult>('relay_start', { request });
}

export async function stopRelay(kind: Extract<EngineKind, 'host' | 'client'>, sessionId?: string): Promise<void> {
  if (isTauriRuntime()) await invoke('relay_stop', { kind, sessionId });
}

export async function relayPeerFailure(kind: 'host' | 'client', sessionId: string): Promise<string | null> {
  if (!isTauriRuntime()) return null;
  return invoke('relay_peer_failure', { kind, sessionId });
}

export async function exportDiagnostics(contents: string): Promise<DiagnosticsExport> {
  if (isTauriRuntime()) return invoke<DiagnosticsExport>('export_diagnostics', { contents });

  const blob = new Blob([contents], { type: 'application/json' });
  const url = URL.createObjectURL(blob);
  const link = document.createElement('a');
  link.href = url;
  link.download = `sanser-diagnostics-${new Date().toISOString().replaceAll(':', '-')}.json`;
  link.click();
  URL.revokeObjectURL(url);
  return { path: link.download };
}
