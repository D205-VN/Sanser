import { get } from 'svelte/store';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const native = vi.hoisted(() => ({ loadNativePreferences: vi.fn(), saveNativePreferences: vi.fn() }));
vi.mock('../lib/platform', () => native);

beforeEach(() => {
  vi.resetModules();
  vi.clearAllMocks();
  localStorage.clear();
  native.loadNativePreferences.mockResolvedValue(null);
  native.saveNativePreferences.mockResolvedValue(undefined);
});

describe('preference persistence', () => {
  it('restores the last durable settings after failed writes and keeps the browser copy consistent', async () => {
    const { preferences } = await import('./preferences');
    await preferences.initialize();
    const initial = get(preferences);
    native.saveNativePreferences.mockRejectedValue(new Error('disk full'));
    const first = preferences.patch({ networkMode: 'direct' });
    const second = preferences.patch({ diagnosticsEnabled: false });
    await expect(first).rejects.toThrow('disk full');
    await expect(second).rejects.toThrow('disk full');
    expect(get(preferences)).toEqual(initial);
    expect(JSON.parse(localStorage.getItem('sanser.preferences.v2') ?? '{}')).toEqual(initial);
  });

  it('serializes writes and recovers after a failed write', async () => {
    const { preferences } = await import('./preferences');
    await preferences.initialize();
    native.saveNativePreferences.mockRejectedValueOnce(new Error('temporary failure'));
    await expect(preferences.patch({ networkMode: 'direct' })).rejects.toThrow();
    await preferences.patch({ networkMode: 'relay' });
    expect(get(preferences).networkMode).toBe('relay');
    expect(JSON.parse(localStorage.getItem('sanser.preferences.v2') ?? '{}')).toMatchObject({ networkMode: 'relay' });
  });
});
