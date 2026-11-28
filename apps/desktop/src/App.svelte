<script lang="ts">
  import { locale, t, tk } from './lib/i18n';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { onMount } from 'svelte';
  import BrandMark from './components/BrandMark.svelte';
  import Sidebar from './components/Sidebar.svelte';
  import { runtimeStatus, stopEngine } from './lib/platform';
  import type { Page, RuntimeStatus, SharingGroup, SharingMember, ConnectionSession } from './lib/types';
  import ActiveSession from './pages/ActiveSession.svelte';
  import About from './pages/About.svelte';
  import Computers from './pages/Computers.svelte';
  import Host from './pages/Host.svelte';
  import History from './pages/History.svelte';
  import HostRecordingNotice from './components/HostRecordingNotice.svelte';
  import Settings from './pages/Settings.svelte';
  import Exam from './pages/Exam.svelte';
  import ExamLockScreen from './components/ExamLockScreen.svelte';
  import { examLocked } from './lib/exam';
  import { examSeat } from './stores/exam';
  import Welcome from './pages/Welcome.svelte';
  import GuestSession from './pages/GuestSession.svelte';
  import { diagnostics, networkDiagnostics, type DirectRouteHealth, type DirectPathsUpdate } from './stores/diagnostics';
  import { nativePerformance, type NativePerformanceUpdate } from './stores/nativePerformance';
  import { connection } from './stores/connection';
  import { fecHealth, type FecHealthUpdate } from './stores/fecHealth';
  import { host } from './stores/host';
  import { preferences } from './stores/preferences';
  import { presence } from './stores/presence';
  import { session } from './stores/session';
  import { sharing } from './stores/sharing';
  import Updater from './components/Updater.svelte';
  import LanguageToggle from './components/LanguageToggle.svelte';
  import ConnectionAlerts from './components/ConnectionAlerts.svelte';
  import FileInvitations from './components/FileInvitations.svelte';
  import { incomingSharingInvitations } from './lib/sharingInvitations';
  import { showSharingInvitationAlert } from './lib/connectionAlerts';
  import { connectionAlerts } from './stores/connectionAlerts';

  let updater = $state<{ checkForUpdates: (manual?: boolean) => Promise<void> }>();
  let updateChecking = $state(false);
  let updateAvailable = $state(false);
  let page = $state<Page>('computers');
  let pageContainer = $state<HTMLElement>();
  let runtime = $state<RuntimeStatus | null>(null);
  let bootError = $state<string | null>(null);
  let signOutError = $state<string | null>(null);
  let signingOut = $state(false);
  let browserOnline = $state(navigator.onLine);
  let autoOnlineAccountId = $state<string | null>(null);
  let sharingActionError = $state<string | null>(null);
  const invitations = $derived(incomingSharingInvitations($sharing.groups, $presence.deviceId, $connection.session?.id));
  const hostFileSession = $derived($host.engineRunning && !$host.sharingActive ? ($host.sessions as ConnectionSession[]).find((item) => item.status === 'accepted')?.id ?? null : null);
  const watchingCount = $derived($sharing.paused ? 0 : $sharing.ownGroup?.members.filter((member) => {
    const health = $sharing.peerVideo[member.id];
    return member.state === 'accepted' && member.startedAt !== undefined && (!health || (health.ready && health.running && health.videoState === 'active'));
  }).length ?? 0);

  $effect(() => {
    if (runtime && $session.mode === 'cloud' && !signingOut) void sharing.initialize(runtime);
    else if ($session.mode === 'signedOut') void sharing.shutdown();
  });

  // Each new invitation alerts once, even while Sanser is in the background.
  // Bookkeeping only: a reactive set would re-run the effect that fills it.
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  const announcedInvitations = new Set<string>();
  $effect(() => {
    for (const { group, member } of invitations) {
      if (member.state !== 'invited' || announcedInvitations.has(member.id)) continue;
      announcedInvitations.add(member.id);
      const name = $sharing.devices.find((device) => device.id === group.hostDeviceId)?.name ?? $t('A computer in your account');
      void showSharingInvitationAlert($t('Screen sharing invitation'), $t('{name} invited you to view its screen.', { name }));
    }
  });

  async function respondInvitation(group: SharingGroup, member: SharingMember, accept: boolean): Promise<void> {
    sharingActionError = null;
    try { await sharing.respond(group, member, accept); if (accept) navigate('session'); }
    catch (error) { sharingActionError = error instanceof Error ? error.message : $t('Unable to respond to the invitation'); }
  }

  async function stopSharing(): Promise<void> {
    sharingActionError = null;
    try { await sharing.stop(); }
    catch (error) { sharingActionError = error instanceof Error ? error.message : $t('Unable to stop sharing'); }
  }

  const pageMeta: Record<Page, { section: string; label: string }> = {
    computers: { section: tk('Workspace'), label: tk('Computers') },
    host: { section: tk('Workspace'), label: tk('Host') },
    session: { section: tk('Workspace'), label: tk('Active session') },
    history: { section: tk('Workspace'), label: tk('History') },
    exam: { section: tk('Workspace'), label: tk('Exam') },
    settings: { section: tk('System'), label: tk('Settings') },
    diagnostics: { section: tk('System'), label: tk('Settings') },
    about: { section: tk('System'), label: tk('About Sanser') }
  };

  $effect(() => { document.documentElement.lang = $locale; });

  function navigate(next: Page): void {
    page = next;
    if (pageContainer) { pageContainer.scrollTop = 0; pageContainer.scrollLeft = 0; }
  }

  $effect(() => {
    if (!signingOut && runtime && $session.mode === 'cloud' && runtime.capabilities.clientEngine.state === 'available') {
      void presence.start(runtime);
    } else if ($session.mode !== 'guest') {
      // A guest's presence is driven by GuestSession, not by an account sign-in.
      void presence.stop();
    }
  });

  // A student computer in an exam: pick the seat up again after a restart,
  // and leave every remote session so nobody can help from elsewhere.
  const studentLocked = $derived(examLocked($examSeat.policy));
  let examTeardown = false;
  $effect(() => {
    if (runtime && $session.mode === 'cloud' && !signingOut) void examSeat.resume();
  });
  $effect(() => {
    if (studentLocked && !examTeardown) { examTeardown = true; void leaveSessionsForExam(); }
    if (!studentLocked) examTeardown = false;
  });

  async function leaveSessionsForExam(): Promise<void> {
    const activeSession = $connection.session;
    const client = session.client();
    await sharing.shutdown().catch(() => undefined);
    connection.clear();
    if (activeSession && client) await client.disconnectSession(activeSession.id).catch(() => undefined);
    if ($host.online || $host.busy || $host.engineRunning) await host.offline().catch(() => undefined);
    if (runtime?.capabilities.desktopShell.state === 'available') await Promise.allSettled([stopEngine('host'), stopEngine('client')]);
    host.setEngineRunning(false);
  }

  $effect(() => {
    const accountId = $session.account?.id ?? null;
    if (signingOut || accountId === null || !$preferences.host.autoOnline || studentLocked) {
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
          message: error instanceof Error ? error.message : $t('Unable to bring the host online automatically')
        });
      });
    }
  });

  async function signOut(): Promise<void> {
    // Signing out would end the exam lock; only the supervisor ends it.
    if (signingOut || examSeat.active()) return;
    signingOut = true;
    signOutError = null;
    try {
      const activeSession = $connection.session;
      const client = session.client();
      await sharing.shutdown();
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
              message: result.reason instanceof Error ? result.reason.message : $t('Unable to stop a native engine during sign out')
            });
          }
        }
      }
      await Promise.all([hostShutdown, presenceShutdown]);
      host.setEngineRunning(false);
      await session.logout();
      page = 'computers';
    } catch (error) {
      signOutError = error instanceof Error ? error.message : $t('Unable to sign out. Please try again.');
      diagnostics.add({ level: 'warn', category: 'auth', message: error instanceof Error ? error.message : $t('Sign out failed') });
    } finally {
      signingOut = false;
    }
  }

  function checkSessionInactivity(): void {
    if (!signingOut && session.isInactive()) void signOut();
  }

  onMount(() => {
    const routeHealthLifecycle = new AbortController();
    let stopRouteHealth: UnlistenFn | undefined;
    let stopPerformance: UnlistenFn | undefined;
    let stopPaths: UnlistenFn | undefined;
    let stopFec: UnlistenFn | undefined;
    let stopVoiceSharing: UnlistenFn | undefined;
    const expiryTimer = window.setInterval(checkSessionInactivity, 60_000);
    window.addEventListener('focus', checkSessionInactivity);
    document.addEventListener('visibilitychange', checkSessionInactivity);
    void (async () => {
      try {
        const loadedPreferences = await preferences.initialize();
        diagnostics.setEnabled(loadedPreferences.diagnosticsEnabled);
        const [loadedRuntime] = await Promise.all([runtimeStatus(), session.initialize(loadedPreferences.serverUrl)]);
        runtime = loadedRuntime;
        if (loadedRuntime.capabilities.desktopShell.state === 'available') {
          const stopVoice = await listen<string>('voice-stop-sharing', (event) => {
            if (event.payload === $sharing.ownGroup?.id) void stopSharing();
          }).catch(() => undefined);
          if (routeHealthLifecycle.signal.aborted) stopVoice?.();
          else stopVoiceSharing = stopVoice;
          const unlisten = await listen<DirectRouteHealth>('direct-route-health', (event) => networkDiagnostics.updateRouteHealth(event.payload))
            .catch(() => { diagnostics.add({ level: 'warn', category: 'network', message: 'Live route comparison is unavailable; measurements are still saved in connection reports.' }); return undefined; });
          if (routeHealthLifecycle.signal.aborted) unlisten?.();
          else stopRouteHealth = unlisten;
          const paths = await listen<DirectPathsUpdate>('direct-paths', (event) => networkDiagnostics.updatePaths(event.payload)).catch(() => undefined);
          if (routeHealthLifecycle.signal.aborted) paths?.();
          else stopPaths = paths;
          const fec = await listen<FecHealthUpdate>('fec-health', (event) => fecHealth.update(event.payload)).catch(() => undefined);
          if (routeHealthLifecycle.signal.aborted) fec?.();
          else stopFec = fec;
          const stop = await listen<NativePerformanceUpdate>('native-performance', (event) => nativePerformance.update(event.payload))
            .catch(() => undefined);
          if (routeHealthLifecycle.signal.aborted) stop?.();
          else stopPerformance = stop;
        }
        diagnostics.add({ level: 'info', category: 'app', message: 'Sanser desktop initialized', details: { platform: loadedRuntime.platform, version: loadedRuntime.version } });
      } catch (error) {
        bootError = error instanceof Error ? error.message : $t('Unable to initialize Sanser');
      }
    })();
    return () => {
      routeHealthLifecycle.abort();
      stopRouteHealth?.();
      stopPerformance?.();
      stopPaths?.();
      stopFec?.();
      stopVoiceSharing?.();
      void sharing.shutdown();
      window.clearInterval(expiryTimer);
      window.removeEventListener('focus', checkSessionInactivity);
      document.removeEventListener('visibilitychange', checkSessionInactivity);
    };
  });
