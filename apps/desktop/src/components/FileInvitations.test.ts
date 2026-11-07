import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
const mocks = vi.hoisted(() => ({ status: vi.fn() }));
vi.mock('../lib/platform', () => ({ sessionFileStatus: mocks.status }));
import FileInvitations from './FileInvitations.svelte';
const incoming = { id: 'file-a', name: 'report.txt', size: 10, bytes: 0, state: 'offered', error: null, savedPath: null };
beforeEach(() => {vi.resetAllMocks(); mocks.status.mockResolvedValue({ available: true, incoming, outgoing: null });});
afterEach(cleanup);
it('shows an incoming file anywhere in the desktop app and navigates to its consent panel', async () => {
  const navigate = vi.fn();
  render(FileInvitations, { hostSession: 'host-a', clientSession: null, navigate });
  await waitFor(() => expect(screen.getByText('Incoming file: report.txt')).toBeInTheDocument());
  expect(mocks.status).toHaveBeenCalledExactlyOnceWith('host', 'host-a');
  expect(navigate).not.toHaveBeenCalled();
  await fireEvent.click(screen.getByRole('button', { name: 'Review file' }));
  expect(navigate).toHaveBeenCalledExactlyOnceWith('host');
});
it('discards notifications returned after the host session changes', async () => {
  let resolve!: (value: unknown) => void;
  mocks.status.mockReturnValue(new Promise((done) => { resolve = done; }));
  const view = render(FileInvitations, { hostSession: 'host-a', clientSession: null, navigate: vi.fn() });
  await view.rerender({ hostSession: null });
  resolve({ available: true, incoming, outgoing: null });
  await waitFor(() => expect(screen.queryByText('Incoming file: report.txt')).not.toBeInTheDocument());
});
