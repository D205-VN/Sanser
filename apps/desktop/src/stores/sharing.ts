import { get, writable } from 'svelte/store';
import { coordinateP2pConnection, directCheckFailure } from '../lib/p2pSignaling';
import { addSharingPeer, engineStatus, launchEngine, relayPeerFailure, removeSharingPeer, startRelay, stopEngine, stopRelay } from '../lib/platform';
import { resolveStreamProfile, resolveStreamSize } from '../lib/streamProfile';
import type { Device, LaunchEngineRequest, RuntimeStatus, SharingGroup, SharingMember } from '../lib/types';
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
}
const initial: SharingState = { groups: [], devices: [], ownGroup: null, busy: false, supported: false, paused: false, error: null, peerErrors: {} };
const message = (error: unknown) => error instanceof Error ? error.message : 'Unable to update screen sharing';

export function createSharingStore() {
  const store = writable<SharingState>({ ...initial });
  let accountId: string | null = null;
  let runtime: RuntimeStatus | null = null;
  let generation = 0;
  let timer: number | undefined;
  let polling = false;
  let lastHeartbeat = 0;
  let nativeStarted = false;
  const hasNative = (): boolean => nativeStarted;
  let ownedGroupId: string | null = null;
  let stoppedGroupId: string | null = null;
  let nativeQueue: Promise<void> = Promise.resolve();
  const peers = new Map<string, { sessionId: string; readyAt: string; relay: boolean }>();
  const tasks = new Map<string, { controller: AbortController; promise: Promise<void> }>();
  const retryAfter = new Map<string, number>();

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
    peers.clear(); retryAfter.clear();
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
    let installed = false;
    try {
      await removeNative(group.id, member.id);
      const credentials = await api.sessionCredentials(sessionId, hostId);
      if (credentials.accessMode !== 'view-only' || credentials.wireProtocol !== 'snv2') throw new Error('Sharing requires updated view-only session credentials');
      if (cancelled()) return;
      let direct: Awaited<ReturnType<typeof coordinateP2pConnection>> | null = null;
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
        sessionToken: credentials.sessionToken, audioEnabled: false, inputEnabled: false, relativeMouse: false
      };
      // Negotiations are independent; creation of the one encoder is serialized.
      const operation = nativeQueue.then(async () => {
        if (cancelled()) return false;
        if (nativeStarted) await addSharingPeer(request); else { await launchEngine(request); nativeStarted = true; }
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
        retryAfter.set(member.id, Date.now() + 10_000);
        diagnostics.add({ level: 'warn', category: 'session', message: 'A sharing viewer could not connect; other viewers remain active' });
      }
    } finally {
      if (relay && !installed) await stopRelay('host', sessionId).catch(() => undefined);
    }
  }

  async function poll(): Promise<void> {
    const api = session.client(); const epoch = generation;
    if (!api || polling || !get(store).supported || accountId !== get(session).account?.id) return;
    polling = true;
    try {
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
      host.setSharingActive(true, own.members.filter((member) => member.state === 'accepted' && member.startedAt !== undefined).length);
      if (Date.now() - lastHeartbeat > 5000 && hostId) { await api.sharingHeartbeat(own.id, hostId); lastHeartbeat = Date.now(); }
      if (nativeStarted && !(await engineStatus('host')).running) {
        for (const member of own.members.filter((member) => member.state === 'accepted')) errorForPeer(member.id, 'Sharing host stopped. Stop sharing and start again.');
        await api.stopSharingGroup(own.id);
        await haltNative(own.id);
        return;
      }
      for (const [memberId, peer] of peers) {
        const member = own.members.find((item) => item.id === memberId);
        if (!member || member.state !== 'accepted') await removeNative(own.id, memberId);
        else if (peer.relay) {
          const failure = await relayPeerFailure('host', peer.sessionId);
          if (failure) { await removeNative(own.id, memberId); errorForPeer(memberId, failure); retryAfter.set(memberId, Number.POSITIVE_INFINITY); }
        }
      }
      const sessions = await api.hostSessions(own.hostDeviceId);
      if (epoch !== generation) return;
      for (const member of own.members) {
        const current = sessions.items.find((item) => item.id === member.sessionId && item.status === 'accepted');
        if (member.state !== 'accepted' || !current?.requesterReadyAt || tasks.has(member.id)) continue;
        if (peers.get(member.id)?.readyAt === current.requesterReadyAt) continue;
        if (Date.now() < (retryAfter.get(member.id) ?? 0)) continue;
        const controller = new AbortController();
        const promise = attach(own, member, current.requesterReadyAt, controller.signal, epoch).finally(() => tasks.delete(member.id));
        tasks.set(member.id, { controller, promise });
      }
    } catch (error) { if (epoch === generation) store.update((state) => ({ ...state, error: message(error) })); }
    finally { polling = false; }
  }

  async function action<T>(operation: () => Promise<T>): Promise<T> {
    if (get(store).busy) throw new Error('Another sharing action is in progress');
    store.update((state) => ({ ...state, busy: true, error: null }));
    try { return await operation(); }
    catch (error) { store.update((state) => ({ ...state, error: message(error) })); throw error; }
    finally { store.update((state) => ({ ...state, busy: false })); }
  }

  return {
    subscribe: store.subscribe,
    async initialize(value: RuntimeStatus): Promise<void> {
      const account = get(session).account?.id;
      if (!account || (accountId === account && get(store).supported)) return;
      generation += 1; const epoch = generation; accountId = account; runtime = value;
      const api = session.client(); if (!api) return;
      try {
        const supported = await api.supportsSharingMedia();
        if (epoch !== generation) return;
        store.update((state) => ({ ...state, supported, error: supported ? null : 'Update the Sanser server to enable screen sharing invitations.' }));
        if (supported) { await poll(); timer = window.setInterval(() => void poll(), 2000); }
      } catch (error) { if (epoch === generation) store.update((state) => ({ ...state, error: message(error) })); }
    },
    refresh: poll,
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
    async shutdown(): Promise<void> {
      generation += 1; if (timer !== undefined) window.clearInterval(timer); timer = undefined;
      const own = get(store).ownGroup; const api = session.client();
      await haltNative(own?.id);
      if (own && api) await api.stopSharingGroup(own.id).catch(() => undefined);
      accountId = null; runtime = null; ownedGroupId = null; lastHeartbeat = 0; store.set({ ...initial });
    }
  };
}
export const sharing = createSharingStore();
