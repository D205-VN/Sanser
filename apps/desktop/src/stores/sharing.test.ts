import { get } from 'svelte/store';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { RuntimeStatus, SharingGroup } from '../lib/types';
const mocks = vi.hoisted(() => ({
  api: { supportsSharingMedia: vi.fn(), sharingGroups: vi.fn(), devices: vi.fn(), createSharingGroup: vi.fn(),
    sessionCredentials: vi.fn(), hostSessions: vi.fn(), sharingHeartbeat: vi.fn(), getAccessToken: vi.fn(),
    removeSharingViewer: vi.fn(), stopSharingGroup: vi.fn(), respondSharingInvitation: vi.fn(), getSession: vi.fn() },
  coordinate: vi.fn(), release: vi.fn(), launch: vi.fn(), add: vi.fn(), remove: vi.fn(), stop: vi.fn(), relay: vi.fn(), stopRelay: vi.fn(), relayFailure: vi.fn()
}));
vi.mock('../lib/p2pSignaling', () => ({ coordinateP2pConnection: mocks.coordinate, releaseP2pConnection: mocks.release, directCheckFailure: () => 'failed' }));
vi.mock('../lib/platform', () => ({ launchEngine: mocks.launch, addSharingPeer: mocks.add, removeSharingPeer: mocks.remove,
  stopEngine: mocks.stop, startRelay: mocks.relay, stopRelay: mocks.stopRelay, relayPeerFailure: mocks.relayFailure,
  engineStatus: () => Promise.resolve({ running: true }) }));
vi.mock('./session', async () => {
  const { writable } = await import('svelte/store');
  return { session: { ...writable({ mode: 'cloud', account: { id: 'owner' } }), client: () => mocks.api } };
});
vi.mock('./host', async () => {
  const { writable } = await import('svelte/store');
  const store = writable({ online: true, deviceId: 'host', sessions: [], engineRunning: false, launchingSessionId: null });
  return { host: { ...store, setSharingActive: (active: boolean, viewers: number) => store.update((state) => ({ ...state, sharingActive: active, sharingViewers: viewers })) } };
});
vi.mock('./presence', async () => {
  const { writable } = await import('svelte/store');
  return { presence: writable({ deviceId: 'viewer-local' }) };
});
vi.mock('./preferences', async () => {
  const { writable } = await import('svelte/store');
  return { preferences: writable({ networkMode: 'auto', stream: { profile: 'balanced', codec: 'h264', fps: 60, bitrateMbps: 20, resolution: '1080p' } }) };
});
import { createSharingStore } from './sharing';

const runtime = { capabilities: { multiViewerHost: true } } as RuntimeStatus;
let group: SharingGroup;
let stores: ReturnType<typeof createSharingStore>[] = [];

beforeEach(() => {
  vi.useFakeTimers(); vi.resetAllMocks();
  group = { id: 'group', hostDeviceId: 'host', state: 'preparing', networkMode: 'auto', qualityProfile: 'balanced', maxViewers: 3, createdAt: 1, updatedAt: 1, members: [] };
  mocks.api.supportsSharingMedia.mockResolvedValue(true);
  mocks.api.sharingGroups.mockImplementation(() => Promise.resolve({ items: [group] }));
  mocks.api.devices.mockResolvedValue({ items: [] }); mocks.api.createSharingGroup.mockImplementation(() => Promise.resolve(group));
  mocks.api.sharingHeartbeat.mockResolvedValue(undefined); mocks.api.getAccessToken.mockResolvedValue('access-token');
  mocks.api.sessionCredentials.mockImplementation((id: string) => Promise.resolve({ wireProtocol: 'snv2', accessMode: 'view-only', sessionToken: `${id}-` + 'a'.repeat(48) }));
  mocks.api.hostSessions.mockImplementation(() => Promise.resolve({ items: group.members.filter((member) => member.sessionId).map((member) => ({ id: member.sessionId, status: 'accepted', requesterReadyAt: '2026-10-14T00:00:00Z' })) }));
  mocks.coordinate.mockImplementation((_api: unknown, id: string) => Promise.resolve({ localPort: 50000 + Number(id.slice(-1)) * 2, remotePort: 50001 + Number(id.slice(-1)) * 2, remoteAddress: '192.0.2.1' }));
  mocks.release.mockResolvedValue(undefined); mocks.launch.mockResolvedValue(undefined); mocks.add.mockResolvedValue(undefined); mocks.remove.mockResolvedValue(undefined); mocks.stop.mockResolvedValue(undefined);
  mocks.stopRelay.mockResolvedValue(undefined); mocks.relayFailure.mockResolvedValue(null); mocks.api.stopSharingGroup.mockResolvedValue(undefined);
});
afterEach(async () => { for (const store of stores) await store.shutdown(); stores = []; vi.useRealTimers(); });

