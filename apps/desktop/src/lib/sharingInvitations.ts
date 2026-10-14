import type { Device, SharingGroup } from './types';

export function incomingSharingInvitations(groups: readonly SharingGroup[], viewerId: string | null, activeSessionId?: string | null) {
  if (!viewerId) return [];
  return groups.filter((group) => group.state !== 'stopped').flatMap((group) => group.members
    .filter((member) => member.viewerDeviceId === viewerId && (member.state === 'invited' || member.state === 'accepted') && (!member.sessionId || member.sessionId !== activeSessionId))
    .map((member) => ({ group, member })));
}

/** A name does not identify a recipient. Invite its current online client role. */
export function sharingViewerCandidates(devices: readonly Device[], localIds: readonly (string | null)[], ownComputer?: string, reserved: readonly string[] = []): Device[] {
  const current = new Map<string, Device>();
  for (const device of devices) {
    if (device.deviceRole !== 'client' || !device.online || !device.crossPlatform || !device.capabilities.nativeTransport ||
        localIds.includes(device.id) || reserved.includes(device.id) || (ownComputer && device.computerId === ownComputer)) continue;
    const key = device.computerId ?? device.id;
    const previous = current.get(key);
    if (!previous || (Date.parse(device.lastSeenAt ?? '') || 0) > (Date.parse(previous.lastSeenAt ?? '') || 0)) current.set(key, device);
  }
  return [...current.values()].sort((left, right) => left.name.localeCompare(right.name));
}
