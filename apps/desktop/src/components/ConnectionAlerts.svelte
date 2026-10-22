<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import Icon from './Icon.svelte';
  import type { ConnectionSession } from '../lib/types';
  import { hostConnectionAlerts, type HostConnectionAlert } from '../lib/connectionAlerts';
  import { connectionAlerts } from '../stores/connectionAlerts';
  import { host } from '../stores/host';
  import { sharing } from '../stores/sharing';
  import { session } from '../stores/session';
  let stopping = $state<string | null>(null);
  let error = $state<string | null>(null);
  $effect(() => {
    const owner = $session.mode === 'cloud' ? $session.account?.id ?? null : null;
    connectionAlerts.observe(owner, hostConnectionAlerts($host.engineRunning, $host.sessions,
      $sharing.ownGroup, $sharing.paused, $sharing.peerVideo, $sharing.devices));
  });
  onMount(() => { void connectionAlerts.checkPermission(); });
  onDestroy(() => connectionAlerts.observe(null, []));
  async function stop(alert: HostConnectionAlert): Promise<void> {
    stopping = alert.id; error = null;
    try {
      if (alert.sharingId) {
        const member = $sharing.ownGroup?.id === alert.sharingId ? $sharing.ownGroup.members.find((item) => item.id === alert.memberId && item.sessionId === alert.sessionId) : null;
        if (member) await sharing.remove(member);
      } else if (($host.sessions as ConnectionSession[]).some((item) => item.id === alert.sessionId && item.status === 'accepted')) await host.disconnect(alert.sessionId);
    } catch (value) { error = value instanceof Error ? value.message : 'Unable to stop access'; }
    finally { stopping = null; }
  }
</script>

{#if $connectionAlerts.items.length}
  <aside class="connection-alerts" aria-label="Incoming computer connections" aria-live="polite">
    {#each $connectionAlerts.items as alert (alert.id)}
      <article class="connection-alert">
        <div class="alert-icon"><Icon name="monitor" /></div>
        <div class="alert-content"><strong>{alert.deviceName}</strong><p>{alert.sharingId ? 'Joined your screen sharing' : 'Remote connection approved'} · {alert.viewOnly ? 'View only' : 'Mouse & keyboard access'}</p>
          <button class="stop-access" disabled={stopping !== null} onclick={() => stop(alert)}>{stopping === alert.id ? 'Stopping…' : alert.viewOnly ? 'Stop viewing' : 'Stop access'}</button>
          {#if $connectionAlerts.permission === 'not-granted' || $connectionAlerts.permission === 'unknown'}<button class="enable-alerts" onclick={() => connectionAlerts.checkPermission(true)}>Enable desktop alerts</button>{/if}
        </div>
        <button class="dismiss-alert" aria-label={`Dismiss connection alert for ${alert.deviceName}`} title="Dismiss notification" onclick={() => connectionAlerts.dismiss(alert.id)}>×</button>
      </article>
    {/each}
    {#if error}<p class="alert-error" role="alert">{error}</p>{/if}
  </aside>
{/if}

<style>
  .connection-alerts { position: fixed; right: 24px; top: 88px; z-index: 80; width: min(380px, calc(100vw - 32px)); display: grid; gap: 10px; }
  .connection-alert { display: flex; align-items: flex-start; gap: 12px; padding: 18px; background: var(--surface, #19212e); border: 1px solid var(--border, #334257); border-radius: 16px; box-shadow: 0 16px 48px #0005; }
  .alert-icon { width: 36px; height: 36px; flex-shrink: 0; padding: 8px; color: var(--accent); background: #61d4b51a; border-radius: 10px; }
  .alert-content { min-width: 0; flex: 1; } strong { display: block; overflow-wrap: anywhere; font-size: 14px; }
  p { margin: 5px 0 12px; font-size: 12px; color: var(--muted); line-height: 1.6; }
  button { cursor: pointer; font: inherit; } .stop-access, .enable-alerts { border: 0; background: none; padding: 0; font-size: 12px; }
  .stop-access { color: #ff9ca5; } .enable-alerts { color: var(--accent); margin-left: 12px; }
  .dismiss-alert { border: 0; color: var(--muted); background: none; padding: 0 3px; font-size: 22px; line-height: 20px; }
  .alert-error { padding: 12px; background: var(--surface); border-radius: 10px; color: #ff9ca5; }
  @media (max-width: 640px) { .connection-alerts { top: 16px; right: 16px; } }
</style>
