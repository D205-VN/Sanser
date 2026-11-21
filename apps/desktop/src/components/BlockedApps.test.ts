import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { get } from 'svelte/store';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
const mocks = vi.hoisted(() => ({ running: vi.fn(), trusted: vi.fn(), protect: vi.fn() }));
vi.mock('../lib/platform', async (original) => ({ ...(await original<typeof import('../lib/platform')>()), listRunningApps: mocks.running, blockedAppsTrusted: mocks.trusted, protectBlockedApps: mocks.protect }));
import BlockedApps from './BlockedApps.svelte';
import { DEFAULT_PREFERENCES, preferences } from '../stores/preferences';
import type { RuntimeStatus } from '../lib/types';
const runtime = (platform: string) => ({ platform, capabilities: { hostEngine: { state: 'available' } } }) as unknown as RuntimeStatus;
beforeEach(async () => {
  vi.resetAllMocks();
  mocks.running.mockResolvedValue([{ id: 'com.google.Chrome', name: 'Google Chrome' }, { id: 'com.apple.Safari', name: 'Safari' }]);
  mocks.trusted.mockResolvedValue(true);
  mocks.protect.mockResolvedValue(undefined);
  await preferences.save(structuredClone(DEFAULT_PREFERENCES));
});
afterEach(cleanup);

it('blocks a running app picked from the list and removes it again', async () => {
  render(BlockedApps, { runtime: runtime('macOS') });
  expect(screen.getByText('No apps are blocked.')).toBeInTheDocument();
  await fireEvent.click(screen.getByRole('button', { name: 'Choose from running apps' }));
  const select = await screen.findByRole('combobox', { name: 'Running apps' });
  await fireEvent.change(select, { target: { value: 'com.google.Chrome' } });
  await fireEvent.click(screen.getByRole('button', { name: 'Block' }));
  await waitFor(() => expect(get(preferences).host.blockedApps).toEqual([{ id: 'com.google.Chrome', name: 'Google Chrome' }]));
  expect(screen.getByText('Google Chrome')).toBeInTheDocument();
  // An app already on the list is no longer offered.
  expect(screen.queryByRole('option', { name: 'Google Chrome' })).not.toBeInTheDocument();
  await fireEvent.click(screen.getByRole('button', { name: 'Remove' }));
  await waitFor(() => expect(get(preferences).host.blockedApps).toEqual([]));
});

it('adds a typed Windows program name and rejects a path', async () => {
  render(BlockedApps, { runtime: runtime('windows') });
  const input = screen.getByRole('textbox', { name: 'App to block' });
  expect(input).toHaveAttribute('placeholder', 'chrome.exe');
  await fireEvent.input(input, { target: { value: 'C:\\Apps\\game.exe' } });
  await fireEvent.click(screen.getByRole('button', { name: 'Add' }));
  expect(await screen.findByText('Enter a program file name such as chrome.exe.')).toBeInTheDocument();
  expect(get(preferences).host.blockedApps).toEqual([]);
  await fireEvent.input(input, { target: { value: 'game.exe' } });
  await fireEvent.click(screen.getByRole('button', { name: 'Add' }));
  await waitFor(() => expect(get(preferences).host.blockedApps).toEqual([{ id: 'game.exe', name: 'game.exe' }]));
});

it('falls back to typing when the running app list is unavailable', async () => {
  mocks.running.mockRejectedValue(new Error('no'));
  render(BlockedApps, { runtime: runtime('macOS') });
  await fireEvent.click(screen.getByRole('button', { name: 'Choose from running apps' }));
  expect(await screen.findByText('Unable to list running apps. Enter the app below instead.')).toBeInTheDocument();
});

it('records each saved list and asks to confirm a list edited outside Sanser', async () => {
  await preferences.save({ ...structuredClone(DEFAULT_PREFERENCES), host: { ...DEFAULT_PREFERENCES.host, blockedApps: [{ id: 'chrome.exe', name: 'Chrome' }] } });
  mocks.trusted.mockResolvedValue(false);
  render(BlockedApps, { runtime: runtime('windows') });
  await waitFor(() => expect(mocks.trusted).toHaveBeenCalledWith(['chrome.exe']));
  const confirm = await screen.findByRole('button', { name: 'Confirm this list' });
  expect(screen.getByRole('alert').textContent).toContain('changed outside Sanser');
  await fireEvent.click(confirm);
  await waitFor(() => expect(mocks.protect).toHaveBeenCalledWith(['chrome.exe']));
  await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
});
