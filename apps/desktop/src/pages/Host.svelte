<script lang="ts">
  import CapabilityNotice from '../components/CapabilityNotice.svelte';
  import StatusPill from '../components/StatusPill.svelte';
  import Toggle from '../components/Toggle.svelte';
  import { stopEngine } from '../lib/platform';
  import type { ConnectionSession, RuntimeStatus } from '../lib/types';
  import { diagnostics } from '../stores/diagnostics';
  import { host } from '../stores/host';
  import { preferences } from '../stores/preferences';
  import { session } from '../stores/session';

  let { runtime }: { runtime: RuntimeStatus } = $props();
  let actionError = $state<string | null>(null);

  const canHost = $derived($session.mode === 'cloud' && runtime.capabilities.hostEngine.state === 'available');
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
        if ($host.engineRunning) await stopEngine('host');
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
    await preferences.save({ ...$preferences, host: { ...$preferences.host, [key]: value } });
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
      if ($host.engineRunning) await stopEngine('host');
      host.setEngineRunning(false);
      await host.disconnect(activeSession.id);
    } catch (error) {
      actionError = error instanceof Error ? error.message : 'Unable to stop the active session';
    }
  }

  async function retryActiveSession(): Promise<void> {
    if (!activeSession) return;
    actionError = null;
    await host.retry(activeSession.id);
  }
</script>

<section class="page">
  <header class="page-header">
    <div><h1>Host</h1><p>Advertise this Windows computer and control what a remote session may use.</p></div>
    <StatusPill state={$host.online ? 'online' : 'offline'} label={$host.online ? 'Online' : 'Offline'} />
  </header>

  {#if actionError ?? $host.error}<div class="notice error" role="alert">{actionError ?? $host.error}</div>{/if}
  <div class="host-hero card">
    <div class="host-state-orb" class:online={$host.online}><span></span></div>
    <div><h2>{$host.online ? 'This computer is visible' : 'This computer is private'}</h2><p>{$host.online ? 'Authenticated devices on this account may request a session.' : 'No remote connection request can be accepted.'}</p></div>
    <button class:danger={$host.online} class:primary={!$host.online} class="button" disabled={$host.busy || (!canHost && !$host.online)} title={!canHost && !$host.online ? runtime.capabilities.hostEngine.reason ?? 'The Windows host engine is unavailable' : undefined} onclick={toggleHost}>{$host.busy ? 'Working…' : $host.online ? 'Go offline' : 'Go online'}</button>
  </div>
  <div class="notice">
    <strong>Internet direct:</strong> Windows reserves UDP {$preferences.host.directUdpPort}.
    {#if $host.routeAddress}
      Manual router rule: UDP {$preferences.host.directUdpPort} → {$host.routeAddress}:{$preferences.host.directUdpPort}.
    {:else}
      Go online to detect this PC's LAN address, or use its reserved DHCP address in the router rule.
    {/if}
    UPnP, global IPv6 and authenticated hole punching are tried automatically.
  </div>

  <div class="grid two host-grid">
    <article class="card card-body stack">
      <div><h2 class="card-title">Capture</h2><p class="card-subtitle">Capture engines are enumerated by the native Windows sidecar.</p></div>
      <div class="field"><label for="monitor">Monitor</label><select id="monitor" class="select" disabled><option>Primary display · enumeration planned</option></select></div>
      <div class="field"><label for="window">Window</label><select id="window" class="select" disabled><option>Entire display · window capture planned</option></select></div>
      <CapabilityNotice title="Windows capture engine" capability={runtime.capabilities.hostEngine} />
    </article>

    <article class="card card-body stack">
      <div><h2 class="card-title">Session permissions</h2><p class="card-subtitle">These controls are passed to the native engine when an authenticated session starts.</p></div>
      <Toggle checked={$preferences.host.audioEnabled} disabled={runtime.capabilities.hostEngine.state !== 'available'} label="System audio" description="Forward WASAPI loopback audio when the engine supports it." onchange={(value) => updateHost('audioEnabled', value)} />
      <Toggle checked={$preferences.host.inputEnabled} disabled={runtime.capabilities.hostEngine.state !== 'available'} label="Remote input" description="Allow authenticated keyboard and mouse injection." onchange={(value) => updateHost('inputEnabled', value)} />
      <Toggle checked={false} disabled label="Clipboard" description={runtime.capabilities.clipboard.reason ?? 'Clipboard is planned.'} onchange={() => undefined} />
    </article>

    <article class="card card-body stack">
      <div><h2 class="card-title">Connection requests</h2><p class="card-subtitle">Manual approval remains required for every requester that is not explicitly trusted on this Windows host.</p></div>
      <Toggle
        checked={$preferences.host.autoAcceptOwnDevices}
        label="Auto accept trusted devices"
        description={`${$preferences.trustedDeviceIds.length} trusted device(s). Manage trust from Computers on this Windows host.`}
        onchange={(value) => updateHost('autoAcceptOwnDevices', value)}
      />
      {#if pendingSessions.length === 0}
        <div class="notice">Listening for authenticated requests. Trusted devices may be accepted automatically; every other request requires approval here.</div>
      {:else}
        <div class="security-list">
          {#each pendingSessions as request (request.id)}
            <div>
              <span><strong>Mac {request.requesterDeviceId.slice(0, 8)}</strong><span>{$preferences.trustedDeviceIds.includes(request.requesterDeviceId) ? 'Trusted · ' : ''}{request.qualityProfile} · {request.requestedCodec === 'hevc' ? 'HEVC' : request.requestedCodec === 'h264' ? 'H.264' : 'Auto codec'}</span></span>
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
      <div><h2 class="card-title">Current client</h2><p class="card-subtitle">The host starts only after the Mac listener reports that it is ready.</p></div>
      {#if activeSession}
        <div class="empty compact-empty"><div>
          <h3>Mac {activeSession.requesterDeviceId.slice(0, 8)}</h3>
          <p>{activeSession.requesterReadyAt ? ($host.engineRunning ? 'Native stream is running.' : $host.failedP2pSessionId === activeSession.id ? 'P2P negotiation failed; the accepted session remains available.' : 'Mac is ready; starting the Windows engine…') : 'Accepted · waiting for the Mac listener.'}</p>
          <span class="button-row">
            {#if $host.failedP2pSessionId === activeSession.id}<button class="button primary" disabled={$host.actionSessionId !== null || $host.launchingSessionId !== null} onclick={retryActiveSession}>Retry connection</button>{/if}
            <button class="button danger" disabled={$host.actionSessionId !== null || $host.launchingSessionId !== null} onclick={stopActiveSession}>Stop session</button>
          </span>
        </div></div>
      {:else}
        <div class="empty compact-empty"><div><h3>No active client</h3><p>Capture does not run while idle, keeping CPU and GPU use low.</p><button class="button danger" disabled title="No active native session">Stop session · Unavailable</button></div></div>
      {/if}
    </article>
  </div>
</section>
