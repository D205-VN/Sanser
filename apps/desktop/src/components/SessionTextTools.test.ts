import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
const mocks = vi.hoisted(() => ({ capabilities: vi.fn(), action: vi.fn() }));
vi.mock('../lib/platform', () => ({ sessionUserCapabilities: mocks.capabilities, sessionUserAction: mocks.action }));
import SessionTextTools from './SessionTextTools.svelte';
beforeEach(() => {
  vi.resetAllMocks();
  mocks.capabilities.mockResolvedValue({ unicodeInput: true, clipboard: true, localClipboard: true });
  mocks.action.mockResolvedValue('');
});
afterEach(cleanup);
async function open() {
  const view = render(SessionTextTools, { sessionId: 'session-a' });
  await fireEvent.click(screen.getByText('Text and clipboard'));
  await waitFor(() => expect(screen.getByRole('button', { name: 'Read host clipboard' })).toBeEnabled());
  return view;
}
it('sends Unicode as one session-scoped action and never reads the local clipboard automatically', async () => {
  await open();
  expect(mocks.action).not.toHaveBeenCalled();
  await fireEvent.input(screen.getByRole('textbox'), { target: { value: 'Tiếng Việt 😀' } });
  await fireEvent.click(screen.getByRole('button', { name: 'Send text' }));
  expect(mocks.action).toHaveBeenCalledExactlyOnceWith('session-a', 'text', 'Tiếng Việt 😀');
});
it('receives text for review without replacing the local clipboard until Copy locally is pressed', async () => {
  mocks.action.mockResolvedValue('private clipboard text');
  await open();
  await fireEvent.click(screen.getByRole('button', { name: 'Read host clipboard' }));
  await waitFor(() => expect(screen.getByRole('textbox')).toHaveValue('private clipboard text'));
  expect(mocks.action).toHaveBeenCalledTimes(1);
  await fireEvent.click(screen.getByRole('button', { name: 'Copy locally' }));
  expect(mocks.action).toHaveBeenLastCalledWith('session-a', 'clipboard-local-write', 'private clipboard text');
});
it('blocks repeated requests while pending and discards a previous session reply', async () => {
  let resolve!: (value: string) => void;
  mocks.action.mockReturnValue(new Promise<string>((done) => { resolve = done; }));
  const view = await open();
  const read = screen.getByRole('button', { name: 'Read host clipboard' });
  await fireEvent.click(read); await fireEvent.click(read);
  expect(mocks.action).toHaveBeenCalledTimes(1);
  await view.rerender({ sessionId: 'session-b' });
  resolve('old-session secret');
  await waitFor(() => expect(screen.getByRole('textbox')).toHaveValue(''));
  expect(screen.queryByText('Received. Review the text, then copy it locally if needed.')).not.toBeInTheDocument();
});
it('keeps unsupported native features disabled', async () => {
  mocks.capabilities.mockResolvedValue({ unicodeInput: false, clipboard: false, localClipboard: false });
  render(SessionTextTools, { sessionId: 'session-a' });
  await fireEvent.click(screen.getByText('Text and clipboard'));
  await fireEvent.input(screen.getByRole('textbox'), { target: { value: 'ế' } });
  expect(screen.getByRole('button', { name: 'Send text' })).toBeDisabled();
  expect(screen.getByRole('button', { name: 'Read host clipboard' })).toBeDisabled();
  expect(screen.getByRole('button', { name: 'Write host clipboard' })).toBeDisabled();
  expect(mocks.action).not.toHaveBeenCalled();
});
it('enforces the clipboard byte limit and the separate Unicode edit limit', async () => {
  await open();
  await fireEvent.input(screen.getByRole('textbox'), { target: { value: 'ế'.repeat(3000) } });
  expect(screen.getByRole('button', { name: 'Send text' })).toBeDisabled();
  expect(screen.getByRole('button', { name: 'Write host clipboard' })).toBeDisabled();
  expect(mocks.action).not.toHaveBeenCalled();
});
it('keeps intermediate IME candidates local and blocks clipboard replacement until composition ends', async () => {
  await open();
  const field = screen.getByRole('textbox');
  await fireEvent.compositionStart(field);
  await fireEvent.input(field, { target: { value: 'tiee' }, isComposing: true });
  for (const button of screen.getAllByRole('button')) expect(button).toBeDisabled();
  await fireEvent.click(screen.getByRole('button', { name: 'Send text' }));
  expect(mocks.action).not.toHaveBeenCalled();
  await fireEvent.input(field, { target: { value: 'tiếng e\u0301 😀' }, isComposing: true });
  await fireEvent.compositionEnd(field, { data: 'tiếng e\u0301 😀' });
  await fireEvent.click(screen.getByRole('button', { name: 'Send text' }));
  expect(mocks.action).toHaveBeenCalledExactlyOnceWith('session-a', 'text', 'tiếng e\u0301 😀');
});
it('cancelling composition sends nothing and a new session cannot inherit the old draft', async () => {
  const view = await open();
  const field = screen.getByRole('textbox');
  await fireEvent.compositionStart(field);
  await fireEvent.input(field, { target: { value: '候補' }, isComposing: true });
  await fireEvent.input(field, { target: { value: '' } });
  await fireEvent.compositionEnd(field);
  expect(screen.getByRole('button', { name: 'Send text' })).toBeDisabled();
  expect(mocks.action).not.toHaveBeenCalled();
  await fireEvent.compositionStart(field);
  await fireEvent.input(field, { target: { value: 'bản nháp cũ' }, isComposing: true });
  await view.rerender({ sessionId: 'session-b' });
  await waitFor(() => expect(field).toHaveValue(''));
  expect(screen.queryByText('Composing text…')).not.toBeInTheDocument();
});
