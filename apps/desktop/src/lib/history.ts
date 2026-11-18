import { tk, type Translate } from './i18n';
import type { HistoryEntry } from './types';

export type HistoryFilter = 'all' | 'this' | 'incoming' | 'outgoing';

/** Server query for a filter; `null` when the filter needs this computer's IDs and none are known. */
export function historyQuery(filter: HistoryFilter, localIds: readonly (string | null | undefined)[]):
  { deviceIds?: string[]; direction?: 'any' | 'incoming' | 'outgoing' } | null {
  if (filter === 'all') return {};
  const deviceIds = [...new Set(localIds.filter((id): id is string => Boolean(id)))];
  if (deviceIds.length === 0) return null;
  return { deviceIds, direction: filter === 'this' ? 'any' : filter };
}

export function formatDuration(seconds: number | null, t: Translate): string {
  if (seconds === null || !Number.isFinite(seconds) || seconds < 0) return '—';
  const total = Math.round(seconds);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  if (hours > 0) return t('{hours} h {minutes} min', { hours, minutes });
  if (minutes > 0) return t('{minutes} min', { minutes });
  return t('{seconds} s', { seconds: total });
}

const REASONS: Record<string, string> = {
  disconnected: tk('Ended'),
  rejected: tk('Declined'),
  expired: tk('Not answered'),
  device_offline: tk('A computer went offline'),
  sharing_stopped: tk('Sharing stopped')
};

/** Untranslated status label for an entry; pass the result through `$t`. */
export function statusLabel(entry: Pick<HistoryEntry, 'state' | 'endReason' | 'endedAt'>): string {
  if (!entry.endedAt) return entry.state === 'pending' ? tk('Waiting for approval') : tk('In progress');
  const reason = entry.endReason ?? entry.state;
  return REASONS[reason] ?? reason;
}
