import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { loginDevice } from './platform';

const native = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => native);
beforeEach(() => { localStorage.clear(); vi.clearAllMocks(); });
afterEach(() => { vi.unstubAllGlobals(); localStorage.clear(); });

it('keeps the non-secret installation identifier across calls and app module reloads', async () => {
  const first = await loginDevice();
  expect(await loginDevice()).toEqual(first);
  vi.resetModules();
  expect(await (await import('./platform')).loginDevice()).toEqual(first);
  expect(localStorage.length).toBe(1);
  expect(localStorage.getItem('sanser.login-installation.v1')).toBe(first.installationId);
});

it('uses the native computer metadata and refuses to invent another ID on native storage failure', async () => {
  vi.stubGlobal('__TAURI_INTERNALS__', {});
  native.invoke.mockRejectedValueOnce(new Error('Cannot read saved identity'));
  await expect(loginDevice()).rejects.toThrow('Cannot read saved identity');
  expect(localStorage.length).toBe(0);
  const device = { installationId: 'native-id', deviceName: 'Office PC', platform: 'windows-x86_64' };
  native.invoke.mockResolvedValueOnce(device);
  expect(await loginDevice()).toEqual(device);
  expect(native.invoke).toHaveBeenLastCalledWith('get_login_device');
});
