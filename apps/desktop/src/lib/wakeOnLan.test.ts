import { afterEach, describe, expect, it, vi } from 'vitest';

const platform = vi.hoisted(() => ({ wakeOnLanInterfaces: vi.fn(), sendWakeOnLan: vi.fn() }));
vi.mock('./platform', () => platform);

import { ApiClient } from './api';
import { canWake, syncWakeOnLan, wakeDevice } from './wakeOnLan';

const adapter = { macAddress: 'aa:bb:cc:00:11:22', ipv4Address: '192.168.1.30', prefixLength: 24, broadcastAddress: '192.168.1.255' };
const client = () => ({ setWakeOnLan: vi.fn().mockResolvedValue({}), clearWakeOnLan: vi.fn().mockResolvedValue(undefined) });

afterEach(() => {
  vi.resetAllMocks();
  vi.unstubAllGlobals();
});

describe('Wake-on-LAN', () => {
  it('shares nothing unless the host opted in', async () => {
    const api = client();
    expect(await syncWakeOnLan(api, 'host', false, false)).toBe('unchanged');
    expect(platform.wakeOnLanInterfaces).not.toHaveBeenCalled();
    expect(api.setWakeOnLan).not.toHaveBeenCalled();
    expect(api.clearWakeOnLan).not.toHaveBeenCalled();
  });

  it('withdraws previously shared details when turned off', async () => {
    const api = client();
    expect(await syncWakeOnLan(api, 'host', false, true)).toBe('cleared');
    expect(api.clearWakeOnLan).toHaveBeenCalledWith('host');
  });

  it('reports local adapters when enabled and handles computers without one', async () => {
    const api = client();
    platform.wakeOnLanInterfaces.mockResolvedValueOnce([adapter]);
    expect(await syncWakeOnLan(api, 'host', true, false)).toBe('shared');
    expect(api.setWakeOnLan).toHaveBeenCalledWith('host', [adapter]);
    platform.wakeOnLanInterfaces.mockResolvedValueOnce([]);
    expect(await syncWakeOnLan(api, 'host', true, true)).toBe('no-adapters');
    expect(api.clearWakeOnLan).toHaveBeenCalledWith('host');
  });

  it('offers Wake only for offline computers with saved adapters', () => {
    expect(canWake({ online: false, wakeOnLan: [adapter] })).toBe(true);
    expect(canWake({ online: true, wakeOnLan: [adapter] })).toBe(false);
    expect(canWake({ online: false })).toBe(false);
    expect(canWake({ online: false, wakeOnLan: [] })).toBe(false);
  });

  it('sends packets through the desktop shell for every saved adapter', async () => {
    platform.sendWakeOnLan.mockResolvedValue({ sameLan: true, packetsSent: 4 });
    expect(await wakeDevice({ wakeOnLan: [adapter] })).toEqual({ sameLan: true, packetsSent: 4 });
    expect(platform.sendWakeOnLan).toHaveBeenCalledWith([adapter]);
    expect(await wakeDevice({})).toEqual({ sameLan: false, packetsSent: 0 });
  });

  it('uses PUT and DELETE on the device wake endpoint and reads wake details from devices', async () => {
    const fetchMock = vi.fn<typeof fetch>()
      .mockResolvedValueOnce(new Response(JSON.stringify({ id: 'host', wakeOnLan: { interfaces: [adapter], updatedAt: 1 } })))
      .mockResolvedValueOnce(new Response(null, { status: 204 }));
    vi.stubGlobal('fetch', fetchMock);
    const api = new ApiClient('https://sanser.example', () => Promise.resolve('token'));
    const device = await api.setWakeOnLan('host', [adapter]);
    expect(device.wakeOnLan).toEqual([adapter]);
    expect(fetchMock.mock.calls[0]?.[0]).toBe('https://sanser.example/api/v2/devices/host/wake-on-lan');
    expect(fetchMock.mock.calls[0]?.[1]).toMatchObject({ method: 'PUT' });
    await api.clearWakeOnLan('host');
    expect(fetchMock.mock.calls[1]?.[1]).toMatchObject({ method: 'DELETE' });
  });
});
