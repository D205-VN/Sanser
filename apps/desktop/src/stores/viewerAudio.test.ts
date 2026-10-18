import { get } from 'svelte/store';
import { beforeEach, expect, it, vi } from 'vitest';
const { setAudio } = vi.hoisted(() => ({ setAudio: vi.fn() }));
vi.mock('../lib/platform', () => ({ setViewerAudio: setAudio }));
import { viewerAudio, prepareViewerAudio, muteViewerAudio } from './viewerAudio';
beforeEach(() => { viewerAudio.set({}); setAudio.mockReset(); setAudio.mockResolvedValue(undefined); });
it('keeps independent session mute preferences across page navigation and reconnect', async () => {
  prepareViewerAudio('a', false); prepareViewerAudio('b', false);
  await muteViewerAudio('a', true);
  expect(prepareViewerAudio('a', false)).toBe(true);
  expect(get(viewerAudio).b).toBe(false);
  expect(setAudio).toHaveBeenCalledWith('a', true);
});
it('does not report a changed mute state when the native session has closed', async () => {
  prepareViewerAudio('a', false);
  setAudio.mockRejectedValue(new Error('viewer session is no longer active'));
  await expect(muteViewerAudio('a', true)).rejects.toThrow('no longer active');
  expect(get(viewerAudio).a).toBe(false);
});
