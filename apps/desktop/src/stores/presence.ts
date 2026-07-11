import { get, writable } from 'svelte/store';
import { deviceIdentity } from '../lib/deviceIdentity';
import { PROTOCOL_VERSION, SANSER_VERSION, type RuntimeStatus } from '../lib/types';
import { diagnostics } from './diagnostics';
import { session } from './session';

export interface PresenceState {
  deviceId: string | null;
  online: boolean;
  busy: boolean;
  error: string | null;
}

const initial: PresenceState = { deviceId: null, online: false, busy: false, error: null };

function displayName(platform: string): string {
  if (platform.toLowerCase().includes('mac')) return 'This Mac';
  if (platform.toLowerCase().includes('windows')) return 'This Windows PC';
  return 'This computer';
}

function createPresenceStore() {
  const store = writable<PresenceState>(initial);
  let heartbeatTimer: number | null = null;
  let starting: Promise<void> | null = null;

  function stopTimer(): void {
    if (heartbeatTimer !== null) window.clearInterval(heartbeatTimer);
    heartbeatTimer = null;
  }

  async function register(runtime: RuntimeStatus): Promise<void> {
    const client = session.client();
    if (!client) throw new Error('Sign in is required before registering this client');
    if (runtime.capabilities.clientEngine.state !== 'available') {
      throw new Error(runtime.capabilities.clientEngine.reason ?? 'The native client engine is unavailable');
    }

    const accountId = get(session).account?.id;
    if (!accountId) throw new Error('The signed-in account is unavailable');
    let deviceId = deviceIdentity(accountId, 'client');

    const nativeTransport = runtime.capabilities.nativeSnv2.state === 'available';
    const webRtc = runtime.capabilities.webRtc.state === 'available';
    const registered = await client.registerDevice({
      id: deviceId,
      name: displayName(runtime.platform),
      platform: runtime.platform,
      osVersion: runtime.platform,
      gpu: 'Not reported',
      version: SANSER_VERSION,
      protocolVersion: PROTOCOL_VERSION,
      codecs: ['auto', 'h264', 'hevc'],
      nativeTransport,
      webRtc,
      audio: true,
      gamepad: runtime.capabilities.gamepad.state === 'available'
    });

    deviceId = registered.id || deviceId;
    store.set({ deviceId, online: true, busy: false, error: null });
    stopTimer();
    heartbeatTimer = window.setInterval(() => {
      const current = get(store);
      const activeClient = session.client();
      if (!activeClient || !current.online || !current.deviceId) return;
      void activeClient.heartbeatDevice(current.deviceId, false).catch((error: unknown) => {
        const message = error instanceof Error ? error.message : 'Client heartbeat failed';
        store.update((state) => ({ ...state, error: message }));
      });
    }, 15_000);
    diagnostics.add({ level: 'info', category: 'device', message: 'This client is registered', details: { deviceId } });
  }

  return {
    subscribe: store.subscribe,
    async start(runtime: RuntimeStatus): Promise<void> {
      if (get(store).online) return;
      if (starting) return starting;
      store.update((state) => ({ ...state, busy: true, error: null }));
      starting = register(runtime)
        .catch((error: unknown) => {
          const message = error instanceof Error ? error.message : 'Unable to register this client';
          store.set({ ...initial, error: message });
          diagnostics.add({ level: 'warn', category: 'device', message });
        })
        .finally(() => {
          starting = null;
        });
      return starting;
    },
    async stop(): Promise<void> {
      const current = get(store);
      const client = session.client();
      stopTimer();
      starting = null;
      store.set(initial);
      if (client && current.deviceId && current.online) {
        try {
          await client.offlineDevice(current.deviceId);
        } catch (error) {
          diagnostics.add({
            level: 'warn',
            category: 'device',
            message: error instanceof Error ? error.message : 'Unable to mark this client offline'
          });
        }
      }
    }
  };
}

export const presence = createPresenceStore();
