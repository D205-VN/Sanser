import { get, writable } from 'svelte/store';

export type DiagnosticLevel = 'info' | 'warn' | 'error';
export type DiagnosticValue = string | number | boolean | null;

export interface DiagnosticEvent {
  id: string;
  timestamp: string;
  level: DiagnosticLevel;
  category: 'app' | 'auth' | 'network' | 'engine' | 'session';
  message: string;
  details?: Record<string, DiagnosticValue>;
}

const MAX_EVENTS = 300;
const SENSITIVE_KEY = /(password|token|credential|secret|authorization|private.?key|database.?url)/i;
const BEARER_VALUE = /bearer\s+\S+/gi;

function sanitizeValue(value: DiagnosticValue): DiagnosticValue {
  return typeof value === 'string' ? value.replace(BEARER_VALUE, 'Bearer [REDACTED]').slice(0, 1_024) : value;
}

export function sanitizeDetails(details?: Record<string, DiagnosticValue>): Record<string, DiagnosticValue> | undefined {
  if (!details) return undefined;
  return Object.fromEntries(
    Object.entries(details).map(([key, value]) => [key, SENSITIVE_KEY.test(key) ? '[REDACTED]' : sanitizeValue(value)])
  );
}

function createDiagnosticStore() {
  const store = writable<DiagnosticEvent[]>([]);
  let enabled = true;

  return {
    subscribe: store.subscribe,
    setEnabled(value: boolean): void {
      enabled = value;
    },
    add(event: Omit<DiagnosticEvent, 'id' | 'timestamp'>): void {
      if (!enabled && event.level !== 'error') return;
      const next: DiagnosticEvent = {
        ...event,
        id: crypto.randomUUID(),
        timestamp: new Date().toISOString(),
        message: event.message.slice(0, 2_048),
        details: sanitizeDetails(event.details)
      };
      store.update((events) => [...events, next].slice(-MAX_EVENTS));
    },
    clear(): void {
      store.set([]);
    },
    exportJson(): string {
      return JSON.stringify({ schemaVersion: 1, exportedAt: new Date().toISOString(), events: get(store) }, null, 2);
    }
  };
}

export const diagnostics = createDiagnosticStore();
