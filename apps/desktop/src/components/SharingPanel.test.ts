import { cleanup, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';
import type { Writable } from 'svelte/store';
import type { SharingState } from '../stores/sharing';
vi.mock('../stores/sharing', async () => {
  const { writable } = await import('svelte/store');
  return { sharing: { ...writable({ supported: true, ownGroup: {
    id: 'group', maxViewers: 3, members: [
      { id: 'a', viewerDeviceId: 'viewer-a', state: 'accepted', startedAt: 1 },
      { id: 'b', viewerDeviceId: 'viewer-b', state: 'accepted', startedAt: 1 }
    ] }, paused: false, devices: [], peerErrors: {}, peerVideo: {
      b: { viewerId: 'b', ready: true, running: true, videoState: 'paused', pauses: 1, backoffMs: 0 }
    }, busy: false, error: null }), stop: vi.fn(), remove: vi.fn(), invite: vi.fn() } };
});
vi.mock('../stores/host', async () => {
  const { writable } = await import('svelte/store');
  return { host: writable({ online: true, deviceId: 'host' }) };
});
vi.mock('../stores/presence', async () => {
  const { writable } = await import('svelte/store');
  return { presence: writable({ deviceId: 'viewer-local' }) };
});
import { sharing } from '../stores/sharing';
import SharingPanel from './SharingPanel.svelte';
afterEach(cleanup);

it('explains automatic pause and probing and counts a viewer again only after recovery', async () => {
  render(SharingPanel);
  expect(screen.getByText(/Video paused · viewer is falling behind/)).toBeInTheDocument();
  expect(screen.getByText('1 watching · 2/3 places reserved')).toBeInTheDocument();
  const store = sharing as unknown as Writable<SharingState>;
  store.update((state) => ({ ...state, peerVideo: { b: { viewerId: 'b', ready: true, running: true, videoState: 'probing', pauses: 1, backoffMs: 500 } } }));
  await waitFor(() => expect(screen.getByText(/Testing recovery/)).toBeInTheDocument());
  expect(screen.getByText('1 watching · 2/3 places reserved')).toBeInTheDocument();
  store.update((state) => ({ ...state, peerVideo: { b: { viewerId: 'b', ready: true, running: true, videoState: 'active', pauses: 1, backoffMs: 0 } } }));
  await waitFor(() => expect(screen.getByText('2 watching · 2/3 places reserved')).toBeInTheDocument());
  expect(screen.queryByText(/Testing recovery/)).not.toBeInTheDocument();
  store.update((state) => ({ ...state, peerVideo: { b: { viewerId: 'b', ready: false, running: false, videoState: 'paused', pauses: 1, backoffMs: 0 } } }));
  await waitFor(() => expect(screen.getByText(/Connection stopped/)).toBeInTheDocument());
  expect(screen.queryByText(/Retrying automatically/)).not.toBeInTheDocument();
  expect(screen.getByText('1 watching · 2/3 places reserved')).toBeInTheDocument();
});
