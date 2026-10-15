import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { connection } from './stores/connection';
import type { ConnectionSession } from './lib/types';

const sharingActions = vi.hoisted(() => ({ respond: vi.fn().mockResolvedValue(undefined), refresh: vi.fn().mockResolvedValue(undefined) }));

const api = vi.hoisted(() => ({
  devices: vi.fn().mockResolvedValue({ items: [] }),
  getSession: vi.fn(),
  loginSessions: vi.fn().mockResolvedValue([])
}));
vi.mock('./stores/session', async () => {
  const { writable } = await import('svelte/store');
  return { session: {
    ...writable({ ready: true, busy: false, mode: 'cloud', account: { id: 'test-account', email: 'test@example.test' }, serverUrl: 'https://sanser.example', error: null }),
    initialize: vi.fn().mockResolvedValue(undefined),
    isInactive: () => false,
    client: () => api
  } };
});
vi.mock('./lib/platform', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./lib/platform')>();
  return { ...actual, loadNativePreferences: vi.fn().mockResolvedValue(null), saveNativePreferences: vi.fn().mockResolvedValue(undefined) };
});
vi.mock('./stores/sharing', async () => {
  const { writable } = await import('svelte/store');
  return { sharing: { ...writable({ groups: [], devices: [], ownGroup: null, supported: true, paused: false, busy: false, error: null, peerErrors: {}, peerVideo: {} }),
    initialize: vi.fn().mockResolvedValue(undefined), shutdown: vi.fn().mockResolvedValue(undefined), refresh: sharingActions.refresh, respond: sharingActions.respond } };
});
vi.mock('./stores/presence', async () => {
  const { writable } = await import('svelte/store');
  return { presence: { ...writable({ deviceId: null, online: false, busy: false, routeAddress: null, error: null }), start: vi.fn(), stop: vi.fn() } };
});
import { sharing, type SharingState } from './stores/sharing';
import type { Writable } from 'svelte/store';
import { presence, type PresenceState } from './stores/presence';
import type { SharingGroup } from './lib/types';
import App from './App.svelte';

const sharingState = sharing as unknown as Writable<SharingState>;
const presenceState = presence as unknown as Writable<PresenceState>;

beforeEach(() => {
  sharingState.update((state) => ({ ...state, groups: [], devices: [], ownGroup: null, error: null }));
  presenceState.set({ deviceId: null, online: false, busy: false, routeAddress: null, error: null });
});

afterEach(() => { cleanup(); connection.clear(); vi.restoreAllMocks(); });

it('keeps session monitoring mounted while navigating every workspace page', async () => {
  const pending = { id: 'session-123', status: 'pending', networkMode: 'auto', transport: null } as ConnectionSession;
  api.getSession.mockResolvedValue(pending);
  connection.begin(pending);
  const { container } = render(App);
  await screen.findByRole('heading', { name: 'Your computers' });
  const persistent = container.querySelector('.persistent-session');
  expect(persistent).not.toBeNull();
  await fireEvent.click(screen.getByRole('button', { name: 'Active session' }));
  expect(persistent).not.toHaveAttribute('hidden');
  const scroller = container.querySelector<HTMLElement>('.page-scroll');
  if (!scroller) throw new Error('Workspace scroll container is missing');
  scroller.scrollTop = 240;
  await fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
  expect(scroller.scrollTop).toBe(0);
  expect(persistent).toHaveAttribute('hidden');
  expect(container.querySelector('.persistent-session')).toBe(persistent);
  await waitFor(() => expect(api.getSession).toHaveBeenCalledWith('session-123'), { timeout: 3_000 });
  expect(within(screen.getByRole('navigation', { name: 'Main navigation' })).queryByRole('button', { name: 'Diagnostics' })).not.toBeInTheDocument();
  await fireEvent.click(within(screen.getByRole('navigation', { name: 'Settings sections' })).getByRole('button', { name: 'Diagnostics' }));
  expect(screen.queryByRole('heading', { name: 'Connection details' })).not.toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Export diagnostics' })).toBeInTheDocument();
  await fireEvent.click(screen.getByRole('button', { name: 'Show connection details' }));
  expect(screen.getByRole('heading', { name: 'Connection details' })).toBeInTheDocument();
  expect(container.querySelector('.persistent-session')).toBe(persistent);
  await fireEvent.click(screen.getByRole('button', { name: 'Hide connection details' }));
  expect(screen.queryByRole('heading', { name: 'Connection details' })).not.toBeInTheDocument();
  for (const page of ['Host', 'About', 'Computers']) {
    await fireEvent.click(within(screen.getByRole('navigation', { name: 'Main navigation' })).getByRole('button', { name: page }));
    expect(container.querySelector('.persistent-session')).toBe(persistent);
  }
});

it('clears status filters from the empty search state', async () => {
  api.devices.mockResolvedValueOnce({ items: [{
    id: 'remote-pc', name: 'Studio PC', platform: 'Windows', gpu: 'GPU', online: false,
    streaming: false, pinned: false, latencyMs: null, networkQuality: 'unknown', route: null,
    capabilities: { codecs: ['h264'], nativeTransport: true, webRtc: false, audio: true, gamepad: false }
  }] });
  render(App);
  await screen.findByRole('heading', { name: 'Studio PC' });
  await fireEvent.input(screen.getByRole('searchbox', { name: 'Search computers' }), { target: { value: 'not-found' } });
  expect(screen.getByRole('heading', { name: 'No matching computers' })).toBeInTheDocument();
  await fireEvent.click(screen.getByRole('button', { name: 'Clear filters' }));
  expect(screen.getByRole('heading', { name: 'Studio PC' })).toBeInTheDocument();
});

