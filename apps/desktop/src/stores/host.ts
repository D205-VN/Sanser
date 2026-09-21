import { get, writable, type Readable } from 'svelte/store';
import { deviceIdentity } from '../lib/deviceIdentity';
import { ApiError } from '../lib/api';
import { engineStatus, getLocalRouteAddress, launchEngine, startRelay, stopEngine, stopRelay } from '../lib/platform';
import { coordinateP2pConnection } from '../lib/p2pSignaling';
import { resolveStreamProfile } from '../lib/streamProfile';
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
  errorCode: string | null;
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
  error: null,
  errorCode: null
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

export function createHostStore(): HostStore {
  const store = writable<HostState>(initial);
  let heartbeatTimer: number | null = null;
  let sessionTimer: number | null = null;
  let sessionPollInFlight = false;
  let lifecycleGeneration = 0;
  let runtimeForHost: RuntimeStatus | null = null;
  let negotiationController: AbortController | null = null;
  let launchTask: Promise<void> | null = null;
  let registrationTask: Promise<void> | null = null;
  let pollError: string | null = null;
  const retryCounts = new Map<string, number>();
  const retryAfter = new Map<string, number>();
  const readyGenerations = new Map<string, string>();
  const autoAcceptRetryAfter = new Map<string, number>();

  async function acceptPendingSession(sessionId: string, automatic: boolean): Promise<ConnectionSession> {
    const client = session.client();
    if (!client) throw new Error('Cloud sign-in is required to accept a session');
    const current = get(store);
    const generation = lifecycleGeneration;
    if (!current.online || current.busy) throw new Error('This host is offline');
    if (current.actionSessionId) {
      throw new Error('Another session action is already in progress');
    }
    store.update((state) => ({ ...state, actionSessionId: sessionId, error: null }));
    try {
      const accepted = await client.acceptSession(sessionId);
      if (generation !== lifecycleGeneration) {
        await client.disconnectSession(accepted.id).catch(() => undefined);
        return accepted;
      }
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
      if (generation === lifecycleGeneration) store.update((state) => ({ ...state, actionSessionId: null }));
      throw error;
    }
  }

  function resolutionSize(resolution: string): { width: number; height: number } {
    switch (resolution) {
      case '720p': return { width: 1280, height: 720 };
      case '1440p': return { width: 2560, height: 1440 };
      case '2160p': return { width: 3840, height: 2160 };
      default: return { width: 1920, height: 1080 };
    }
  }

  function startAcceptedSession(request: ConnectionSession, force = false): Promise<void> {
    if (launchTask) return launchTask;
    const controller = new AbortController();
    negotiationController = controller;
    launchTask = runAcceptedSession(request, controller.signal, force).finally(() => {
      negotiationController = null;
      launchTask = null;
    });
    return launchTask;
  }

  async function runAcceptedSession(request: ConnectionSession, signal: AbortSignal, force: boolean): Promise<void> {
    const current = get(store);
    const client = session.client();
    if (
      !client ||
      !runtimeForHost ||
      !current.online ||
      current.busy ||
      current.actionSessionId !== null ||
      !current.deviceId ||
      !request.requesterReadyAt ||
      request.transport !== 'native' ||
      current.engineRunning ||
      current.launchingSessionId
    ) return;
    if (!force && (retryCounts.get(request.id) ?? 0) >= 3) return;
    if (!force && Date.now() < (retryAfter.get(request.id) ?? 0)) return;

    const hostDeviceId = current.deviceId;
    const generation = lifecycleGeneration;
    const cancelled = () => signal.aborted || generation !== lifecycleGeneration || !get(store).online;
    store.update((state) => ({ ...state, launchingSessionId: request.id, error: null }));
    try {
      const credentials = await client.sessionCredentials(request.id, hostDeviceId);
      if (cancelled()) return;
      const settings = get(preferences);
      const stream = resolveStreamProfile(settings.stream, request.qualityProfile);
      const size = resolutionSize(stream.resolution);
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
          settings.host.directUdpPort,
          signal
        );
      } catch (directError) {
        if (cancelled()) return;
        if (request.networkMode === 'direct') throw directError;
        const accessToken = await client.getAccessToken();
        if (cancelled()) return;
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
      if (cancelled() || get(store).deviceId !== hostDeviceId) {
        if (relayRoute) await stopRelay('host').catch(() => undefined);
        return;
      }
      await launchEngine({
        kind: 'host',
        wireProtocol: credentials.wireProtocol,
        sessionId: request.id,
        address: relayRoute ? '127.0.0.1' : route?.remoteAddress,
        port: relayRoute?.proxyPort ?? route?.remotePort,
        codec: request.requestedCodec,
        fps: stream.fps,
        bitrateKbps: Math.round(stream.bitrateMbps * 1_000),
        width: size.width,
        height: size.height,
        networkMode: request.networkMode,
        audioEnabled: credentials.wireProtocol !== 'snv2' && settings.host.audioEnabled,
        inputEnabled: settings.host.inputEnabled,
        relativeMouse: false,
        sessionToken: credentials.sessionToken,
        udpBindPort: relayRoute?.enginePort ?? route?.localPort,
        relay: relayRoute !== null
      });
      if (cancelled()) {
        await stopEngine('host').catch(() => undefined);
        return;
      }
      retryCounts.delete(request.id);
      retryAfter.delete(request.id);
      store.update((state) => ({
        ...state,
        engineRunning: true,
        launchingSessionId: null,
        failedP2pSessionId: null,
        error: null
      }));
      diagnostics.add({ level: 'info', category: 'engine', message: 'Native host started' });
    } catch (error) {
      await stopRelay('host').catch(() => undefined);
      if (cancelled()) return;
      const attempts = (retryCounts.get(request.id) ?? 0) + 1;
      retryCounts.set(request.id, attempts);
      retryAfter.set(request.id, Date.now() + Math.min(1_500 * attempts, 5_000));
      const message = error instanceof Error ? error.message : 'Unable to start the native host';
      store.update((state) => ({
        ...state,
        launchingSessionId: null,
        failedP2pSessionId: attempts >= 3 ? request.id : null,
        error: attempts >= 3 ? message : `P2P attempt ${attempts}/3 failed; retrying automatically`
      }));
      diagnostics.add({ level: attempts >= 3 ? 'error' : 'warn', category: 'network', message });
    } finally {
      if (generation === lifecycleGeneration) {
        store.update((state) => ({ ...state, launchingSessionId: null }));
      }
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
    const generation = lifecycleGeneration;
    if (sessionPollInFlight || !client || !current.online || current.busy || !current.deviceId) return;
    sessionPollInFlight = true;
    try {
      const response = await client.hostSessions(current.deviceId);
      const live = get(store);
      if (generation !== lifecycleGeneration || !live.online || live.deviceId !== current.deviceId) return;
      store.update((state) => ({ ...state, sessions: response.items, error: state.error === pollError ? null : state.error }));
      pollError = null;
      const launchingId = get(store).launchingSessionId;
      if (launchingId && !response.items.some((item) => item.id === launchingId && item.status === 'accepted')) {
        negotiationController?.abort();
        await launchTask;
        if (generation !== lifecycleGeneration) return;
      }
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
            if (generation !== lifecycleGeneration) return;
            autoAcceptRetryAfter.set(trustedRequest.id, Date.now() + 5_000);
            diagnostics.add({
              level: 'warn',
              category: 'session',
              message: error instanceof Error ? error.message : 'Unable to auto-accept trusted device'
            });
          }
        }
      }
      if (generation !== lifecycleGeneration) return;
      const latest = get(store);
      if (latest.engineRunning) {
        const active = latest.sessions.find((item) => item.status === 'accepted');
        if (!active) {
          await stopEngine('host').catch(() => undefined);
          if (generation !== lifecycleGeneration) return;
          store.update((state) => ({ ...state, engineRunning: false }));
        } else {
          const status = await engineStatus('host');
          if (generation !== lifecycleGeneration) return;
          if (!status.running) {
            const message = status.lastError ?? 'The native host stopped unexpectedly';
            await stopEngine('host').catch(() => undefined);
            if (generation !== lifecycleGeneration) return;
            retryCounts.set(active.id, 3);
            retryAfter.delete(active.id);
            store.update((state) => ({
              ...state,
              engineRunning: false,
              launchingSessionId: null,
              failedP2pSessionId: active.id,
              error: message
            }));
            diagnostics.add({
              level: 'warn',
              category: 'engine',
              message: `${message}; the accepted session remains available for retry`
            });
          }
        }
      }
      const ready = get(store).sessions.find((item) => item.status === 'accepted' && item.requesterReadyAt);
      if (ready?.requesterReadyAt && !latest.engineRunning) {
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
      if (reportError && generation === lifecycleGeneration) {
        pollError = error instanceof Error ? error.message : 'Unable to load connection requests';
        store.update((state) => ({
          ...state,
          error: pollError
        }));
      }
    } finally {
      sessionPollInFlight = false;
    }
  }

  async function registerHost(runtime: RuntimeStatus): Promise<void> {
    const generation = ++lifecycleGeneration;
    const client = session.client();
    if (!client) throw new Error('Cloud sign-in is required to advertise this host');
    if (runtime.capabilities.hostEngine.state !== 'available') {
      throw new Error(runtime.capabilities.hostEngine.reason ?? 'Host engine is unavailable');
    }

    store.update((state) => ({ ...state, busy: true, error: null, errorCode: null }));
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
          message: error instanceof Error ? error.message : 'Unable to discover this host route'
        });
      }
      if (generation !== lifecycleGeneration) return;
      if (runtime.capabilities.nativeDirect.state !== 'available' && runtime.capabilities.webRtc.state !== 'available') {
        throw new Error(runtime.capabilities.nativeDirect.reason ?? 'No verified media transport is available on this host');
      }
      const registered = await client.registerDevice({
        deviceRole: 'host',
        crossPlatform: runtime.capabilities.crossPlatformHost === true,
        id: deviceId,
        name: 'Sanser Host',
        platform: runtime.platform,
        osVersion: runtime.platform,
        gpu: 'Not reported by desktop shell',
        version: SANSER_VERSION,
        protocolVersion: PROTOCOL_VERSION,
        codecs: runtime.capabilities.hostCodecs ?? ['h264', 'hevc'],
        nativeTransport: runtime.capabilities.nativeDirect.state === 'available',
        webRtc: runtime.capabilities.webRtc.state === 'available',
        audio: runtime.capabilities.hostAudio ?? false,
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
          .then(() => {
            if (generation !== lifecycleGeneration) return;
            store.update((state) => ({ ...state, error: state.error === 'Host heartbeat failed; retrying' ? null : state.error }));
          })
          .catch(() => {
            if (generation !== lifecycleGeneration) return;
            store.update((state) => ({ ...state, error: state.error ?? 'Host heartbeat failed; retrying' }));
          });
      }, HEARTBEAT_INTERVAL_MS);
      await refreshSessions();
      if (generation !== lifecycleGeneration) return;
      sessionTimer = window.setInterval(() => void refreshSessions(), 2_000);
      diagnostics.add({ level: 'info', category: 'engine', message: 'Host is online', details: { deviceId } });
    } catch (error) {
      if (generation !== lifecycleGeneration) return;
      runtimeForHost = null;
      const message = error instanceof Error ? error.message : 'Unable to bring host online';
      store.set({ ...initial, error: message, errorCode: error instanceof ApiError ? error.code : null });
      diagnostics.add({ level: 'warn', category: 'engine', message });
      throw error;
    }
  }

  return {
    subscribe: store.subscribe,
    online(runtime: RuntimeStatus): Promise<void> {
      if (registrationTask) return registrationTask;
      if (get(store).online || get(store).busy || launchTask) return Promise.resolve();
      registrationTask = registerHost(runtime).finally(() => { registrationTask = null; });
      return registrationTask;
    },
    async offline(): Promise<void> {
      lifecycleGeneration += 1;
      const generation = lifecycleGeneration;
      const current = get(store);
      const client = session.client();
      stopTimers();
      runtimeForHost = null;
      retryCounts.clear();
      retryAfter.clear();
      readyGenerations.clear();
      autoAcceptRetryAfter.clear();
      pollError = null;
      store.update((current) => ({ ...current, online: false, busy: true }));
      negotiationController?.abort();
      let nativeStopped = false;
      try {
        await registrationTask?.catch(() => undefined);
        await launchTask;
        await stopEngine('host');
        nativeStopped = true;
        await stopRelay('host').catch(() => undefined);
        if (client && current.deviceId && current.online) {
          await client.offlineDevice(current.deviceId);
        }
      } catch (error) {
        const message = error instanceof Error ? error.message : 'Unable to mark this host offline';
        diagnostics.add({
          level: 'warn',
          category: 'engine',
          message
        });
        if (!nativeStopped && generation === lifecycleGeneration) {
          const failure = new Error(`Unable to stop screen sharing. Try going offline again. ${message}`);
          store.set({ ...current, busy: false, launchingSessionId: null, error: failure.message });
          throw failure;
        }
      } finally {
        if (nativeStopped && generation === lifecycleGeneration) store.set(initial);
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
      const generation = lifecycleGeneration;
      if (!get(store).online || get(store).busy || get(store).actionSessionId) throw new Error('This host is busy or offline');
      store.update((state) => ({ ...state, actionSessionId: sessionId, error: null }));
      try {
        await client.rejectSession(sessionId);
        if (generation !== lifecycleGeneration) return;
        retryCounts.delete(sessionId);
        retryAfter.delete(sessionId);
        readyGenerations.delete(sessionId);
        store.update((state) => ({
          ...state,
          sessions: state.sessions.filter((item) => item.id !== sessionId),
          actionSessionId: null
        }));
      } catch (error) {
        if (generation === lifecycleGeneration) store.update((state) => ({ ...state, actionSessionId: null }));
        throw error;
      }
    },
    async disconnect(sessionId: string): Promise<void> {
      const client = session.client();
      if (!client) throw new Error('Cloud sign-in is required to disconnect a session');
      const generation = lifecycleGeneration;
      if (!get(store).online || get(store).busy || get(store).actionSessionId) throw new Error('This host is busy or offline');
      store.update((state) => ({ ...state, actionSessionId: sessionId, error: null }));
      negotiationController?.abort();
      try {
        await launchTask;
        if (generation !== lifecycleGeneration) return;
        await stopEngine('host');
        await stopRelay('host').catch(() => undefined);
        if (generation !== lifecycleGeneration) return;
        store.update((state) => ({ ...state, engineRunning: false }));
        await client.disconnectSession(sessionId);
        if (generation !== lifecycleGeneration) return;
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
        if (generation === lifecycleGeneration) {
          retryCounts.set(sessionId, 3);
          store.update((state) => ({ ...state, actionSessionId: null, failedP2pSessionId: sessionId }));
        }
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
