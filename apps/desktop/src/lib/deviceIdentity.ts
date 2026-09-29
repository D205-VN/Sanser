import { invoke, isTauri } from '@tauri-apps/api/core';

const STORAGE_KEY = 'sanser.device-identities.v2';
const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

let browserIdentities: Record<string, string> = {};

function loadIdentities(): Record<string, string> {
  try {
    const value = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '{}') as unknown;
    if (!value || typeof value !== 'object' || Array.isArray(value)) return browserIdentities;
    return { ...browserIdentities, ...Object.fromEntries(
      Object.entries(value)
        .filter((entry): entry is [string, string] => typeof entry[1] === 'string' && UUID_PATTERN.test(entry[1]))
    ) };
  } catch {
    return browserIdentities;
  }
}

export interface ComputerIdentity { id: string; computerId: string; name: string }
const identitiesInFlight = new Map<string, Promise<ComputerIdentity>>();

/** Native storage survives webview resets, sign-out, updates and dev/release origins. */
export function deviceIdentity(accountId: string, role: 'client' | 'host', platform = 'computer'): Promise<ComputerIdentity> {
  const key = `${role}:${accountId}`;
  const pending = identitiesInFlight.get(key);
  if (pending) return pending;
  const task = Promise.resolve().then(async () => {
    const saved = loadIdentities();
    if (isTauri()) {
      // Never replace a failed native identity read with a random browser ID.
      return invoke<ComputerIdentity>('get_device_identity', {
        accountId, role, legacyClientId: saved[`client:${accountId}`] ?? null,
        legacyHostId: saved[`host:${accountId}`] ?? null
      });
    }
    const computerKey = `computer:${accountId}`;
    const id = saved[key] ?? crypto.randomUUID();
    const computerId = saved[computerKey] ?? crypto.randomUUID();
    // Keep the in-memory map up to date even when storage is unavailable.
    browserIdentities = { ...saved, [key]: id, [computerKey]: computerId };
    try { localStorage.setItem(STORAGE_KEY, JSON.stringify(browserIdentities)); } catch { /* Browser-only fallback remains stable for this process. */ }
    return { id, computerId, name: `${platform.toLowerCase().includes('mac') ? 'Mac' : platform.toLowerCase().includes('win') ? 'Windows PC' : 'Computer'} · ${computerId.slice(0, 6).toUpperCase()}` };
  });
  identitiesInFlight.set(key, task);
  void task.catch(() => identitiesInFlight.delete(key));
  return task;
}
