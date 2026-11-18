import { get, writable } from 'svelte/store';
import { collabPost, collabSnapshot, validChat, type CollabEvent, type CollabOutgoing, type CollabScope, type CollabSnapshot } from '../lib/collab';

export interface ChatMessage { seq: number; from: string; name: string; own: boolean; text: string; atMs: number }
export interface CollabState {
  /** Session engine reachable and reporting chat support state. */
  live: boolean;
  /** Local native engine supports chat/annotation. */
  available: boolean;
  peers: number;
  legacyPeers: number;
  annotations: boolean;
  messages: ChatMessage[];
  unread: number;
  sending: boolean;
  error: string | null;
}
const MAX_MESSAGES = 100;
export const initialCollab: CollabState = { live: false, available: false, peers: 0, legacyPeers: 0, annotations: true, messages: [], unread: 0, sending: false, error: null };

export interface CollabDeps {
  snapshot: (scope: CollabScope, after: number) => Promise<CollabSnapshot>;
  post: (scope: CollabScope, message: CollabOutgoing) => Promise<number>;
  /** Called for each chat message from someone else (desktop notification). */
  incoming?: (message: ChatMessage) => void;
}

/** Session-only chat state. Nothing is persisted; detach() forgets it all. */
export function createCollab(deps: CollabDeps = { snapshot: collabSnapshot, post: collabPost }) {
  const store = writable<CollabState>({ ...initialCollab });
  let scope: CollabScope | null = null;
  let cursor = 0;
  let generation = 0;
  let polling = false;
  let reading = false;

  function absorb(events: CollabEvent[]): void {
    const chats = events.filter((event) => event.kind === 'chat' && typeof event.text === 'string')
      .map((event) => ({ seq: event.seq, from: event.from, name: event.name, own: event.own, text: event.text ?? '', atMs: event.atMs }));
    if (!chats.length) return;
    const fresh = chats.filter((message) => !message.own);
    store.update((state) => {
      const known = new Set(state.messages.map((message) => message.seq));
      const added = chats.filter((message) => !known.has(message.seq));
      return {
        ...state,
        messages: [...state.messages, ...added].sort((a, b) => a.seq - b.seq).slice(-MAX_MESSAGES),
        unread: reading ? 0 : state.unread + added.filter((message) => !message.own).length
      };
    });
    for (const message of fresh) deps.incoming?.(message);
  }

  return {
    subscribe: store.subscribe,
    attach(next: CollabScope): void {
      if (scope && scope.kind === next.kind && scope.sessionId === next.sessionId) return;
      generation += 1; scope = next; cursor = 0;
      store.set({ ...initialCollab });
    },
    detach(): void { generation += 1; scope = null; cursor = 0; store.set({ ...initialCollab }); },
    /** True while the chat panel is open and visible: incoming messages are read. */
    setReading(value: boolean): void {
      reading = value;
      if (value) store.update((state) => ({ ...state, unread: 0 }));
    },
    async refresh(): Promise<void> {
      const current = scope, epoch = generation;
      if (!current || polling) return;
      polling = true;
      try {
        const snapshot = await deps.snapshot(current, cursor);
        if (epoch !== generation) return;
        // A new native session restarts sequence numbers; drop stale history.
        if (snapshot.cursor < cursor) { cursor = 0; store.update((state) => ({ ...state, messages: [], unread: 0 })); }
        cursor = Math.max(cursor, snapshot.cursor);
        store.update((state) => ({ ...state, live: true, available: snapshot.available, peers: snapshot.peers, legacyPeers: snapshot.legacyPeers, annotations: snapshot.annotations }));
        absorb(snapshot.events);
      } catch {
        if (epoch === generation) store.update((state) => ({ ...state, live: false, peers: 0 }));
      } finally { polling = false; }
    },
    async send(text: string, name: string): Promise<boolean> {
      const current = scope, epoch = generation;
      if (!current || get(store).sending) return false;
      if (!validChat(text)) { store.update((state) => ({ ...state, error: 'Messages must be plain text up to 1 KB' })); return false; }
      store.update((state) => ({ ...state, sending: true, error: null }));
      try {
        await deps.post(current, { type: 'chat', name, text });
        if (epoch !== generation) return false;
        await this.refresh();
        return true;
      } catch (error) {
        if (epoch === generation) store.update((state) => ({ ...state, error: error instanceof Error ? error.message : typeof error === 'string' ? error : 'Unable to send the message' }));
        return false;
      } finally { if (epoch === generation) store.update((state) => ({ ...state, sending: false })); }
    }
  };
}
