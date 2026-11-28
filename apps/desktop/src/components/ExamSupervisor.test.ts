import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { ExamRoomDetail } from '../lib/exam';

const mocks = vi.hoisted(() => ({
  client: {
    examRooms: vi.fn(), examRoom: vi.fn(), createExamRoom: vi.fn(), examScreens: vi.fn(),
    examMemberAction: vi.fn(), endExamRoom: vi.fn(), updateExamRoom: vi.fn()
  }
}));
vi.mock('../stores/session', () => ({ session: { client: () => mocks.client } }));
vi.mock('../lib/platform', () => ({ listRunningApps: () => Promise.resolve([]) }));

import ExamSupervisor from './ExamSupervisor.svelte';

const student = { id: 'm1', displayName: 'Nguyễn An', platform: 'windows', state: 'active' as const, online: true, joinedAt: 1, lastSeenAt: 1, foregroundApp: 'WINWORD', violationCount: 2 };
const room: ExamRoomDetail = {
  id: 'room-1', title: 'Toán 10A', code: 'ABC234', allowedApps: [{ id: 'WINWORD.EXE', name: 'Microsoft Word' }], blockAi: true,
  state: 'open', createdAt: 1, expiresAt: 4_000_000_000, endedAt: null,
  members: [student],
  violations: [{ memberId: 'm1', displayName: 'Nguyễn An', kind: 'ai-app', detail: 'ChatGPT', createdAt: 1 }]
};

beforeEach(() => {
  vi.resetAllMocks();
  mocks.client.examRooms.mockResolvedValue([]);
  mocks.client.examScreens.mockResolvedValue([{ memberId: 'm1', jpegBase64: '/9j/AAA=', capturedAt: 5 }]);
});
afterEach(cleanup);

it('starts an exam with the chosen apps and shows the code, students and violations', async () => {
  mocks.client.createExamRoom.mockResolvedValue(room);
  mocks.client.examMemberAction.mockResolvedValueOnce(room).mockResolvedValueOnce({ ...room, members: [{ ...student, state: 'paused' }] });
  render(ExamSupervisor);
  await fireEvent.input(screen.getByLabelText('Exam name'), { target: { value: 'Toán 10A' } });
  await fireEvent.click(screen.getByLabelText('Microsoft Word'));
  await fireEvent.click(screen.getByRole('button', { name: 'Start exam' }));
  expect(mocks.client.createExamRoom).toHaveBeenCalledWith('Toán 10A', [
    { id: 'com.microsoft.Word', name: 'Microsoft Word' }, { id: 'WINWORD.EXE', name: 'Microsoft Word' }
  ], true, 2);
  expect(await screen.findByText('ABC 234')).toBeInTheDocument();
  expect(screen.getByText(/AI app closed · ChatGPT/)).toBeInTheDocument();
  expect(await screen.findByAltText('Screen of Nguyễn An')).toHaveAttribute('src', 'data:image/jpeg;base64,/9j/AAA=');

  await fireEvent.click(screen.getByRole('button', { name: /Nguyễn An/ }));
  expect(mocks.client.examMemberAction).toHaveBeenCalledWith('room-1', 'm1', 'watch');
  await fireEvent.click(screen.getByRole('button', { name: 'Pause this computer' }));
  expect(mocks.client.examMemberAction).toHaveBeenCalledWith('room-1', 'm1', 'pause');
  expect(await screen.findByRole('button', { name: 'Let them continue' })).toBeInTheDocument();
});

it('reopens the open exam and ends it after confirmation', async () => {
  mocks.client.examRooms.mockResolvedValue([room]);
  mocks.client.examRoom.mockResolvedValue(room);
  mocks.client.endExamRoom.mockResolvedValue({ ...room, state: 'ended' });
  render(ExamSupervisor);
  await fireEvent.click(await screen.findByRole('button', { name: 'End exam' }));
  // The first click only arms the button; the web view has no confirm dialog.
  expect(mocks.client.endExamRoom).not.toHaveBeenCalled();
  await fireEvent.click(screen.getByRole('button', { name: 'Click again to end for everyone' }));
  expect(mocks.client.endExamRoom).toHaveBeenCalledWith('room-1');
  expect(await screen.findByText(/has ended/)).toBeInTheDocument();
});
