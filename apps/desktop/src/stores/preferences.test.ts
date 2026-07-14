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

  it('keeps a safe fixed host UDP port', () => {
    expect(migratePreferences({ host: { directUdpPort: 50_123 } }).host.directUdpPort).toBe(50_123);
    expect(migratePreferences({ host: { directUdpPort: 80 } }).host.directUdpPort).toBe(50_000);
  });

  it('does not let a stale preference override a configured release endpoint', () => {
    const configured = 'https://api.sanser.example';
    const migrated = migratePreferences(
      { serverUrl: 'https://stale-or-attacker.example', server_url: 'https://also-stale.example' },
      configured
    );

    expect(migrated.serverUrl).toBe(configured);
  });

  it('keeps only bounded UUID device identities in the unattended-access allowlist', () => {
    const trusted = '22222222-2222-4222-8222-222222222222';
    const migrated = migratePreferences({
      trustedDeviceIds: [trusted, trusted, 'not-a-device-id', '', 7]
    });
    expect(migrated.trustedDeviceIds).toEqual([trusted]);
  });
});
