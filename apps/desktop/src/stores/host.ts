import { get, writable, type Readable } from 'svelte/store';
import { deviceIdentity } from '../lib/deviceIdentity';
import { engineStatus, getLocalRouteAddress, launchEngine, startRelay, stopEngine, stopRelay } from '../lib/platform';
import { coordinateP2pConnection } from '../lib/p2pSignaling';
import { trustedPendingSession } from '../lib/trustedDevice';
import { PROTOCOL_VERSION, SANSER_VERSION, type ConnectionSession, type RuntimeStatus } from '../lib/types';
import { diagnostics } from './diagnostics';
import { preferences } from './preferences';
import { session } from './session';

export interface HostState {
  online: boolean;
  busy: boolean;
  deviceId: string | null;
  routeAddress: string | null;
  sessions: ConnectionSession[];
  actionSessionId: string | null;
  launchingSessionId: string | null;
  failedP2pSessionId: string | null;
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
  launchingSessionId: null,
  failedP2pSessionId: null,
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
  retry(sessionId: string): Promise<void>;
  setEngineRunning(running: boolean): void;
}

function createHostStore(): HostStore {
  const store = writable<HostState>(initial);
  let heartbeatTimer: number | null = null;
  let sessionTimer: number | null = null;
  let sessionPollInFlight = false;
  let lifecycleGeneration = 0;
  let runtimeForHost: RuntimeStatus | null = null;
  const retryCounts = new Map<string, number>();
  const retryAfter = new Map<string, number>();
  const readyGenerations = new Map<string, string>();
  const autoAcceptRetryAfter = new Map<string, number>();

  async function acceptPendingSession(sessionId: string, automatic: boolean): Promise<ConnectionSession> {
    const client = session.client();
    if (!client) throw new Error('Cloud sign-in is required to accept a session');
    const current = get(store);
    if (current.actionSessionId && current.actionSessionId !== sessionId) {
      throw new Error('Another session action is already in progress');
    }
    store.update((state) => ({ ...state, actionSessionId: sessionId, error: null }));
    try {
      const accepted = await client.acceptSession(sessionId);
      store.update((state) => ({
        ...state,
        sessions: state.sessions.map((item) => (item.id === accepted.id ? accepted : item)),
        actionSessionId: null
      }));
      diagnostics.add({
        level: 'info',
        category: 'session',
        message: automatic
          ? 'Automatically accepted a request from a trusted device'
          : 'Host accepted the connection request',
        details: { sessionId: accepted.id, requesterDeviceId: accepted.requesterDeviceId }
      });
      return accepted;
    } catch (error) {
      store.update((state) => ({ ...state, actionSessionId: null }));
      throw error;
    }
  }

  function resolutionSize(): { width: number; height: number } {
    switch (get(preferences).stream.resolution) {
      case '720p': return { width: 1280, height: 720 };
      case '1440p': return { width: 2560, height: 1440 };
      case '2160p': return { width: 3840, height: 2160 };
      default: return { width: 1920, height: 1080 };
    }
  }

  async function startAcceptedSession(request: ConnectionSession, force = false): Promise<void> {
    const current = get(store);
    const client = session.client();
    if (
      !client ||
      !runtimeForHost ||
      !current.online ||
      !current.deviceId ||
      !request.requesterReadyAt ||
      request.transport !== 'native' ||
      current.engineRunning ||
      current.launchingSessionId
    ) return;
    if (!force && (retryCounts.get(request.id) ?? 0) >= 3) return;
    if (!force && Date.now() < (retryAfter.get(request.id) ?? 0)) return;

    const hostDeviceId = current.deviceId;
    store.update((state) => ({ ...state, launchingSessionId: request.id, error: null }));
    try {
      const credentials = await client.sessionCredentials(request.id, hostDeviceId);
      const settings = get(preferences);
      const size = resolutionSize();
      if (!request.requesterDeviceId) throw new Error('Missing requester device ID for P2P connection');
      let route: Awaited<ReturnType<typeof coordinateP2pConnection>> | null = null;
      let relayRoute: Awaited<ReturnType<typeof startRelay>> | null = null;
      try {
        if (request.networkMode === 'relay') throw new Error('Relay-only mode selected');
        route = await coordinateP2pConnection(
          client,
          request.id,
          hostDeviceId,
          request.requesterDeviceId,
          true,
          credentials.sessionToken,
          settings.host.directUdpPort
        );
      } catch (directError) {
        if (request.networkMode === 'direct') throw directError;
        const accessToken = await client.getAccessToken();
        if (!accessToken) throw new Error('Relay fallback requires an authenticated access token');
        diagnostics.add({
          level: 'warn',
          category: 'network',
          message: `Direct P2P failed; switching to encrypted Sanser relay: ${directError instanceof Error ? directError.message : String(directError)}`
        });
        relayRoute = await startRelay({
          kind: 'host',
          serverUrl: client.serverUrl,
          sessionId: request.id,
          deviceId: hostDeviceId,
          peerDeviceId: request.requesterDeviceId,
          accessToken,
          sessionCredential: credentials.sessionToken,
          preferredEnginePort: settings.host.directUdpPort
        });
        diagnostics.add({ level: 'info', category: 'network', message: 'Encrypted relay route is ready' });
      }
      if (!get(store).online || get(store).deviceId !== hostDeviceId) {
        if (relayRoute) await stopRelay('host').catch(() => undefined);
        return;
      }
      await launchEngine({
        kind: 'host',
        sessionId: request.id,
        address: relayRoute ? '127.0.0.1' : route?.remoteAddress,
        port: relayRoute?.proxyPort ?? route?.remotePort,
        codec: request.requestedCodec,
        fps: settings.stream.fps,
        bitrateKbps: Math.round(settings.stream.bitrateMbps * 1_000),
        width: size.width,
        height: size.height,
        networkMode: request.networkMode,
        audioEnabled: settings.host.audioEnabled,
        inputEnabled: settings.host.inputEnabled,
        relativeMouse: false,
        sessionToken: credentials.sessionToken,
        udpBindPort: relayRoute?.enginePort ?? route?.localPort,
        relay: relayRoute !== null
      });
      retryCounts.delete(request.id);
      retryAfter.delete(request.id);
      store.update((state) => ({
        ...state,
        engineRunning: true,
        launchingSessionId: null,
        failedP2pSessionId: null,
        error: null
      }));
      diagnostics.add({ level: 'info', category: 'engine', message: 'Native Windows host started' });
    } catch (error) {
      const attempts = (retryCounts.get(request.id) ?? 0) + 1;
      retryCounts.set(request.id, attempts);
      retryAfter.set(request.id, Date.now() + Math.min(1_500 * attempts, 5_000));
      const message = error instanceof Error ? error.message : 'Unable to start the native Windows host';
      store.update((state) => ({
        ...state,
        launchingSessionId: null,
        failedP2pSessionId: attempts >= 3 ? request.id : null,
        error: attempts >= 3 ? message : `P2P attempt ${attempts}/3 failed; retrying automatically`
      }));
      diagnostics.add({ level: attempts >= 3 ? 'error' : 'warn', category: 'network', message });
    }
  }

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
      if (get(preferences).host.autoAcceptOwnDevices && get(store).actionSessionId === null) {
        const trustedRequest = trustedPendingSession(
          response.items,
          current.deviceId,
          get(preferences).trustedDeviceIds
        );
        if (
          trustedRequest &&
          Date.now() >= (autoAcceptRetryAfter.get(trustedRequest.id) ?? 0)
        ) {
          try {
            await acceptPendingSession(trustedRequest.id, true);
            autoAcceptRetryAfter.delete(trustedRequest.id);
          } catch (error) {
            autoAcceptRetryAfter.set(trustedRequest.id, Date.now() + 5_000);
            diagnostics.add({
              level: 'warn',
              category: 'session',
              message: error instanceof Error ? error.message : 'Unable to auto-accept trusted device'
            });
          }
        }
      }
      const latest = get(store);
      if (latest.engineRunning) {
        const active = latest.sessions.find((item) => item.status === 'accepted');
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
      const ready = get(store).sessions.find((item) => item.status === 'accepted' && item.requesterReadyAt);
      if (ready?.requesterReadyAt) {
        if (readyGenerations.get(ready.id) !== ready.requesterReadyAt) {
          readyGenerations.set(ready.id, ready.requesterReadyAt);
          retryCounts.delete(ready.id);
          retryAfter.delete(ready.id);
          store.update((state) => ({
            ...state,
            failedP2pSessionId: state.failedP2pSessionId === ready.id ? null : state.failedP2pSessionId,
            error: state.failedP2pSessionId === ready.id ? null : state.error
          }));
          diagnostics.add({
            level: 'info',
            category: 'session',
            message: 'Host joined a fresh native negotiation generation',
            details: { sessionId: ready.id, requesterReadyAt: ready.requesterReadyAt }
          });
        }
        void startAcceptedSession(ready);
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
        runtimeForHost = runtime;
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
          nativeTransport: runtime.capabilities.nativeDirect.state === 'available',
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
        runtimeForHost = null;
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
      runtimeForHost = null;
      retryCounts.clear();
      retryAfter.clear();
      readyGenerations.clear();
      autoAcceptRetryAfter.clear();
      store.update((current) => ({ ...current, busy: true }));
      try {
        await stopEngine('host').catch(() => undefined);
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
      return acceptPendingSession(sessionId, false);
    },
    async reject(sessionId: string): Promise<void> {
      const client = session.client();
      if (!client) throw new Error('Cloud sign-in is required to reject a session');
      store.update((state) => ({ ...state, actionSessionId: sessionId, error: null }));
      try {
        await client.rejectSession(sessionId);
        retryCounts.delete(sessionId);
        retryAfter.delete(sessionId);
        readyGenerations.delete(sessionId);
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
        await stopEngine('host').catch(() => undefined);
        retryCounts.delete(sessionId);
        retryAfter.delete(sessionId);
        readyGenerations.delete(sessionId);
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
    async retry(sessionId: string): Promise<void> {
      const target = get(store).sessions.find((item) => item.id === sessionId);
      if (!target) throw new Error('The accepted session is no longer available');
      retryCounts.delete(sessionId);
      retryAfter.delete(sessionId);
      store.update((state) => ({ ...state, failedP2pSessionId: null, error: null }));
      await startAcceptedSession(target, true);
    },
    setEngineRunning(running: boolean): void {
      store.update((state) => ({ ...state, engineRunning: running }));
    }
  };
}

export const host: HostStore = createHostStore();
