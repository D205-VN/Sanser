import { get, writable } from 'svelte/store';
import { coordinateP2pConnection, directCheckFailure, releaseP2pConnection } from '../lib/p2pSignaling';
import { addSharingPeer, engineStatus, launchEngine, relayPeerFailure, removeSharingPeer, sharingPeerStatus, startRelay, stopEngine, stopRelay } from '../lib/platform';
import { resolveStreamProfile, resolveStreamSize } from '../lib/streamProfile';
import type { Device, LaunchEngineRequest, RuntimeStatus, SharingGroup, SharingMember, SharingPeerVideoStatus } from '../lib/types';
import { connection } from './connection';
import { diagnostics } from './diagnostics';
import { host } from './host';
import { preferences } from './preferences';
import { presence } from './presence';
import { session } from './session';

export interface SharingState {
  groups: SharingGroup[];
  devices: Device[];
  ownGroup: SharingGroup | null;
  busy: boolean;
  supported: boolean;
  paused: boolean;
  error: string | null;
  peerErrors: Record<string, string>;
  peerVideo: Record<string, SharingPeerVideoStatus>;
}
const initial: SharingState = { groups: [], devices: [], ownGroup: null, busy: false, supported: false, paused: false, error: null, peerErrors: {}, peerVideo: {} };
const message = (error: unknown) => error instanceof Error ? error.message : 'Unable to update screen sharing';

