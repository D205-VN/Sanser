import { get } from 'svelte/store';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { ExamPolicy } from '../lib/exam';

const mocks = vi.hoisted(() => ({
  apply: vi.fn(),
  report: vi.fn(),
  stop: vi.fn(),
  capture: vi.fn(),
  client: { joinExam: vi.fn(), examHeartbeat: vi.fn(), examPolicy: vi.fn(), unlockExam: vi.fn() }
}));
vi.mock('../lib/platform', () => ({
  examGuardApply: mocks.apply,
  examGuardReport: mocks.report,
  examGuardStop: mocks.stop,
  examCaptureScreen: mocks.capture
}));
vi.mock('./session', () => ({ session: { client: () => mocks.client } }));

import { createExamSeat } from './exam';

const policy = (changes: Partial<ExamPolicy> = {}): ExamPolicy => ({
  memberId: 'member-1', roomId: 'room-1', title: 'Toán', supervisorName: 'Cô Lan', roomState: 'open', memberState: 'active',
  allowedApps: [{ id: 'WINWORD.EXE', name: 'Word' }], blockAi: true, heartbeatSeconds: 5, screenSeconds: 10, ...changes
});

beforeEach(() => {
  vi.useFakeTimers();
  localStorage.clear();
  vi.resetAllMocks();
  mocks.apply.mockResolvedValue(undefined);
  mocks.stop.mockResolvedValue(undefined);
  mocks.capture.mockResolvedValue('/9j/AAA=');
  mocks.report.mockResolvedValue({ running: true, foregroundApp: 'Word', violations: [{ kind: 'left-app', detail: 'Safari' }] });
});
afterEach(() => vi.useRealTimers());

it('locks on join, reports what the guard saw and follows a pause', async () => {
  const seat = createExamSeat();
  mocks.client.joinExam.mockResolvedValue(policy());
  await seat.join('ABC234', 'An', 'device-1');
  expect(mocks.apply).toHaveBeenLastCalledWith(['WINWORD.EXE'], true, false);
  expect(seat.active()).toBe(true);

  mocks.client.examHeartbeat.mockResolvedValue(policy({ memberState: 'paused' }));
  await vi.advanceTimersByTimeAsync(600);
  expect(mocks.client.examHeartbeat).toHaveBeenCalledWith('member-1', {
    foregroundApp: 'Word', violations: [{ kind: 'left-app', detail: 'Safari' }], screenshot: '/9j/AAA='
  });
  // Paused: nothing but Sanser, which covers the screen.
  expect(mocks.apply).toHaveBeenLastCalledWith([], true, true);
});

it('keeps violations while offline and unlocks when the exam ends', async () => {
  const seat = createExamSeat();
  mocks.client.joinExam.mockResolvedValue(policy());
  await seat.join('ABC234', 'An', 'device-1');
  mocks.client.examHeartbeat.mockRejectedValueOnce(new Error('Unable to reach the server'));
  await vi.advanceTimersByTimeAsync(600);
  expect(get(seat).error).toBe('Unable to reach the server');
  expect(mocks.stop).not.toHaveBeenCalled();

  mocks.report.mockResolvedValue({ running: true, foregroundApp: 'Word', violations: [] });
  mocks.client.examHeartbeat.mockResolvedValue(policy({ roomState: 'ended' }));
  await vi.advanceTimersByTimeAsync(5_000);
  expect(mocks.client.examHeartbeat).toHaveBeenLastCalledWith('member-1', expect.objectContaining({ violations: [{ kind: 'left-app', detail: 'Safari' }] }));
  expect(mocks.stop).toHaveBeenCalled();
  expect(get(seat).finished).toBe('ended');
  expect(seat.active()).toBe(false);
});

it('locks again after a restart and stays strict while the server is unreachable', async () => {
  localStorage.setItem('sanser.exam.seat.v1', JSON.stringify({ memberId: 'member-1', lockedAt: Date.now() }));
  const seat = createExamSeat();
  mocks.client.examPolicy.mockRejectedValue(Object.assign(new Error('offline'), { status: 0 }));
  mocks.client.examHeartbeat.mockRejectedValue(new Error('offline'));
  await seat.resume();
  expect(mocks.apply).toHaveBeenCalledWith([], true, false);
  expect(get(seat).policy?.roomState).toBe('open');

  const removed = createExamSeat();
  mocks.client.examPolicy.mockRejectedValue(Object.assign(new Error('gone'), { status: 404 }));
  await removed.resume();
  expect(removed.active()).toBe(false);
});

const unlock = { salt: 'AAECAwQFBgcICQoLDA0ODw==', hash: '7zlNmKeyQK+qOwTP1VPS4oD+8R31S77kGKQ0V3wl/rk=', iterations: 1000 };

it('the emergency code unlocks through the server, and a wrong code does not', async () => {
  vi.useRealTimers();
  const seat = createExamSeat();
  mocks.client.joinExam.mockResolvedValue(policy({ unlock }));
  await seat.join('84DWNR', 'An', 'device-1');
  mocks.client.unlockExam.mockRejectedValueOnce(Object.assign(new Error('the unlock code is not right'), { status: 422 }));
  expect(await seat.unlock('ab3k-9mzq')).toBe(false);
  expect(seat.active()).toBe(true);
  mocks.client.unlockExam.mockResolvedValueOnce(policy({ memberState: 'removed' }));
  expect(await seat.unlock('ab3k-9mzq')).toBe(true);
  expect(mocks.client.unlockExam).toHaveBeenLastCalledWith('member-1', 'AB3K9MZQ');
  expect(get(seat).finished).toBe('unlocked');
  expect(seat.active()).toBe(false);
  expect(mocks.stop).toHaveBeenCalled();
});

it('without the server the stored hash decides, even after a restart', async () => {
  vi.useRealTimers();
  const first = createExamSeat();
  mocks.client.joinExam.mockResolvedValue(policy({ unlock }));
  await first.join('84DWNR', 'An', 'device-1');
  // A new store, as after Sanser restarts; the network is down.
  const seat = createExamSeat();
  mocks.client.unlockExam.mockRejectedValue(Object.assign(new Error('Unable to reach the server'), { status: 0 }));
  expect(await seat.unlock('WRONGCOD')).toBe(false);
  expect(seat.active()).toBe(true);
  expect(await seat.unlock('AB3K 9MZQ')).toBe(true);
  expect(get(seat).finished).toBe('unlocked');
  expect(seat.active()).toBe(false);
});
