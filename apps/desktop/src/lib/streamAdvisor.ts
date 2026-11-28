import type { MachineInfo } from './platform';
import type { Device, NetworkMode, Preferences, RuntimeStatus, VideoCodec } from './types';

/** What the advisor knows about one computer. Unknown values are left out. */
export interface MachineProfile {
  platform: string;
  osVersion?: string;
  gpu?: string;
  cpuModel?: string;
  cpuCores?: number;
  memoryGb?: number;
  displayWidth?: number;
  displayHeight?: number;
  refreshHz?: number;
  codecs: string[];
}

export interface StreamAdviceRequest {
  viewer: MachineProfile;
  host: MachineProfile;
  network: {
    path?: 'direct' | 'relay' | 'unknown';
    rttMs?: number;
    udpWorks?: boolean;
    serverMs?: number;
    /** Measured just now: this computer's download speed from the server. */
    downloadMbps?: number;
    /** Measured just now: the host's upload speed to the server. */
    uploadMbps?: number;
  };
  locale: 'vi' | 'en';
  goal: AdviceGoal;
}

/** What the person cares about most; the server leans the settings that way. */
export type AdviceGoal = 'balanced' | 'smooth' | 'sharp' | 'responsive';

export interface StreamAdvice {
  source: 'model' | 'rules';
  model?: string;
  settings: {
    resolution: Preferences['stream']['resolution'];
    fps: Preferences['stream']['fps'];
    bitrateMbps: number;
    codec: VideoCodec;
    networkMode: NetworkMode;
  };
  reasons: string[];
}

/** Counts animation frames for a moment to estimate the screen refresh rate. */
export function measureRefreshRate(durationMs = 500): Promise<number | undefined> {
  if (typeof requestAnimationFrame !== 'function') return Promise.resolve(undefined);
  return new Promise((resolve) => {
    let frames = 0;
    let started: number | null = null;
    const step = (now: number) => {
      started ??= now;
      frames += 1;
      if (now - started < durationMs) { requestAnimationFrame(step); return; }
      const rate = (frames - 1) * 1000 / (now - started);
      resolve(nearestRefresh(rate));
    };
    requestAnimationFrame(step);
  });
}

/** Snaps a measured frame rate to the common panel rates. */
export function nearestRefresh(rate: number): number | undefined {
  if (!Number.isFinite(rate) || rate < 20) return undefined;
  return [30, 60, 75, 90, 100, 120, 144, 165, 240].reduce((best, value) => (Math.abs(value - rate) < Math.abs(best - rate) ? value : best));
}

/** What the host measured and reported in a live probe. */
export interface HostMeasurement extends MachineInfo {
  /** The host was in a session and skipped the speed test. */
  busy?: boolean;
  serverMs?: number | null;
  uploadMbps?: number | null;
}

/** The system's own numbers win; the web view's estimates fill the gaps. */
export function viewerProfile(runtime: RuntimeStatus, refreshHz?: number, info: MachineInfo = {}): MachineProfile {
  const ratio = window.devicePixelRatio || 1;
  const memory = (navigator as Navigator & { deviceMemory?: number }).deviceMemory;
  return {
    platform: runtime.platform,
    gpu: info.gpu,
    cpuModel: info.cpuModel,
    cpuCores: info.cpuCores ?? (navigator.hardwareConcurrency || undefined),
    memoryGb: info.memoryGb ?? (typeof memory === 'number' ? memory : undefined),
    displayWidth: info.displayWidth ?? Math.round(window.screen.width * ratio),
    displayHeight: info.displayHeight ?? Math.round(window.screen.height * ratio),
    refreshHz: info.refreshHz ?? refreshHz,
    codecs: runtime.capabilities.clientCodecs ?? ['h264']
  };
}

export function hostProfile(device: Device, measured?: HostMeasurement): MachineProfile {
  const reported = device.gpu && device.gpu !== 'Not reported' ? device.gpu : undefined;
  return {
    platform: device.platform,
    gpu: measured?.gpu ?? reported,
    cpuModel: measured?.cpuModel,
    cpuCores: measured?.cpuCores,
    memoryGb: measured?.memoryGb,
    displayWidth: measured?.displayWidth,
    displayHeight: measured?.displayHeight,
    refreshHz: measured?.refreshHz,
    codecs: device.capabilities.codecs
  };
}

/** The settings as they are saved: a custom profile with these values. */
export function adviceToPreferences(preferences: Preferences, advice: StreamAdvice): Preferences {
  const { resolution, fps, bitrateMbps, codec, networkMode } = advice.settings;
  return { ...preferences, networkMode, stream: { ...preferences.stream, profile: 'custom', resolution, fps, bitrateMbps, codec } };
}
