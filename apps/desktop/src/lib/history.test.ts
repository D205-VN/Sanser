import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiClient } from './api';
import { formatDuration, historyQuery, statusLabel } from './history';
import { translateFor } from './i18n';

const en = (text: string, vars?: Record<string, string | number>) => translateFor('en', text, vars);
const vi_ = (text: string, vars?: Record<string, string | number>) => translateFor('vi', text, vars);

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('connection history', () => {
  it('builds filters from this computer\'s registrations', () => {
    expect(historyQuery('all', [])).toEqual({});
    expect(historyQuery('this', [null, undefined])).toBeNull();
    expect(historyQuery('this', ['a', 'b', 'a'])).toEqual({ deviceIds: ['a', 'b'], direction: 'any' });
    expect(historyQuery('incoming', ['a'])).toEqual({ deviceIds: ['a'], direction: 'incoming' });
    expect(historyQuery('outgoing', ['a'])).toEqual({ deviceIds: ['a'], direction: 'outgoing' });
  });

  it('formats durations in both languages', () => {
    expect(formatDuration(null, en)).toBe('—');
    expect(formatDuration(42, en)).toBe('42 s');
    expect(formatDuration(125, en)).toBe('2 min');
    expect(formatDuration(3_900, en)).toBe('1 h 5 min');
    expect(formatDuration(3_900, vi_)).toBe('1 giờ 5 phút');
  });

  it('describes how a session ended', () => {
    expect(statusLabel({ state: 'pending', endReason: null, endedAt: null })).toBe('Waiting for approval');
    expect(statusLabel({ state: 'accepted', endReason: null, endedAt: null })).toBe('In progress');
    expect(statusLabel({ state: 'disconnected', endReason: 'device_offline', endedAt: 'x' })).toBe('A computer went offline');
    expect(statusLabel({ state: 'rejected', endReason: null, endedAt: 'x' })).toBe('Declined');
    expect(statusLabel({ state: 'disconnected', endReason: 'custom', endedAt: 'x' })).toBe('custom');
  });

  it('requests account history with filters and pagination and normalizes rows', async () => {
    const fetchMock = vi.fn<typeof fetch>().mockImplementation(() => Promise.resolve(new Response(JSON.stringify({
      items: [{ id: 's1', requesterDeviceId: 'r', requesterName: 'Laptop', requesterPlatform: 'macos-aarch64', hostDeviceId: 'h',
        hostName: 'Office PC', hostPlatform: 'windows-x86_64', state: 'disconnected', accessMode: 'view-only', route: 'relay',
        sharedView: false, startedAt: 1_700_000_000, connectedAt: 1_700_000_005, endedAt: 1_700_000_065, durationSeconds: 60, endReason: 'disconnected' },
      { id: 's2', state: 'weird', route: 'carrier-pigeon', startedAt: 1_700_000_000 }],
      nextCursor: 'next'
    }))));
    vi.stubGlobal('fetch', fetchMock);
    const client = new ApiClient('https://sanser.example', () => Promise.resolve('token'));
    const page = await client.connectionHistory({ cursor: 'abc', limit: 25, deviceIds: ['h', 'r'], direction: 'incoming' });
    expect(fetchMock.mock.calls[0]?.[0]).toBe('https://sanser.example/api/v2/account/history?cursor=abc&limit=25&deviceIds=h%2Cr&direction=incoming');
    expect(page.nextCursor).toBe('next');
    expect(page.items[0]).toMatchObject({ accessMode: 'view-only', route: 'relay', durationSeconds: 60, startedAt: new Date(1_700_000_000_000).toISOString() });
    expect(page.items[1]).toMatchObject({ state: 'disconnected', route: null, connectedAt: null, durationSeconds: null });
    await client.connectionHistory({ direction: 'any' });
    expect(fetchMock.mock.calls[1]?.[0]).toBe('https://sanser.example/api/v2/account/history');
  });
});
