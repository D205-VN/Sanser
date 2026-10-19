import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { GroupVoiceScope, GroupVoiceStatus } from '../lib/platform';
const { setVoice, status } = vi.hoisted(() => ({ setVoice: vi.fn(), status: vi.fn() }));
vi.mock('../lib/platform', () => ({ setGroupVoice: setVoice, groupVoiceStatus: status }));
import GroupVoice from './GroupVoice.svelte';
const scope = { kind: 'client' as const, sharingId: 'group', sessionId: 'session' };
beforeEach(() => {
  setVoice.mockReset(); status.mockReset();
  status.mockResolvedValue({ joined: false, microphone: false, deafened: false, capture: false, code: 'idle' });
  setVoice.mockImplementation((_scope: GroupVoiceScope, consent: Pick<GroupVoiceStatus, 'joined' | 'microphone' | 'deafened'>) => {
    status.mockResolvedValue({ ...consent, capture: consent.microphone, code: consent.microphone ? 'running' : 'listening' }); return Promise.resolve();
  });
});
afterEach(cleanup);
it('offers listening first and opens microphone only after a separate unmute action', async () => {
  render(GroupVoice, scope);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Join voice' })).toBeEnabled());
  expect(setVoice).not.toHaveBeenCalled();
  await fireEvent.click(screen.getByRole('button', { name: 'Join voice' }));
  await waitFor(() => expect(screen.getByRole('button', { name: 'Unmute mic' })).toBeEnabled());
  expect(setVoice).toHaveBeenLastCalledWith(scope, { joined: true, microphone: false, deafened: false });
  await fireEvent.click(screen.getByRole('button', { name: 'Unmute mic' }));
  await waitFor(() => expect(screen.getByText('Microphone on')).toBeInTheDocument());
  await fireEvent.click(screen.getByRole('button', { name: 'Deafen voice' }));
  await waitFor(() => expect(screen.getByText('Voice deafened · microphone off')).toBeInTheDocument());
  expect(setVoice).toHaveBeenLastCalledWith(scope, { joined: true, microphone: false, deafened: true });
});
it('shows permission refusal instead of claiming the microphone is on', async () => {
  render(GroupVoice, scope);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Join voice' })).toBeEnabled());
  await fireEvent.click(screen.getByRole('button', { name: 'Join voice' }));
  await waitFor(() => expect(screen.getByRole('button', { name: 'Unmute mic' })).toBeEnabled());
  setVoice.mockImplementation(() => { status.mockResolvedValue({ joined: true, microphone: true, deafened: false, capture: false, code: 'permission-denied' }); return Promise.resolve(); });
  await fireEvent.click(screen.getByRole('button', { name: 'Unmute mic' }));
  await waitFor(() => expect(screen.getByText(/Microphone access was denied/)).toBeInTheDocument());
  expect(screen.queryByText('Microphone on')).not.toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Unmute mic' })).toBeInTheDocument();
});

it('explains an older native engine without sending an unsupported voice flag or command', async () => {
  status.mockResolvedValue({ joined: false, microphone: false, deafened: false, capture: false, code: 'unsupported' });
  render(GroupVoice, scope);
  await waitFor(() => expect(screen.getByText(/Update Sanser on this computer to enable group voice/)).toBeInTheDocument());
  expect(screen.getByRole('button', { name: 'Join voice' })).toBeDisabled();
  expect(setVoice).not.toHaveBeenCalled();
});
