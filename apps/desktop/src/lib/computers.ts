import type { Device } from './types';

export interface Computer extends Device {
  memberIds: string[];
  requesterId?: string;
}

export function isOlderRegistration(device: Device): boolean {
  return !device.computerId && !device.online &&
    ['Sanser Host', 'This Mac', 'This Windows PC', 'This computer'].includes(device.name);
}

/** Only a saved installation ID proves two role registrations are one computer. */
export function groupComputers(devices: readonly Device[], localIds: readonly (string | null)[] = []): Computer[] {
  const groups = new Map<string, Device[]>();
  for (const device of devices) {
    const key = device.computerId ? `computer:${device.computerId}` : `device:${device.id}`;
    const members = groups.get(key) ?? [];
    if (!members.some((item) => item.id === device.id)) members.push(device);
    groups.set(key, members);
  }
  return [...groups.values()].filter((members) => !members.some((device) => localIds.includes(device.id))).flatMap((members) => {
    // Connect must use the host identity and its actual availability/capabilities.
    const primary = members.find((device) => device.deviceRole === 'host') ?? members[0];
    if (!primary) return [];
    const requester = members.find((device) => device.deviceRole === 'client' || (!device.deviceRole && device.platform.toLowerCase().includes('mac')));
    return [{ ...primary, memberIds: members.map((device) => device.id), requesterId: requester?.id,
      pinned: members.some((device) => device.pinned), streaming: members.some((device) => device.streaming) }];
  });
}
