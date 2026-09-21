<script lang="ts">
  import StatusPill from '../components/StatusPill.svelte';
  import Toggle from '../components/Toggle.svelte';
  import type { ConnectionSession, RuntimeStatus } from '../lib/types';
  import { diagnostics } from '../stores/diagnostics';
  import { host } from '../stores/host';
  import { preferences } from '../stores/preferences';
  import { session } from '../stores/session';

  let { runtime }: { runtime: RuntimeStatus } = $props();
  let actionError = $state<string | null>(null);

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
        await host.offline();
      }
      else {
        await host.online(runtime);
      }
    } catch (error) {
      actionError = error instanceof Error ? error.message : 'Unable to change host state';
    }
  }

  async function updateHost(
    key: 'audioEnabled' | 'inputEnabled' | 'autoOnline' | 'autoAcceptOwnDevices',
    value: boolean
  ): Promise<void> {
    actionError = null;
    try { await preferences.save({ ...$preferences, host: { ...$preferences.host, [key]: value } }); }
    catch (error) { actionError = error instanceof Error ? error.message : 'Unable to save host settings'; }
  }

  async function acceptRequest(request: ConnectionSession): Promise<void> {
    actionError = null;
    try {
      await host.accept(request.id);
    } catch (error) {
      actionError = error instanceof Error ? error.message : 'Unable to accept the connection request';
    }
  }

  async function rejectRequest(request: ConnectionSession): Promise<void> {
    actionError = null;
    try {
      await host.reject(request.id);
      diagnostics.add({ level: 'info', category: 'session', message: 'Host rejected the connection request' });
    } catch (error) {
      actionError = error instanceof Error ? error.message : 'Unable to reject the connection request';
    }
  }

  async function stopActiveSession(): Promise<void> {
    if (!activeSession) return;
    actionError = null;
    try {
      await host.disconnect(activeSession.id);
    } catch (error) {
      actionError = error instanceof Error ? error.message : 'Unable to stop the active session';
    }
  }

  async function retryActiveSession(): Promise<void> {
    if (!activeSession) return;
    actionError = null;
    try { await host.retry(activeSession.id); }
    catch (error) { actionError = error instanceof Error ? error.message : 'Unable to retry connection'; }
  }
</script>

<section class="page">
  <header class="page-header">
    <div><h1>Host</h1><p>Let your other computers access this screen.</p></div>
    <StatusPill state={$host.online ? 'online' : 'offline'} label={$host.online ? 'Online' : 'Offline'} />
  </header>

  {#if needsServerUpdate}
    <div class="notice warning" role="alert">Screen sharing on this Mac requires a Sanser server update. After the server is updated, select “Check again”.</div>
  {:else if actionError ?? $host.error}<div class="notice error" role="alert">{actionError ?? $host.error}</div>{/if}
  <div class="host-hero card">
    <div class="host-state-orb" class:online={$host.online}><span></span></div>
    <div><h2>{$host.online ? 'This computer is visible' : needsServerUpdate ? 'Screen sharing is unavailable' : 'This computer is private'}</h2><p>{$host.online ? 'Your other computers can request access.' : needsServerUpdate ? 'Your screen is not being shared.' : 'Go online when you want to share your screen.'}</p></div>
    <button class:danger={$host.online} class:primary={!$host.online} class="button" disabled={$host.busy || (!canHost && !$host.online)} title={!canHost && !$host.online ? runtime.capabilities.hostEngine.reason ?? 'The Host engine is unavailable' : undefined} onclick={toggleHost}>{$host.busy ? 'Working…' : $host.online ? 'Go offline' : needsServerUpdate ? 'Check again' : 'Go online'}</button>
  </div>
  {#if !canHost}<div class="notice warning">{runtime.capabilities.hostEngine.reason ?? 'Screen sharing is unavailable on this computer.'}</div>{/if}
  {#if runtime.platform.toLowerCase().includes('mac') && canHost}
    <details class="sharing-help"><summary>Mac permissions</summary><p>Allow Screen Recording in System Settings to share your screen. Allow Accessibility for remote keyboard and mouse control.</p></details>
  {/if}
  <div class="grid two host-grid">
    <article class="card card-body stack">
      <div><h2 class="card-title">Screen sharing</h2><p class="card-subtitle">Your main display is shared during a connection.</p></div>
      {#if runtime.capabilities.hostAudio === true}<Toggle checked={$preferences.host.audioEnabled} disabled={runtime.capabilities.hostEngine.state !== 'available'} label="System audio" description="Share sound from this computer." onchange={(value) => updateHost('audioEnabled', value)} />{/if}
      <Toggle checked={$preferences.host.inputEnabled} disabled={runtime.capabilities.hostEngine.state !== 'available'} label="Remote input" description="Allow the other computer to use your keyboard and mouse." onchange={(value) => updateHost('inputEnabled', value)} />
    </article>

    <article class="card card-body stack">
      <div><h2 class="card-title">Connection requests</h2><p class="card-subtitle">Approve who can access this computer.</p></div>
      <Toggle
        checked={$preferences.host.autoAcceptOwnDevices}
        disabled={!canHost}
        label="Auto accept trusted devices"
        description={`${$preferences.trustedDeviceIds.length} trusted device(s). Manage trust from Computers on this host.`}
        onchange={(value) => updateHost('autoAcceptOwnDevices', value)}
      />
      {#if pendingSessions.length === 0}
        <div class="notice">{$host.online ? 'Listening for connection requests. You can approve each device here.' : 'Go online to receive connection requests from your other computers.'}</div>
      {:else}
        <div class="security-list">
          {#each pendingSessions as request (request.id)}
            <div>
              <span><strong>Client {request.requesterDeviceId.slice(0, 8)}</strong><span>{$preferences.trustedDeviceIds.includes(request.requesterDeviceId) ? 'Trusted · ' : ''}{request.qualityProfile} · {request.requestedCodec === 'hevc' ? 'HEVC' : request.requestedCodec === 'h264' ? 'H.264' : 'Auto codec'}</span></span>
              <span class="button-row">
                <button class="button small primary" disabled={$host.actionSessionId !== null} onclick={() => acceptRequest(request)}>{$host.actionSessionId === request.id ? 'Accepting…' : 'Accept'}</button>
                <button class="button small danger" disabled={$host.actionSessionId !== null} onclick={() => rejectRequest(request)}>Reject</button>
              </span>
            </div>
          {/each}
        </div>
      {/if}
    </article>

    <article class="card card-body stack">
      <div><h2 class="card-title">Current client</h2><p class="card-subtitle">The computer currently accessing your screen.</p></div>
      {#if activeSession}
        <div class="empty compact-empty"><div>
          <h3>Client {activeSession.requesterDeviceId.slice(0, 8)}</h3>
          <p>{activeSession.requesterReadyAt ? ($host.engineRunning ? 'Screen sharing has started.' : $host.failedP2pSessionId === activeSession.id ? 'Connection failed. You can try again.' : 'Starting screen sharing…') : 'Waiting for the other computer…'}</p>
          <span class="button-row">
            {#if $host.failedP2pSessionId === activeSession.id}<button class="button primary" disabled={$host.actionSessionId !== null || $host.launchingSessionId !== null} onclick={retryActiveSession}>Retry connection</button>{/if}
            <button class="button danger" disabled={$host.busy || $host.actionSessionId !== null} onclick={stopActiveSession}>Stop session</button>
          </span>
        </div></div>
      {:else}
        <div class="empty compact-empty"><div><h3>No active client</h3><p>No computer is accessing your screen.</p></div></div>
      {/if}
    </article>
  </div>
</section>
