import { invoke } from '@tauri-apps/api/core';
import { isTauriRuntime } from './platform';
import type { ProblemReport, RecordingConsentState, RecordingStatus } from './types';

export const ISSUE_URL = 'https://github.com/D205-VN/Sanser/issues/new';

export const IDLE_RECORDING: RecordingStatus = {
  capable: false, state: 'idle', audio: false, reason: null, bytes: 0, durationMs: 0, parts: 0, path: null
};

export async function recordingStatus(sessionId: string): Promise<RecordingStatus> {
  if (!isTauriRuntime()) return IDLE_RECORDING;
  return invoke<RecordingStatus>('session_recording_status', { sessionId });
}
export async function startRecording(sessionId: string): Promise<RecordingStatus> {
  if (!isTauriRuntime()) throw new Error('Recording requires the Sanser desktop app');
  return invoke<RecordingStatus>('start_session_recording', { sessionId });
}
export async function stopRecording(sessionId: string): Promise<RecordingStatus> {
  if (!isTauriRuntime()) return IDLE_RECORDING;
  return invoke<RecordingStatus>('stop_session_recording', { sessionId });
}
export async function openRecordingsFolder(): Promise<void> {
  if (isTauriRuntime()) await invoke('open_recordings_folder');
}

export async function createProblemReport(diagnostics: string, capabilities: unknown): Promise<ProblemReport> {
  if (!isTauriRuntime()) throw new Error('Problem reports require the Sanser desktop app');
  return invoke<ProblemReport>('create_problem_report', { diagnostics, capabilities: JSON.stringify(capabilities ?? {}) });
}
export async function openProblemReportFolder(): Promise<void> {
  if (isTauriRuntime()) await invoke('open_problem_report_folder');
}
export async function openGithubIssue(url: string): Promise<void> {
  if (isTauriRuntime()) await invoke('open_github_issue', { url });
  else window.open(url, '_blank', 'noopener');
}

/**
 * Prefilled "new issue" link. Only the app version and OS family go into the
 * URL; the user attaches the locally saved zip themselves after reviewing it.
 */
export function issueUrl(version: string, platform: string): string {
  // Allow-list shapes so nothing personal can ever reach the URL.
  const safeVersion = /^\d+\.\d+\.\d+(?:[.-][A-Za-z0-9]{1,16})?$/.test(version) ? version : 'unknown';
  const family = /^(macos|windows|linux)(?:-(x86_64|aarch64|arm64|x64))?$/.exec(platform);
  const safePlatform = family ? family[0] : 'unknown';
  const body = [
    '### What happened?', '', '', '### What did you expect?', '', '',
    '### Steps to reproduce', '1. ', '',
    '### Environment', `- Sanser: ${safeVersion}`, `- OS: ${safePlatform}`, '',
    '### Problem report', 'Settings → Diagnostics → Report a problem saves a redacted zip in Downloads/Sanser/Reports.',
    'Review it, then drag it into this issue if you want to share it.'
  ].join('\n');
  const params = new URLSearchParams({ title: 'Problem: ', body });
  return `${ISSUE_URL}?${params.toString()}`;
}

export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  const pad = (value: number) => String(value).padStart(2, '0');
  return hours ? `${hours}:${pad(minutes)}:${pad(seconds)}` : `${pad(minutes)}:${pad(seconds)}`;
}

/** English message keys (translated with $t) for why a recording ended. */
export function stopReasonMessage(reason: string | null): string {
  switch (reason) {
    case 'size-limit': return 'Recording stopped at the file size limit';
    case 'duration-limit': return 'Recording stopped at the 2-hour limit';
    case 'write-error': return 'Recording stopped: the file could not be written';
    case 'unsupported':
    case 'unsupported-codec': return 'Recording is unavailable for HEVC sessions';
    case 'session-ended': return 'Recording stopped because the session ended';
    case 'host-denied': return 'The host does not allow recording';
    case 'host-timeout': return 'The host did not confirm the recording request';
    case 'user':
    case 'none':
    case null: return 'Recording saved';
    default: return reason;
  }
}

/** Host-side automatic answer to a viewer's request, from the Host setting. */
export function hostDecision(allowRecording: boolean, state: RecordingConsentState): 'allow' | 'deny' | null {
  if (state === 'requested') return allowRecording ? 'allow' : 'deny';
  if (state === 'allowed' && !allowRecording) return 'deny';
  return null;
}
