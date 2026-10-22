import { get } from 'svelte/store';
import { expect, it, vi } from 'vitest';
import { createConnectionAlerts } from './connectionAlerts';
import type { HostConnectionAlert } from '../lib/connectionAlerts';
const first: HostConnectionAlert = { id: 'one', sessionId: 'one', deviceName: 'Mac', viewOnly: true };
it('announces once across repeated polls, navigation/dismissal and a name refresh', () => {
  const deliver = vi.fn<(alert: HostConnectionAlert, current: () => boolean) => Promise<void>>().mockResolvedValue(undefined); const store = createConnectionAlerts(deliver);
  store.observe('account', [first]); store.observe('account', [first]); store.dismiss(first.id);
  store.observe('account', [{ ...first, deviceName: 'Updated name' }]);
  expect(deliver).toHaveBeenCalledOnce(); expect(get(store).items).toEqual([]);
});
it('removes an ended connection and allows a new invitation to announce independently', () => {
  const deliver = vi.fn<(alert: HostConnectionAlert, current: () => boolean) => Promise<void>>().mockResolvedValue(undefined); const store = createConnectionAlerts(deliver);
  store.observe('account', [first]); store.observe('account', []);
  expect(get(store).items).toEqual([]);
  store.observe('account', [{ ...first, id: 'new', sessionId: 'new' }]); expect(deliver).toHaveBeenCalledTimes(2);
});
it('bounds notices, preserves deduplication for a long-lived viewer, and resets on sign-out', () => {
  const deliver = vi.fn<(alert: HostConnectionAlert, current: () => boolean) => Promise<void>>().mockResolvedValue(undefined); const store = createConnectionAlerts(deliver);
  for (let i = 0; i < 100; i++) store.observe('account', [first, { ...first, id: `other${i}`, sessionId: `other${i}` }]);
  expect(deliver.mock.calls.filter(([item]) => item.id === first.id)).toHaveLength(1);
  expect(get(store).items.length).toBeLessThanOrEqual(3);
  const guard = deliver.mock.calls[0]?.[1] as () => boolean;
  store.observe(null, []); expect(guard()).toBe(false); expect(get(store).items).toEqual([]);
  store.observe('other-account', [first]); expect(get(store).items).toHaveLength(1);
});
