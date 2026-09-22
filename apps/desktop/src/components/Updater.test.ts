import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeAll, beforeEach, expect, it, vi } from 'vitest';
import { connection } from '../stores/connection';

const mocks = vi.hoisted(() => ({ check: vi.fn(), relaunch: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true }));
vi.mock('@tauri-apps/plugin-updater', () => ({ check: mocks.check }));
vi.mock('@tauri-apps/plugin-process', () => ({ relaunch: mocks.relaunch }));
import Updater from './Updater.svelte';

function renderUpdater(onAvailabilityChange = vi.fn()) {
  const { component } = render(Updater, { onAvailabilityChange });
  return component as unknown as { checkForUpdates: (manual?: boolean) => Promise<void> };
}

beforeAll(() => {
  HTMLDialogElement.prototype.showModal = function () { this.setAttribute('open', ''); };
  HTMLDialogElement.prototype.close = function () { this.removeAttribute('open'); };
});
beforeEach(() => { mocks.check.mockReset(); mocks.relaunch.mockReset(); connection.clear(); });
afterEach(() => { cleanup(); connection.clear(); vi.useRealTimers(); });

it('checks manually and reports that the current version is up to date', async () => {
  mocks.check.mockResolvedValue(null);
  const component = renderUpdater();
  await component.checkForUpdates();
  expect(await screen.findByRole('heading', { name: 'You’re up to date' })).toBeInTheDocument();
  expect(screen.getByRole('dialog')).toHaveTextContent('Sanser 2.1.2');
  await fireEvent.click(screen.getByRole('button', { name: 'Close' }));
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
});

it('lets a user retry a failed manual check', async () => {
  mocks.check.mockRejectedValueOnce(new Error('offline')).mockResolvedValueOnce(null);
  const component = renderUpdater();
  await component.checkForUpdates();
  expect(await screen.findByRole('alert')).toHaveTextContent('Unable to check for updates');
  await fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
  expect(await screen.findByRole('heading', { name: 'You’re up to date' })).toBeInTheDocument();
  expect(mocks.check).toHaveBeenCalledTimes(2);
});

it('prevents overlapping checks and keeps the update available after Not now', async () => {
  const update = { version: '2.2.0', close: vi.fn().mockResolvedValue(undefined) };
  let finish!: (value: typeof update) => void;
  mocks.check.mockReturnValue(new Promise((resolve) => { finish = resolve; }));
  const component = renderUpdater();
  const pending = component.checkForUpdates();
  await component.checkForUpdates();
  expect(mocks.check).toHaveBeenCalledTimes(1);
  finish(update);
  await pending;
  await fireEvent.click(await screen.findByRole('button', { name: 'Not now' }));
  expect(update.close).not.toHaveBeenCalled();
  await component.checkForUpdates();
  expect(await screen.findByRole('button', { name: 'Install update' })).toBeInTheDocument();
  await fireEvent.click(screen.getByRole('button', { name: 'Not now' }));
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
});

it('blocks installation during a remote session and supports install then restart', async () => {
  const update = { version: '2.2.0', close: vi.fn().mockResolvedValue(undefined), downloadAndInstall: vi.fn().mockResolvedValue(undefined) };
  mocks.check.mockResolvedValue(update);
  connection.setEngineRunning(true);
  const component = renderUpdater();
  await component.checkForUpdates();
  expect(await screen.findByRole('button', { name: 'Install update' })).toBeDisabled();
  expect(update.downloadAndInstall).not.toHaveBeenCalled();
  connection.setEngineRunning(false);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Install update' })).toBeEnabled());
  await fireEvent.click(screen.getByRole('button', { name: 'Install update' }));
  expect(await screen.findByRole('button', { name: 'Restart Sanser' })).toBeEnabled();
  await fireEvent.click(screen.getByRole('button', { name: 'Restart Sanser' }));
  expect(update.downloadAndInstall).toHaveBeenCalledTimes(1);
  expect(mocks.relaunch).toHaveBeenCalledTimes(1);
  await fireEvent.click(screen.getByRole('button', { name: 'Restart later' }));
  await component.checkForUpdates(false);
  expect(mocks.check).toHaveBeenCalledTimes(1);
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
});

it('reports an available update silently and opens only when requested', async () => {
  mocks.check.mockResolvedValue({ version: '2.2.0', close: vi.fn().mockResolvedValue(undefined) });
  const availability = vi.fn();
  const component = renderUpdater(availability);
  await component.checkForUpdates(false);
  await waitFor(() => expect(availability).toHaveBeenLastCalledWith(true));
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  await component.checkForUpdates();
  expect(await screen.findByRole('dialog')).toHaveTextContent('Update available');
});

it.each([false, true])('keeps the indicator hidden when a background check has no update or fails (%s)', async (fails) => {
  if (fails) mocks.check.mockRejectedValue(new Error('offline'));
  else mocks.check.mockResolvedValue(null);
  const availability = vi.fn();
  const component = renderUpdater(availability);
  await component.checkForUpdates(false);
  await waitFor(() => expect(availability).toHaveBeenLastCalledWith(false));
  expect(availability).not.toHaveBeenCalledWith(true);
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
});