it('provides a list layout without changing the device list', async () => {
  api.devices.mockResolvedValue({ items: [{
    id: 'remote-pc', name: 'Studio PC', platform: 'Windows', gpu: null, online: false,
    streaming: false, pinned: false, latencyMs: null, networkQuality: 'unknown', route: null,
    capabilities: { codecs: ['h264'], nativeTransport: true, webRtc: false, audio: false, gamepad: false }
  }] });
  const { container } = render(App);
  await screen.findByRole('heading', { name: 'Studio PC' });
  await fireEvent.click(screen.getByRole('button', { name: 'List view' }));
  expect(container.querySelector('.computer-list')).not.toBeNull();
  expect(screen.getByRole('heading', { name: 'Studio PC' })).toBeInTheDocument();
  await fireEvent.click(screen.getByRole('button', { name: 'Card view' }));
  expect(container.querySelector('.computer-list')).toBeNull();
  await fireEvent.click(screen.getByLabelText('Manage Studio PC'));
  await fireEvent.click(screen.getByRole('button', { name: 'View diagnostics' }));
  expect(await screen.findByRole('heading', { name: 'Connection details' })).toBeInTheDocument();
  expect(within(screen.getByRole('navigation', { name: 'Main navigation' })).getByRole('button', { name: 'Settings' })).toHaveAttribute('aria-current', 'page');
  expect(within(screen.getByRole('navigation', { name: 'Settings sections' })).getByRole('button', { name: 'Diagnostics' })).toHaveAttribute('aria-current', 'page');
  await fireEvent.click(within(screen.getByRole('navigation', { name: 'Main navigation' })).getByRole('button', { name: 'Settings' }));
  expect(screen.getByRole('heading', { name: 'Quality' })).toBeInTheDocument();
});


it('hides Update until an available update has been confirmed', async () => {
  render(App);
  await screen.findByRole('heading', { name: 'Your computers' });
  expect(screen.queryByRole('button', { name: 'Update' })).not.toBeInTheDocument();
});

it('shows one named computer for both roles and keeps older registrations available', async () => {
  const common = {
    name: 'Studio PC', platform: 'Windows', gpu: null, online: true,
    streaming: false, pinned: false, latencyMs: null, networkQuality: 'unknown', route: null,
    capabilities: { codecs: ['h264'], nativeTransport: true, webRtc: false, audio: false, gamepad: false }
  };
  api.devices.mockResolvedValue({ items: [
    { ...common, id: 'host-id', computerId: 'abcdef00-1234-4567-8901-123456789012', deviceRole: 'host' },
    { ...common, id: 'client-id', computerId: 'abcdef00-1234-4567-8901-123456789012', deviceRole: 'client' },
    { ...common, id: 'old-id', name: 'Sanser Host', online: false }
  ] });
  render(App);
  await screen.findByRole('heading', { name: 'Studio PC' });
  expect(screen.getAllByRole('heading', { name: 'Studio PC' })).toHaveLength(1);
  expect(screen.getByText(/ABCDEF/)).toBeInTheDocument();
  expect(screen.queryByRole('heading', { name: 'Sanser Host' })).not.toBeInTheDocument();
  await fireEvent.click(screen.getByText('Older offline registrations (1)'));
  expect(await screen.findByRole('heading', { name: 'Sanser Host' })).toBeInTheDocument();
});

it('shows Join for this computer before a sharing session exists', async () => {
  const group: SharingGroup = { id: 'sharing-group', hostDeviceId: 'remote-host', state: 'preparing', networkMode: 'auto', qualityProfile: 'balanced', maxViewers: 3, createdAt: 1, updatedAt: 1, members: [
    { id: 'invitation', viewerDeviceId: 'local-viewer', state: 'invited', accessMode: 'view-only', createdAt: 1, updatedAt: 1 }
  ] };
  sharingState.update((state) => ({ ...state, groups: [group] }));
  presenceState.set({ deviceId: 'local-viewer', online: true, busy: false, routeAddress: null, error: null });
  render(App);
  await screen.findByRole('heading', { name: 'Your computers' });
  const join = await screen.findByRole('button', { name: 'Join' });
  expect(join).toBeEnabled();
  await fireEvent.click(join);
  expect(sharingActions.respond).toHaveBeenCalledWith(group, group.members[0], true);
});

it('surfaces an invitation loading failure and offers a retry on Computers', async () => {
  sharingState.update((state) => ({ ...state, error: 'Server restarting' }));
  render(App);
  await screen.findByRole('heading', { name: 'Your computers' });
  expect(screen.getByText(/Screen sharing invitations could not be loaded/)).toBeInTheDocument();
  await fireEvent.click(screen.getByRole('button', { name: 'Retry invitations' }));
  expect(sharingActions.refresh).toHaveBeenCalled();
});
