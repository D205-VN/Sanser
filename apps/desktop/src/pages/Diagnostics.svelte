<script lang="ts">
  import { onMount } from 'svelte';
  import CapabilityNotice from '../components/CapabilityNotice.svelte';
  import StatusPill from '../components/StatusPill.svelte';
  import { runtimeStatus } from '../lib/platform';
  import type { RuntimeStatus } from '../lib/types';
  import { connection } from '../stores/connection';
  import { diagnostics, networkDiagnostics, type DiagnosticLevel } from '../stores/diagnostics';
  import { preferences } from '../stores/preferences';

  let { runtime }: { runtime: RuntimeStatus } = $props();
  let filter = $state<'all' | DiagnosticLevel>('all');
  let liveRuntime = $state<RuntimeStatus | null>(null);
  let runtimeBusy = $state(false);
  const displayedRuntime = $derived(liveRuntime ?? runtime);
  let message = $state<string | null>(null);
  const events = $derived($diagnostics.filter((event) => filter === 'all' || event.level === filter).slice().reverse());

  function isLegacyLocalServer(engine: unknown): boolean {
    return typeof engine === 'object' && engine !== null && 'kind' in engine && engine.kind === 'localServer';
  }

  async function refreshRuntime(): Promise<void> {
    if (runtimeBusy) return;
    runtimeBusy = true;
    try { liveRuntime = await runtimeStatus(); }
    catch (error) { message = error instanceof Error ? error.message : 'Unable to refresh runtime status'; }
    finally { runtimeBusy = false; }
  }
  onMount(() => { void refreshRuntime(); });
</script>

<section class="stack">
  <header class="page-header">
    <div><h3>Connection details</h3><p>Connection status and recent errors.</p></div>
    <button class="button" disabled={runtimeBusy} onclick={refreshRuntime}>{runtimeBusy ? 'Refreshing…' : 'Refresh status'}</button>
  </header>
  {#if message}<div class="notice">{message}</div>{/if}

  <div class="diagnostic-summary grid three">
    <article class="card metric-card"><span>Connection</span><strong>{$connection.engineRunning ? 'Window open' : $connection.session?.status ?? 'Idle'}</strong><small>{$connection.session?.transport?.toUpperCase() ?? 'No transport'}</small></article>
    <article class="card metric-card"><span>RTT</span><strong>{$connection.metrics ? `${$connection.metrics.rttMs} ms` : '—'}</strong><small>Measured only from engine feedback</small></article>
    <article class="card metric-card"><span>Frame rate</span><strong>{$connection.metrics ? `${$connection.metrics.fps} FPS` : '—'}</strong><small>{$connection.metrics ? `${$connection.metrics.droppedFrames} dropped` : 'No media metrics'}</small></article>
  </div>

  <article class="card card-body stack">
    <div><h2 class="card-title">Internet direct route</h2><p class="card-subtitle">Updated during the latest P2P attempt. The host uses a stable UDP port for UPnP or manual forwarding.</p></div>
    <div class="advanced-metrics">
      <div><span>Host UDP port</span><strong>{$preferences.host.directUdpPort}</strong></div>
      <div><span>Last bound port</span><strong>{$networkDiagnostics.localPort ?? 'Not tested'}</strong></div>
      <div><span>STUN public route</span><strong>{$networkDiagnostics.stunSucceeded === null ? 'Not tested' : $networkDiagnostics.stunSucceeded ? 'Detected' : 'Unavailable'}</strong></div>
      <div><span>UPnP mapping</span><strong>{$networkDiagnostics.portMappingSucceeded === null ? 'Not tested' : $networkDiagnostics.portMappingSucceeded ? 'Mapped' : 'Not mapped'}</strong></div>
      <div><span>Public endpoint</span><strong>{$networkDiagnostics.publicEndpoint ?? 'Not detected'}</strong></div>
      <div><span>Global IPv6</span><strong>{$networkDiagnostics.ipv6Available === null ? 'Not tested' : $networkDiagnostics.ipv6Available ? 'Detected' : 'Not detected'}</strong></div>
      <div><span>Candidates</span><strong>{$networkDiagnostics.candidateCount || '—'}</strong></div>
      <div><span>Gathering time</span><strong>{$networkDiagnostics.gatheringDurationMs === null ? '—' : `${$networkDiagnostics.gatheringDurationMs} ms`}</strong></div>
    </div>
    <div class="notice warning">If STUN works but direct connection still times out, forward UDP {$preferences.host.directUdpPort} on the home router to the host computer. The current media engine requires IPv4. Auto mode tries authenticated direct connectivity and then the encrypted relay.</div>
  </article>

  <div class="grid two diagnostics-grid">
    <article class="card card-body stack">
      <div><h2 class="card-title">Runtime capabilities</h2><p class="card-subtitle">Availability comes from the packaged shell and installed sidecars.</p></div>
      <CapabilityNotice title="Desktop shell" capability={displayedRuntime.capabilities.desktopShell} />
      <CapabilityNotice title="Secure storage" capability={displayedRuntime.capabilities.secureStorage} />
      <CapabilityNotice title="Host engine" capability={displayedRuntime.capabilities.hostEngine} />
      <CapabilityNotice title="Client engine" capability={displayedRuntime.capabilities.clientEngine} />
      <CapabilityNotice title="WebRTC" capability={displayedRuntime.capabilities.webRtc} />
      <CapabilityNotice title="Native direct" capability={displayedRuntime.capabilities.nativeDirect} />
      <CapabilityNotice title="SNV2" capability={displayedRuntime.capabilities.nativeSnv2} />
      <CapabilityNotice title="P2P Transport" capability={displayedRuntime.capabilities.p2pV2} />
    </article>

    <article class="card card-body stack">
      <div><h2 class="card-title">Native processes</h2><p class="card-subtitle">Process identifiers are shown only inside the local app.</p></div>
      <div class="engine-list">
        {#each displayedRuntime.engines.filter((engine) => !isLegacyLocalServer(engine)) as engine}
          <div><div><strong>{engine.kind}</strong><span>{engine.lastError ?? (engine.installed ? 'Sidecar installed' : 'Sidecar not bundled')}</span></div><StatusPill state={engine.running ? 'online' : engine.installed ? 'neutral' : 'offline'} label={engine.running ? `Running · PID ${engine.processId ?? '—'}` : engine.installed ? 'Stopped' : 'Missing'} /></div>
        {/each}
      </div>

    </article>
  </div>

  <article class="card card-body diagnostic-events">
    <div class="event-header"><div><h2 class="card-title">Event buffer</h2><p class="card-subtitle">Latest 300 lifecycle events; no per-frame or per-mouse-move logging.</p></div><div class="filter-tabs">{#each ['all', 'info', 'warn', 'error'] as level}<button class:active={filter === level} aria-pressed={filter === level} onclick={() => (filter = level as typeof filter)}>{level}</button>{/each}</div></div>
    {#if events.length === 0}<div class="empty compact-empty"><div><h3>{filter === 'all' ? 'No diagnostic events' : `No ${filter} events`}</h3><p>Events appear when auth, network, session or engine state changes.</p></div></div>{:else}<div class="event-list">{#each events as event (event.id)}<div class="event-row"><span class="event-level {event.level}">{event.level}</span><time datetime={event.timestamp}>{new Date(event.timestamp).toLocaleTimeString()}</time><strong>{event.category}</strong><p>{event.message}</p>{#if event.details}<code>{JSON.stringify(event.details)}</code>{/if}</div>{/each}</div>{/if}
  </article>
</section>
