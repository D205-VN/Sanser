import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { RuntimeStatus } from '../lib/types';

const mocks = vi.hoisted(() => ({ accept: vi.fn(), save: vi.fn() }));
const requesterDeviceId = '22222222-2222-4222-8222-222222222222';
vi.mock('../stores/session', async () => {
  const { writable } = await import('svelte/store');
  return { session: writable({ mode: 'cloud' }) };
});
vi.mock('../stores/host', async () => {
  const { writable } = await import('svelte/store');
  return { host: { ...writable({
    online: true, busy: false, actionSessionId: null, error: null,
    sessions: [{ id: 'pending-request', requesterDeviceId: '22222222-2222-4222-8222-222222222222', status: 'pending', qualityProfile: 'auto', requestedCodec: 'auto' }]
  }), accept: mocks.accept } };
});
vi.mock('../stores/preferences', async () => {
  const { writable } = await import('svelte/store');
  return { preferences: { ...writable({
    host: { autoAcceptOwnDevices: true, inputEnabled: true, audioEnabled: false },
    trustedDeviceIds: []
  }), save: mocks.save } };
});
import Host from './Host.svelte';
const runtime = { platform: 'macOS', capabilities: { hostEngine: { state: 'available' } } } as RuntimeStatus;

beforeEach(() => {
  vi.resetAllMocks();
  mocks.accept.mockResolvedValue({ id: 'pending-request', requesterDeviceId });
  mocks.save.mockResolvedValue(undefined);
});
afterEach(cleanup);

it('explains why zero trusted computers still require approval and remembers explicit trust', async () => {
  render(Host, { runtime });
  expect(screen.getByText(/No trusted computers yet/)).toBeInTheDocument();
  await fireEvent.click(screen.getByRole('button', { name: 'Accept & trust' }));
  await waitFor(() => expect(mocks.save).toHaveBeenCalledWith(expect.objectContaining({ trustedDeviceIds: [requesterDeviceId] })));
  expect(mocks.accept).toHaveBeenCalledWith('pending-request');
});

it('does not remember ordinary one-time approval', async () => {
  render(Host, { runtime });
  await fireEvent.click(screen.getByRole('button', { name: /^Accept$/ }));
  expect(mocks.accept).toHaveBeenCalledWith('pending-request');
  expect(mocks.save).not.toHaveBeenCalled();
});

it('does not grant future access when accepting the request fails', async () => {
  mocks.accept.mockRejectedValue(new Error('Request expired'));
  render(Host, { runtime });
  await fireEvent.click(screen.getByRole('button', { name: 'Accept & trust' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Request expired');
  expect(mocks.save).not.toHaveBeenCalled();
});

it('reports accepted-but-not-remembered separately when saving trust fails', async () => {
  mocks.save.mockRejectedValue(new Error('Storage unavailable'));
  render(Host, { runtime });
  await fireEvent.click(screen.getByRole('button', { name: 'Accept & trust' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Connection accepted, but this computer could not be saved as trusted');
});
