import { beforeEach, expect, it, vi } from 'vitest';
import type { ConnectionSession, Device, SharingGroup, SharingPeerVideoStatus } from './types';
const mocks = vi.hoisted(() => ({ native: false, granted: vi.fn(), request: vi.fn(), send: vi.fn() }));
vi.mock('./platform', () => ({ isTauriRuntime: () => mocks.native }));
vi.mock('@tauri-apps/plugin-notification', () => ({ isPermissionGranted: mocks.granted, requestPermission: mocks.request, sendNotification: mocks.send }));
import { alertPermission, hostConnectionAlerts, showHostConnectionAlert } from './connectionAlerts';
const target = { id: 'session', status: 'accepted', requesterReadyAt: 'ready', requesterDeviceId: 'viewer', accessMode: 'control' } as ConnectionSession;
const group = { id: 'group', members: [
  { id: 'm1', sessionId: 's1', state: 'accepted', viewerDeviceId: 'viewer', startedAt: 1 },
  { id: 'm2', sessionId: 's2', state: 'invited', viewerDeviceId: 'other' },
  { id: 'm3', sessionId: 's3', state: 'removed', viewerDeviceId: 'removed' }
] } as SharingGroup;
const devices = [{ id: 'viewer', name: 'Living room Mac' }] as Device[];
beforeEach(() => { vi.resetAllMocks(); mocks.native = true; });

it('includes actual accepted viewers, not invitations, removed members or paused groups', () => {
  const alerts = hostConnectionAlerts(false, [], group, false, {}, devices);
  expect(alerts).toEqual([{ id: 's1', sessionId: 's1', sharingId: 'group', memberId: 'm1', deviceName: 'Living room Mac', viewOnly: true }]);
  expect(hostConnectionAlerts(false, [], group, true, {}, devices)).toEqual([]);
  expect(hostConnectionAlerts(false, [], group, false, { m1: { running: false } as SharingPeerVideoStatus }, devices)).toEqual([]);
});
it('announces a newly authenticated native viewer without claiming an invitation is already connected', () => {
  const member = group.members[0]; if (!member) throw new Error('Missing sharing fixture');
  const waiting = { ...group, members: [{ ...member, startedAt: undefined }] };
  expect(hostConnectionAlerts(false, [], waiting, false, {}, devices)).toEqual([]);
  expect(hostConnectionAlerts(false, [], waiting, false, { m1: { ready: true, running: true } as SharingPeerVideoStatus }, devices)).toHaveLength(1);
});
it('identifies approved control access only after the native host starts', () => {
  expect(hostConnectionAlerts(false, [target], null, false, {}, devices)).toEqual([]);
  expect(hostConnectionAlerts(true, [target, { ...target, id: 'pending', status: 'pending' }], null, false, {}, devices)).toMatchObject([{ id: 'session', viewOnly: false }]);
});
it('does not prompt for permission during a connection and keeps OS denial harmless', async () => {
  mocks.granted.mockResolvedValue(false);
  await showHostConnectionAlert({ id: 's', sessionId: 's', deviceName: 'Mac', viewOnly: true }, () => true);
  expect(mocks.request).not.toHaveBeenCalled(); expect(mocks.send).not.toHaveBeenCalled();
});
it('only requests permission through the explicit enable action', async () => {
  mocks.granted.mockResolvedValue(false); mocks.request.mockResolvedValue('granted');
  expect(await alertPermission(true)).toBe('granted'); expect(mocks.request).toHaveBeenCalledOnce();
});
it('shows the device and access level when native notifications are allowed', async () => {
  mocks.granted.mockResolvedValue(true);
  await showHostConnectionAlert({ id: 's', sessionId: 's', sharingId: 'group', deviceName: 'Office PC', viewOnly: true }, () => true);
  expect(mocks.send).toHaveBeenCalledWith(expect.objectContaining({ body: 'Office PC joined your screen sharing. View only.' }));
});
it('does not notify about an ended session or sign-out after an asynchronous permission check', async () => {
  mocks.granted.mockResolvedValue(true);
  await showHostConnectionAlert({ id: 's', sessionId: 's', deviceName: 'PC', viewOnly: false }, () => false);
  expect(mocks.send).not.toHaveBeenCalled();
});
it('keeps browser previews working without OS access', async () => {
  mocks.native = false;
  expect(await alertPermission(true)).toBe('unavailable'); expect(mocks.request).not.toHaveBeenCalled();
});
