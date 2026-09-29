import type { Preferences, QualityProfile } from './types';

/** Shared encoder/client defaults for the profile carried by a session request. */
export function resolveStreamProfile(stream: Preferences['stream'], profile: QualityProfile = stream.profile): Preferences['stream'] {
  switch (profile) {
    case 'competitive': return { ...stream, profile, resolution: '720p', fps: 120, bitrateMbps: 12 };
    case 'balanced': return { ...stream, profile, resolution: '1080p', fps: 60, bitrateMbps: 20 };
    case 'quality': return { ...stream, profile, resolution: '1440p', fps: 60, bitrateMbps: 40 };
    case 'auto': return { ...stream, profile, resolution: 'auto', fps: 60, bitrateMbps: 25 };
    case 'custom': return { ...stream, profile };
  }
}

/** Legacy Windows capture fits these bounds without upscaling the source. */
export function resolveStreamSize(resolution: Preferences['stream']['resolution'], wireProtocol?: string): { width: number; height: number } {
  switch (resolution) {
    case '720p': return { width: 1280, height: 720 };
    case '1440p': return { width: 2560, height: 1440 };
    case '2160p': return { width: 3840, height: 2160 };
    // Preserve WUXGA desktop text instead of reducing 1920x1200 to 1728x1080.
    // SNV2 uses a fixed output size; keep its existing dimensions.
    case 'auto': return { width: 1920, height: wireProtocol === 'snv2' ? 1080 : 1200 };
    default: return { width: 1920, height: 1080 };
  }
}
