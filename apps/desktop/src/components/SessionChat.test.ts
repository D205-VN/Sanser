import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
const mocks = vi.hoisted(() => ({ snapshot: vi.fn(), post: vi.fn(), annotations: vi.fn(), open: vi.fn(), close: vi.fn(), pen: vi.fn() }));
vi.mock('../lib/collab', async (original) => ({
  ...(await original<typeof import('../lib/collab')>()),
  collabSnapshot: mocks.snapshot, collabPost: mocks.post, setGroupAnnotations: mocks.annotations,
  openAnnotationOverlay: mocks.open, closeAnnotationOverlay: mocks.close, setAnnotationPen: mocks.pen
}));
vi.mock('../lib/platform', () => ({ isTauriRuntime: () => true }));
import SessionChat from './SessionChat.svelte';

const base = { available: true, host: false, peers: 1, legacyPeers: 0, annotations: true, cursor: 0, events: [] };
beforeEach(() => {
  vi.resetAllMocks();
  mocks.snapshot.mockResolvedValue(base);
  mocks.post.mockResolvedValue(1);
  for (const mock of [mocks.annotations, mocks.open, mocks.close, mocks.pen]) mock.mockResolvedValue(undefined);
});
afterEach(cleanup);

it('renders chat as plain text and sends with Enter', async () => {
  mocks.snapshot.mockResolvedValueOnce({ ...base, cursor: 1, events: [{ seq: 1, kind: 'chat', from: 'host', name: 'Office PC', own: false, text: '<img src=x onerror=alert(1)>', atMs: 0 }] });
  const view = render(SessionChat, { kind: 'client', sessionId: 'session-a', selfName: 'Laptop' });
  await fireEvent.click(screen.getByText('Chat and annotations'));
  await waitFor(() => expect(screen.getByText('<img src=x onerror=alert(1)>')).toBeInTheDocument());
  expect(view.container.querySelector('.messages img')).toBeNull();
  expect(screen.getByText('Office PC (host)')).toBeInTheDocument();
  const box = screen.getByRole('textbox');
  await waitFor(() => expect(box).toBeEnabled());
  await fireEvent.input(box, { target: { value: 'On my way' } });
  await fireEvent.keyDown(box, { key: 'Enter' });
  await waitFor(() => expect(mocks.post).toHaveBeenCalledWith({ kind: 'client', sessionId: 'session-a' }, { type: 'chat', name: 'Laptop', text: 'On my way' }));
});

it('opens the capture-excluded overlay for a capable session and closes it on unmount', async () => {
  const view = render(SessionChat, { kind: 'client', sessionId: 'session-a' });
  await waitFor(() => expect(mocks.open).toHaveBeenCalledWith({ kind: 'client', sessionId: 'session-a' }, 'en'));
  view.unmount();
  expect(mocks.close).toHaveBeenCalled();
});

it('lets the host turn annotations off for the group', async () => {
  mocks.snapshot.mockResolvedValue({ ...base, host: true });
  render(SessionChat, { kind: 'host' });
  await fireEvent.click(screen.getByText('Chat and annotations'));
  const toggle = await screen.findByRole('checkbox', { name: 'Allow everyone to annotate' });
  await waitFor(() => expect(toggle).toBeEnabled());
  await fireEvent.click(toggle);
  expect(mocks.annotations).toHaveBeenCalledWith(false);
});

it('hides chat with an update note when this engine lacks support', async () => {
  mocks.snapshot.mockResolvedValue({ ...base, available: false, peers: 0 });
  render(SessionChat, { kind: 'client', sessionId: 'session-a' });
  await fireEvent.click(screen.getByText('Chat and annotations'));
  expect(await screen.findByText(/Update Sanser on this computer/)).toBeInTheDocument();
  expect(screen.queryByRole('textbox')).toBeNull();
  expect(mocks.open).not.toHaveBeenCalled();
});
