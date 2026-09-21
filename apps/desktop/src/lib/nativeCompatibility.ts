import type { Device, RuntimeStatus, VideoCodec } from './types';

export function nativeCompatibilityReason(runtime: RuntimeStatus, host: Device, codec: VideoCodec): string | null {
  const windowsHost = host.platform.toLowerCase().includes('windows');
  const macClient = runtime.platform.toLowerCase().includes('mac');
  const role = host.deviceRole ?? (windowsHost ? 'host' : 'client');
  if (role !== 'host') return 'This device is registered as a client. Enable Host on that computer to receive connections.';
  const legacy = windowsHost && macClient;
  if (!legacy && (!runtime.capabilities.crossPlatformClient || !host.crossPlatform)) {
    return 'Update Sanser on both computers to enable this connection direction.';
  }
  if (codec === 'hevc' && !legacy) return 'This connection uses H.264. Select Auto or H.264 in Video settings.';
  if (codec !== 'auto' && !host.capabilities.codecs.includes(codec)) return 'The host does not support the selected video codec.';
  return null;
}
