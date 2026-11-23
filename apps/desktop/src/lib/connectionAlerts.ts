import type { ConnectionSession, Device, SharingGroup, SharingPeerVideoStatus } from './types';
import { isTauriRuntime } from './platform';

export interface HostConnectionAlert {
  id: string;
  sessionId: string;
  deviceName: string;
  viewOnly: boolean;
  sharingId?: string;
  memberId?: string;
}
export type AlertPermission = 'unknown' | 'granted' | 'not-granted' | 'unavailable';

export function hostConnectionAlerts(
  running: boolean, sessions: ConnectionSession[], group: SharingGroup | null,
  paused: boolean, health: Record<string, SharingPeerVideoStatus>, devices: Device[]
): HostConnectionAlert[] {
  const name = (id: string) => (devices.find((device) => device.id === id)?.name ?? `Computer ${id.slice(0, 8)}`).replace(/\p{Cc}/gu, ' ').slice(0, 120);
  const alerts: HostConnectionAlert[] = running ? sessions.filter((item) => !item.sharingId && item.status === 'accepted' && item.requesterReadyAt).map((item) => ({
    id: item.id, sessionId: item.id, deviceName: name(item.requesterDeviceId), viewOnly: item.accessMode === 'view-only'
  })) : [];
  if (group && !paused) for (const member of group.members) {
    const status = health[member.id];
    if (member.state !== 'accepted' || !member.sessionId || (status && !status.running)) continue;
    if (member.startedAt === undefined && !(status?.ready && status.running)) continue;
    alerts.push({ id: member.sessionId, sessionId: member.sessionId, sharingId: group.id, memberId: member.id, deviceName: name(member.viewerDeviceId), viewOnly: true });
  }
  return alerts;
}

// Checking/sending never opens a permission prompt during a connection. Only
// the explicit Enable notifications action may request OS permission.
export async function alertPermission(request = false): Promise<AlertPermission> {
  if (!isTauriRuntime()) return 'unavailable';
  try {
    const plugin = await import('@tauri-apps/plugin-notification');
    if (await plugin.isPermissionGranted()) return 'granted';
    return request && await plugin.requestPermission() === 'granted' ? 'granted' : 'not-granted';
  } catch { return 'unavailable'; }
}
export async function showHostConnectionAlert(alert: HostConnectionAlert, current: () => boolean): Promise<void> {
  if (await alertPermission() !== 'granted' || !current()) return;
  const { sendNotification } = await import('@tauri-apps/plugin-notification');
  if (!current()) return;
  sendNotification({ title: 'Sanser · Remote access', body: `${alert.deviceName} ${alert.sharingId ? 'joined your screen sharing' : 'is connecting to your computer'}. ${alert.viewOnly ? 'View only.' : 'Mouse and keyboard access approved.'}` });
}
/** Optional chat notification while Sanser is in the background. Never prompts. */
export async function showChatAlert(name: string, text: string, title: string, current: () => boolean): Promise<void> {
  if (await alertPermission() !== 'granted' || !current()) return;
  const { sendNotification } = await import('@tauri-apps/plugin-notification');
  if (!current()) return;
  const clean = (value: string, limit: number) => value.replace(/\p{Cc}/gu, ' ').trim().slice(0, limit);
  sendNotification({ title, body: `${clean(name, 64)}: ${clean(text, 140)}` });
}
/** A screen-sharing invitation, which may arrive while Sanser is in the
 * background: flash the window in the Dock/taskbar and, when allowed, show a
 * system notification. Never prompts. */
export async function showSharingInvitationAlert(title: string, body: string): Promise<void> {
  if (!isTauriRuntime()) return;
  try {
    const { getCurrentWindow, UserAttentionType } = await import('@tauri-apps/api/window');
    await getCurrentWindow().requestUserAttention(UserAttentionType.Informational);
  } catch { /* attention is best effort */ }
  if (await alertPermission() !== 'granted') return;
  const { sendNotification } = await import('@tauri-apps/plugin-notification');
  sendNotification({ title, body: body.replace(/\p{Cc}/gu, ' ').slice(0, 200) });
}
