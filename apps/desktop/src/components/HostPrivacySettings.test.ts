import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { get } from 'svelte/store';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
const mocks = vi.hoisted(() => ({ uac: vi.fn() }));
vi.mock('../lib/platform', async (original) => ({ ...(await original<typeof import('../lib/platform')>()), windowsUacStatus: mocks.uac }));
import HostPrivacySettings from './HostPrivacySettings.svelte';
import { DEFAULT_PREFERENCES, preferences } from '../stores/preferences';
import type { RuntimeStatus } from '../lib/types';
const runtime = (platform: string) => ({ platform, capabilities: { hostEngine: { state: 'available' } } }) as unknown as RuntimeStatus;
beforeEach(async () => {
  vi.resetAllMocks();
  mocks.uac.mockResolvedValue({ available: true, enabled: true, ready: true });
  await preferences.save(structuredClone(DEFAULT_PREFERENCES));
});
afterEach(cleanup);
it('is off by default and turning it on is a local host preference', async () => {
  render(HostPrivacySettings, { runtime: runtime('windows') });
  expect(screen.queryByText('Block local keyboard and mouse')).not.toBeInTheDocument();
  await fireEvent.click(screen.getByRole('switch', { name: 'Privacy screen' }));
  await waitFor(() => expect(get(preferences).host.privacyScreen).toBe(true));
  expect(screen.getByText(/Ctrl\+Alt\+Shift\+P/)).toBeInTheDocument();
  await fireEvent.click(await screen.findByRole('switch', { name: 'Block local keyboard and mouse' }));
  await waitFor(() => expect(get(preferences).host.privacyBlockInput).toBe(true));
  await fireEvent.click(screen.getByRole('switch', { name: 'Privacy screen' }));
  await waitFor(() => expect(get(preferences).host).toMatchObject({ privacyScreen: false, privacyBlockInput: false }));
});
it('says blocking local input is unavailable without the Windows helper or on macOS', async () => {
  mocks.uac.mockResolvedValue({ available: true, enabled: false, ready: false });
  await preferences.save({ ...structuredClone(DEFAULT_PREFERENCES), host: { ...DEFAULT_PREFERENCES.host, privacyScreen: true } });
  render(HostPrivacySettings, { runtime: runtime('windows') });
  expect(await screen.findByText('Unavailable: turn on Windows administrator prompts support first.')).toBeInTheDocument();
  expect(screen.getByRole('switch', { name: 'Block local keyboard and mouse' })).toBeDisabled();
  cleanup();
  render(HostPrivacySettings, { runtime: runtime('macOS') });
  expect(screen.getByText('Unavailable on this operating system.')).toBeInTheDocument();
  expect(screen.getByText(/Control\+Option\+Command\+P/)).toBeInTheDocument();
  expect(mocks.uac).toHaveBeenCalledTimes(1);
});
