<script lang="ts">
  import { t } from '../lib/i18n';
  import BrandMark from '../components/BrandMark.svelte';
  import LanguageToggle from '../components/LanguageToggle.svelte';
  import { stopEngine } from '../lib/platform';
  import type { RuntimeStatus } from '../lib/types';
  import { connection } from '../stores/connection';
  import { presence } from '../stores/presence';
  import { session } from '../stores/session';
  import ActiveSession from './ActiveSession.svelte';

  let { runtime }: { runtime: RuntimeStatus } = $props();
  let leaving = $state(false);
  const status = $derived($connection.session?.status ?? null);
  // A revoked guest token also means the host ended the session.
  const ended = $derived(status === null || $connection.sessionGone || ['rejected', 'disconnected', 'expired', 'closed', 'failed'].includes(status));

  // A guest has nothing else to do here once the session ends, so return to
  // the sign-in screen on its own after the reason has been visible briefly.
  const GUEST_EXIT_DELAY_MS = 3_000;
  $effect(() => {
    if (!ended) return;
    const timer = setTimeout(() => { void leave(); }, GUEST_EXIT_DELAY_MS);
    return () => clearTimeout(timer);
  });

  async function leave(): Promise<void> {
    if (leaving) return;
    leaving = true;
    const current = $connection.session;
    const client = session.client();
    try {
      if (current && client && !ended) await client.disconnectSession(current.id).catch(() => undefined);
      connection.clear();
      if (runtime.capabilities.desktopShell.state === 'available') await stopEngine('client').catch(() => undefined);
      await presence.stop();
    } finally {
      session.endGuest();
      leaving = false;
    }
  }
</script>

<div class="guest-shell">
  <header class="titlebar">
    <div class="title-context"><BrandMark size={24} label="Sanser" /><strong>{$t('Support session')}</strong><span class="breadcrumb-divider" aria-hidden="true">/</span><span>{$t('Connecting to {name}', { name: $session.guest?.hostName ?? '' })}</span></div>
    <div class="title-actions"><LanguageToggle /><button class="button small ghost" disabled={leaving} onclick={leave}>{leaving ? $t('Leaving…') : $t('Leave')}</button></div>
  </header>
  <main class="page-scroll">
    {#if status === 'pending'}<div class="notice" role="status">{$t('Waiting for the other person to accept. You will only see their screen after they accept.')}</div>{/if}
    {#if ended}
      <div class="empty"><div>
        <h3>{status === 'rejected' ? $t('The other person declined the request.') : status === 'expired' ? $t('The request was not answered in time.') : $t('The support session has ended.')}</h3>
        <p class="muted" role="status">{$t('Returning to the sign-in screen…')}</p>
        <button class="button primary" disabled={leaving} onclick={leave}>{$t('Back to sign in')}</button>
      </div></div>
    {:else}
      <ActiveSession {runtime} navigate={() => undefined} />
    {/if}
  </main>
</div>

<style>
  .guest-shell { display: flex; flex-direction: column; height: 100vh; }
  .guest-shell main { flex: 1; padding: 24px; }
  .title-context { display: flex; align-items: center; gap: 8px; }
</style>
