import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { HistoryEntry } from '../lib/types';

const mocks = vi.hoisted(() => ({ connectionHistory: vi.fn() }));
vi.mock('../stores/session', async () => {
  const { writable } = await import('svelte/store');
  return { session: { ...writable({ mode: 'cloud' }), client: () => ({ connectionHistory: mocks.connectionHistory }) } };
});
vi.mock('../stores/presence', async () => {
  const { writable } = await import('svelte/store');
  return { presence: writable({ deviceId: 'local-client' }) };
});
vi.mock('../stores/host', async () => {
  const { writable } = await import('svelte/store');
  return { host: writable({ deviceId: 'local-host' }) };
});
import History from './History.svelte';

const entry = (id: string, overrides: Partial<HistoryEntry> = {}): HistoryEntry => ({
  id, requesterDeviceId: 'local-client', requesterName: 'Laptop', requesterPlatform: 'macos-aarch64',
  hostDeviceId: 'remote-host', hostName: 'Office PC', hostPlatform: 'windows-x86_64', state: 'disconnected',
  accessMode: 'control', route: 'direct', sharedView: false, startedAt: '2026-11-01T10:00:00.000Z',
  connectedAt: '2026-11-01T10:00:05.000Z', endedAt: '2026-11-01T11:05:05.000Z', durationSeconds: 3_900,
  endReason: 'disconnected', ...overrides
});

beforeEach(() => vi.resetAllMocks());
afterEach(cleanup);

it('lists who connected to which computer and how', async () => {
  mocks.connectionHistory.mockResolvedValue({ items: [entry('a'), entry('b', { accessMode: 'view-only', route: 'relay', endedAt: null, durationSeconds: null, state: 'accepted' })], nextCursor: null });
  render(History);
  expect(await screen.findAllByText('Office PC')).toHaveLength(2);
  expect(screen.getAllByText('Laptop (this computer)')).toHaveLength(2);
  expect(screen.getByText('1 h 5 min')).toBeInTheDocument();
  expect(screen.getByText('View only')).toBeInTheDocument();
  expect(screen.getByText('Relay')).toBeInTheDocument();
  expect(screen.getByText('In progress')).toBeInTheDocument();
  expect(mocks.connectionHistory).toHaveBeenCalledWith({ limit: 50, cursor: undefined });
});

it('filters by this computer and loads further pages', async () => {
  mocks.connectionHistory
    .mockResolvedValueOnce({ items: [entry('a')], nextCursor: null })
    .mockResolvedValueOnce({ items: [entry('b')], nextCursor: 'more' })
    .mockResolvedValueOnce({ items: [entry('c')], nextCursor: null });
  render(History);
  await screen.findByText('1 h 5 min');
  await fireEvent.click(screen.getByRole('button', { name: 'Connections to this computer' }));
  await waitFor(() => expect(mocks.connectionHistory).toHaveBeenLastCalledWith({ deviceIds: ['local-client', 'local-host'], direction: 'incoming', limit: 50, cursor: undefined }));
  await fireEvent.click(await screen.findByRole('button', { name: 'Load more' }));
  await waitFor(() => expect(mocks.connectionHistory).toHaveBeenLastCalledWith(expect.objectContaining({ cursor: 'more', direction: 'incoming' })));
  await waitFor(() => expect(screen.getAllByText('Office PC')).toHaveLength(2));
});

it('shows an empty state and errors', async () => {
  mocks.connectionHistory.mockResolvedValueOnce({ items: [], nextCursor: null });
  render(History);
  expect(await screen.findByText('No connections yet')).toBeInTheDocument();
  cleanup();
  mocks.connectionHistory.mockRejectedValueOnce(new Error('Server unavailable'));
  render(History);
  expect(await screen.findByRole('alert')).toHaveTextContent('Server unavailable');
});
