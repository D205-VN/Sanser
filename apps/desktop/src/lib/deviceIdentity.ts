const STORAGE_KEY = 'sanser.device-identities.v2';
const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

function loadIdentities(): Record<string, string> {
  try {
    const value = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '{}') as unknown;
    if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
    return Object.fromEntries(
      Object.entries(value)
        .filter((entry): entry is [string, string] => typeof entry[1] === 'string' && UUID_PATTERN.test(entry[1]))
        .slice(-20)
    );
  } catch {
    return {};
  }
}

/** Device IDs are non-secret and scoped per account and native role. */
export function deviceIdentity(accountId: string, role: 'client' | 'host'): string {
  const key = `${role}:${accountId}`;
  const identities = loadIdentities();
  const existing = identities[key];
  if (existing) return existing;

  const created = crypto.randomUUID();
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ ...identities, [key]: created }));
  } catch {
    // The UUID remains valid for this process when persistent storage is unavailable.
  }
  return created;
}
