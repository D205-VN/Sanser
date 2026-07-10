import { get, writable } from 'svelte/store';
import { loadNativePreferences, saveNativePreferences } from '../lib/platform';
import type { NetworkMode, Preferences, QualityProfile, VideoCodec } from '../lib/types';

const STORAGE_KEY = 'sanser.preferences.v2';

export const DEFAULT_PREFERENCES: Preferences = {
  schemaVersion: 2,
  serverUrl: 'http://127.0.0.1:5174',
  networkMode: 'auto',
  stream: {
    profile: 'auto',
    codec: 'auto',
    resolution: 'auto',
    fps: 60,
    bitrateMbps: 20
  },
  host: {
    autoOnline: false,
    autoAcceptOwnDevices: false,
    audioEnabled: true,
    inputEnabled: true,
    clipboardEnabled: false
  },
  input: {
    mouseMode: 'auto',
    pollingRate: 120,
    releaseShortcut: 'Control+Option+Escape',
    gamepadEnabled: false
  },
  locale: 'vi',
  startMinimized: false,
  diagnosticsEnabled: true,
  pinnedDeviceIds: []
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function pickString(value: unknown, fallback: string): string {
  return typeof value === 'string' && value.trim() ? value.trim() : fallback;
}

function pickBoolean(value: unknown, fallback: boolean): boolean {
  return typeof value === 'boolean' ? value : fallback;
}

function pickNumber(value: unknown, fallback: number, min: number, max: number): number {
  return typeof value === 'number' && Number.isFinite(value) && value >= min && value <= max ? value : fallback;
}

function networkMode(value: unknown): NetworkMode {
  if (value === 'direct' || value === 'relay') return value;
  // Sanser 1.x exposed Tailscale as a route. v2 owns route selection itself.
  return 'auto';
}

function qualityProfile(value: unknown): QualityProfile {
  if (value === 'competitive' || value === 'balanced' || value === 'quality' || value === 'custom') return value;
  if (value === 'tailscale' || value === 'internet') return 'balanced';
  return 'auto';
}

function codec(value: unknown): VideoCodec {
  return value === 'h264' || value === 'hevc' ? value : 'auto';
}

function stringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return [...new Set(value.filter((item): item is string => typeof item === 'string' && item.length > 0))].slice(0, 200);
}

export function migratePreferences(value: unknown): Preferences {
  if (!isRecord(value)) return structuredClone(DEFAULT_PREFERENCES);

  const stream = isRecord(value.stream) ? value.stream : value;
  const host = isRecord(value.host) ? value.host : {};
  const input = isRecord(value.input) ? value.input : {};
  const oldNetwork = value.networkMode ?? value.network_mode ?? value.networkProfile;
  const oldQuality = stream.profile ?? value.qualityProfile ?? value.profile;
  const fpsValue = pickNumber(stream.fps, DEFAULT_PREFERENCES.stream.fps, 30, 120);
  const polling = pickNumber(input.pollingRate, DEFAULT_PREFERENCES.input.pollingRate, 60, 240);

  return {
    schemaVersion: 2,
    serverUrl: pickString(value.serverUrl ?? value.server_url, DEFAULT_PREFERENCES.serverUrl),
    networkMode: networkMode(oldNetwork),
    stream: {
      profile: qualityProfile(oldQuality),
      codec: codec(stream.codec ?? value.videoCodec),
      resolution:
        stream.resolution === '720p' ||
        stream.resolution === '1080p' ||
        stream.resolution === '1440p' ||
        stream.resolution === '2160p'
          ? stream.resolution
          : 'auto',
      fps: fpsValue === 30 || fpsValue === 90 || fpsValue === 120 ? fpsValue : 60,
      bitrateMbps: pickNumber(stream.bitrateMbps ?? stream.bitrate, DEFAULT_PREFERENCES.stream.bitrateMbps, 1, 200)
    },
    host: {
      autoOnline: pickBoolean(host.autoOnline, DEFAULT_PREFERENCES.host.autoOnline),
      autoAcceptOwnDevices: pickBoolean(
        host.autoAcceptOwnDevices ?? host.autoAccept,
        DEFAULT_PREFERENCES.host.autoAcceptOwnDevices
      ),
      audioEnabled: pickBoolean(host.audioEnabled, DEFAULT_PREFERENCES.host.audioEnabled),
      inputEnabled: pickBoolean(host.inputEnabled, DEFAULT_PREFERENCES.host.inputEnabled),
      clipboardEnabled: pickBoolean(host.clipboardEnabled, false)
    },
    input: {
      mouseMode: input.mouseMode === 'absolute' || input.mouseMode === 'relative' ? input.mouseMode : 'auto',
      pollingRate: polling === 60 || polling === 90 || polling === 240 ? polling : 120,
      releaseShortcut: pickString(input.releaseShortcut, DEFAULT_PREFERENCES.input.releaseShortcut),
      gamepadEnabled: pickBoolean(input.gamepadEnabled, false)
    },
    locale: value.locale === 'en' ? 'en' : 'vi',
    startMinimized: pickBoolean(value.startMinimized, false),
    diagnosticsEnabled: pickBoolean(value.diagnosticsEnabled, true),
    pinnedDeviceIds: stringArray(value.pinnedDeviceIds)
  };
}

function readBrowserPreferences(): Preferences | null {
  try {
    const serialized = localStorage.getItem(STORAGE_KEY);
    return serialized ? migratePreferences(JSON.parse(serialized) as unknown) : null;
  } catch {
    return null;
  }
}

function writeBrowserPreferences(preferences: Preferences): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(preferences));
  } catch {
    // Native persistence remains authoritative when browser storage is unavailable.
  }
}

function createPreferencesStore() {
  const store = writable<Preferences>(structuredClone(DEFAULT_PREFERENCES));

  return {
    subscribe: store.subscribe,
    async initialize(): Promise<Preferences> {
      let loaded: Preferences | null = null;
      try {
        loaded = await loadNativePreferences();
      } catch {
        // Browser storage remains a valid development fallback; secrets never use it.
      }
      loaded ??= readBrowserPreferences();
      const migrated = migratePreferences(loaded);
      store.set(migrated);
      await this.save(migrated);
      return migrated;
    },
    async save(next?: Preferences): Promise<void> {
      const value = migratePreferences(next ?? get(store));
      store.set(value);
      writeBrowserPreferences(value);
      await saveNativePreferences(value);
    },
    async patch(update: Partial<Preferences>): Promise<void> {
      await this.save({ ...get(store), ...update });
    }
  };
}

export const preferences = createPreferencesStore();
