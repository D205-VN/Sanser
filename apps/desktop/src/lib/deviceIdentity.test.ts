import { beforeEach, afterEach, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => mocks);
beforeEach(() => { vi.resetModules(); vi.resetAllMocks(); localStorage.clear(); mocks.isTauri.mockReturnValue(false); });
afterEach(() => { vi.restoreAllMocks(); localStorage.clear(); });

it('reuses one installation and separate role IDs across reloads and concurrent registration', async () => {
  let module = await import('./deviceIdentity');
  const [a, b] = await Promise.all([module.deviceIdentity('account', 'host'), module.deviceIdentity('account', 'host')]);
  const client = await module.deviceIdentity('account', 'client');
  expect(a).toEqual(b); expect(client.computerId).toBe(a.computerId); expect(client.id).not.toBe(a.id);
  vi.resetModules(); module = await import('./deviceIdentity');
  expect(await module.deviceIdentity('account', 'host')).toEqual(a);
  expect((await module.deviceIdentity('other-account', 'host')).id).not.toBe(a.id);
});
it('keeps browser identity stable when storage cannot be written', async () => {
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('disabled'); });
  const module = await import('./deviceIdentity');
  const host = await module.deviceIdentity('account', 'host');
  expect(await module.deviceIdentity('account', 'host')).toEqual(host);
  expect((await module.deviceIdentity('account', 'client')).computerId).toBe(host.computerId);
});
it('migrates the old IDs into native storage and never creates a random ID on native failure', async () => {
  const old = '22222222-2222-4222-8222-222222222222';
  localStorage.setItem('sanser.device-identities.v2', JSON.stringify({ 'host:account': old }));
  mocks.isTauri.mockReturnValue(true);
  mocks.invoke.mockRejectedValueOnce(new Error('Cannot read saved identity'));
  const module = await import('./deviceIdentity');
  await expect(module.deviceIdentity('account', 'host')).rejects.toThrow('Cannot read saved identity');
  mocks.invoke.mockResolvedValue({ id: old, computerId: 'native-computer', name: 'Studio PC' });
  expect(await module.deviceIdentity('account', 'host')).toMatchObject({ id: old, name: 'Studio PC' });
  expect(mocks.invoke).toHaveBeenLastCalledWith('get_device_identity', { accountId: 'account', role: 'host', legacyHostId: old, legacyClientId: null });
});
