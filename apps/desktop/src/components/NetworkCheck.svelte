<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '../lib/i18n';
  import { checkNetwork, recentNetworkCheck, type NetworkCheckResult } from '../lib/networkCheck';
  import { isTauriRuntime } from '../lib/platform';
  import { connection } from '../stores/connection';
  import { host as hostStore } from '../stores/host';
  import { session } from '../stores/session';

  // compact: run once quietly and speak only when this network will struggle.
  let { compact = false }: { compact?: boolean } = $props();
  let result = $state<NetworkCheckResult | null>(null);
  let busy = $state(false);

  const sessionActive = $derived(
    $connection.engineRunning || $connection.busy || $hostStore.engineRunning || $hostStore.launchingSessionId !== null
  );

  async function run(cached = false): Promise<void> {
    const client = session.client();
    if (!client || busy || sessionActive || !isTauriRuntime()) return;
    busy = true;
    try { result = await (cached ? recentNetworkCheck(client) : checkNetwork(client)); }
    finally { busy = false; }
  }

  onMount(() => { if (compact) void run(true); });

  const summary = $derived(
    !result ? null
      : result.verdict === 'offline' ? $t('Sanser server is unreachable. Check the internet connection.')
        : result.verdict === 'relay' ? $t('This network blocks direct UDP, so connections will go through the relay and feel slower. Try another Wi-Fi, a phone hotspot, or allow UDP on the router.')
          : $t('Direct connections should work from here. The other computer\'s network also counts.')
  );
</script>

{#if compact}
  {#if result && result.verdict !== 'direct'}
    <div class="notice warning network-check-notice" role="status">
      <span><strong>{$t('Network check')}:</strong> {summary}</span>
      <button class="button small" disabled={busy || sessionActive} onclick={() => run()}>{busy ? $t('Checking…') : $t('Check again')}</button>
    </div>
  {/if}
{:else}
  <article class="card card-body stack">
    <div class="heading">
      <div><h2 class="card-title">{$t('Network check')}</h2><p class="card-subtitle">{$t('See before connecting whether this network allows a direct connection or will use the slower relay.')}</p></div>
      <button class="button small" disabled={busy || sessionActive || !isTauriRuntime()} onclick={() => run()}>{busy ? $t('Checking…') : result ? $t('Check again') : $t('Check network')}</button>
    </div>
    {#if sessionActive}<p class="card-subtitle">{$t('Available when no session is running.')}</p>{/if}
    {#if result}
      <div class="results">
        <div><span>{$t('Sanser server')}</span><strong>{result.serverMs === null ? $t('Unreachable') : `${result.serverMs} ms`}</strong></div>
        <div><span>{$t('Direct UDP')}</span><strong>{result.udp ? $t('Works') : $t('Blocked')}</strong></div>
        <div><span>{$t('Router opens ports')}</span><strong>{result.portMapping ? $t('Yes') : $t('No')}</strong></div>
        <div><span>IPv6</span><strong>{result.publicIpv6 ? $t('Yes') : $t('No')}</strong></div>
      </div>
      <div class="notice" class:warning={result.verdict !== 'direct'} role="status">{summary}</div>
    {/if}
  </article>
{/if}

<style>
  .heading { display: flex; justify-content: space-between; align-items: flex-start; gap: 12px; }
  .results { display: grid; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); gap: 10px; }
  .results div { display: flex; flex-direction: column; gap: 2px; }
  .results span { color: var(--muted); font-size: 12px; }
  .network-check-notice { display: flex; align-items: center; justify-content: space-between; gap: 16px; }
  .network-check-notice .button { flex-shrink: 0; }
</style>
