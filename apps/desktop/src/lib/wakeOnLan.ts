import type { ApiClient } from './api';
import { sendWakeOnLan, wakeOnLanInterfaces } from './platform';
import type { Device, WakeResult } from './types';

export type WakeSyncResult = 'shared' | 'cleared' | 'no-adapters' | 'unchanged';

/**
 * Shares (or withdraws) this host's wakeable adapters with the account.
 * Off by default: nothing is reported until the user enables it in Settings.
 */
export async function syncWakeOnLan(
  client: Pick<ApiClient, 'setWakeOnLan' | 'clearWakeOnLan'>,
  hostDeviceId: string,
  enabled: boolean,
  currentlyShared: boolean
): Promise<WakeSyncResult> {
  if (!enabled) {
    if (!currentlyShared) return 'unchanged';
    await client.clearWakeOnLan(hostDeviceId);
    return 'cleared';
  }
  const interfaces = await wakeOnLanInterfaces();
  if (interfaces.length === 0) {
    if (currentlyShared) await client.clearWakeOnLan(hostDeviceId);
    return 'no-adapters';
  }
  await client.setWakeOnLan(hostDeviceId, interfaces);
  return 'shared';
}

/** A Wake button is offered only for an offline computer that shared its adapters. */
export function canWake(device: Pick<Device, 'online' | 'wakeOnLan'>): boolean {
  return !device.online && (device.wakeOnLan?.length ?? 0) > 0;
}

export async function wakeDevice(device: Pick<Device, 'wakeOnLan'>): Promise<WakeResult> {
  const targets = device.wakeOnLan ?? [];
  if (targets.length === 0) return { sameLan: false, packetsSent: 0 };
  return sendWakeOnLan(targets);
}
