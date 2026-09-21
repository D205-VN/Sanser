<script lang="ts">
  import { onMount } from 'svelte';
  import BrandMark from './components/BrandMark.svelte';
  import Sidebar from './components/Sidebar.svelte';
  import { runtimeStatus, stopEngine } from './lib/platform';
  import type { Page, RuntimeStatus } from './lib/types';
  import ActiveSession from './pages/ActiveSession.svelte';
  import About from './pages/About.svelte';
  import Computers from './pages/Computers.svelte';
  import Host from './pages/Host.svelte';
  import Settings from './pages/Settings.svelte';
  import Welcome from './pages/Welcome.svelte';
  import { diagnostics } from './stores/diagnostics';
  import { connection } from './stores/connection';
  import { host } from './stores/host';
  import { preferences } from './stores/preferences';
  import { presence } from './stores/presence';
  import { session } from './stores/session';
  import Updater from './components/Updater.svelte';

  let page = $state<Page>('computers');
  let pageContainer = $state<HTMLElement>();
  let runtime = $state<RuntimeStatus | null>(null);
  let bootError = $state<string | null>(null);
  let signOutError = $state<string | null>(null);
  let signingOut = $state(false);
  let browserOnline = $state(navigator.onLine);
  let autoOnlineAccountId = $state<string | null>(null);

  const pageMeta: Record<Page, { section: string; label: string }> = {
    computers: { section: 'Workspace', label: 'Computers' },
    host: { section: 'Workspace', label: 'Host' },
    session: { section: 'Workspace', label: 'Active session' },
    settings: { section: 'System', label: 'Settings' },
    diagnostics: { section: 'System', label: 'Settings' },
    about: { section: 'System', label: 'About Sanser' }
  };

  function navigate(next: Page): void {
    page = next;
    if (pageContainer) { pageContainer.scrollTop = 0; pageContainer.scrollLeft = 0; }
  }

  $effect(() => {
    if (!signingOut && runtime && $session.mode === 'cloud' && runtime.capabilities.clientEngine.state === 'available') {
      void presence.start(runtime);
    } else {
      void presence.stop();
    }
  });

  $effect(() => {
    const accountId = $session.account?.id ?? null;
    if (signingOut || accountId === null || !$preferences.host.autoOnline) {
      autoOnlineAccountId = null;
      return;
    }
    const activeRuntime = runtime;
    if (
      activeRuntime === null ||
      $session.mode !== 'cloud' ||
      activeRuntime.capabilities.hostEngine.state !== 'available'
    ) return;
    if (
      autoOnlineAccountId !== accountId &&
      !$host.online &&
      !$host.busy
    ) {
      autoOnlineAccountId = accountId;
      void host.online(activeRuntime).catch((error: unknown) => {
        diagnostics.add({
          level: 'warn',
          category: 'engine',
          message: error instanceof Error ? error.message : 'Unable to bring the host online automatically'
        });
      });
    }
  });

  async function signOut(): Promise<void> {
    if (signingOut) return;
    signingOut = true;
    signOutError = null;
    try {
      const activeSession = $connection.session;
      const client = session.client();
      connection.clear();
      if (activeSession && client) {
        await client.disconnectSession(activeSession.id).catch(() => undefined);
      }
      const hostShutdown = $host.online || $host.busy || $host.engineRunning ? host.offline() : Promise.resolve();
      const presenceShutdown = presence.stop();
      if (runtime?.capabilities.desktopShell.state === 'available') {
        const results = await Promise.allSettled([stopEngine('host'), stopEngine('client')]);
        for (const result of results) {
          if (result.status === 'rejected') {
            diagnostics.add({
              level: 'warn',
              category: 'engine',
              message: result.reason instanceof Error ? result.reason.message : 'Unable to stop a native engine during sign out'
            });
          }
        }
      }
      await Promise.all([hostShutdown, presenceShutdown]);
      host.setEngineRunning(false);
      await session.logout();
      page = 'computers';
    } catch (error) {
      signOutError = error instanceof Error ? error.message : 'Unable to sign out. Please try again.';
      diagnostics.add({ level: 'warn', category: 'auth', message: error instanceof Error ? error.message : 'Sign out failed' });
    } finally {
      signingOut = false;
    }
  }

  function checkSessionInactivity(): void {
    if (!signingOut && session.isInactive()) void signOut();
  }

  onMount(() => {
    const expiryTimer = window.setInterval(checkSessionInactivity, 60_000);
    window.addEventListener('focus', checkSessionInactivity);
    document.addEventListener('visibilitychange', checkSessionInactivity);
    void (async () => {
      try {
        const loadedPreferences = await preferences.initialize();
        diagnostics.setEnabled(loadedPreferences.diagnosticsEnabled);
        const [loadedRuntime] = await Promise.all([runtimeStatus(), session.initialize(loadedPreferences.serverUrl)]);
        runtime = loadedRuntime;
        diagnostics.add({ level: 'info', category: 'app', message: 'Sanser desktop initialized', details: { platform: loadedRuntime.platform, version: loadedRuntime.version } });
      } catch (error) {
        bootError = error instanceof Error ? error.message : 'Unable to initialize Sanser';
      }
    })();
    return () => {
      window.clearInterval(expiryTimer);
      window.removeEventListener('focus', checkSessionInactivity);
      document.removeEventListener('visibilitychange', checkSessionInactivity);
    };
  });
</script>

<svelte:window ononline={() => (browserOnline = true)} onoffline={() => (browserOnline = false)} />

{#if bootError}
  <main class="fatal-screen"><BrandMark size={68} label="Sanser" /><h1>Sanser could not start</h1><p>{bootError}</p><button class="button" onclick={() => window.location.reload()}>Retry</button></main>
{:else if !runtime || !$session.ready}
  <main class="boot-screen"><BrandMark size={68} label="Sanser" /><span>Starting Sanser…</span></main>
{:else if $session.mode === 'signedOut'}
  <Welcome />
{:else}
  <div class="app-shell">
    <Sidebar active={page === 'diagnostics' ? 'settings' : page} {navigate} />
    <div class="content-shell">
      <header class="titlebar">
        <div class="title-context">
          <strong>{pageMeta[page].label}</strong>
        </div>
        <div class="title-actions">
          <span class="title-account">{$session.account?.email}</span>
          <button class="button small ghost" disabled={signingOut} onclick={signOut}>{signingOut ? 'Signing out…' : 'Sign out'}</button>
        </div>
      </header>
      <main bind:this={pageContainer} class:session-scroll={page === 'session'} class="page-scroll">
        {#if signOutError}<div class="notice error" role="alert">{signOutError}</div>{/if}
        {#if !browserOnline}<div class="notice warning" role="status">You are offline. Check your connection; your saved settings are still available.</div>{/if}
        <div class="persistent-session" hidden={page !== 'session'}><ActiveSession {runtime} {navigate} /></div>
        {#if page === 'computers'}<Computers {runtime} {navigate} />
        {:else if page === 'host'}<Host {runtime} />
        {:else if page === 'settings' || page === 'diagnostics'}<Settings {runtime} onSignOut={signOut} initialSection={page === 'diagnostics' ? 'diagnostics' : 'stream'} />
        {:else if page === 'about'}<About />{/if}
      </main>
    </div>
  </div>
{/if}

<Updater />
