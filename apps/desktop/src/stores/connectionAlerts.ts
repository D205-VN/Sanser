import { writable } from 'svelte/store';
import { alertPermission, showHostConnectionAlert, type AlertPermission, type HostConnectionAlert } from '../lib/connectionAlerts';

export function createConnectionAlerts(deliver = showHostConnectionAlert) {
  const store = writable<{ items: HostConnectionAlert[]; permission: AlertPermission }>({ items: [], permission: 'unknown' });
  let account: string | null = null;
  let generation = 0;
  const seen = new Set<string>();
  let active = new Set<string>();
  return {
    subscribe: store.subscribe,
    observe(owner: string | null, connections: HostConnectionAlert[]): void {
      if (owner !== account) { generation += 1; account = owner; seen.clear(); store.update((state) => ({ ...state, items: [] })); }
      active = new Set(owner ? connections.map((item) => item.id) : []);
      store.update((state) => ({ ...state, items: state.items.filter((item) => active.has(item.id)) }));
      if (!owner) return;
      for (const item of connections) {
        if (seen.has(item.id)) continue;
        seen.add(item.id);
        if (seen.size > 64) { const old = [...seen].find((id) => !active.has(id)); if (old) seen.delete(old); }
        store.update((state) => ({ ...state, items: [...state.items, item].slice(-3) }));
        const epoch = generation;
        void deliver(item, () => epoch === generation && active.has(item.id)).catch(() => undefined);
      }
    },
    dismiss(id: string): void { store.update((state) => ({ ...state, items: state.items.filter((item) => item.id !== id) })); },
    async checkPermission(request = false): Promise<void> {
      const permission = await alertPermission(request);
      store.update((state) => ({ ...state, permission }));
    }
  };
}
export const connectionAlerts = createConnectionAlerts();
