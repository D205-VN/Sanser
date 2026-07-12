import { get, writable, type Readable } from 'svelte/store';
import { deviceIdentity } from '../lib/deviceIdentity';
import { engineStatus, getLocalRouteAddress, stopEngine } from '../lib/platform';
import { PROTOCOL_VERSION, SANSER_VERSION, type ConnectionSession, type RuntimeStatus } from '../lib/types';
import { diagnostics } from './diagnostics';
import { session } from './session';

export interface HostState {
  online: boolean;
  busy: boolean;
  deviceId: string | null;
  routeAddress: string | null;
  sessions: ConnectionSession[];
  actionSessionId: string | null;
  engineRunning: boolean;
  error: string | null;
}

const initial: HostState = {
  online: false,
  busy: false,
  deviceId: null,
  routeAddress: null,
  sessions: [],
  actionSessionId: null,
  engineRunning: false,
  error: null
};

const HEARTBEAT_INTERVAL_MS = 6_000;

export interface HostStore extends Readable<HostState> {
  online(runtime: RuntimeStatus): Promise<void>;
  offline(): Promise<void>;
  refreshRequests(): Promise<void>;
  accept(sessionId: string): Promise<ConnectionSession>;
  reject(sessionId: string): Promise<void>;
  disconnect(sessionId: string): Promise<void>;
  setEngineRunning(running: boolean): void;
}

