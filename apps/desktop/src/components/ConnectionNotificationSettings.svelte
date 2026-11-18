<script lang="ts">
  import { t } from '../lib/i18n';
  import { onMount } from 'svelte';
  import { connectionAlerts } from '../stores/connectionAlerts';
  let busy = $state(false);
  onMount(() => { void connectionAlerts.checkPermission(); });
  async function enable(): Promise<void> { busy = true; try { await connectionAlerts.checkPermission(true); } finally { busy = false; } }
</script>
<div class="notice notification-settings">
  <div><strong>{$t('Connection alerts')}</strong><p>{$t('Sanser shows the computer name and its access level when it connects. Enable desktop notifications to also see alerts while Sanser is hidden.')}</p>
    {#if $connectionAlerts.permission === 'not-granted'}<p>{$t('If permission was denied, allow Sanser notifications in system settings.')}</p>{/if}
  </div>
  {#if $connectionAlerts.permission === 'granted'}<span role="status">{$t('Desktop alerts enabled')}</span>
  {:else}<button class="button small" disabled={busy || $connectionAlerts.permission === 'unavailable'} onclick={enable}>{busy ? $t('Enabling…') : $t('Enable notifications')}</button>{/if}
</div>
<style>.notification-settings { display: flex; align-items: center; justify-content: space-between; gap: 20px; margin-bottom: 20px; } p { margin: 6px 0 0; font-size: 13px; line-height: 1.6; } span { color: var(--accent); font-size: 13px; white-space: nowrap; }</style>
