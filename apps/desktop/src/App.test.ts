import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';
import { connection } from './stores/connection';
import type { ConnectionSession } from './lib/types';

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
import App from './App.svelte';

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


it('shows Update in the top-right title actions and opens its status dialog', async () => {
  HTMLDialogElement.prototype.showModal = function () { this.setAttribute('open', ''); };
  HTMLDialogElement.prototype.close = function () { this.removeAttribute('open'); };
  const { container } = render(App);
  await screen.findByRole('heading', { name: 'Your computers' });
  const actions = container.querySelector('.title-actions');
  if (!actions) throw new Error('Title actions are missing');
  const button = within(actions as HTMLElement).getByRole('button', { name: 'Update' });
  await fireEvent.click(button);
  expect(await screen.findByRole('dialog')).toHaveTextContent('Open the desktop app to update');
});
