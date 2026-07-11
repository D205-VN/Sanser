import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { deviceIdentity } from './deviceIdentity';

describe('deviceIdentity', () => {
  beforeEach(() => {
    const values = new Map<string, string>();
    vi.stubGlobal('localStorage', {
      get length() { return values.size; },
      clear: () => values.clear(),
      getItem: (key: string) => values.get(key) ?? null,
      key: (index: number) => [...values.keys()][index] ?? null,
      removeItem: (key: string) => { values.delete(key); },
      setItem: (key: string, value: string) => { values.set(key, value); }
    } satisfies Storage);
  });

  afterEach(() => vi.unstubAllGlobals());

  it('is stable and isolated by account and role', () => {
    const first = deviceIdentity('account-a', 'client');
    expect(deviceIdentity('account-a', 'client')).toBe(first);
    expect(deviceIdentity('account-b', 'client')).not.toBe(first);
    expect(deviceIdentity('account-a', 'host')).not.toBe(first);
  });
});
