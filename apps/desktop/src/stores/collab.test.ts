import { get } from 'svelte/store';
import { describe, expect, it, vi } from 'vitest';
import type { CollabEvent, CollabSnapshot } from '../lib/collab';
import { createCollab } from './collab';

const chat = (seq: number, own = false, text = `message ${seq}`): CollabEvent => ({ seq, kind: 'chat', from: own ? 'self' : 'a', name: 'Viewer', own, text, atMs: seq });
const snapshot = (events: CollabEvent[], cursor = events.at(-1)?.seq ?? 0, extra: Partial<CollabSnapshot> = {}): CollabSnapshot =>
  ({ available: true, host: true, peers: 1, legacyPeers: 0, annotations: true, cursor, events, ...extra });

describe('session chat store', () => {
  it('keeps only session chat, counts unread messages from others and notifies once', async () => {
    const incoming = vi.fn();
    const responses = [snapshot([chat(1), chat(2, true), { seq: 3, kind: 'stroke', from: 'a', name: '', own: false, points: [1, 1], atMs: 3 }]), snapshot([chat(4)])];
    const snapshotFn = vi.fn<(scope: unknown, after: number) => Promise<CollabSnapshot>>(() => Promise.resolve(responses.shift() ?? snapshot([], 4)));
    const store = createCollab({ snapshot: snapshotFn, post: vi.fn(), incoming });
    store.attach({ kind: 'host' });
    await store.refresh();
    expect(get(store).messages.map((message) => message.seq)).toEqual([1, 2]);
    expect(get(store).unread).toBe(1);
    await store.refresh();
    expect(snapshotFn).toHaveBeenLastCalledWith({ kind: 'host' }, 3);
    expect(get(store).unread).toBe(2);
    expect(incoming).toHaveBeenCalledTimes(2);
    store.setReading(true);
    expect(get(store).unread).toBe(0);
    store.detach();
    expect(get(store).messages).toEqual([]);
  });

  it('drops stale history when a new native session restarts the sequence', async () => {
    const responses = [snapshot([chat(5)]), snapshot([chat(1, false, 'new session')])];
    const store = createCollab({ snapshot: () => Promise.resolve(responses.shift() ?? snapshot([], 1)), post: vi.fn() });
    store.attach({ kind: 'client', sessionId: 's' });
    await store.refresh(); await store.refresh();
    expect(get(store).messages.map((message) => message.text)).toEqual(['new session']);
  });

  it('validates before posting and reports native errors', async () => {
    const post = vi.fn().mockRejectedValueOnce('Chat needs an updated Sanser on the other computer').mockResolvedValue(1);
    const store = createCollab({ snapshot: () => Promise.resolve(snapshot([])), post });
    store.attach({ kind: 'client', sessionId: 's' });
    expect(await store.send('   ', 'Me')).toBe(false);
    expect(post).not.toHaveBeenCalled();
    expect(await store.send('hi', 'Me')).toBe(false);
    expect(get(store).error).toBe('Chat needs an updated Sanser on the other computer');
    expect(await store.send('hi', 'Me')).toBe(true);
    expect(post).toHaveBeenLastCalledWith({ kind: 'client', sessionId: 's' }, { type: 'chat', name: 'Me', text: 'hi' });
  });

  it('marks the session unreachable without throwing', async () => {
    const store = createCollab({ snapshot: () => Promise.reject(new Error('Session is no longer active')), post: vi.fn() });
    store.attach({ kind: 'host' });
    await store.refresh();
    expect(get(store).live).toBe(false);
  });
});
