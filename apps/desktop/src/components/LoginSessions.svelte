<script lang="ts">
  import { t } from '../lib/i18n';
  import StatusPill from './StatusPill.svelte';
  import type { LoginSession } from '../lib/types';

  let { sessions, busy, onRevoke }: {
    sessions: LoginSession[];
    busy: boolean;
    onRevoke: (id: string) => Promise<void>;
  } = $props();
  const isEarlier = (item: LoginSession): boolean => !item.current &&
    item.deviceName === 'Sanser Desktop' && item.platform === 'unknown';
  const identified = $derived(sessions.filter((item) => !isEarlier(item)).sort((a, b) => Number(b.current) - Number(a.current)));
  const earlier = $derived(sessions.filter(isEarlier));

  function platformLabel(platform: string): string {
    if (platform.startsWith('macos')) return $t('macOS');
    if (platform.startsWith('windows')) return $t('Windows');
    if (platform.startsWith('linux')) return $t('Linux');
    return platform === 'unknown' ? $t('Device details unavailable') : platform;
  }
  function lastUsed(value: string): string {
    const date = new Date(value);
    return Number.isNaN(date.valueOf()) ? $t('Last use unavailable') :
      $t('Last used {date}', { date: date.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' }) });
  }
</script>

{#snippet rows(items: LoginSession[])}
  <div class="login-session-list">
    {#each items as item (item.id)}
      <div>
        <div>
          <strong>{item.current ? $t('This sign-in') : isEarlier(item) ? $t('Earlier sign-in') : item.deviceName}</strong>
          {#if item.current && item.deviceName !== 'Sanser Desktop'}<span>{item.deviceName}</span>{/if}
          <span>{platformLabel(item.platform)} · {lastUsed(item.lastSeenAt)}</span>
        </div>
        <StatusPill state={item.current ? 'online' : 'neutral'} label={item.current ? $t('Current') : $t('Signed in')} />
        {#if !item.current}<button class="button small danger" disabled={busy} onclick={() => onRevoke(item.id)}>{$t('Sign out')}</button>{/if}
      </div>
    {/each}
  </div>
{/snippet}

<div><h3>{$t('Signed-in sessions')}</h3><p>{$t('These are account sign-ins, not a list of online computers.')}</p></div>
{#if identified.length > 0}{@render rows(identified)}{/if}
{#if earlier.length > 0}
  <details class="earlier-sign-ins">
    <summary>{$t('Earlier sign-ins without device information ({count})', { count: earlier.length })}</summary>
    <p>{$t('Older versions did not save the computer name. You can sign out each session you no longer use.')}</p>
    {@render rows(earlier)}
  </details>
{/if}

<style>
  .earlier-sign-ins summary { cursor: pointer; color: var(--muted); padding-block: 8px; }
  .earlier-sign-ins p { margin-block: 8px 16px; }
  .login-session-list > div { flex-wrap: wrap; }
</style>