function createHostStore(): HostStore {
  const store = writable<HostState>(initial);
  let heartbeatTimer: number | null = null;
  let sessionTimer: number | null = null;
  let sessionPollInFlight = false;
  let lifecycleGeneration = 0;

  function stopTimers(): void {
    if (heartbeatTimer !== null) window.clearInterval(heartbeatTimer);
    if (sessionTimer !== null) window.clearInterval(sessionTimer);
    heartbeatTimer = null;
    sessionTimer = null;
  }

  async function refreshSessions(reportError = false): Promise<void> {
    const current = get(store);
    const client = session.client();
    if (sessionPollInFlight || !client || !current.online || !current.deviceId) return;
    sessionPollInFlight = true;
    try {
      const response = await client.hostSessions(current.deviceId);
      const live = get(store);
      if (!live.online || live.deviceId !== current.deviceId) return;
      store.update((state) => ({ ...state, sessions: response.items, error: null }));
      const latest = get(store);
      if (latest.engineRunning) {
        const active = response.items.find((item) => item.status === 'accepted');
        if (!active) {
          await stopEngine('host').catch(() => undefined);
          store.update((state) => ({ ...state, engineRunning: false }));
        } else {
          const status = await engineStatus('host');
          if (!status.running) {
            const message = status.lastError ?? 'The native Windows host stopped unexpectedly';
            store.update((state) => ({
              ...state,
              engineRunning: false,
              actionSessionId: active.id,
              error: message
            }));
            await client.disconnectSession(active.id).catch(() => undefined);
            store.update((state) => ({
              ...state,
              sessions: state.sessions.filter((item) => item.id !== active.id),
              actionSessionId: null
            }));
            diagnostics.add({ level: 'warn', category: 'engine', message });
          }
        }
      }
    } catch (error) {
      if (reportError) {
        store.update((state) => ({
          ...state,
          error: error instanceof Error ? error.message : 'Unable to load connection requests'
        }));
      }
    } finally {
      sessionPollInFlight = false;
    }
  }

  return {
    subscribe: store.subscribe,
    async online(runtime: RuntimeStatus): Promise<void> {
      const generation = ++lifecycleGeneration;
      const client = session.client();
      if (!client) throw new Error('Cloud sign-in is required to advertise this host');
      if (runtime.capabilities.hostEngine.state !== 'available') {
        throw new Error(runtime.capabilities.hostEngine.reason ?? 'Windows host engine is unavailable');
      }

      store.update((state) => ({ ...state, busy: true, error: null }));
      try {
        const accountId = get(session).account?.id;
        if (!accountId) throw new Error('The signed-in account is unavailable');
        let deviceId = deviceIdentity(accountId, 'host');
        let routeAddress: string | null = null;
        try {
          routeAddress = await getLocalRouteAddress(client.serverUrl);
        } catch (error) {
          diagnostics.add({
            level: 'warn',
            category: 'network',
            message: error instanceof Error ? error.message : 'Unable to discover this Windows host route'
          });
        }
        if (runtime.capabilities.nativeDirect.state === 'available' && routeAddress === null && runtime.capabilities.webRtc.state !== 'available') {
          throw new Error('No usable IPv4 route was detected for this Windows host. Connect both computers to the same LAN or configure WebRTC/TURN.');
        }
        if (runtime.capabilities.nativeDirect.state !== 'available' && runtime.capabilities.webRtc.state !== 'available') {
          throw new Error(runtime.capabilities.nativeDirect.reason ?? 'No verified media transport is available on this Windows host');
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
          nativeTransport: runtime.capabilities.nativeDirect.state === 'available' && routeAddress !== null,
          webRtc: runtime.capabilities.webRtc.state === 'available',
          audio: true,
          gamepad: runtime.capabilities.gamepad.state === 'available',
          routeAddress: routeAddress ?? undefined
        });
        if (generation !== lifecycleGeneration) {
          await client.offlineDevice(registered.id).catch(() => undefined);
          return;
        }
        deviceId = registered.id || deviceId;
        store.set({ ...initial, online: true, deviceId, routeAddress });
        stopTimers();
        heartbeatTimer = window.setInterval(() => {
          const current = get(store);
          const activeClient = session.client();
          if (!activeClient || !current.online || !current.deviceId) return;
          void activeClient
            .heartbeatDevice(current.deviceId, current.engineRunning, current.routeAddress ?? undefined)
            .catch(() => {
              store.update((state) => ({ ...state, error: 'Host heartbeat failed; retrying' }));
            });
        }, HEARTBEAT_INTERVAL_MS);
        await refreshSessions();
        sessionTimer = window.setInterval(() => void refreshSessions(), 2_000);
        diagnostics.add({ level: 'info', category: 'engine', message: 'Host is online', details: { deviceId } });
      } catch (error) {
        if (generation !== lifecycleGeneration) return;
        const message = error instanceof Error ? error.message : 'Unable to bring host online';
        store.set({ ...initial, error: message });
        throw error;
      }
    },
    async offline(): Promise<void> {
      lifecycleGeneration += 1;
      const current = get(store);
      const client = session.client();
      stopTimers();
      store.update((current) => ({ ...current, busy: true }));
      try {
        if (client && current.deviceId && current.online) {
          await client.offlineDevice(current.deviceId);
        }
      } catch (error) {
        diagnostics.add({
          level: 'warn',
          category: 'engine',
          message: error instanceof Error ? error.message : 'Unable to mark this host offline'
        });
      } finally {
        store.set(initial);
      }
      diagnostics.add({ level: 'info', category: 'engine', message: 'Host is offline' });
    },
    async refreshRequests(): Promise<void> {
      await refreshSessions(true);
    },
    async accept(sessionId: string): Promise<ConnectionSession> {
      const client = session.client();
      if (!client) throw new Error('Cloud sign-in is required to accept a session');
      store.update((state) => ({ ...state, actionSessionId: sessionId, error: null }));
      try {
        const accepted = await client.acceptSession(sessionId);
        store.update((state) => ({
          ...state,
          sessions: state.sessions.map((item) => (item.id === accepted.id ? accepted : item)),
          actionSessionId: null
        }));
        return accepted;
      } catch (error) {
        store.update((state) => ({ ...state, actionSessionId: null }));
        throw error;
      }
    },
    async reject(sessionId: string): Promise<void> {
      const client = session.client();
      if (!client) throw new Error('Cloud sign-in is required to reject a session');
      store.update((state) => ({ ...state, actionSessionId: sessionId, error: null }));
      try {
        await client.rejectSession(sessionId);
        store.update((state) => ({
          ...state,
          sessions: state.sessions.filter((item) => item.id !== sessionId),
          actionSessionId: null
        }));
      } catch (error) {
        store.update((state) => ({ ...state, actionSessionId: null }));
        throw error;
      }
    },
    async disconnect(sessionId: string): Promise<void> {
      const client = session.client();
      if (!client) throw new Error('Cloud sign-in is required to disconnect a session');
      store.update((state) => ({ ...state, actionSessionId: sessionId, error: null }));
      try {
        await client.disconnectSession(sessionId);
        store.update((state) => ({
          ...state,
          sessions: state.sessions.filter((item) => item.id !== sessionId),
          actionSessionId: null,
          engineRunning: false
        }));
      } catch (error) {
        store.update((state) => ({ ...state, actionSessionId: null }));
        throw error;
      }
    },
    setEngineRunning(running: boolean): void {
      store.update((state) => ({ ...state, engineRunning: running }));
    }
  };
}

export const host: HostStore = createHostStore();
