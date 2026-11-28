import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';

const mocks = vi.hoisted(() => ({
  client: { streamAdvisorStatus: vi.fn(), devices: vi.fn(), streamAdvice: vi.fn(), connectionHistory: vi.fn() }
}));
vi.mock('../stores/session', () => ({ session: { client: () => mocks.client } }));
vi.mock('../lib/platform', async (original) => ({ ...(await original<typeof import('../lib/platform')>()), isTauriRuntime: () => false, saveNativePreferences: () => Promise.resolve(), loadNativePreferences: () => Promise.resolve(null) }));

import StreamAdvisor from './StreamAdvisor.svelte';
import { nearestRefresh } from '../lib/streamAdvisor';
import { preferences } from '../stores/preferences';
import type { RuntimeStatus } from '../lib/types';

const runtime = { platform: 'macos-aarch64', capabilities: { clientCodecs: ['h264', 'hevc'] } } as unknown as RuntimeStatus;

beforeEach(() => {
  vi.resetAllMocks();
  mocks.client.streamAdvisorStatus.mockResolvedValue({ modelAvailable: true });
  mocks.client.devices.mockResolvedValue({ items: [{ id: 'pc', name: 'Lab PC', platform: 'windows', gpu: 'RTX 3060', online: true, deviceRole: 'host', latencyMs: 25, capabilities: { codecs: ['h264'] } }] });
  mocks.client.connectionHistory.mockResolvedValue({ items: [{ route: 'direct' }] });
});
afterEach(cleanup);

it('snaps measured refresh rates to real panel rates', () => {
  expect(nearestRefresh(59.4)).toBe(60);
  expect(nearestRefresh(118)).toBe(120);
  expect(nearestRefresh(5)).toBeUndefined();
});

it('asks for advice about the chosen host and applies it as custom settings', async () => {
  mocks.client.streamAdvice.mockResolvedValue({
    source: 'model', model: 'qwen2.5-3b-instruct', reasons: ['Mạng ổn định nên dùng 1080p.'],
    settings: { resolution: '1080p', fps: 60, bitrateMbps: 18, codec: 'h264', networkMode: 'auto' }
  });
  render(StreamAdvisor, { runtime });
  await fireEvent.click(await screen.findByRole('button', { name: 'Suggest' }));
  expect(await screen.findByText('18 Mb/s')).toBeInTheDocument();
  expect(mocks.client.streamAdvice).toHaveBeenCalledWith(expect.objectContaining({
    host: { platform: 'windows', gpu: 'RTX 3060', codecs: ['h264'] },
    network: expect.objectContaining({ path: 'direct', rttMs: 25 })
  }));
  expect(screen.getByText('Suggested by qwen2.5-3b-instruct')).toBeInTheDocument();
  await fireEvent.click(screen.getByRole('button', { name: 'Use these settings' }));
  expect(await screen.findByRole('button', { name: 'Applied' })).toBeDisabled();
  expect(get(preferences).stream).toMatchObject({ profile: 'custom', resolution: '1080p', bitrateMbps: 18, codec: 'h264' });
});
