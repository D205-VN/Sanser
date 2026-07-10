import { get, writable } from 'svelte/store';
import { secureGet, secureSet } from '../lib/platform';
import { PROTOCOL_VERSION, SANSER_VERSION, type RuntimeStatus } from '../lib/types';
import { diagnostics } from './diagnostics';
import { session } from './session';

export interface HostState {
  online: boolean;
  busy: boolean;
  deviceId: string | null;
  error: string | null;
}

const initial: HostState = { online: false, busy: false, deviceId: null, error: null };

function createHostStore() {
  const store = writable<HostState>(initial);
  let heartbeatTimer: number | null = null;

  function stopTimer(): void {
    if (heartbeatTimer !== null) window.clearInterval(heartbeatTimer);
    heartbeatTimer = null;
  }

  return {
    subscribe: store.subscribe,
    async online(runtime: RuntimeStatus): Promise<void> {
      const client = session.client();
      if (!client) throw new Error('Cloud sign-in is required to advertise this host');
      if (runtime.capabilities.hostEngine.state !== 'available') {
        throw new Error(runtime.capabilities.hostEngine.reason ?? 'Windows host engine is unavailable');
      }

      store.update((state) => ({ ...state, busy: true, error: null }));
      try {
        let deviceId = await secureGet('device_identity');
        if (!deviceId) {
          deviceId = crypto.randomUUID();
          await secureSet('device_identity', deviceId);
        }
        const registered = await client.registerDevice({
          id: deviceId,
          name: 'Sanser Host',
          platform: runtime.platform,
          osVersion: runtime.platform,
          gpu: 'Not reported by desktop shell',
          version: SANSER_VERSION,
          protocolVersion: PROTOCOL_VERSION,
          codecs: ['auto', 'h264', 'hevc'],
          nativeTransport: runtime.capabilities.nativeSnv2.state === 'available',
          webRtc: runtime.capabilities.webRtc.state === 'available',
          audio: true,
          gamepad: runtime.capabilities.gamepad.state === 'available'
        });
        deviceId = registered.id || deviceId;
        store.set({ online: true, busy: false, deviceId, error: null });
        stopTimer();
        heartbeatTimer = window.setInterval(() => {
          const current = get(store);
          if (!current.online || !current.deviceId) return;
          void client.heartbeatDevice(current.deviceId, true).catch(() => {
            store.update((state) => ({ ...state, error: 'Host heartbeat failed; retrying' }));
          });
        }, 15_000);
        diagnostics.add({ level: 'info', category: 'engine', message: 'Host is online', details: { deviceId } });
      } catch (error) {
        const message = error instanceof Error ? error.message : 'Unable to bring host online';
        store.set({ ...initial, error: message });
        throw error;
      }
    },
    async offline(): Promise<void> {
      const state = get(store);
      stopTimer();
      store.update((current) => ({ ...current, busy: true }));
      try {
        const client = session.client();
        if (client && state.deviceId) await client.heartbeatDevice(state.deviceId, false);
      } finally {
        store.set(initial);
        diagnostics.add({ level: 'info', category: 'engine', message: 'Host is offline' });
      }
    }
  };
}

export const host = createHostStore();
