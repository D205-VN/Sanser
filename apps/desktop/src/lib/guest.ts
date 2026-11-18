import { PROTOCOL_VERSION, SANSER_VERSION, type GuestDeviceInfo, type RuntimeStatus } from './types';

/** What this computer reports when it joins a support session as a guest. */
export function guestDeviceInfo(runtime: RuntimeStatus, routeAddress: string | null): GuestDeviceInfo {
  if (runtime.capabilities.clientEngine.state !== 'available') {
    throw new Error(runtime.capabilities.clientEngine.reason ?? 'The native client engine is unavailable');
  }
  return {
    platform: runtime.platform,
    crossPlatform: runtime.capabilities.crossPlatformClient === true,
    osVersion: runtime.platform,
    version: SANSER_VERSION,
    protocolVersion: PROTOCOL_VERSION,
    codecs: runtime.capabilities.clientCodecs ?? ['h264', 'hevc'],
    nativeTransport: runtime.capabilities.nativeDirect.state === 'available',
    webRtc: runtime.capabilities.webRtc.state === 'available',
    audio: runtime.capabilities.clientAudio ?? false,
    gamepad: runtime.capabilities.gamepad.state === 'available',
    ...(routeAddress ? { routeAddress } : {})
  };
}

/** Accepts "123456789", "123 456 789" or "123-456-789". */
export function normalizeAccessCode(value: string): string | null {
  const digits = value.replace(/[\s-]/g, '');
  return /^\d{9}$/.test(digits) ? digits : null;
}

/** Shows a 9-digit code as three groups of three. */
export function formatAccessCode(value: string): string {
  const digits = value.replace(/\D/g, '');
  return digits.length === 9 ? `${digits.slice(0, 3)} ${digits.slice(3, 6)} ${digits.slice(6)}` : value;
}
