<script lang="ts">
  import { onMount } from 'svelte';
  import { t, tk } from '../lib/i18n';
  import { formatDuration, historyQuery, statusLabel, type HistoryFilter } from '../lib/history';
  import type { HistoryEntry } from '../lib/types';
  import Icon from '../components/Icon.svelte';
  import { host as hostStore } from '../stores/host';
  import { presence } from '../stores/presence';
  import { session } from '../stores/session';

  let entries = $state<HistoryEntry[]>([]);
  let nextCursor = $state<string | null>(null);
  let filter = $state<HistoryFilter>('all');
  let loading = $state(false);
  let error = $state<string | null>(null);
  let generation = 0;

  const filters: { id: HistoryFilter; label: string }[] = [
    { id: 'all', label: tk('All computers') },
    { id: 'this', label: tk('This computer') },
    { id: 'incoming', label: tk('Connections to this computer') },
    { id: 'outgoing', label: tk('From this computer') }
  ];
  const localIds = $derived([$presence.deviceId, $hostStore.deviceId].filter((id): id is string => Boolean(id)));

  function platformLabel(platform: string): string {
    const value = platform.toLowerCase();
    return value.includes('mac') ? 'macOS' : value.includes('windows') ? 'Windows' : platform;
  }
  function when(value: string): string {
    const date = new Date(value);
    return Number.isFinite(date.getTime())
      ? date.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })
      : '—';
  }
  function name(id: string, label: string): string {
    return localIds.includes(id) ? $t('{name} (this computer)', { name: label }) : label;
  }

  async function load(append = false): Promise<void> {
    const client = session.client();
    if (!client) return;
    const query = historyQuery(filter, localIds);
    const current = ++generation;
    if (query === null) {
      entries = [];
      nextCursor = null;
      error = tk('This computer is not registered yet. Turn on Host or sign in again to filter by it.');
      return;
    }
    loading = true;
    error = null;
    try {
      const page = await client.connectionHistory({ ...query, limit: 50, cursor: append ? nextCursor ?? undefined : undefined });
      if (current !== generation) return;
      const merged = append ? [...entries, ...page.items] : page.items;
      entries = [...new Map(merged.map((entry) => [entry.id, entry])).values()];
      nextCursor = page.nextCursor ?? null;
    } catch (caught) {
      if (current !== generation) return;
      error = caught instanceof Error ? caught.message : tk('Unable to load connection history');
    } finally {
      if (current === generation) loading = false;
    }
  }

  function choose(next: HistoryFilter): void {
    filter = next;
    void load();
  }

  onMount(() => { void load(); });
</script>

<section class="page history-page">
  <header class="page-header">
    <div><h1>{$t('Connection history')}</h1><p>{$t('Who connected to which computer during the last 90 days.')}</p></div>
    <div class="button-row"><button class="button" onclick={() => load()} disabled={loading}><Icon name="refresh" />{loading ? $t('Refreshing…') : $t('Refresh')}</button></div>
  </header>

  <div class="filter-tabs history-filters" role="group" aria-label={$t('History filter')}>
    {#each filters as option (option.id)}
      <button class:active={filter === option.id} aria-pressed={filter === option.id} disabled={option.id !== 'all' && localIds.length === 0} onclick={() => choose(option.id)}>{$t(option.label)}</button>
    {/each}
  </div>

  {#if error}<div class="notice error" role="alert">{$t(error)}</div>{/if}

  {#if entries.length === 0 && !loading && !error}
    <div class="card empty"><div><div class="empty-mark"><Icon name="history" /></div><h2>{$t('No connections yet')}</h2><p>{$t('Connections between your computers will appear here.')}</p></div></div>
  {:else if entries.length > 0}
    <div class="card history-list">
      <table>
        <thead><tr><th>{$t('When')}</th><th>{$t('From')}</th><th>{$t('To')}</th><th>{$t('Access')}</th><th>{$t('Route')}</th><th>{$t('Duration')}</th><th>{$t('Status')}</th></tr></thead>
        <tbody>
          {#each entries as entry (entry.id)}
            <tr>
              <td>{when(entry.startedAt)}</td>
              <td><strong>{name(entry.requesterDeviceId, entry.requesterName)}</strong><small>{platformLabel(entry.requesterPlatform)}</small></td>
              <td><strong>{name(entry.hostDeviceId, entry.hostName)}</strong><small>{platformLabel(entry.hostPlatform)}</small></td>
              <td>{entry.accessMode === 'view-only' ? $t('View only') : $t('Control')}{#if entry.sharedView}<small>{$t('Shared screen')}</small>{/if}</td>
              <td>{entry.route === 'relay' ? $t('Relay') : entry.route === 'direct' ? $t('Direct') : '—'}</td>
              <td>{formatDuration(entry.durationSeconds, $t)}</td>
              <td>{$t(statusLabel(entry))}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
  {#if nextCursor}<div class="history-more"><button class="button" disabled={loading} onclick={() => load(true)}>{loading ? $t('Loading…') : $t('Load more')}</button></div>{/if}
</section>

<style>
  .history-filters { width: fit-content; margin-bottom: 18px; }
  .history-filters button { text-transform: none; }
  .history-list { overflow-x: auto; }
  table { width: 100%; border-collapse: collapse; font-size: 13px; }
  th { padding: 12px 14px; color: var(--muted); font-weight: 620; text-align: left; white-space: nowrap; border-bottom: 1px solid var(--border); }
  td { padding: 11px 14px; vertical-align: top; border-bottom: 1px solid var(--border); }
  tbody tr:last-child td { border-bottom: 0; }
  td small { display: block; margin-top: 2px; color: var(--muted); font-size: 12px; }
  .history-more { display: flex; justify-content: center; margin-top: 16px; }
</style>
