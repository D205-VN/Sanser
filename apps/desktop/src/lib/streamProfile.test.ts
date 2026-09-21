import { describe, expect, it } from 'vitest';
import { DEFAULT_PREFERENCES } from '../stores/preferences';
import { resolveStreamProfile } from './streamProfile';

describe('stream profiles', () => {
  it('uses the requested profile on both endpoints, independently of the saved host profile', () => {
    const stream = { ...DEFAULT_PREFERENCES.stream, fps: 30 as const, bitrateMbps: 5 };
    expect(resolveStreamProfile(stream, 'competitive')).toMatchObject({ resolution: '720p', fps: 120, bitrateMbps: 12 });
    expect(resolveStreamProfile(stream, 'balanced')).toMatchObject({ resolution: '1080p', fps: 60, bitrateMbps: 20 });
    expect(resolveStreamProfile(stream, 'quality')).toMatchObject({ resolution: '1440p', fps: 60, bitrateMbps: 40 });
    expect(stream.bitrateMbps).toBe(5);
  });
  it('preserves custom values and the requested codec', () => {
    const stream = { ...DEFAULT_PREFERENCES.stream, profile: 'custom' as const, codec: 'hevc' as const, bitrateMbps: 35 };
    expect(resolveStreamProfile(stream)).toEqual(stream);
    expect(resolveStreamProfile(stream, 'quality').codec).toBe('hevc');
  });
});