export function createSharingStore() {
  const store = writable<SharingState>({ ...initial });
  let accountId: string | null = null;
  let runtime: RuntimeStatus | null = null;
  let generation = 0;
  let timer: number | undefined;
  let pollingGeneration: number | null = null;
  let shutdownTask: Promise<void> | null = null;
  let initializationTask: Promise<void> | null = null;
  let supportRetryAt = 0;
  let lastHeartbeat = 0;
  let nativeStarted = false;
  let nativeAudioEnabled = false;
  const hasNative = (): boolean => nativeStarted;
  let ownedGroupId: string | null = null;
  let stoppedGroupId: string | null = null;
  let nativeQueue: Promise<void> = Promise.resolve();
  const peers = new Map<string, { sessionId: string; readyAt: string; relay: boolean }>();
  const tasks = new Map<string, { controller: AbortController; promise: Promise<void> }>();
  const retryAfter = new Map<string, { readyAt: string; at: number }>();

  function errorForPeer(id: string, error: string | null): void {
    store.update((state) => {
      const peerErrors = { ...state.peerErrors };
      if (error) peerErrors[id] = error;
      return { ...state, peerErrors: error ? peerErrors : Object.fromEntries(Object.entries(peerErrors).filter(([key]) => key !== id)) };
    });
  }

  async function removeNative(group: string, member: string): Promise<void> {
    const peer = peers.get(member);
    if (!peer) return;
    await removeSharingPeer(group, member);
    if (peer.relay) await stopRelay('host', peer.sessionId);
    peers.delete(member);
    store.update((state) => ({ ...state, peerVideo: Object.fromEntries(Object.entries(state.peerVideo).filter(([id]) => id !== member)) }));
  }

  async function haltNative(group?: string): Promise<void> {
    for (const task of tasks.values()) task.controller.abort();
    if (nativeStarted || group) { await stopEngine('host'); await stopRelay('host'); nativeStarted = false; }
    await Promise.allSettled([...tasks.values()].map((task) => task.promise));
    await nativeQueue;
    // This store only stops a host process that it launched.
    // A queued launch can finish while its cancellation is being awaited.
    if (hasNative()) { await stopEngine('host'); await stopRelay('host'); }
    nativeStarted = false;
    nativeAudioEnabled = false;
    peers.clear(); retryAfter.clear();
    store.update((state) => ({ ...state, peerVideo: {} }));
    host.setSharingActive(false);
    if (group) store.update((state) => ({ ...state, ownGroup: null }));
  }

  async function attach(group: SharingGroup, member: SharingMember, readyAt: string, signal: AbortSignal, epoch: number): Promise<void> {
    const api = session.client();
    const hostId = get(host).deviceId;
    if (!api || !hostId || !member.sessionId || !runtime) return;
    const sessionId = member.sessionId;
    const cancelled = () => signal.aborted || epoch !== generation || !get(host).online || get(store).ownGroup?.id !== group.id;
    let relay: Awaited<ReturnType<typeof startRelay>> | null = null;
    let direct: Awaited<ReturnType<typeof coordinateP2pConnection>> | null = null;
    let installed = false;
    try {
      await removeNative(group.id, member.id);
      const credentials = await api.sessionCredentials(sessionId, hostId);
      if (credentials.accessMode !== 'view-only' || credentials.wireProtocol !== 'snv2') throw new Error('Sharing requires updated view-only session credentials');
      if (cancelled()) return;
      let directCheck: LaunchEngineRequest['directCheck'] = 'passed';
      try {
        if (group.networkMode === 'relay') throw new Error('Relay selected');
        direct = await coordinateP2pConnection(api, sessionId, hostId, member.viewerDeviceId, true, credentials.sessionToken, undefined, signal, readyAt);
      } catch (error) {
        if (cancelled()) return;
        if (group.networkMode === 'direct') throw error;
        directCheck = directCheckFailure(error, group.networkMode === 'relay');
        const accessToken = await api.getAccessToken();
        if (!accessToken || cancelled()) return;
        relay = await startRelay({ kind: 'host', sharingId: group.id, serverUrl: api.serverUrl, sessionId, deviceId: hostId, peerDeviceId: member.viewerDeviceId, accessToken, sessionCredential: credentials.sessionToken });
      }
      if (cancelled()) return;
      const stream = resolveStreamProfile(get(preferences).stream, group.qualityProfile);
      const size = resolveStreamSize(stream.resolution, 'snv2');
      const request: LaunchEngineRequest = {
        kind: 'host', sharingId: group.id, sharingViewerId: member.id, sessionId,
        wireProtocol: 'snv2', codec: 'h264', networkMode: group.networkMode,
        ultraLowLatency: stream.profile === 'competitive', directCheck,
        fps: stream.fps, bitrateKbps: Math.round(stream.bitrateMbps * 1000), width: size.width, height: size.height,
        address: relay ? '127.0.0.1' : direct?.remoteAddress, port: relay?.proxyPort ?? direct?.remotePort,
        udpBindPort: relay?.enginePort ?? direct?.localPort, relay: relay !== null,
        sessionToken: credentials.sessionToken,
        audioEnabled: runtime.capabilities.hostAudio === true && get(preferences).host.audioEnabled,
        inputEnabled: false, relativeMouse: false
      };
      // Negotiations are independent; creation of the one encoder is serialized.
      const operation = nativeQueue.then(async () => {
        if (cancelled()) return false;
        if (nativeStarted) { request.audioEnabled = nativeAudioEnabled; await addSharingPeer(request); }
        else { await launchEngine(request); nativeAudioEnabled = request.audioEnabled === true; nativeStarted = true; }
        peers.set(member.id, { sessionId, readyAt, relay: relay !== null });
        return true;
      });
      nativeQueue = operation.then(() => undefined, () => undefined);
      installed = await operation;
      if (cancelled() && installed) await removeNative(group.id, member.id);
      else if (installed) { errorForPeer(member.id, null); retryAfter.delete(member.id); }
    } catch (error) {
      if (!cancelled()) {
        errorForPeer(member.id, message(error));
        retryAfter.set(member.id, { readyAt, at: Date.now() + 10_000 });
        diagnostics.add({ level: 'warn', category: 'session', message: 'A sharing viewer could not connect; other viewers remain active' });
      }
    } finally {
      if (relay && !installed) await stopRelay('host', sessionId).catch(() => undefined);
      if (direct && !installed) await releaseP2pConnection(direct).catch(() => undefined);
    }
  }

  async function poll(): Promise<void> {
    const api = session.client(); const epoch = generation;
    if (!api || pollingGeneration === epoch || accountId !== get(session).account?.id) return;
    pollingGeneration = epoch;
    try {
      if (!get(store).supported) {
        if (Date.now() < supportRetryAt) return;
        supportRetryAt = Date.now() + 10_000;
        try {
          const supported = await api.supportsSharingMedia();
          if (epoch !== generation) return;
          store.update((state) => ({ ...state, supported, error: supported ? null : 'Update the Sanser server to receive screen sharing invitations.' }));
          if (!supported) return;
        } catch (error) {
          if (epoch === generation) store.update((state) => ({ ...state, error: message(error) }));
          return;
        }
      }
      const [groups, devices] = await Promise.all([api.sharingGroups(), api.devices()]);
      if (epoch !== generation) return;
      const hostId = get(host).deviceId;
      const own = groups.items.find((group) => group.hostDeviceId === hostId) ?? null;
      const previous = get(store).ownGroup;
      store.update((state) => ({ ...state, groups: groups.items, devices: devices.items, ownGroup: own, error: null }));
      if (previous && previous.id !== own?.id) await haltNative(previous.id);
      if (epoch !== generation || !own) return;
      if (own.id !== ownedGroupId || stoppedGroupId === own.id) {
        store.update((state) => ({ ...state, paused: true }));
        host.setSharingActive(true,0);
        return;
      }
      store.update((state) => ({ ...state, paused: false }));
      if (!get(host).online) { await api.stopSharingGroup(own.id); await haltNative(own.id); return; }
      if (Date.now() - lastHeartbeat > 5000 && hostId) { await api.sharingHeartbeat(own.id, hostId); lastHeartbeat = Date.now(); }
      if (nativeStarted && !(await engineStatus('host')).running) {
        for (const member of own.members.filter((member) => member.state === 'accepted')) errorForPeer(member.id, 'Sharing host stopped. Stop sharing and start again.');
        await api.stopSharingGroup(own.id);
        await haltNative(own.id);
        return;
      }
      const nativeStatuses = nativeStarted ? await sharingPeerStatus(own.id) : [];
      if (epoch !== generation || get(store).ownGroup?.id !== own.id || stoppedGroupId === own.id) return;
      const peerVideo = Object.fromEntries(nativeStatuses.filter((status) =>
        peers.has(status.viewerId) && own.members.some((member) => member.id === status.viewerId && member.state === 'accepted')
      ).map((status) => [status.viewerId, status]));
      store.update((state) => ({ ...state, peerVideo }));
      host.setSharingActive(true, own.members.filter((member) => member.state === 'accepted' && member.startedAt !== undefined &&
        (!peerVideo[member.id] || (peerVideo[member.id]?.videoState === 'active' && peerVideo[member.id]?.ready && peerVideo[member.id]?.running))).length);
      for (const [memberId, peer] of peers) {
        const member = own.members.find((item) => item.id === memberId);
        if (!member || member.state !== 'accepted') await removeNative(own.id, memberId);
        else if (peer.relay) {
          const failure = await relayPeerFailure('host', peer.sessionId);
          if (failure) { await removeNative(own.id, memberId); errorForPeer(memberId, failure); retryAfter.set(memberId, { readyAt: peer.readyAt, at: Number.POSITIVE_INFINITY }); }
        }
      }
      const sessions = await api.hostSessions(own.hostDeviceId);
      if (epoch !== generation) return;
      for (const member of own.members) {
        const current = sessions.items.find((item) => item.id === member.sessionId && item.status === 'accepted');
        if (member.state !== 'accepted' || !current?.requesterReadyAt || tasks.has(member.id)) continue;
        if (peers.get(member.id)?.readyAt === current.requesterReadyAt) continue;
        const retry = retryAfter.get(member.id);
        if (retry?.readyAt === current.requesterReadyAt && Date.now() < retry.at) continue;
        const controller = new AbortController();
        const promise = attach(own, member, current.requesterReadyAt, controller.signal, epoch).finally(() => tasks.delete(member.id));
        tasks.set(member.id, { controller, promise });
      }
    } catch (error) { if (epoch === generation) store.update((state) => ({ ...state, error: message(error) })); }
    finally { if (pollingGeneration === epoch) pollingGeneration = null; }
  }

  async function action<T>(operation: () => Promise<T>): Promise<T> {
    if (get(store).busy) throw new Error('Another sharing action is in progress');
    store.update((state) => ({ ...state, busy: true, error: null }));
    try { return await operation(); }
    catch (error) { store.update((state) => ({ ...state, error: message(error) })); throw error; }
    finally { store.update((state) => ({ ...state, busy: false })); }
  }

  function shutdown(): Promise<void> {
    if (shutdownTask) return shutdownTask;
    generation += 1;
    if (timer !== undefined) window.clearInterval(timer);
    timer = undefined;
    const own = get(store).ownGroup; const api = session.client();
    shutdownTask = (async () => {
      await haltNative(own?.id);
      if (own && api) await api.stopSharingGroup(own.id).catch(() => undefined);
      accountId = null; runtime = null; ownedGroupId = null; stoppedGroupId = null;
      lastHeartbeat = 0; supportRetryAt = 0; initializationTask = null; store.set({ ...initial });
    })().finally(() => { shutdownTask = null; });
    return shutdownTask;
  }

  return {
    subscribe: store.subscribe,
    async initialize(value: RuntimeStatus): Promise<void> {
      if (shutdownTask) await shutdownTask;
      if (accountId && accountId !== get(session).account?.id) await shutdown();
      const account = get(session).account?.id;
      if (!account) return;
      if (accountId === account) { if (initializationTask) await initializationTask; return; }
      generation += 1; const epoch = generation; accountId = account; runtime = value; supportRetryAt = 0;
      if (timer !== undefined) window.clearInterval(timer);
      // Keep polling after a transient startup failure. Otherwise a viewer can
      // miss every invitation until logout, with no way to recover from Refresh.
      timer = window.setInterval(() => void poll(), 2000);
      initializationTask = poll().finally(() => { if (epoch === generation) initializationTask = null; });
      await initializationTask;
    },
    async refresh(): Promise<void> { supportRetryAt = 0; await poll(); },
    async create(): Promise<void> {
      await action(async () => {
        const current = get(host); const api = session.client();
        if (!api || !current.online || !current.deviceId || runtime?.capabilities.multiViewerHost !== true || !get(store).supported) throw new Error('Go online with an updated native host and server before sharing');
        if (current.engineRunning || current.launchingSessionId || current.sessions.length) throw new Error('End the current remote-control session before starting a sharing group');
        host.setSharingActive(true);
        try {
          const settings = get(preferences).stream;
          const quality = settings.profile === 'custom' ? 'balanced' : settings.profile;
          const mode = get(preferences).networkMode;
          const group = await api.createSharingGroup(current.deviceId, 3, mode, quality);
          ownedGroupId = group.id; stoppedGroupId = null;
          store.update((state) => ({ ...state, paused: false, ownGroup: group, groups: [...state.groups, group] }));
        } catch (error) { host.setSharingActive(false); throw error; }
      });
    },
    async invite(viewer: string): Promise<void> {
      await action(async () => {
        const api = session.client(); const own = get(store).ownGroup;
        if (!api || !own) throw new Error('Start a sharing group first');
        const group = await api.inviteViewer(own.id, viewer);
        store.update((state) => ({ ...state, ownGroup: group }));
      });
    },
    async remove(member: SharingMember): Promise<void> {
      await action(async () => {
        const api = session.client(); const own = get(store).ownGroup;
        if (!api || !own) return;
        tasks.get(member.id)?.controller.abort(); await tasks.get(member.id)?.promise;
        // Disable the native sender before releasing the server reservation.
        await removeNative(own.id, member.id);
        const group = await api.removeSharingViewer(own.id, member.id);
        store.update((state) => ({ ...state, ownGroup: group })); errorForPeer(member.id, null);
      });
    },
    async stop(): Promise<void> {
      await action(async () => {
        const api = session.client(); const own = get(store).ownGroup;
        if (own) stoppedGroupId = own.id;
        const stopNative = haltNative(own?.id);
        const results = await Promise.allSettled([stopNative, api && own ? api.stopSharingGroup(own.id) : Promise.resolve()]);
        for (const result of results) if (result.status === 'rejected') throw result.reason;
        ownedGroupId = null;
        store.update((state) => ({ ...state, ownGroup: null, groups: state.groups.filter((group) => group.id !== own?.id) }));
      });
    },
    async respond(group: SharingGroup, member: SharingMember, accept: boolean): Promise<void> {
      await action(async () => {
        const api = session.client(); const viewer = get(presence).deviceId;
        if (!api || !viewer || viewer !== member.viewerDeviceId) throw new Error('This invitation belongs to another computer');
        if (accept && get(connection).session) throw new Error('Disconnect the current session before joining');
        const response = member.state === 'accepted' ? group : await api.respondSharingInvitation(group.id, member.id, viewer, accept);
        if (accept) {
          const invited = response.members.find((item) => item.id === member.id);
          if (!invited?.sessionId) throw new Error('Sharing session is unavailable; update the server');
          const target = await api.getSession(invited.sessionId);
          if (target.accessMode !== 'view-only' || target.sharingId !== group.id) throw new Error('Server returned incompatible sharing permissions');
          connection.begin(target);
        }
        await poll();
      });
    },
    shutdown
  };
}
export const sharing = createSharingStore();
