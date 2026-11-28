import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';

const mocks = vi.hoisted(() => ({
  client: { serverUrl: 'https://sanser.test', streamAdvisorStatus: vi.fn(), devices: vi.fn(), streamAdvice: vi.fn(), connectionHistory: vi.fn(), createAdvisorProbe: vi.fn(), downloadProbeChunk: vi.fn(), advisorProbe: vi.fn() }
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
  // By default the server has no probes (an older server).
  mocks.client.createAdvisorProbe.mockRejectedValue(new Error('not found'));
  vi.stubGlobal('fetch', vi.fn(() => Promise.resolve(new Response('{}'))));
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

it('sends the chosen goal and clears an old answer when it changes', async () => {
  mocks.client.streamAdvice.mockResolvedValue({
    source: 'rules', reasons: [], settings: { resolution: '1080p', fps: 30, bitrateMbps: 10, codec: 'h264', networkMode: 'auto' }
  });
  render(StreamAdvisor, { runtime });
  await fireEvent.click(await screen.findByRole('button', { name: 'Suggest' }));
  expect(await screen.findByText('10 Mb/s')).toBeInTheDocument();
  expect(mocks.client.streamAdvice).toHaveBeenLastCalledWith(expect.objectContaining({ goal: 'balanced' }));
  await fireEvent.click(screen.getByRole('radio', { name: /Sharp and stable/ }));
  expect(screen.queryByText('10 Mb/s')).not.toBeInTheDocument();
  await fireEvent.click(screen.getByRole('button', { name: 'Suggest' }));
  await screen.findByText('10 Mb/s');
  expect(mocks.client.streamAdvice).toHaveBeenLastCalledWith(expect.objectContaining({ goal: 'sharp' }));
});

it('measures the host live and sends the measured speeds and host screen', async () => {
  mocks.client.createAdvisorProbe.mockResolvedValue({ probeId: 'p1', expiresInSeconds: 45, chunkBytes: 262144 });
  mocks.client.downloadProbeChunk.mockResolvedValue(262144);
  mocks.client.advisorProbe.mockResolvedValue({
    hostAnswered: true,
    host: { serverMs: 30, uploadMbps: 18.5, displayWidth: 2560, displayHeight: 1440, refreshHz: 144, cpuModel: 'AMD Ryzen 5 5600X', cpuCores: 12 }
  });
  mocks.client.streamAdvice.mockResolvedValue({
    source: 'rules', reasons: ['Đo được khoảng 18 Mb/s giữa hai máy.'], settings: { resolution: '1080p', fps: 60, bitrateMbps: 11, codec: 'h264', networkMode: 'auto' }
  });
  render(StreamAdvisor, { runtime });
  await fireEvent.click(await screen.findByRole('button', { name: 'Suggest' }));
  expect(await screen.findByText('11 Mb/s', {}, { timeout: 8000 })).toBeInTheDocument();
  expect(mocks.client.createAdvisorProbe).toHaveBeenCalledWith('pc');
  const request = mocks.client.streamAdvice.mock.lastCall?.[0] as { host: Record<string, unknown>; network: Record<string, unknown> };
  expect(request.host).toMatchObject({ displayWidth: 2560, displayHeight: 1440, refreshHz: 144, cpuModel: 'AMD Ryzen 5 5600X', cpuCores: 12 });
  expect(request.network.uploadMbps).toBe(18.5);
  expect(request.network.downloadMbps).toBeGreaterThan(0);
  expect(screen.getByText(/the host uploads 18.5 Mb\/s/)).toBeInTheDocument();
}, 10000);
