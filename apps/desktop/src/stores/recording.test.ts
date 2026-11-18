import { get } from 'svelte/store';
import { describe, expect, it, vi } from 'vitest';
import type { ApiClient } from '../lib/api';
import type { RecordingConsent, RecordingStatus } from '../lib/types';
import { IDLE_RECORDING } from '../lib/recording';
import { createViewerRecording, pollHostRecordings } from './recording';

const status = (state: RecordingStatus['state'], extra: Partial<RecordingStatus> = {}): RecordingStatus =>
  ({ ...IDLE_RECORDING, capable: true, state, ...extra });

function harness(answers: RecordingConsent['state'][], nativeStates: RecordingStatus[] = []) {
  let clock = 0;
  const actions: string[] = [];
  const queue = [...answers];
  const client = {
    updateRecordingConsent: vi.fn((_id: string, _device: string, action: string) => {
      actions.push(action);
      return Promise.resolve({ state: action === 'stop' ? 'stopped' : queue.shift() ?? 'requested' } as RecordingConsent);
    }),
    recordingConsent: vi.fn(() => Promise.resolve({ state: queue.shift() ?? 'requested' } as RecordingConsent))
  } as unknown as ApiClient;
  const states = [...nativeStates];
  const native = {
    start: vi.fn(() => Promise.resolve(status('waiting'))),
    stop: vi.fn(() => Promise.resolve(status('stopped', { reason: 'user' }))),
    status: vi.fn(() => Promise.resolve(states.shift() ?? status('recording')))
  };
  const recording = createViewerRecording({
    client: () => client, deviceId: () => 'viewer', now: () => clock,
    sleep: async (ms) => { clock += ms; await Promise.resolve(); }, native
  });
  return { recording, native, actions };
}

describe('viewer recording consent flow', () => {
  it('starts the native recorder only after the host allows it', async () => {
    const { recording, native } = harness(['requested', 'allowed']);
    await recording.start('s1');
    expect(native.start).toHaveBeenCalledWith('s1');
    await recording.stop();
    expect(native.stop).toHaveBeenCalled();
    expect(get(recording).message).toBe('user');
  });

  it('does not record when the host denies', async () => {
    const { recording, native, actions } = harness(['denied']);
    await recording.start('s1');
    expect(native.start).not.toHaveBeenCalled();
    expect(get(recording)).toMatchObject({ phase: 'idle', message: 'host-denied', error: true });
    expect(actions).toContain('stop');
  });

  it('does not record when the host never answers', async () => {
    const { recording, native } = harness(Array(20).fill('requested'));
    await recording.start('s1');
    expect(native.start).not.toHaveBeenCalled();
    expect(get(recording).message).toBe('host-timeout');
  });

  it('reports a native limit stop and clears the host indicator', async () => {
    const { recording, actions } = harness(['allowed'], [status('recording'), status('stopped', { reason: 'size-limit' })]);
    await recording.start('s1');
    await vi.waitFor(() => expect(get(recording).message).toBe('size-limit'));
    expect(actions.at(-1)).toBe('stop');
  });
});

describe('host recording poll', () => {
  it('answers from the setting and lists recording viewers', async () => {
    const updates: string[] = [];
    const client = {
      recordingConsent: vi.fn((id: string) => Promise.resolve({ state: id === 'a' ? 'requested' : 'none' } as RecordingConsent)),
      updateRecordingConsent: vi.fn((_id: string, _host: string, action: string) => {
        updates.push(action);
        return Promise.resolve({ state: action === 'allow' ? 'allowed' : 'denied', viewerName: 'Office Mac' } as RecordingConsent);
      })
    } as unknown as ApiClient;
    expect(await pollHostRecordings(client, 'host', ['a', 'b'], true)).toEqual([{ sessionId: 'a', viewerName: 'Office Mac' }]);
    expect(await pollHostRecordings(client, 'host', ['a'], false)).toEqual([]);
    expect(updates).toEqual(['allow', 'deny']);
  });
});
