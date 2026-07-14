import { get, writable } from 'svelte/store';
import { deviceIdentity } from '../lib/deviceIdentity';
import { getLocalRouteAddress } from '../lib/platform';
import { PROTOCOL_VERSION, SANSER_VERSION, type RuntimeStatus } from '../lib/types';
import { diagnostics } from './diagnostics';
import { session } from './session';

export interface PresenceState {
  deviceId: string | null;
  online: boolean;
  busy: boolean;
  routeAddress: string | null;
  error: string | null;
}

const initial: PresenceState = { deviceId: null, online: false, busy: false, routeAddress: null, error: null };
const HEARTBEAT_INTERVAL_MS = 6_000;

function displayName(platform: string): string {
  if (platform.toLowerCase().includes('mac')) return 'This Mac';
  if (platform.toLowerCase().includes('windows')) return 'This Windows PC';
  return 'This computer';
}

function createPresenceStore() {
  const store = writable<PresenceState>(initial);
  let heartbeatTimer: number | null = null;
  let starting: Promise<void> | null = null;
  let lifecycleGeneration = 0;

  function stopTimer(): void {
    if (heartbeatTimer !== null) window.clearInterval(heartbeatTimer);
    heartbeatTimer = null;
  }

  async function register(runtime: RuntimeStatus, generation: number): Promise<void> {
    const client = session.client();
    if (!client) throw new Error('Sign in is required before registering this client');
    if (runtime.capabilities.clientEngine.state !== 'available') {
      throw new Error(runtime.capabilities.clientEngine.reason ?? 'The native client engine is unavailable');
    }

    const accountId = get(session).account?.id;
    if (!accountId) throw new Error('The signed-in account is unavailable');
    let deviceId = deviceIdentity(accountId, 'client');

    let routeAddress: string | null = null;
    try {
      routeAddress = await getLocalRouteAddress(client.serverUrl);
    } catch (error) {
      diagnostics.add({
        level: 'warn',
        category: 'network',
        message: error instanceof Error ? error.message : 'Unable to discover this Mac route'
      });
    }
    if (runtime.capabilities.nativeDirect.state !== 'available' && runtime.capabilities.webRtc.state !== 'available') {
      throw new Error(runtime.capabilities.nativeDirect.reason ?? 'No verified media transport is available on this Mac');
    }
    // The native engine is also used behind the local UDP-to-WSS relay bridge,
    // so lack of an advertisable LAN address must not disable relay sessions.
    const nativeTransport = runtime.capabilities.nativeDirect.state === 'available';
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
      gamepad: runtime.capabilities.gamepad.state === 'available',
      routeAddress: routeAddress ?? undefined
    });
    if (generation !== lifecycleGeneration) {
      await client.offlineDevice(registered.id).catch(() => undefined);
      return;
    }

    deviceId = registered.id || deviceId;
    store.set({ deviceId, online: true, busy: false, routeAddress, error: null });
    stopTimer();
    heartbeatTimer = window.setInterval(() => {
      const current = get(store);
      const activeClient = session.client();
      if (!activeClient || !current.online || !current.deviceId) return;
      void activeClient.heartbeatDevice(current.deviceId, false, current.routeAddress ?? undefined).catch((error: unknown) => {
        const message = error instanceof Error ? error.message : 'Client heartbeat failed';
        store.update((state) => ({ ...state, error: message }));
      });
    }, HEARTBEAT_INTERVAL_MS);
    diagnostics.add({ level: 'info', category: 'device', message: 'This client is registered', details: { deviceId } });
  }

  return {
    subscribe: store.subscribe,
    async start(runtime: RuntimeStatus): Promise<void> {
      if (get(store).online) return;
      if (starting) return starting;
      const generation = ++lifecycleGeneration;
      store.update((state) => ({ ...state, busy: true, error: null }));
      starting = register(runtime, generation)
        .catch((error: unknown) => {
          if (generation !== lifecycleGeneration) return;
          const message = error instanceof Error ? error.message : 'Unable to register this client';
          store.set({ ...initial, error: message });
          diagnostics.add({ level: 'warn', category: 'device', message });
        })
        .finally(() => {
          if (generation === lifecycleGeneration) starting = null;
        });
      return starting;
    },
    async stop(): Promise<void> {
      lifecycleGeneration += 1;
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
