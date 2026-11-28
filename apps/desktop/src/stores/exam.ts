import { get, writable } from 'svelte/store';
import { examLocked, guardApps, normalizeUnlockCode, verifyUnlockCode, type ExamHeartbeat, type ExamPolicy, type ExamScreenPace, type ExamUnlockCheck } from '../lib/exam';
import { examCaptureScreen, examGuardApply, examGuardReport, examGuardStop } from '../lib/platform';
import { screenChanged, screenSignature } from '../lib/screenSignature';
import { session } from './session';

/** The student seat survives a restart, so quitting is no way out of an exam. */
const SEAT_KEY = 'sanser.exam.seat.v1';
/** Longest exam the server allows, plus margin: past it the seat is let go even offline. */
const OFFLINE_RELEASE_MS = 9 * 60 * 60 * 1000;
const MAX_PENDING = 50;
/** An unchanged screen is still sent this often, so the grid shows the seat is alive. */
const UNCHANGED_SCREEN_MS = 10_000;
/** Never faster than this, whatever the server says. */
const MIN_SCREEN_MS = 200;
/** Breathing room between two captures, even when a capture took longer than the pace. */
const MIN_CAPTURE_GAP_MS = 50;

export interface ExamSeatState {
  policy: ExamPolicy | null;
  busy: boolean;
  error: string | null;
  /** Unix ms of the last heartbeat the server answered. */
  lastSyncAt: number | null;
  /** Set when the exam ended, the supervisor removed this computer or the emergency code was used. */
  finished: 'ended' | 'removed' | 'unlocked' | null;
}

/** `unlock` is kept so the emergency code still works after a restart offline. */
interface Seat { memberId: string; lockedAt: number; unlock?: ExamUnlockCheck }

function readSeat(): Seat | null {
  try {
    const value = JSON.parse(localStorage.getItem(SEAT_KEY) ?? 'null') as Partial<Seat> | null;
    if (!value || typeof value.memberId !== 'string') return null;
    const unlock = value.unlock && typeof value.unlock.salt === 'string' && typeof value.unlock.hash === 'string' && Number.isInteger(value.unlock.iterations)
      ? value.unlock : undefined;
    return { memberId: value.memberId, lockedAt: Number(value.lockedAt) || Date.now(), unlock };
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
  let screenTimer: number | null = null;
  let lastSignature: Uint8Array | null = null;
  let lastScreenAt = 0;
  let lastScreenLarge = false;
  /** Bumped on stop, so a capture still in flight does not start a second loop. */
  let screenLoop = 0;
  let screenBusy = false;
  /** The newest pace: from the last heartbeat, or sooner from the last picture's answer. */
  let pace: ExamScreenPace | null = null;

  function stopScreens(): void {
    if (screenTimer !== null) window.clearTimeout(screenTimer);
    screenTimer = null;
    screenLoop += 1;
    lastSignature = null;
    lastScreenAt = 0;
    pace = null;
  }

  function stopTimer(): void {
    if (timer !== null) window.clearTimeout(timer);
    timer = null;
    stopScreens();
  }

  /** Newer servers take screens between heartbeats, every second when they change. */
  function liveScreens(policy: ExamPolicy | null): policy is ExamPolicy & { liveScreenSeconds: number } {
    return typeof policy?.liveScreenSeconds === 'number' && policy.liveScreenSeconds > 0;
  }

  /** Time between pictures: about 250 ms while watched, one second otherwise. */
  function policyPace(policy: ExamPolicy & { liveScreenSeconds: number }): ExamScreenPace {
    const ms = typeof policy.liveScreenMs === 'number' && policy.liveScreenMs > 0 ? policy.liveScreenMs : policy.liveScreenSeconds * 1000;
    return { liveScreenMs: ms, screenLarge: policy.screenLarge === true };
  }

  async function screenTick(loop: number): Promise<void> {
    screenTimer = null;
    const current = get(store).policy;
    const seat = readSeat();
    if (loop !== screenLoop || !running || !seat || !liveScreens(current)) return;
    screenBusy = true;
    const startedAt = Date.now();
    try {
      const large = (pace ?? policyPace(current)).screenLarge;
      const jpeg = await examCaptureScreen(large);
      const signature = await screenSignature(jpeg);
      const due = Date.now() - lastScreenAt >= UNCHANGED_SCREEN_MS || large !== lastScreenLarge;
      if (due || screenChanged(lastSignature, signature)) {
        const client = session.client();
        if (client && loop === screenLoop) {
          const answer = await client.examScreen(seat.memberId, jpeg);
          if (loop === screenLoop && typeof answer.liveScreenMs === 'number') pace = answer;
          lastSignature = signature;
          lastScreenAt = Date.now();
          lastScreenLarge = large;
        }
      }
    } catch {
      // No picture this second, or no heartbeat yet (409): the next second tries again.
    } finally {
      screenBusy = false;
    }
    const next = get(store).policy;
    // A stop in the meantime bumped the loop number; a start waited for this capture.
    if (loop === screenLoop && liveScreens(next)) {
      // The capture itself counts: a 400 ms capture leaves 600 ms of a second.
      const every = Math.max(MIN_SCREEN_MS, (pace ?? policyPace(next)).liveScreenMs);
      screenTimer = window.setTimeout(() => void screenTick(loop), Math.max(MIN_CAPTURE_GAP_MS, every - (Date.now() - startedAt)));
    }
  }

  function startScreens(): void {
    if (screenTimer !== null || screenBusy || !running) return;
    const loop = screenLoop;
    screenTimer = window.setTimeout(() => void screenTick(loop), 0);
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
    const seat = readSeat();
    if (seat && policy.unlock && seat.unlock?.hash !== policy.unlock.hash) writeSeat({ ...seat, unlock: policy.unlock });
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
      if (!liveScreens(current) && Date.now() - lastShotAt >= current.screenSeconds * 1000) {
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
      if (liveScreens(policy)) { pace = policyPace(policy); startScreens(); }
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
        writeSeat({ memberId: policy.memberId, lockedAt: Date.now(), unlock: policy.unlock });
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

    /**
     * The supervisor read out the emergency code. The server checks it and
     * records it; when the server is out of reach the stored hash decides.
     * Resolves false for a wrong code.
     */
    async unlock(code: string): Promise<boolean> {
      const seat = readSeat();
      const normalized = normalizeUnlockCode(code);
      if (!seat || !normalized) return false;
      const client = session.client();
      try {
        if (!client) throw new Error('offline');
        const policy = await client.unlockExam(seat.memberId, normalized);
        if (examLocked(policy)) return false;
        await release('unlocked', policy);
        return true;
      } catch (error) {
        // Only the server's "not right" is final; anything else may be the network.
        if ((error as { status?: number }).status === 422) return false;
        const check = seat.unlock ?? get(store).policy?.unlock;
        if (!check || !(await verifyUnlockCode(normalized, check))) return false;
        await release('unlocked', get(store).policy);
        return true;
      }
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
