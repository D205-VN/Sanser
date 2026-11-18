<script lang="ts">
  import { t } from '../lib/i18n';
  import StatusPill from '../components/StatusPill.svelte';
  import Icon from '../components/Icon.svelte';
  import Toggle from '../components/Toggle.svelte';
  import SharingPanel from '../components/SharingPanel.svelte';
  import GuestAccessCard from '../components/GuestAccessCard.svelte';
  import SessionFiles from '../components/SessionFiles.svelte';
  import WindowsUac from '../components/WindowsUac.svelte';
  import ConnectionNotificationSettings from '../components/ConnectionNotificationSettings.svelte';
  import { sharing } from '../stores/sharing';
  import type { ConnectionSession, RuntimeStatus } from '../lib/types';
  import { diagnostics } from '../stores/diagnostics';
  import { host } from '../stores/host';
  import { preferences } from '../stores/preferences';
  import { session } from '../stores/session';

  let { runtime }: { runtime: RuntimeStatus } = $props();
  let actionError = $state<string | null>(null);
  // Host errors are not shown on this page; they are kept in Diagnostics.
  let loggedHostError: string | null = null;
  $effect(() => {
    const error = actionError ?? $host.error;
    if (error && error !== loggedHostError) diagnostics.add({ level: 'warn', category: 'session', message: error });
    loggedHostError = error;
  });

  const canHost = $derived($session.mode === 'cloud' && runtime.capabilities.hostEngine.state === 'available');
  const needsServerUpdate = $derived($host.errorCode === 'server_upgrade_required');
  const pendingSessions: ConnectionSession[] = $derived(
    ($host.sessions as ConnectionSession[]).filter((item) => item.status === 'pending')
  );
  const activeSession: ConnectionSession | null = $derived(
    ($host.sessions as ConnectionSession[]).find((item) => item.status === 'accepted') ?? null
  );

  async function toggleHost(): Promise<void> {
    actionError = null;
    try {
      if ($host.online) {
        if ($host.sharingActive) await sharing.stop();
        await host.offline();
      }
      else {
        await host.online(runtime);
      }
    } catch (error) {
      actionError = error instanceof Error ? error.message : $t('Unable to change host state');
    }
  }

  async function updateHost(
    key: 'audioEnabled' | 'inputEnabled' | 'autoOnline' | 'autoAcceptOwnDevices',
    value: boolean
  ): Promise<void> {
    actionError = null;
    try { await preferences.save({ ...$preferences, host: { ...$preferences.host, [key]: value } }); }
    catch (error) { actionError = error instanceof Error ? error.message : $t('Unable to save host settings'); }
  }

  // Guests start view-only; the host opts in to control per request.
  let guestControl = $state<Record<string, boolean>>({});

  async function acceptRequest(request: ConnectionSession, trust = false): Promise<void> {
    actionError = null;
    try {
      const accepted = request.guest
        ? await host.accept(request.id, { allowControl: guestControl[request.id] === true })
        : await host.accept(request.id);
      if (trust) {
        try {
          await preferences.save({
            ...$preferences,
            trustedDeviceIds: [...new Set([...$preferences.trustedDeviceIds, accepted.requesterDeviceId])]
          });
        } catch {
          actionError = $t('Connection accepted, but this computer could not be saved as trusted. Please try again from Computers.');
        }
      }
    } catch (error) {
      actionError = error instanceof Error ? error.message : $t('Unable to accept the connection request');
    }
  }

  async function rejectRequest(request: ConnectionSession): Promise<void> {
    actionError = null;
    try {
      await host.reject(request.id);
      diagnostics.add({ level: 'info', category: 'session', message: 'Host rejected the connection request' });
    } catch (error) {
      actionError = error instanceof Error ? error.message : $t('Unable to reject the connection request');
    }
  }

  async function stopActiveSession(): Promise<void> {
    if (!activeSession) return;
    actionError = null;
    try {
      await host.disconnect(activeSession.id);
    } catch (error) {
      actionError = error instanceof Error ? error.message : $t('Unable to stop the active session');
    }
  }

  async function retryActiveSession(): Promise<void> {
    if (!activeSession) return;
    actionError = null;
    try { await host.retry(activeSession.id); }
    catch (error) { actionError = error instanceof Error ? error.message : $t('Unable to retry connection'); }
  }