async function startedStore() {
  mocks.api.sharingGroups.mockResolvedValueOnce({ items: [] });
  const store = createSharingStore(); stores.push(store); await store.initialize(runtime); await store.create();
  group = { ...group, state: 'active', members: [1, 2, 3].map((index) => ({ id: `member-${index}`, viewerDeviceId: `viewer-${index}`, sessionId: `session-${index}`, state: 'accepted', accessMode: 'view-only', createdAt: 1, updatedAt: 1 })) };
  await store.refresh(); await vi.advanceTimersByTimeAsync(1);
  return store;
}

it('starts one encoder for three viewers and does not claim they received a frame from command acknowledgement', async () => {
  const store = await startedStore();
  expect(mocks.launch).toHaveBeenCalledTimes(1); expect(mocks.add).toHaveBeenCalledTimes(2);
  for (const [request] of [...mocks.launch.mock.calls, ...mocks.add.mock.calls]) {
    expect(request).toMatchObject({ inputEnabled: false, audioEnabled: false, wireProtocol: 'snv2', sharingId: 'group', fps: 60 });
  }
  expect(get(store).ownGroup?.members.every((member) => member.startedAt === undefined)).toBe(true);
  mocks.api.removeSharingViewer.mockImplementation(() => {
    group = { ...group, members: group.members.map((member) => member.id === 'member-2' ? { ...member, state: 'removed' as const } : member) };
    return Promise.resolve(group);
  });
  const member = group.members[1]; if (!member) throw new Error('missing fixture');
  await store.remove(member);
  expect(mocks.remove).toHaveBeenCalledWith('group', 'member-2'); expect(mocks.stop).not.toHaveBeenCalled();
  expect(group.members.filter((viewer) => viewer.state === 'accepted')).toHaveLength(2);
});

it('keeps existing viewers and creates an independent relay for the failed Direct viewer', async () => {
  mocks.coordinate.mockImplementation((_api: unknown, id: string) => id === 'session-2'
    ? Promise.reject(new Error('No direct route')) : Promise.resolve({ localPort: 50010 + Number(id.slice(-1)), remotePort: 51000 + Number(id.slice(-1)), remoteAddress: '192.0.2.1' }));
  mocks.relay.mockResolvedValue({ enginePort: 52000, proxyPort: 52001, transport: 'wss' });
  await startedStore();
  expect(mocks.relay).toHaveBeenCalledWith(expect.objectContaining({ sharingId: 'group', sessionId: 'session-2' }));
  expect(mocks.launch.mock.calls.length + mocks.add.mock.calls.length).toBe(3);
  expect(mocks.stop).not.toHaveBeenCalled();
});

it('stops locally even when the server fails and does not restart from the old group on the next poll', async () => {
  const store = await startedStore(); mocks.api.stopSharingGroup.mockRejectedValue(new Error('Server unavailable'));
  await expect(store.stop()).rejects.toThrow('Server unavailable');
  expect(mocks.stop).toHaveBeenCalledWith('host');
  const attempts = mocks.launch.mock.calls.length + mocks.add.mock.calls.length;
  await store.refresh(); await vi.advanceTimersByTimeAsync(1);
  expect(get(store).paused).toBe(true); expect(mocks.launch.mock.calls.length + mocks.add.mock.calls.length).toBe(attempts);
});

it('does not automatically resume a sharing group from a previous application lifecycle', async () => {
  group = { ...group, state: 'active', members: [{ id: 'old', viewerDeviceId: 'old-viewer', sessionId: 'old-session', state: 'accepted', accessMode: 'view-only', createdAt: 1, updatedAt: 1 }] };
  const store = createSharingStore(); stores.push(store); await store.initialize(runtime);
  expect(get(store).paused).toBe(true); expect(mocks.coordinate).not.toHaveBeenCalled(); expect(mocks.launch).not.toHaveBeenCalled();
});

it('rejects a response for another computer before calling the consent API', async () => {
  const store = createSharingStore(); stores.push(store);
  await expect(store.respond(group, { id: 'foreign', viewerDeviceId: 'another-computer', state: 'invited', accessMode: 'view-only', createdAt: 1, updatedAt: 1 }, true)).rejects.toThrow('another computer');
  expect(mocks.api.respondSharingInvitation).not.toHaveBeenCalled();
});

