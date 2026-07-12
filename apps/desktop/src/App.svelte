<script lang="ts">
  import { onMount } from 'svelte';
  import BrandMark from './components/BrandMark.svelte';
  import Sidebar from './components/Sidebar.svelte';
  import { runtimeStatus, stopEngine } from './lib/platform';
  import type { Page, RuntimeStatus } from './lib/types';
  import ActiveSession from './pages/ActiveSession.svelte';
  import About from './pages/About.svelte';
  import Computers from './pages/Computers.svelte';
  import Diagnostics from './pages/Diagnostics.svelte';
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
  let runtime = $state<RuntimeStatus | null>(null);
  let bootError = $state<string | null>(null);

  const pageMeta: Record<Page, { section: string; label: string }> = {
    computers: { section: 'Workspace', label: 'Computers' },
    host: { section: 'Workspace', label: 'Host' },
    session: { section: 'Workspace', label: 'Active session' },
    settings: { section: 'System', label: 'Settings' },
    diagnostics: { section: 'System', label: 'Diagnostics' },
    about: { section: 'System', label: 'About Sanser' }
  };

  function navigate(next: Page): void {
    page = next;
  }

  $effect(() => {
    if (runtime && $session.mode === 'cloud' && runtime.capabilities.clientEngine.state === 'available') {
      void presence.start(runtime);
    } else {
      void presence.stop();
    }
  });

  async function signOut(): Promise<void> {
    try {
      connection.clear();
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
      diagnostics.add({ level: 'warn', category: 'auth', message: error instanceof Error ? error.message : 'Sign out failed' });
    }
  }

  onMount(() => {
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
  });
</script>

{#if bootError}
  <main class="fatal-screen"><BrandMark size={68} label="Sanser" /><h1>Sanser could not start</h1><p>{bootError}</p><button class="button" onclick={() => window.location.reload()}>Retry</button></main>
{:else if !runtime || !$session.ready}
  <main class="boot-screen"><BrandMark size={68} label="Sanser" /><span>Starting Sanser…</span></main>
{:else if $session.mode === 'signedOut'}
  <Welcome />
{:else}
  <div class="app-shell">
    <Sidebar active={page} {runtime} {navigate} />
    <div class="content-shell">
      <header class="titlebar">
        <div class="title-context">
          <span>{pageMeta[page].section}</span>
          <strong>{pageMeta[page].label}</strong>
        </div>
        <div class="title-actions">
          <span class="mode-indicator"><i></i>Sanser connected</span>
          <span class="title-account">{$session.account?.email}</span>
          <button class="button small ghost" onclick={signOut}>Sign out</button>
        </div>
      </header>
      <main class:session-scroll={page === 'session'} class="page-scroll">
        {#if page === 'computers'}<Computers {runtime} {navigate} />
        {:else if page === 'host'}<Host {runtime} />
        {:else if page === 'session'}<ActiveSession {runtime} />
        {:else if page === 'settings'}<Settings {runtime} />
        {:else if page === 'diagnostics'}<Diagnostics {runtime} />
        {:else}<About {runtime} />{/if}
      </main>
    </div>
  </div>
{/if}

<Updater />
