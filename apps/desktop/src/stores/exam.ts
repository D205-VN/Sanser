import { get, writable } from 'svelte/store';
import { examLocked, guardApps, type ExamHeartbeat, type ExamPolicy } from '../lib/exam';
import { examCaptureScreen, examGuardApply, examGuardReport, examGuardStop } from '../lib/platform';
import { session } from './session';

/** The student seat survives a restart, so quitting is no way out of an exam. */
const SEAT_KEY = 'sanser.exam.seat.v1';
/** Longest exam the server allows, plus margin: past it the seat is let go even offline. */
const OFFLINE_RELEASE_MS = 9 * 60 * 60 * 1000;
const MAX_PENDING = 50;

export interface ExamSeatState {
  policy: ExamPolicy | null;
  busy: boolean;
  error: string | null;
  /** Unix ms of the last heartbeat the server answered. */
  lastSyncAt: number | null;
  /** Set when the exam ended or the supervisor removed this computer. */
  finished: 'ended' | 'removed' | null;
}

interface Seat { memberId: string; lockedAt: number }

function readSeat(): Seat | null {
  try {
    const value = JSON.parse(localStorage.getItem(SEAT_KEY) ?? 'null') as Partial<Seat> | null;
    return value && typeof value.memberId === 'string' ? { memberId: value.memberId, lockedAt: Number(value.lockedAt) || Date.now() } : null;
  } catch { return null; }
}
function writeSeat(seat: Seat | null): void {
  try {
    if (seat) localStorage.setItem(SEAT_KEY, JSON.stringify(seat));
    else localStorage.removeItem(SEAT_KEY);
  } catch { /* storage blocked: a restart then ends the lock, which the supervisor sees */ }
}

function message(error: unknown, fallback: string): string {
  return error instanceof Error ? error.message : fallback;
}

export function createExamSeat() {
  const store = writable<ExamSeatState>({ policy: null, busy: false, error: null, lastSyncAt: null, finished: null });
  let timer: number | null = null;
  let pending: ExamHeartbeat['violations'] = [];
  let lastShotAt = 0;
  let running = false;

  function stopTimer(): void {
    if (timer !== null) window.clearTimeout(timer);
    timer = null;
  }

  async function release(finished: ExamSeatState['finished'], policy: ExamPolicy | null): Promise<void> {
    stopTimer();
    running = false;
    pending = [];
    writeSeat(null);
    await examGuardStop().catch(() => undefined);
    store.update((state) => ({ ...state, policy, finished, busy: false }));
  }

  async function enforce(policy: ExamPolicy): Promise<void> {
    if (!examLocked(policy)) {
      await release(policy.memberState === 'removed' ? 'removed' : 'ended', policy);
      return;
    }
    await examGuardApply(guardApps(policy), policy.blockAi, policy.memberState === 'paused');
    store.update((state) => ({ ...state, policy, finished: null }));
  }

  async function tick(): Promise<void> {
    const seat = readSeat();
    const client = session.client();
    const current = get(store).policy;
    if (!seat || !current) { running = false; return; }
    let nextSeconds = current.heartbeatSeconds;
    try {
      const report = await examGuardReport();
      pending = [...pending, ...report.violations].slice(-MAX_PENDING);
      if (!report.running) await examGuardApply(guardApps(current), current.blockAi, current.memberState === 'paused').catch(() => undefined);
      let screenshot: string | undefined;
      if (Date.now() - lastShotAt >= current.screenSeconds * 1000) {
        screenshot = await examCaptureScreen(current.screenSeconds < 10).catch(() => undefined);
        lastShotAt = Date.now();
      }
      if (!client) throw new Error('Sign in to keep reporting to the supervisor');
      const sending = pending;
      const policy = await client.examHeartbeat(seat.memberId, { foregroundApp: report.foregroundApp ?? undefined, violations: sending, screenshot });
      pending = pending.slice(sending.length);
      store.update((state) => ({ ...state, lastSyncAt: Date.now(), error: null }));
      await enforce(policy);
      nextSeconds = policy.heartbeatSeconds;
    } catch (error) {
      // Offline: stay locked and keep the violations for the next heartbeat.
      store.update((state) => ({ ...state, error: message(error, 'The supervisor could not be reached') }));
      if (Date.now() - seat.lockedAt > OFFLINE_RELEASE_MS) { await release('ended', current); return; }
    }
    if (running) timer = window.setTimeout(() => void tick(), Math.max(2, nextSeconds) * 1000);
  }

  function startLoop(): void {
    stopTimer();
    running = true;
    lastShotAt = 0;
    timer = window.setTimeout(() => void tick(), 500);
  }

  return {
    subscribe: store.subscribe,

    async join(code: string, displayName: string, deviceId: string): Promise<void> {
      const client = session.client();
      if (!client) throw new Error('Sign in is required');
      store.update((state) => ({ ...state, busy: true, error: null, finished: null }));
      try {
        const policy = await client.joinExam(code, deviceId, displayName);
        if (!examLocked(policy)) { await release('ended', policy); return; }
        writeSeat({ memberId: policy.memberId, lockedAt: Date.now() });
        await enforce(policy);
        store.update((state) => ({ ...state, busy: false, lastSyncAt: Date.now() }));
        startLoop();
      } catch (error) {
        store.update((state) => ({ ...state, busy: false, error: message(error, 'Unable to join the exam') }));
        throw error;
      }
    },

    /** After a restart: lock again at once, then confirm with the server. */
    async resume(): Promise<void> {
      const seat = readSeat();
      if (!seat || running) return;
      const client = session.client();
      try {
        if (!client) throw new Error('Sign in to continue the exam');
        const policy = await client.examPolicy(seat.memberId);
        if (!examLocked(policy)) { await release(null, null); return; }
        await enforce(policy);
        store.update((state) => ({ ...state, lastSyncAt: Date.now(), error: null }));
      } catch (error) {
        const status = (error as { status?: number }).status;
        if (status === 404 || status === 403) { await release(null, null); return; }
        // Unknown state while offline: keep the strictest lock until it answers.
        const strict: ExamPolicy = { memberId: seat.memberId, roomId: '', title: '', supervisorName: '', roomState: 'open', memberState: 'paused', allowedApps: [], blockAi: true, heartbeatSeconds: 5, screenSeconds: 10 };
        await examGuardApply([], true, false).catch(() => undefined);
        store.update((state) => ({ ...state, policy: state.policy ?? strict, error: message(error, 'The supervisor could not be reached') }));
      }
      startLoop();
    },

    dismiss(): void {
      store.update((state) => ({ ...state, finished: null, policy: null, error: null }));
    },

    /** Seat present: Sanser must not sign out or forget this computer. */
    active(): boolean {
      return readSeat() !== null;
    }
  };
}

export const examSeat = createExamSeat();
