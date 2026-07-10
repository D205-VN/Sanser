<script lang="ts">
  import { onMount } from 'svelte';
  import Sidebar from './components/Sidebar.svelte';
  import { runtimeStatus } from './lib/platform';
  import type { Page, RuntimeStatus } from './lib/types';
  import ActiveSession from './pages/ActiveSession.svelte';
  import About from './pages/About.svelte';
  import Computers from './pages/Computers.svelte';
  import Diagnostics from './pages/Diagnostics.svelte';
  import Host from './pages/Host.svelte';
  import Settings from './pages/Settings.svelte';
  import Welcome from './pages/Welcome.svelte';
  import { diagnostics } from './stores/diagnostics';
  import { host } from './stores/host';
  import { preferences } from './stores/preferences';
  import { session } from './stores/session';

  let page = $state<Page>('computers');
  let runtime = $state<RuntimeStatus | null>(null);
  let bootError = $state<string | null>(null);

  function navigate(next: Page): void {
    page = next;
  }

  async function signOut(): Promise<void> {
    try {
      if ($host.online) await host.offline();
      await session.logout();
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
  <main class="fatal-screen"><img src="/sanser-mark.svg" alt="" /><h1>Sanser could not start</h1><p>{bootError}</p><button class="button" onclick={() => window.location.reload()}>Retry</button></main>
{:else if !runtime || !$session.ready}
  <main class="boot-screen"><img src="/sanser-mark.svg" alt="" /><span>Starting Sanser…</span></main>
{:else if $session.mode === 'signedOut'}
  <Welcome />
{:else}
  <div class="app-shell">
    <Sidebar active={page} {runtime} {navigate} />
    <div class="content-shell">
      <header class="titlebar">
        <span class="title-account">{$session.mode === 'cloud' ? $session.account?.email : 'Local mode'}</span>
        <button class="button small ghost" onclick={signOut}>{$session.mode === 'cloud' ? 'Sign out' : 'Exit local mode'}</button>
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
