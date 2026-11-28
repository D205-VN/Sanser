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

export type ExamViolationKind = 'left-app' | 'ai-app' | 'ai-site' | 'guard-failed' | 'quit-attempt' | 'disconnected';

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

export interface ExamScreen { memberId: string; jpegBase64: string; capturedAt: number }

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