</script>

<svelte:window ononline={() => (browserOnline = true)} onoffline={() => (browserOnline = false)} />

{#if bootError}
  <main class="fatal-screen"><BrandMark size={68} label="Sanser" /><h1>{$t('Sanser could not start')}</h1><p>{$t(bootError)}</p><button class="button" onclick={() => window.location.reload()}>{$t('Retry')}</button></main>
{:else if !runtime || !$session.ready}
  <main class="boot-screen"><BrandMark size={68} label="Sanser" /><span>{$t('Starting Sanser…')}</span></main>
{:else if $session.mode === 'guest'}
  <GuestSession {runtime} />
{:else if $session.mode === 'signedOut'}
  {#if updateAvailable}<div class="welcome-update"><button class="button small ghost" disabled={updateChecking} onclick={() => updater?.checkForUpdates()}>{updateChecking ? $t('Checking…') : $t('Update')}</button></div>{/if}
  <Welcome {runtime} />
{:else if studentLocked || $examSeat.finished}
  <ExamLockScreen />
{:else}
  <ConnectionAlerts />
  <div class="app-shell">
    <Sidebar active={page === 'diagnostics' ? 'settings' : page} {navigate} />
    <div class="content-shell">
      <header class="titlebar">
        <div class="title-context">
          <span>{$t(pageMeta[page].section)}</span><span class="breadcrumb-divider" aria-hidden="true">/</span>
          <strong>{$t(pageMeta[page].label)}</strong>
        </div>
        <div class="title-actions">
          <LanguageToggle />
          {#if $sharing.ownGroup}<span class="sharing-indicator" role="status">{$sharing.paused ? $t('Sharing paused') : $t('Sharing')} · {$t('{count} watching', { count: watchingCount })} <button class="button small danger" disabled={$sharing.busy} onclick={stopSharing}>{$t('Stop')}</button></span>{/if}
          <span class="account-avatar" aria-hidden="true">{($session.account?.email ?? 'S').slice(0, 1).toUpperCase()}</span>
          <span class="title-account" title={$session.account?.email}>{$session.account?.email}</span>
          <button class="button small ghost" disabled={signingOut} onclick={signOut}>{signingOut ? $t('Signing out…') : $t('Sign out')}</button>
          {#if updateAvailable}<button class="button small primary" disabled={updateChecking} onclick={() => updater?.checkForUpdates()}>{updateChecking ? $t('Checking…') : $t('Update')}</button>{/if}
        </div>
      </header>
      <main bind:this={pageContainer} class:session-scroll={page === 'session'} class="page-scroll">
        {#if signOutError}<div class="notice error" role="alert">{$t(signOutError)}</div>{/if}
        {#if sharingActionError}<div class="notice error" role="alert">{$t(sharingActionError)}</div>{/if}
        <FileInvitations hostSession={hostFileSession} clientSession={$connection.engineRunning && $connection.session?.accessMode !== 'view-only' ? $connection.session?.id ?? null : null} {navigate} />
        {#if $sharing.error && page !== 'host'}<div class="notice warning" role="status">{$t('Screen sharing invitations could not be loaded.')} {$t($sharing.error)} <button class="button small" onclick={() => sharing.refresh()}>{$t('Retry invitations')}</button></div>{/if}
        {#each invitations as invitation (invitation.member.id)}
          <div class="notice sharing-invitation" role="status"><div><strong>{$t('{name} invited you to view its screen.', { name: $sharing.devices.find((device) => device.id === invitation.group.hostDeviceId)?.name ?? $t('A computer in your account') })}</strong><p>{$t('View only · Your keyboard and mouse remain on this computer.')}</p></div><span class="button-row"><button class="button primary" disabled={$sharing.busy || $connection.session !== null} onclick={() => respondInvitation(invitation.group, invitation.member, true)}>{invitation.member.state === 'accepted' ? $t('Open shared screen') : $t('Join')}</button>{#if invitation.member.state === 'invited'}<button class="button" disabled={$sharing.busy} onclick={() => respondInvitation(invitation.group, invitation.member, false)}>{$t('Decline')}</button>{/if}{#if $connectionAlerts.permission === 'not-granted' || $connectionAlerts.permission === 'unknown'}<button class="button" onclick={() => connectionAlerts.checkPermission(true)}>{$t('Enable desktop alerts')}</button>{/if}</span></div>
        {/each}
        <HostRecordingNotice />
        {#if !browserOnline}<div class="notice warning" role="status">{$t('You are offline. Check your connection; your saved settings are still available.')}</div>{/if}
        <div class="persistent-session" hidden={page !== 'session'}><ActiveSession {runtime} {navigate} /></div>
        {#if page === 'computers'}<Computers {runtime} {navigate} />
        {:else if page === 'host'}<Host {runtime} />
        {:else if page === 'history'}<History />
        {:else if page === 'exam'}<Exam />
        {:else if page === 'settings' || page === 'diagnostics'}<Settings {runtime} onSignOut={signOut} initialSection={page === 'diagnostics' ? 'diagnostics' : 'stream'} />
        {:else if page === 'about'}<About checking={updateChecking} {updateAvailable} onCheckForUpdates={() => { void updater?.checkForUpdates(); }} />{/if}
      </main>
    </div>
  </div>
{/if}

<Updater bind:this={updater} bind:checking={updateChecking} onAvailabilityChange={(available: boolean) => { updateAvailable = available; }} />

<style>
  .welcome-update { position: fixed; top: 16px; right: 20px; z-index: 10; }
  .sharing-indicator { display: inline-flex; align-items: center; gap: 10px; color: var(--accent); font-size: 14px; }
  .sharing-invitation { display: flex; align-items: center; justify-content: space-between; gap: 16px; margin-bottom: 16px; }
  .sharing-invitation p { margin: 6px 0 0; }
</style>
