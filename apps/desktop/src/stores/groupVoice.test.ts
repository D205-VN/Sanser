import { get } from 'svelte/store';
import { beforeEach, expect, it, vi } from 'vitest';
const { setVoice, status } = vi.hoisted(() => ({ setVoice: vi.fn(), status: vi.fn() }));
vi.mock('../lib/platform', () => ({ setGroupVoice: setVoice, groupVoiceStatus: status }));
import { createGroupVoice } from './groupVoice';
import type { GroupVoiceScope, GroupVoiceStatus } from '../lib/platform';
const scope = { kind: 'client' as const, sharingId: 'group', sessionId: 'session' };
const off = { floatingControls: true, joined: false, microphone: false, deafened: false, capture: false, code: 'idle' };
beforeEach(() => {
  setVoice.mockReset(); status.mockReset();
  setVoice.mockImplementation((_scope: GroupVoiceScope, consent: Pick<GroupVoiceStatus, 'joined' | 'microphone' | 'deafened'>) => { status.mockResolvedValue({ floatingControls: true, ...consent, capture: consent.microphone, code: consent.microphone ? 'running' : 'listening' }); return Promise.resolve(); });
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
it('page navigation detaches only the observer; floating native controls retain the call', async () => {
  const voice = createGroupVoice(); voice.attach(scope); await voice.join();
  let release!: () => void;
  setVoice.mockImplementationOnce(() => new Promise<void>((done) => { release = done; }));
  const unmute = voice.toggleMicrophone(); await Promise.resolve();
  const detached = voice.detach();
  release(); await unmute; await detached;
  expect(setVoice).toHaveBeenLastCalledWith(scope, { joined: true, microphone: true, deafened: false });
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

it('does not return to Join when stdin succeeded but native status still contains the previous idle record', async () => {
  const voice = createGroupVoice(); voice.attach(scope); await voice.refresh();
  setVoice.mockResolvedValue(undefined); status.mockResolvedValue(off);
  await voice.join();
  expect(get(voice)).toMatchObject({ joined: true, microphone: false, busy: true });
  status.mockResolvedValue({ ...off, joined: true, code: 'listening' }); await voice.refresh();
  expect(get(voice)).toMatchObject({ joined: true, busy: false });
  await voice.toggleMicrophone();
  expect(get(voice)).toMatchObject({ joined: true, microphone: true, capture: false, busy: true });
  status.mockResolvedValue({ ...off, joined: true, microphone: true, capture: true, code: 'running' }); await voice.refresh();
  expect(get(voice)).toMatchObject({ microphone: true, capture: true, busy: false });
});
it('adopts an explicit native floating-panel join and permits the page microphone button afterward', async () => {
  const voice = createGroupVoice(); voice.attach(scope);
  status.mockResolvedValue({ ...off, joined: true, code: 'listening' }); await voice.refresh();
  await voice.toggleMicrophone();
  expect(setVoice).toHaveBeenLastCalledWith(scope, { joined: true, microphone: true, deafened: false });
});
it('leaves on navigation with an older engine that has no floating microphone controls', async () => {
  const voice = createGroupVoice(); voice.attach(scope); await voice.join();
  status.mockResolvedValue({ ...off, floatingControls: false, joined: true, code: 'listening' }); await voice.refresh();
  let release!: () => void;
  setVoice.mockImplementationOnce(() => new Promise<void>((done) => { release = done; }));
  const unmute = voice.toggleMicrophone(); await Promise.resolve();
  const detached = voice.detach(); release(); await unmute; await detached;
  expect(setVoice).toHaveBeenLastCalledWith(scope, { joined: false, microphone: false, deafened: false });
});
it('does not remain busy forever when native never confirms a voice change', async () => {
  const now = vi.spyOn(Date, 'now').mockReturnValue(10000);
  try {
    const voice = createGroupVoice(); voice.attach(scope); await voice.refresh();
    setVoice.mockResolvedValue(undefined); status.mockResolvedValue(off); await voice.join();
    expect(get(voice).busy).toBe(true);
    now.mockReturnValue(14000); await voice.refresh();
    expect(get(voice)).toMatchObject({ busy: false, joined: false, microphone: false, capture: false });
    expect(get(voice).error).toMatch(/not confirmed/);
  } finally { now.mockRestore(); }
});
it('rejects a status response started before the latest Join command', async () => {
  const voice = createGroupVoice(); voice.attach(scope);
  let resolve!: (value: typeof off) => void;
  status.mockReturnValueOnce(new Promise((done) => { resolve = done; }));
  const stale = voice.refresh(); await voice.join();
  resolve(off); await stale;
  expect(get(voice)).toMatchObject({ joined: true, busy: true });
  status.mockResolvedValue({ ...off, joined: true, code: 'listening' }); await voice.refresh();
  expect(get(voice)).toMatchObject({ joined: true, busy: false });
});
it('does not label a microphone off until native acknowledges mute or leave', async () => {
  const voice = createGroupVoice(); voice.attach(scope); await voice.join(); await voice.toggleMicrophone();
  setVoice.mockResolvedValue(undefined);
  status.mockResolvedValue({ ...off, joined: true, microphone: true, capture: true, code: 'running' });
  await voice.toggleMicrophone();
  expect(get(voice)).toMatchObject({ joined: true, microphone: true, capture: true, busy: true, code: 'changing' });
  status.mockResolvedValue({ ...off, joined: true, code: 'listening' }); await voice.refresh();
  expect(get(voice)).toMatchObject({ microphone: false, capture: false, busy: false });
  await voice.leave();
  expect(get(voice)).toMatchObject({ joined: true, busy: true, code: 'leaving' });
  status.mockResolvedValue(off); await voice.refresh();
  expect(get(voice)).toMatchObject({ joined: false, microphone: false, capture: false, busy: false });
});
