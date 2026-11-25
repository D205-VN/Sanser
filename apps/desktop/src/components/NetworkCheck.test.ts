import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ check: vi.fn(), client: { serverUrl: 'https://sanser.example' } }));
vi.mock('../lib/networkCheck', async (original) => ({ ...(await original<typeof import('../lib/networkCheck')>()), checkNetwork: mocks.check, recentNetworkCheck: mocks.check }));
vi.mock('../lib/platform', () => ({ isTauriRuntime: () => true }));
vi.mock('../stores/session', () => ({ session: { client: () => mocks.client } }));

import NetworkCheck from './NetworkCheck.svelte';
import { networkVerdict } from '../lib/networkCheck';

beforeEach(() => vi.resetAllMocks());
afterEach(cleanup);

it('classifies offline, relay and direct networks', () => {
  expect(networkVerdict(null, true)).toBe('offline');
  expect(networkVerdict(40, false)).toBe('relay');
  expect(networkVerdict(40, true)).toBe('direct');
});

it('warns on the home page only when the network will need the relay', async () => {
  mocks.check.mockResolvedValue({ serverMs: 90, udp: false, portMapping: false, publicIpv6: false, verdict: 'relay' });
  render(NetworkCheck, { compact: true });
  expect(await screen.findByText(/go through the relay/)).toBeInTheDocument();
  cleanup();
  mocks.check.mockResolvedValue({ serverMs: 30, udp: true, portMapping: true, publicIpv6: false, verdict: 'direct' });
  render(NetworkCheck, { compact: true });
  await vi.waitFor(() => expect(mocks.check).toHaveBeenCalledTimes(2));
  expect(screen.queryByRole('status')).not.toBeInTheDocument();
});

it('runs on request in Diagnostics and shows every measurement', async () => {
  mocks.check.mockResolvedValue({ serverMs: 42, udp: true, portMapping: false, publicIpv6: true, verdict: 'direct' });
  render(NetworkCheck);
  expect(mocks.check).not.toHaveBeenCalled();
  await fireEvent.click(screen.getByRole('button', { name: 'Check network' }));
  expect(await screen.findByText('42 ms')).toBeInTheDocument();
  expect(screen.getByText('Works')).toBeInTheDocument();
  expect(screen.getByRole('status')).toHaveTextContent('Direct connections should work');
});
