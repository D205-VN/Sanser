import { describe, expect, it } from 'vitest';
import { nativeCompatibilityReason } from './nativeCompatibility';
import type { Device, RuntimeStatus } from './types';
const runtime = (platform: string, supported = true) => ({ platform, capabilities: { crossPlatformClient: supported } }) as RuntimeStatus;
const host = (platform: string, supported = true) => ({ platform, deviceRole: 'host', crossPlatform: supported, capabilities: { codecs: ['h264', 'hevc'] } }) as Device;

describe('native connection compatibility', () => {
  it.each([['Windows', 'Windows'], ['macOS', 'macOS'], ['macOS', 'Windows'], ['Windows', 'macOS']])('allows verified %s client to %s host', (client, target) => {
    expect(nativeCompatibilityReason(runtime(client), host(target), 'auto')).toBeNull();
  });
  it('preserves the legacy macOS client to Windows host', () => {
    expect(nativeCompatibilityReason(runtime('macOS', false), host('Windows', false), 'hevc')).toBeNull();
  });
  it('requires both engines for new directions and rejects client-only identities', () => {
    expect(nativeCompatibilityReason(runtime('Windows', false), host('macOS'), 'auto')).toContain('Update');
    expect(nativeCompatibilityReason(runtime('macOS'), host('macOS', false), 'auto')).toContain('Update');
    expect(nativeCompatibilityReason(runtime('macOS'), { ...host('Windows'), deviceRole: 'client' }, 'auto')).toContain('client');
  });
  it('blocks unsupported HEVC before a session is created', () => {
    expect(nativeCompatibilityReason(runtime('Windows'), host('macOS'), 'hevc')).toContain('H.264');
  });
});
