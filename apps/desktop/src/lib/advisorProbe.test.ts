import { afterEach, expect, it, vi } from 'vitest';
import type { ApiClient } from './api';

vi.mock('./platform', () => ({ machineInfo: () => Promise.resolve({ cpuModel: 'Apple M2', refreshHz: 120 }) }));

import { answerProbe, timedTransfer } from './advisorProbe';

afterEach(() => vi.unstubAllGlobals());

it('turns bytes moved over time into Mb/s and stops at the time limit', async () => {
  let clock = 0;
  const move = vi.fn(() => { clock += 100; return Promise.resolve(250_000); });
  // 2 Mb every 100 ms of fake time = 20 Mb/s, for one second.
  const mbps = await timedTransfer(move, 1, 1, () => clock);
  expect(mbps).toBe(20);
  expect(move).toHaveBeenCalledTimes(10);
  expect(await timedTransfer(() => Promise.reject(new Error('down')), 1, 2, () => clock)).toBeNull();
});

it('a busy host reports its machine without testing its speed', async () => {
  const client = { serverUrl: 'https://sanser.test', uploadProbeChunk: vi.fn(), finishAdvisorProbe: vi.fn(() => Promise.resolve()) };
  await answerProbe(client as unknown as ApiClient, 'p1', true);
  expect(client.uploadProbeChunk).not.toHaveBeenCalled();
  expect(client.finishAdvisorProbe).toHaveBeenCalledWith('p1', expect.objectContaining({ busy: true, cpuModel: 'Apple M2', refreshHz: 120 }));
});

it('an idle host measures its round trip and upload', async () => {
  vi.stubGlobal('fetch', vi.fn(() => Promise.resolve(new Response('{}'))));
  const client = { serverUrl: 'https://sanser.test', uploadProbeChunk: vi.fn(() => Promise.resolve(262_144)), finishAdvisorProbe: vi.fn(() => Promise.resolve()) };
  await answerProbe(client as unknown as ApiClient, 'p2', false);
  expect(client.uploadProbeChunk).toHaveBeenCalled();
  const [, sent] = client.finishAdvisorProbe.mock.lastCall as unknown as [string, { busy: boolean; serverMs: number; uploadMbps: number }];
  expect(sent.busy).toBe(false);
  expect(sent.serverMs).toBeGreaterThanOrEqual(0);
  expect(sent.uploadMbps).toBeGreaterThan(0);
});
