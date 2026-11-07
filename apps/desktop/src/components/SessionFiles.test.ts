import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
const mocks = vi.hoisted(() => ({ status: vi.fn(), action: vi.fn() }));
vi.mock('../lib/platform', () => ({ sessionFileStatus: mocks.status, sessionFileAction: mocks.action }));
import SessionFiles from './SessionFiles.svelte';
import type { FileTransferAction } from '../lib/platform';
const transfer = { id: 'file-a', name: 'notes.txt', size: 10, bytes: 0, state: 'offered' as const, error: null, savedPath: null };
beforeEach(() => {
  vi.resetAllMocks();
  mocks.status.mockResolvedValue({ available: true, incoming: null, outgoing: null });
  mocks.action.mockResolvedValue({ available: true, incoming: null, outgoing: null });
});
afterEach(cleanup);
async function open(kind: 'host' | 'client' = 'client') {
  const view = render(SessionFiles, { sessionId: 'session-a', kind });
  await fireEvent.click(screen.getByText('File transfer'));
  await waitFor(() => expect(screen.getByLabelText('Choose a file')).toBeEnabled());
  return view;
}
it('never accepts an invitation or starts an upload automatically', async () => {
  mocks.status.mockResolvedValue({ available: true, incoming: transfer, outgoing: null });
  await open('host');
  expect(mocks.action).not.toHaveBeenCalled();
  await fireEvent.click(screen.getByRole('button', { name: 'Accept and save' }));
  expect(mocks.action).toHaveBeenCalledExactlyOnceWith('host', 'session-a', { operation: 'accept', id: 'file-a' });
});
it('offers only file metadata before the receiver accepts and cancels when the upload panel closes', async () => {
  mocks.action.mockResolvedValue({ available: true, incoming: null, outgoing: transfer });
  const view = await open();
  const file = new File(['secret-file'], 'notes.txt', { type: 'text/plain' });
  await fireEvent.change(screen.getByLabelText('Choose a file'), { target: { files: [file] } });
  expect(mocks.action).not.toHaveBeenCalled();
  await fireEvent.click(screen.getByRole('button', { name: 'Offer file' }));
  expect(mocks.action).toHaveBeenCalledExactlyOnceWith('client', 'session-a', { operation: 'offer', name: 'notes.txt', size: 11 });
  view.unmount();
  expect(mocks.action).toHaveBeenLastCalledWith('client', 'session-a', { operation: 'cancel', id: 'file-a' });
});
it('keeps view-only or unsupported native transfers disabled', async () => {
  mocks.status.mockResolvedValue({ available: false, incoming: null, outgoing: null });
  render(SessionFiles, { sessionId: 'session-a' });
  await fireEvent.click(screen.getByText('File transfer'));
  expect(screen.getByLabelText('Choose a file')).toBeDisabled();
  expect(screen.getByRole('button', { name: 'Offer file' })).toBeDisabled();
  expect(mocks.action).not.toHaveBeenCalled();
});
it('blocks double acceptance while pending and discards previous-session replies', async () => {
  mocks.status.mockResolvedValue({ available: true, incoming: transfer, outgoing: null });
  let resolve!: (value: unknown) => void;
  mocks.action.mockReturnValue(new Promise((done) => { resolve = done; }));
  const view = await open();
  const accept = screen.getByRole('button', { name: 'Accept and save' });
  await fireEvent.click(accept); await fireEvent.click(accept);
  expect(mocks.action).toHaveBeenCalledTimes(1);
  await view.rerender({ sessionId: 'session-b' });
  resolve({ available: true, incoming: { ...transfer, name: 'old-secret.txt', state: 'receiving' }, outgoing: null });
  await waitFor(() => expect(screen.queryByText(/old-secret/)).not.toBeInTheDocument());
});
it('declines explicitly and displays a verified save without opening the file', async () => {
  mocks.status.mockResolvedValue({ available: true, incoming: transfer, outgoing: null });
  await open();
  await fireEvent.click(screen.getByRole('button', { name: 'Decline' }));
  expect(mocks.action).toHaveBeenCalledExactlyOnceWith('client', 'session-a', { operation: 'cancel', id: 'file-a' });
  mocks.status.mockResolvedValue({ available: true, incoming: { ...transfer, state: 'completed', bytes: 10, savedPath: '/Downloads/Sanser/unique-notes.txt' }, outgoing: null });
  await waitFor(() => expect(screen.getByText('Saved: /Downloads/Sanser/unique-notes.txt')).toBeInTheDocument());
  expect(mocks.action).toHaveBeenCalledTimes(1);
});
it('reads bounded file slices, waits for write ACK and then finishes', async () => {
  const content = new Uint8Array(4097).fill(42);
  const file = new File([content], 'bytes.bin');
  const slice = vi.fn((start: number, end: number) => ({ arrayBuffer: () => Promise.resolve(content.slice(start, end).buffer) }));
  Object.defineProperty(file, 'slice', { value: slice });
  let outgoing = { ...transfer, name: 'bytes.bin', size: 4097, state: 'offered' as string };
  let offered = false;
  const chunkLengths: number[] = [];
  const snapshot = () => ({ available: true, incoming: null, outgoing: offered ? { ...outgoing } : null });
  mocks.status.mockImplementation(() => Promise.resolve(snapshot()));
  mocks.action.mockImplementation((_kind: string, _session: string, command: FileTransferAction) => {
    if (command.operation === 'offer') offered = true;
    if (command.operation === 'chunk') chunkLengths.push(command.bytes.length);
    if (command.operation === 'chunk' && command.offset === 4096) outgoing.bytes = 4097;
    if (command.operation === 'finish') outgoing.state = 'completed';
    return Promise.resolve(snapshot());
  });
  await open();
  await fireEvent.change(screen.getByLabelText('Choose a file'), { target: { files: [file] } });
  await fireEvent.click(screen.getByRole('button', { name: 'Offer file' }));
  outgoing = { ...outgoing, state: 'sending' };
  await waitFor(() => expect(slice).toHaveBeenCalledExactlyOnceWith(0, 4096));
  await new Promise<void>((resolve) => setTimeout(resolve, 150));
  expect(slice).toHaveBeenCalledTimes(1);
  expect(chunkLengths).toEqual([4096]);
  outgoing.bytes = 4096;
  await waitFor(() => expect(slice).toHaveBeenCalledTimes(2));
  expect(slice).toHaveBeenLastCalledWith(4096, 8192);
  await waitFor(() => expect(mocks.action).toHaveBeenCalledWith('client', 'session-a', { operation: 'finish', id: 'file-a' }));
  expect(chunkLengths).toEqual([4096, 1]);
});
