/** Exam mode types shared by the API client, the store and the pages. See docs/exam-mode.md. */

export interface ExamApp { id: string; name: string }

export interface ExamRoom {
  id: string;
  title: string;
  code: string;
  allowedApps: ExamApp[];
  blockAi: boolean;
  state: 'open' | 'ended';
  createdAt: number;
  expiresAt: number;
  endedAt: number | null;
  /** Emergency code for one stuck student; only the supervisor sees it. */
  unlockCode?: string;
}

export interface ExamMember {
  id: string;
  displayName: string;
  platform: string;
  state: 'active' | 'paused' | 'removed';
  online: boolean;
  joinedAt: number;
  lastSeenAt: number;
  foregroundApp: string | null;
  violationCount: number;
}

export type ExamViolationKind = 'left-app' | 'ai-app' | 'ai-site' | 'guard-failed' | 'quit-attempt' | 'disconnected' | 'emergency-unlock';

export interface ExamViolation {
  memberId: string;
  displayName: string;
  kind: ExamViolationKind;
  detail: string;
  createdAt: number;
}

export interface ExamRoomDetail extends ExamRoom {
  members: ExamMember[];
  violations: ExamViolation[];
}

/** `seq` (newer servers) orders screens even within one second. */
/** How fast and how sharp the next picture should be, answered with each picture. */
export interface ExamScreenPace { liveScreenMs: number; screenLarge: boolean }

export interface ExamScreen { memberId: string; jpegBase64: string; capturedAt: number; seq?: number }

/** What the student computer must enforce, as the server sees it now. */
export interface ExamPolicy {
  memberId: string;
  roomId: string;
  title: string;
  supervisorName: string;
  roomState: 'open' | 'ended';
  memberState: 'active' | 'paused' | 'removed';
  allowedApps: ExamApp[];
  blockAi: boolean;
  heartbeatSeconds: number;
  screenSeconds: number;
  /** Newer servers: send screens on their own every this many seconds, outside the heartbeat. */
  liveScreenSeconds?: number;
  /** Newest servers: the same in milliseconds, about 250 while the supervisor has this computer open. */
  liveScreenMs?: number;
  /** The supervisor is watching this computer: send the sharp picture. */
  screenLarge?: boolean;
  /** Lets this computer accept the emergency code while the server is out of reach. */
  unlock?: ExamUnlockCheck;
}

/** Base64 salt and PBKDF2-HMAC-SHA256 hash of the upper-case emergency code. */
export interface ExamUnlockCheck { salt: string; hash: string; iterations: number }

/** Upper case without spaces or dashes, as the server compares it. */
export function normalizeUnlockCode(value: string): string {
  return value.replace(/[^a-z0-9]/gi, '').toUpperCase();
}

function fromBase64(value: string): Uint8Array<ArrayBuffer> {
  const text = atob(value);
  const bytes = new Uint8Array(text.length);
  for (let index = 0; index < text.length; index += 1) bytes[index] = text.charCodeAt(index);
  return bytes;
}

/** Checks the code offline against the hash the server gave this computer. */
export async function verifyUnlockCode(code: string, check: ExamUnlockCheck): Promise<boolean> {
  const normalized = normalizeUnlockCode(code);
  if (!normalized) return false;
  try {
    const key = await crypto.subtle.importKey('raw', new TextEncoder().encode(normalized), 'PBKDF2', false, ['deriveBits']);
    const bits = new Uint8Array(await crypto.subtle.deriveBits({ name: 'PBKDF2', hash: 'SHA-256', salt: fromBase64(check.salt), iterations: check.iterations }, key, 256));
    const expected = fromBase64(check.hash);
    if (bits.length !== expected.length) return false;
    let difference = 0;
    for (let index = 0; index < bits.length; index += 1) difference |= (bits[index] ?? 0) ^ (expected[index] ?? 0);
    return difference === 0;
  } catch { return false; }
}

export interface ExamHeartbeat {
  foregroundApp?: string;
  violations: { kind: ExamViolationKind; detail: string }[];
  screenshot?: string;
}

/** Common exam apps with both their macOS and Windows identifiers. */
export const EXAM_APP_PRESETS: { name: string; apps: ExamApp[] }[] = [
  { name: 'Microsoft Word', apps: [{ id: 'com.microsoft.Word', name: 'Microsoft Word' }, { id: 'WINWORD.EXE', name: 'Microsoft Word' }] },
  { name: 'Microsoft Excel', apps: [{ id: 'com.microsoft.Excel', name: 'Microsoft Excel' }, { id: 'EXCEL.EXE', name: 'Microsoft Excel' }] },
  { name: 'Microsoft PowerPoint', apps: [{ id: 'com.microsoft.Powerpoint', name: 'Microsoft PowerPoint' }, { id: 'POWERPNT.EXE', name: 'Microsoft PowerPoint' }] },
  { name: 'Google Chrome', apps: [{ id: 'com.google.Chrome', name: 'Google Chrome' }, { id: 'chrome.exe', name: 'Google Chrome' }] },
  { name: 'Microsoft Edge', apps: [{ id: 'com.microsoft.edgemac', name: 'Microsoft Edge' }, { id: 'msedge.exe', name: 'Microsoft Edge' }] },
  { name: 'Safari', apps: [{ id: 'com.apple.Safari', name: 'Safari' }] },
  { name: 'Visual Studio Code', apps: [{ id: 'com.microsoft.VSCode', name: 'Visual Studio Code' }, { id: 'Code.exe', name: 'Visual Studio Code' }] }
];

/** The student is locked while the room is open and they have not been removed. */
export function examLocked(policy: ExamPolicy | null): boolean {
  return policy !== null && policy.roomState === 'open' && policy.memberState !== 'removed';
}

/** Apps the guard keeps in front. Pausing leaves only Sanser itself. */
export function guardApps(policy: ExamPolicy): string[] {
  return policy.memberState === 'paused' ? [] : policy.allowedApps.map((app) => app.id);
}
