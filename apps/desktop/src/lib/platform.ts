import { invoke } from '@tauri-apps/api/core';
import type { DiagnosticsExport, EngineKind, LaunchEngineRequest, Preferences, RuntimeStatus } from './types';
import { PROTOCOL_VERSION, SANSER_VERSION, unavailableCapability } from './types';

export function isTauriRuntime(): boolean {
  return '__TAURI_INTERNALS__' in window;
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
      localServer: unavailable,
      localDiscovery: unavailableCapability('Local discovery backend is not installed', 'planned'),
      webRtc: unavailableCapability('Native WebRTC backend is not installed', 'planned'),
      nativeSnv2: unavailableCapability('SNV2 engine is not installed'),
      gamepad: unavailableCapability('Native gamepad input is not installed', 'planned'),
      clipboard: unavailableCapability('Clipboard sharing is not installed', 'planned')
    },
    engines: [
      { kind: 'host', installed: false, running: false, processId: null, lastError: null },
      { kind: 'client', installed: false, running: false, processId: null, lastError: null },
      { kind: 'localServer', installed: false, running: false, processId: null, lastError: null }
    ]
  };
}

export async function loadNativePreferences(): Promise<Preferences | null> {
  return isTauriRuntime() ? invoke<Preferences | null>('load_preferences') : null;
}

export async function saveNativePreferences(preferences: Preferences): Promise<void> {
  if (isTauriRuntime()) await invoke('save_preferences', { preferences });
}

const allowedSecretKeys = new Set(['access_token', 'refresh_token', 'device_identity']);

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

export async function stopEngine(kind: EngineKind): Promise<void> {
  if (!isTauriRuntime()) throw new Error('Native engine requires the Sanser desktop app');
  await invoke('stop_engine', { kind });
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
