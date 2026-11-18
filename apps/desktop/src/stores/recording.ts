import { get, writable } from 'svelte/store';
import type { ApiClient } from '../lib/api';
import {
  IDLE_RECORDING, hostDecision, recordingStatus, startRecording, stopRecording
} from '../lib/recording';
import type { RecordingConsent, RecordingStatus } from '../lib/types';
import { diagnostics } from './diagnostics';
import { presence } from './presence';
import { session } from './session';

const CONSENT_TIMEOUT_MS = 10_000;
const POLL_MS = 1_000;
const HEARTBEAT_MS = 10_000;

export interface ViewerRecordingState {
  sessionId: string | null;
  /** asking = waiting for the host's answer */
  phase: 'idle' | 'asking' | 'starting' | 'recording' | 'stopping';
  status: RecordingStatus;
  /** English message key for the last outcome (translated by the page). */
  message: string | null;
  error: boolean;
}

interface Dependencies {
  client: () => ApiClient | null;
  deviceId: () => string | null;
  now?: () => number;
  sleep?: (ms: number) => Promise<void>;
  native?: {
    status: typeof recordingStatus;
    start: typeof startRecording;
    stop: typeof stopRecording;
  };
}

const initial: ViewerRecordingState = { sessionId: null, phase: 'idle', status: IDLE_RECORDING, message: null, error: false };

/**
 * Viewer flow: ask the host through the server, start the native recorder only
 * after an explicit "allowed", heartbeat while recording, and stop when the
 * host revokes consent or the native recorder stops on its own (limits).
 */
export function createViewerRecording(deps: Dependencies) {
  const store = writable<ViewerRecordingState>({ ...initial });
  const native = deps.native ?? { status: recordingStatus, start: startRecording, stop: stopRecording };
  const now = deps.now ?? (() => Date.now());
  const sleep = deps.sleep ?? ((ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms)));
  let generation = 0;

  async function consent(sessionId: string, action: 'request' | 'stop'): Promise<RecordingConsent | null> {
    const client = deps.client(); const deviceId = deps.deviceId();
    if (!client || !deviceId) return null;
    return client.updateRecordingConsent(sessionId, deviceId, action);
  }

  async function finish(sessionId: string, message: string | null, error = false): Promise<void> {
    let status = get(store).status;
    try { status = await native.stop(sessionId); } catch { /* viewer may already be closed */ }
    try { await consent(sessionId, 'stop'); } catch { /* the host indicator expires on its own */ }
    store.set({ sessionId, phase: 'idle', status, message, error });
  }

  async function watch(sessionId: string, run: number): Promise<void> {
    let lastBeat = now();
    while (run === generation) {
      await sleep(POLL_MS);
      if (run !== generation) return;
      let status: RecordingStatus;
      try { status = await native.status(sessionId); }
      catch { store.update((state) => ({ ...state, phase: 'idle', message: 'Recording stopped because the session ended' })); return; }
      store.update((state) => ({ ...state, status, phase: status.state === 'recording' ? 'recording' : state.phase }));
      if (status.state === 'stopped' || status.state === 'error') {
        generation += 1;
        await finish(sessionId, status.state === 'error' ? status.reason ?? 'write-error' : status.reason, status.state === 'error');
        return;
      }
      if (now() - lastBeat >= HEARTBEAT_MS) {
        lastBeat = now();
        try {
          const answer = await consent(sessionId, 'request');
          if (answer?.state === 'denied') {
            generation += 1;
            await finish(sessionId, 'host-denied', true);
            return;
          }
        } catch { /* transient network errors keep recording; the server TTL covers longer outages */ }
      }
    }
  }

  return {
    subscribe: store.subscribe,
    async start(sessionId: string): Promise<void> {
      const current = get(store);
      if (current.phase !== 'idle') return;
      const run = ++generation;
      store.set({ ...initial, sessionId, phase: 'asking' });
      try {
        let answer = await consent(sessionId, 'request');
        if (!answer) throw new Error('Sign in is required to record');
        const deadline = now() + CONSENT_TIMEOUT_MS;
        while (answer.state === 'requested' && now() < deadline && run === generation) {
          await sleep(POLL_MS);
          answer = await deps.client()?.recordingConsent(sessionId) ?? answer;
        }
        if (run !== generation) return;
        if (answer.state !== 'allowed') {
          generation += 1;
          await consent(sessionId, 'stop').catch(() => undefined);
          store.set({ ...initial, sessionId, message: answer.state === 'denied' ? 'host-denied' : 'host-timeout', error: true });
          return;
        }
        store.update((state) => ({ ...state, phase: 'starting' }));
        const status = await native.start(sessionId);
        store.update((state) => ({ ...state, status }));
        diagnostics.add({ level: 'info', category: 'session', message: 'Viewer recording started' });
        void watch(sessionId, run);
      } catch (error) {
        if (run !== generation) return;
        generation += 1;
        await consent(sessionId, 'stop').catch(() => undefined);
        store.set({ ...initial, sessionId, message: error instanceof Error ? error.message : 'Unable to start recording', error: true });
      }
    },
    async stop(): Promise<void> {
      const { sessionId, phase } = get(store);
      if (!sessionId || phase === 'idle') return;
      generation += 1;
      store.update((state) => ({ ...state, phase: 'stopping' }));
      await finish(sessionId, 'user');
      diagnostics.add({ level: 'info', category: 'session', message: 'Viewer recording stopped' });
    },
    /** Session closed: forget local state (the engine already finalized the file). */
    reset(): void {
      generation += 1;
      store.set({ ...initial });
    },
    dismiss(): void {
      store.update((state) => ({ ...state, message: null, error: false }));
    }
  };
}

export interface HostRecordingNotice {
  sessionId: string;
  viewerName: string;
}

/**
 * Host side: answers requests from the Host setting and lists viewers that
 * are recording now so the page can show "X is recording".
 */
export async function pollHostRecordings(
  client: ApiClient,
  hostDeviceId: string,
  sessionIds: string[],
  allowRecording: boolean
): Promise<HostRecordingNotice[]> {
  const notices: HostRecordingNotice[] = [];
  for (const sessionId of sessionIds.slice(0, 8)) {
    let consent: RecordingConsent;
    try { consent = await client.recordingConsent(sessionId); } catch { continue; }
    const decision = hostDecision(allowRecording, consent.state);
    if (decision) {
      try { consent = await client.updateRecordingConsent(sessionId, hostDeviceId, decision); }
      catch { continue; }
      diagnostics.add({ level: 'info', category: 'session', message: decision === 'allow' ? 'Viewer recording allowed by Host setting' : 'Viewer recording refused by Host setting' });
    }
    if (consent.state === 'allowed') notices.push({ sessionId, viewerName: consent.viewerName ?? 'Viewer' });
  }
  return notices;
}

export async function revokeHostRecording(client: ApiClient, hostDeviceId: string, sessionId: string): Promise<void> {
  await client.updateRecordingConsent(sessionId, hostDeviceId, 'deny');
}

// App-wide viewer controller bound to the signed-in account and this client.
export const viewerRecording = createViewerRecording({
  client: () => session.client(),
  deviceId: () => get(presence).deviceId
});
