import { get } from 'svelte/store';
import { beforeEach, expect, it, vi } from 'vitest';
const { setVoice, status } = vi.hoisted(() => ({ setVoice: vi.fn(), status: vi.fn() }));
vi.mock('../lib/platform', () => ({ setGroupVoice: setVoice, groupVoiceStatus: status }));
import { createGroupVoice } from './groupVoice';
import type { GroupVoiceScope, GroupVoiceStatus } from '../lib/platform';
const scope = { kind: 'client' as const, sharingId: 'group', sessionId: 'session' };
const off = { joined: false, microphone: false, deafened: false, capture: false, code: 'idle' };
beforeEach(() => {
  setVoice.mockReset(); status.mockReset();
  setVoice.mockImplementation((_scope: GroupVoiceScope, consent: Pick<GroupVoiceStatus, 'joined' | 'microphone' | 'deafened'>) => { status.mockResolvedValue({ ...consent, capture: consent.microphone, code: consent.microphone ? 'running' : 'listening' }); return Promise.resolve(); });
  status.mockResolvedValue(off);
});
it('does not open microphone on mount, passive status, or join; only explicit unmute captures', async () => {
  const voice = createGroupVoice(); voice.attach(scope);
  await voice.refresh(); await voice.toggleMicrophone();
  expect(setVoice).not.toHaveBeenCalled();
  await voice.join();
  expect(setVoice).toHaveBeenLastCalledWith(scope, { joined: true, microphone: false, deafened: false });
  expect(get(voice).capture).toBe(false);
  await voice.toggleMicrophone();
  expect(setVoice).toHaveBeenLastCalledWith(scope, { joined: true, microphone: true, deafened: false });
  expect(get(voice).capture).toBe(true);
  await voice.toggleDeafen();
  expect(setVoice).toHaveBeenLastCalledWith(scope, { joined: true, microphone: false, deafened: true });
  await voice.toggleDeafen();
  expect(get(voice).microphone).toBe(false);
});
it('does not claim an active microphone when the native permission is denied', async () => {
  const voice = createGroupVoice(); voice.attach(scope); await voice.join();
  setVoice.mockResolvedValue(undefined);
  status.mockResolvedValue({ ...off, joined: true, microphone: false, code: 'permission-denied' });
  await voice.toggleMicrophone();
  expect(get(voice)).toMatchObject({ capture: false, code: 'permission-denied' });
  await voice.toggleMicrophone();
  expect(setVoice).toHaveBeenLastCalledWith(scope, { joined: true, microphone: true, deafened: false });
});
it('does not apply old status to a new sharing group', async () => {
  const voice = createGroupVoice(); voice.attach(scope);
  let resolve!: (value: typeof off) => void;
  status.mockReturnValueOnce(new Promise((done) => { resolve = done; }));
  const pending = voice.refresh();
  voice.attach({ ...scope, sharingId: 'new-group' });
  resolve({ ...off, joined: true, microphone: true, capture: true }); await pending;
  expect(get(voice)).toMatchObject({ joined: false, microphone: false, capture: false });
});
it('queues leave after an in-flight unmute so cleanup cannot reopen the microphone', async () => {
  const voice = createGroupVoice(); voice.attach(scope); await voice.join();
  let release!: () => void;
  setVoice.mockImplementationOnce(() => new Promise<void>((done) => { release = done; }));
  const unmute = voice.toggleMicrophone(); await Promise.resolve();
  const detached = voice.detach();
  release(); await unmute; await detached;
  expect(setVoice).toHaveBeenLastCalledWith(scope, { joined: false, microphone: false, deafened: false });
  expect(get(voice)).toMatchObject({ joined: false, microphone: false, capture: false });
});
it('keeps voice consent off when a stale native session rejects the command', async () => {
  const voice = createGroupVoice(); voice.attach(scope);
  setVoice.mockRejectedValue(new Error('Group voice session is no longer active'));
  await voice.join();
  expect(get(voice)).toMatchObject({ joined: false, microphone: false, capture: false });
  expect(get(voice).error).toMatch(/no longer active/);
});

it('leaves screen sharing untouched when the native engine does not implement voice', async () => {
  const voice = createGroupVoice(); voice.attach(scope);
  status.mockResolvedValue({ ...off, code: 'unsupported' });
  await voice.refresh(); await voice.join();
  expect(get(voice)).toMatchObject({ available: false, joined: false, capture: false, code: 'unsupported' });
  expect(setVoice).not.toHaveBeenCalled();
});
it('preserves prior explicit microphone consent while the default device reconnects', async () => {
  const voice = createGroupVoice(); voice.attach(scope); await voice.join(); await voice.toggleMicrophone();
  status.mockResolvedValue({ ...off, joined: true, microphone: true, code: 'device-changed' });
  await voice.refresh();
  expect(get(voice)).toMatchObject({ microphone: true, capture: false, code: 'device-changed' });
  await voice.toggleMicrophone();
  expect(setVoice).toHaveBeenLastCalledWith(scope, { joined: true, microphone: false, deafened: false });
});
it('preserves a known active microphone when status IPC fails and still allows mute and leave', async () => {
  const voice = createGroupVoice(); voice.attach(scope); await voice.join(); await voice.toggleMicrophone();
  expect(get(voice)).toMatchObject({ joined: true, microphone: true, capture: true });
  const sent = setVoice.mock.calls.length;
  status.mockRejectedValueOnce(new Error('temporary IPC failure'));
  await voice.refresh();
  expect(get(voice)).toMatchObject({ joined: true, microphone: true, capture: true, deafened: false, available: false, code: 'running' });
  expect(get(voice).error).toMatch(/Unable to verify voice status/);
  expect(setVoice).toHaveBeenCalledTimes(sent);
  await voice.toggleMicrophone();
  expect(setVoice).toHaveBeenLastCalledWith(scope, { joined: true, microphone: false, deafened: false });
  expect(get(voice)).toMatchObject({ joined: true, microphone: false, capture: false, available: true, error: null });
  await voice.leave();
  expect(setVoice).toHaveBeenLastCalledWith(scope, { joined: false, microphone: false, deafened: false });
});
it('does not silently undeafen an existing listener after a failed status read', async () => {
  const voice = createGroupVoice(); voice.attach(scope); await voice.join(); await voice.toggleDeafen();
  status.mockRejectedValueOnce(new Error('temporary IPC failure'));
  await voice.refresh();
  expect(get(voice)).toMatchObject({ joined: true, microphone: false, capture: false, deafened: true, available: false });
  await voice.refresh();
  expect(get(voice)).toMatchObject({ deafened: true, available: true, error: null });
});
