import { describe, expect, it } from 'vitest';
import { migratePreferences } from './preferences';

describe('migratePreferences', () => {
  it('removes the legacy Tailscale route', () => {
    const migrated = migratePreferences({ networkMode: 'tailscale', profile: 'tailscale', serverUrl: 'https://example.test' });
    expect(migrated.networkMode).toBe('auto');
    expect(migrated.stream.profile).toBe('balanced');
    expect(JSON.stringify(migrated)).not.toContain('tailscale');
  });

  it('bounds invalid manual quality values', () => {
    const migrated = migratePreferences({ stream: { fps: 999, bitrateMbps: -1, codec: 'av1' } });
    expect(migrated.stream.fps).toBe(60);
    expect(migrated.stream.bitrateMbps).toBe(20);
    expect(migrated.stream.codec).toBe('auto');
  });
});
