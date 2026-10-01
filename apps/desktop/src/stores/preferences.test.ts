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

  it('preserves an explicit self-hosted endpoint across saves and upgrades', () => {
    const value = { serverUrl: 'https://my-mac.example/', customServer: true };
    const migrated = migratePreferences(value, 'https://default.example');
    expect(migrated.serverUrl).toBe('https://my-mac.example');
    expect(migratePreferences(migrated, 'https://new-default.example').serverUrl).toBe('https://my-mac.example');
  });

  it.each(['http://192.168.1.2', 'https://user:password@example.test', 'invalid'])('rejects unsafe custom server %s', (serverUrl) => {
    const migrated = migratePreferences({ serverUrl, customServer: true }, 'https://default.example');
    expect(migrated.serverUrl).toBe('https://default.example');
    expect(migrated.customServer).toBe(false);
  });
});
