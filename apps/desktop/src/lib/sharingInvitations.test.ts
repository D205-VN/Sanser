import { expect, it } from 'vitest';
import { incomingSharingInvitations, sharingViewerCandidates } from './sharingInvitations';
import type { Device, SharingGroup } from './types';
const group: SharingGroup = { id: 'group', hostDeviceId: 'host', state: 'preparing', networkMode: 'auto', qualityProfile: 'balanced', maxViewers: 3, createdAt: 1, updatedAt: 1, members: [
  { id: 'pending', viewerDeviceId: 'local-client', state: 'invited', accessMode: 'view-only', createdAt: 1, updatedAt: 1 },
  { id: 'foreign', viewerDeviceId: 'other-client', state: 'invited', accessMode: 'view-only', createdAt: 1, updatedAt: 1 }
] };
it('keeps a pending invitation visible when neither it nor the local connection has a session ID', () => {
  expect(incomingSharingInvitations([group], 'local-client').map(({ member }) => member.id)).toEqual(['pending']);
  expect(incomingSharingInvitations([group], null)).toEqual([]);
});
it('hides only an already-open accepted session and never reopens a stopped group', () => {
  const pending = group.members[0]; if (!pending) throw new Error('Missing fixture');
  const accepted: SharingGroup = { ...group, state: 'active', members: [{ ...pending, state: 'accepted', sessionId: 'opened-session' }] };
  expect(incomingSharingInvitations([accepted], 'local-client', 'opened-session')).toEqual([]);
  expect(incomingSharingInvitations([accepted], 'local-client', 'another-session')).toHaveLength(1);
  expect(incomingSharingInvitations([{ ...group, state: 'stopped' }], 'local-client')).toEqual([]);
});
it('offers the online client registration, retaining distinct computers even with identical names', () => {
  const device = { id: 'online', computerId: 'current-computer', deviceRole: 'client', name: 'HUY', online: true, crossPlatform: true, capabilities: { nativeTransport: true }, lastSeenAt: '2026-10-14T13:00:00Z' } as Device;
  const candidates = sharingViewerCandidates([
    { ...device, id: 'offline-old', online: false }, device,
    { ...device, id: 'host-role', deviceRole: 'host' },
    { ...device, id: 'older-online', lastSeenAt: '2026-10-14T12:00:00Z' },
    { ...device, id: 'distinct-computer', computerId: 'other-computer' },
    { ...device, id: 'local', computerId: 'our-computer' },
    { ...device, id: 'reserved', computerId: 'reserved-computer' }
  ], ['local'], 'our-computer', ['reserved']);
  expect(candidates.map((item) => item.id)).toEqual(['online', 'distinct-computer']);
});