</script>

<section class="page host-page">
  <header class="page-header">
    <div><h1>{$t('Share this computer')}</h1><p>{$t('Manage who can see and control your screen.')}</p></div>
    <StatusPill state={$host.online ? 'online' : 'offline'} label={$host.online ? $t('Online') : $t('Offline')} />
  </header>

  {#if needsServerUpdate}
    <div class="notice warning" role="alert">{$t('Screen sharing on this Mac requires a Sanser server update. After the server is updated, select “Check again”.')}</div>
  {/if}
  <div class="host-hero card">
    <div class="host-state-orb" class:online={$host.online}><Icon name="monitor" /></div>
    <div><h2>{$host.online ? $t('This computer is visible') : needsServerUpdate ? $t('Screen sharing is unavailable') : $t('This computer is private')}</h2><p>{$host.online ? $t('Your other computers can request access.') : needsServerUpdate ? $t('Your screen is not being shared.') : $t('Go online when you want to share your screen.')}</p></div>
    <button class:danger={$host.online} class:primary={!$host.online} class="button" disabled={$host.busy || (!canHost && !$host.online)} title={!canHost && !$host.online ? runtime.capabilities.hostEngine.reason ?? $t('The Host engine is unavailable') : undefined} onclick={toggleHost}>{$host.busy ? $t('Working…') : $host.online ? $t('Go offline') : needsServerUpdate ? $t('Check again') : $t('Go online')}</button>
  </div>
  {#if !canHost}<div class="notice warning">{runtime.capabilities.hostEngine.reason ?? $t('Screen sharing is unavailable on this computer.')}</div>{/if}
  {#if runtime.platform.toLowerCase().includes('mac') && canHost}
    <details class="sharing-help"><summary>{$t('Mac permissions')}</summary><p>{$t('On this Mac, allow Sanser in System Settings → Privacy & Security → Screen & System Audio Recording. Allow Accessibility for remote keyboard and mouse control. After changing permissions, quit Sanser completely (Cmd+Q), reopen /Applications/Sanser.app, and reconnect. If permission is already enabled but capture still fails after an update, turn Sanser’s recording permission off and on, then quit and reopen the app.')}</p></details>
  {/if}
  <ConnectionNotificationSettings />
  {#if runtime.capabilities.multiViewerHost === true}<SharingPanel />{/if}
  {#if runtime.platform.toLowerCase().includes('win') && canHost}
    <WindowsUac sessionRunning={$host.engineRunning || $host.sharingActive} />
  {/if}
  <div class="grid two host-grid">
    <article class="card card-body stack">
      <div class="panel-heading"><span class="panel-icon"><Icon name="monitor" /></span><div><h2 class="card-title">{$t('Screen sharing')}</h2><p class="card-subtitle">{$t('Choose what a connected computer can use.')}</p></div></div>
      {#if runtime.capabilities.hostAudio === true}<Toggle checked={$preferences.host.audioEnabled} disabled={runtime.capabilities.hostEngine.state !== 'available'} label={$t('System audio')} description={$t('Share sound from this computer.')} onchange={(value) => updateHost('audioEnabled', value)} />{/if}
      <Toggle checked={$preferences.host.inputEnabled} disabled={runtime.capabilities.hostEngine.state !== 'available'} label={$t('Remote input')} description={$t('Allow the other computer to use your keyboard and mouse.')} onchange={(value) => updateHost('inputEnabled', value)} />
    </article>

    <article class="card card-body stack">
      <div class="panel-heading"><span class="panel-icon"><Icon name="link" /></span><div><h2 class="card-title">{$t('Connection requests')}</h2><p class="card-subtitle">{$t('You decide who can access this computer.')}</p></div></div>
      <Toggle
        checked={$preferences.host.autoAcceptOwnDevices}
        disabled={!canHost}
        label={$t('Auto accept trusted devices')}
        description={$preferences.trustedDeviceIds.length > 0 ? $t('{count} trusted computer(s) can connect without asking. Other computers need approval.', { count: $preferences.trustedDeviceIds.length }) : $t('No trusted computers yet. Choose Accept & trust on a request to allow automatic connections next time.')}
        onchange={(value) => updateHost('autoAcceptOwnDevices', value)}
      />
      {#if pendingSessions.length === 0}
        <div class="notice">{$host.online ? $preferences.host.autoAcceptOwnDevices && $preferences.trustedDeviceIds.length > 0 ? $t('Trusted computers will be accepted automatically.') : $t('Waiting for a connection request. Approve once with Accept & trust to remember that computer.') : $t('Go online to receive connection requests from your other computers.')}</div>
      {:else}
        <div class="security-list">
          {#each pendingSessions as request (request.id)}
            {#if request.guest}
            <div>
              <span><strong>{$t('Guest: {name}', { name: request.guest.name })}</strong><span>{$t('Joined with your support code · no Sanser account')}</span>
                <label class="remember-sign-in"><input type="checkbox" checked={guestControl[request.id] === true} onchange={(event) => { guestControl = { ...guestControl, [request.id]: (event.currentTarget as HTMLInputElement).checked }; }} /><span>{$t('Allow control of keyboard and mouse')}</span></label></span>
              <span class="button-row">
                <button class="button small primary" disabled={$host.actionSessionId !== null} onclick={() => acceptRequest(request)}>{$host.actionSessionId === request.id ? $t('Accepting…') : $t('Accept')}</button>
                <button class="button small danger" disabled={$host.actionSessionId !== null} onclick={() => rejectRequest(request)}>{$t('Reject')}</button>
              </span>
            </div>
            {:else}
            <div>
              <span><strong>{$t('Client {id}', { id: request.requesterDeviceId.slice(0, 8) })}</strong><span>{$preferences.trustedDeviceIds.includes(request.requesterDeviceId) ? $t('Trusted') + ' · ' : ''}{request.qualityProfile} · {request.requestedCodec === 'hevc' ? 'HEVC' : request.requestedCodec === 'h264' ? 'H.264' : $t('Auto codec')}</span></span>
              <span class="button-row">
                {#if $preferences.host.autoAcceptOwnDevices && !$preferences.trustedDeviceIds.includes(request.requesterDeviceId)}
                  <button class="button small primary" disabled={$host.actionSessionId !== null} onclick={() => acceptRequest(request, true)}>{$t('Accept & trust')}</button>
                {/if}
                <button class="button small primary" disabled={$host.actionSessionId !== null} onclick={() => acceptRequest(request)}>{$host.actionSessionId === request.id ? $t('Accepting…') : $t('Accept')}</button>
                <button class="button small danger" disabled={$host.actionSessionId !== null} onclick={() => rejectRequest(request)}>{$t('Reject')}</button>
              </span>
            </div>
            {/if}
          {/each}
        </div>
      {/if}
    </article>

    <article class="card card-body stack current-client">
      <div class="panel-heading"><span class="panel-icon"><Icon name="session" /></span><div><h2 class="card-title">{$t('Current client')}</h2><p class="card-subtitle">{$t('The computer currently accessing your screen.')}</p></div></div>
      {#if activeSession}
        <div class="empty compact-empty"><div>
          <h3>{activeSession.guest ? $t('Guest: {name}', { name: activeSession.guest.name }) : $t('Client {id}', { id: activeSession.requesterDeviceId.slice(0, 8) })}</h3>
          <p>{activeSession.requesterReadyAt ? ($host.engineRunning ? $t('Screen sharing has started.') : $host.failedP2pSessionId === activeSession.id ? $t('Connection failed. You can try again.') : $t('Starting screen sharing…')) : $t('Waiting for the other computer…')}</p>
          <span class="button-row">
            {#if $host.failedP2pSessionId === activeSession.id}<button class="button primary" disabled={$host.actionSessionId !== null || $host.launchingSessionId !== null} onclick={retryActiveSession}>{$t('Retry connection')}</button>{/if}
            <button class="button danger" disabled={$host.busy || $host.actionSessionId !== null} onclick={stopActiveSession}>{$t('Stop session')}</button>
          </span>
        </div></div>
      {:else}
        <div class="empty compact-empty"><div><h3>{$t('No active client')}</h3><p>{$t('No computer is accessing your screen.')}</p></div></div>
      {/if}
    </article>
    {#if canHost}<GuestAccessCard />{/if}
    {#if activeSession && !activeSession.guest && $host.engineRunning && !$host.sharingActive}
      {#key activeSession.id}<SessionFiles sessionId={activeSession.id} kind="host" />{/key}
    {/if}
  </div>
</section>
