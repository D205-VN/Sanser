import type { Preferences, QualityProfile } from './types';

/** Shared encoder/client defaults for the profile carried by a session request. */
export function resolveStreamProfile(stream: Preferences['stream'], profile: QualityProfile = stream.profile): Preferences['stream'] {
  switch (profile) {
    case 'competitive': return { ...stream, profile, resolution: '720p', fps: 120, bitrateMbps: 12 };
    case 'balanced': return { ...stream, profile, resolution: '1080p', fps: 60, bitrateMbps: 20 };
    case 'quality': return { ...stream, profile, resolution: '1440p', fps: 60, bitrateMbps: 40 };
    case 'auto': return { ...stream, profile, resolution: '1080p', fps: 60, bitrateMbps: 20 };
    case 'custom': return { ...stream, profile };
  }
}
