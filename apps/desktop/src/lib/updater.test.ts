import { beforeEach, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ check: vi.fn(), invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/plugin-updater', () => ({
  check: mocks.check,
  Update: class { constructor(public metadata: unknown) {} }
}));
import { checkForDesktopUpdates } from './updater';

beforeEach(() => { mocks.check.mockReset(); mocks.invoke.mockReset(); });

it('keeps primary results authoritative, including no newer version', async () => {
  const update = { version: '2.3.7' };
  mocks.check.mockResolvedValueOnce(update).mockResolvedValueOnce(null);
  expect(await checkForDesktopUpdates()).toBe(update);
  expect(await checkForDesktopUpdates()).toBeNull();
  expect(mocks.check).toHaveBeenCalledWith({ timeout: 8_000 });
  expect(mocks.invoke).not.toHaveBeenCalled();
});

it('uses the native Tauri update resource after a primary HTTP failure', async () => {
  const metadata = { rid: 7, currentVersion: '2.3.6', version: '2.3.7', rawJson: {} };
  mocks.check.mockRejectedValue(new Error('HTTP 504'));
  mocks.invoke.mockResolvedValue(metadata);
  expect(await checkForDesktopUpdates()).toMatchObject({ metadata });
  expect(mocks.invoke).toHaveBeenCalledWith('check_update_via_github');
});

it('does not report an update when the fallback is current or unavailable', async () => {
  mocks.check.mockRejectedValue(new Error('offline'));
  mocks.invoke.mockResolvedValueOnce(null).mockRejectedValueOnce(new Error('API unavailable'));
  expect(await checkForDesktopUpdates()).toBeNull();
  await expect(checkForDesktopUpdates()).rejects.toThrow('API unavailable');
});