it('allows a viewer to retry a failed relay with a fresh ready generation without disturbing other viewers', async () => {
  mocks.coordinate.mockImplementation((_api: unknown, id: string) => id === 'session-2'
    ? Promise.reject(new Error('No direct route')) : Promise.resolve({ localPort: 50100, remotePort: 50101, remoteAddress: '192.0.2.1' }));
  mocks.relay.mockResolvedValue({ enginePort: 52000, proxyPort: 52001, transport: 'wss' });
  const store = await startedStore();
  mocks.relayFailure.mockResolvedValueOnce('relay peer offline');
  await store.refresh(); await vi.advanceTimersByTimeAsync(1);
  expect(mocks.relay).toHaveBeenCalledTimes(1);
  mocks.api.hostSessions.mockResolvedValue({ items: group.members.map((member) => ({ id: member.sessionId, status: 'accepted', requesterReadyAt: '2026-10-14T00:00:01Z' })) });
  await store.refresh(); await vi.advanceTimersByTimeAsync(1);
  expect(mocks.relay).toHaveBeenCalledTimes(2);
  expect(mocks.stop).not.toHaveBeenCalled();
});

it('waits for a pending shutdown before initializing again', async () => {
  const store = await startedStore();
  let finishStop!: () => void;
  mocks.api.stopSharingGroup.mockReturnValueOnce(new Promise<void>((resolve) => { finishStop = resolve; }));
  const ending = store.shutdown();
  const duplicate = store.shutdown();
  const starting = store.initialize(runtime);
  await vi.advanceTimersByTimeAsync(1);
  expect(mocks.api.supportsSharingMedia).toHaveBeenCalledTimes(1);
  group = { ...group, members: [] }; mocks.api.sharingGroups.mockResolvedValue({ items: [] });
  finishStop(); await Promise.all([ending, duplicate, starting]);
  expect(mocks.api.supportsSharingMedia).toHaveBeenCalledTimes(2);
  expect(get(store).supported).toBe(true);
  expect(get(store).ownGroup).toBeNull();
});

it('releases a selected Direct port if sharing stops before native launch', async () => {
  mocks.api.sharingGroups.mockResolvedValueOnce({ items: [] });
  const store = createSharingStore(); stores.push(store); await store.initialize(runtime); await store.create();
  group = { ...group, state: 'active', members: [{ id: 'member-1', viewerDeviceId: 'viewer-1', sessionId: 'session-1', state: 'accepted', accessMode: 'view-only', createdAt: 1, updatedAt: 1 }] };
  let finishCheck!: (result: object) => void;
  mocks.coordinate.mockReturnValueOnce(new Promise((resolve) => { finishCheck = resolve; }));
  await store.refresh(); await vi.advanceTimersByTimeAsync(1);
  const stopping = store.stop();
  finishCheck({ attemptId: 'cancelled-attempt', localPort: 50100, remotePort: 50101, remoteAddress: '192.0.2.1' });
  await stopping;
  expect(mocks.release).toHaveBeenCalledWith(expect.objectContaining({ attemptId: 'cancelled-attempt' }));
  expect(mocks.launch).not.toHaveBeenCalled();
});

it('recovers invitation polling after the initial support request fails', async () => {
  mocks.api.supportsSharingMedia.mockRejectedValueOnce(new Error('Server restarting')).mockResolvedValue(true);
  group = { ...group, hostDeviceId: 'remote-host', members: [{ id: 'pending', viewerDeviceId: 'viewer-local', state: 'invited', accessMode: 'view-only', createdAt: 1, updatedAt: 1 }] };
  const store = createSharingStore(); stores.push(store); await store.initialize(runtime);
  expect(get(store).error).toBe('Server restarting');
  expect(mocks.api.sharingGroups).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(10_001);
  expect(get(store).supported).toBe(true); expect(get(store).error).toBeNull();
  expect(get(store).groups[0]?.members[0]?.id).toBe('pending');
});

it('lets Refresh retry invitation support immediately without signing out', async () => {
  mocks.api.supportsSharingMedia.mockRejectedValueOnce(new Error('Temporary failure')).mockResolvedValue(true);
  const store = createSharingStore(); stores.push(store); await store.initialize(runtime);
  await store.refresh();
  expect(get(store).supported).toBe(true); expect(mocks.api.supportsSharingMedia).toHaveBeenCalledTimes(2);
});

it('shares an in-flight initialization rather than invalidating its generation', async () => {
  let finish!: (value: boolean) => void;
  mocks.api.supportsSharingMedia.mockReturnValueOnce(new Promise<boolean>((resolve) => { finish = resolve; }));
  const store = createSharingStore(); stores.push(store);
  const first = store.initialize(runtime); const second = store.initialize(runtime);
  expect(mocks.api.supportsSharingMedia).toHaveBeenCalledTimes(1);
  finish(true); await Promise.all([first, second]);
  expect(get(store).supported).toBe(true); expect(mocks.api.sharingGroups).toHaveBeenCalledTimes(1);
});
