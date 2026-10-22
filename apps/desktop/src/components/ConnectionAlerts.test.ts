import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { ConnectionSession, Device, SharingGroup, SharingPeerVideoStatus } from '../lib/types';
const actions = vi.hoisted(() => ({ disconnect: vi.fn(), remove: vi.fn() }));
vi.mock('../stores/host', async () => {
  const { writable } = await import('svelte/store');
  return { host: { ...writable({ engineRunning: false, sessions: [] as ConnectionSession[] }), disconnect: actions.disconnect } };
});
vi.mock('../stores/sharing', async () => {
  const { writable } = await import('svelte/store');
  return { sharing: { ...writable({ ownGroup: null as SharingGroup | null, paused: false, peerVideo: {} as Record<string, SharingPeerVideoStatus>, devices: [] as Device[] }), remove: actions.remove } };
});
vi.mock('../stores/session', async () => {
  const { writable } = await import('svelte/store');
  return { session: writable({ mode: 'cloud', account: { id: 'account' } }) };
});
import { host } from '../stores/host';
import { sharing } from '../stores/sharing';
import { connectionAlerts } from '../stores/connectionAlerts';
import type { Writable } from 'svelte/store';
import ConnectionAlerts from './ConnectionAlerts.svelte';
const localHost = host as unknown as Writable<{ engineRunning: boolean; sessions: ConnectionSession[] }>;
const localShare = sharing as unknown as Writable<{ ownGroup: SharingGroup | null; paused: boolean; peerVideo: Record<string, SharingPeerVideoStatus>; devices: Device[] }>;
const target = { id: 'control-session', requesterDeviceId: 'device', status: 'accepted', requesterReadyAt: 'ready', accessMode: 'control' } as ConnectionSession;
const group = { id: 'group', members: [{ id: 'member', sessionId: 'viewer-session', viewerDeviceId: 'device', state: 'accepted', startedAt: 1, accessMode: 'view-only' }] } as SharingGroup;
beforeEach(() => {
  actions.disconnect.mockReset().mockResolvedValue(undefined); actions.remove.mockReset().mockResolvedValue(undefined);
  connectionAlerts.observe(null, []);
  localHost.set({ engineRunning: false, sessions: [] });
  localShare.set({ ownGroup: null, paused: false, peerVideo: {}, devices: [{ id: 'device', name: 'Office PC' }] as Device[] });
});
afterEach(cleanup);
it('shows a named sharing alert and stops only that viewer when requested', async () => {
  localShare.update((state) => ({ ...state, ownGroup: group })); render(ConnectionAlerts);
  expect(await screen.findByText('Office PC')).toBeInTheDocument();
  expect(screen.getByText(/Joined your screen sharing.*View only/)).toBeInTheDocument();
  await fireEvent.click(screen.getByRole('button', { name: 'Stop viewing' }));
  await waitFor(() => expect(actions.remove).toHaveBeenCalledExactlyOnceWith(group.members[0]));
  expect(actions.disconnect).not.toHaveBeenCalled();
});
it('distinguishes control approval and scopes Stop access to that session', async () => {
  localHost.set({ engineRunning: true, sessions: [target] }); render(ConnectionAlerts);
  expect(await screen.findByText(/Mouse & keyboard access/)).toBeInTheDocument();
  await fireEvent.click(screen.getByRole('button', { name: 'Stop access' }));
  await waitFor(() => expect(actions.disconnect).toHaveBeenCalledExactlyOnceWith(target.id));
  expect(actions.remove).not.toHaveBeenCalled();
});
it('dismisses a notice without ending the share and removes a notice when its viewer leaves', async () => {
  localShare.update((state) => ({ ...state, ownGroup: group })); render(ConnectionAlerts);
  await screen.findByText('Office PC');
  await fireEvent.click(screen.getByRole('button', { name: 'Dismiss connection alert for Office PC' }));
  expect(screen.queryByLabelText('Incoming computer connections')).not.toBeInTheDocument();
  expect(actions.remove).not.toHaveBeenCalled();
  localShare.update((state) => ({ ...state, ownGroup: { ...group, members: [{ ...group.members[0], id: 'new-member', sessionId: 'new-session' } as SharingGroup['members'][number]] } }));
  await screen.findByText('Office PC');
  localShare.update((state) => ({ ...state, ownGroup: null }));
  await waitFor(() => expect(screen.queryByLabelText('Incoming computer connections')).not.toBeInTheDocument());
});
