<script lang="ts">
  import CapabilityNotice from '../components/CapabilityNotice.svelte';
  import StatusPill from '../components/StatusPill.svelte';
  import { exportDiagnostics } from '../lib/platform';
  import type { RuntimeStatus } from '../lib/types';
  import { connection } from '../stores/connection';
  import { diagnostics, networkDiagnostics, type DiagnosticLevel } from '../stores/diagnostics';
  import { preferences } from '../stores/preferences';

  let { runtime }: { runtime: RuntimeStatus } = $props();
  let filter = $state<'all' | DiagnosticLevel>('all');
  let message = $state<string | null>(null);
  const events = $derived($diagnostics.filter((event) => filter === 'all' || event.level === filter).slice().reverse());

  function isLegacyLocalServer(engine: unknown): boolean {
    return typeof engine === 'object' && engine !== null && 'kind' in engine && engine.kind === 'localServer';
  }

  async function exportEvents(): Promise<void> {
    try {
      const result = await exportDiagnostics(diagnostics.exportJson());
      message = `Exported sanitized diagnostics to ${result.path}`;
    } catch (error) {
      message = error instanceof Error ? error.message : 'Unable to export diagnostics';
    }
  }
</script>

<section class="page">
  <header class="page-header">
    <div><h1>Diagnostics</h1><p>Bounded, sanitized runtime state. No password, token, TURN credential or packet payload is exported.</p></div>
    <div class="button-row"><button class="button" onclick={exportEvents}>Export JSON</button><button class="button danger" onclick={() => diagnostics.clear()}>Clear</button></div>
  </header>
  {#if message}<div class="notice">{message}</div>{/if}

  <div class="diagnostic-summary grid three">
    <article class="card metric-card"><span>Connection</span><strong>{$connection.session?.status ?? 'Idle'}</strong><small>{$connection.session?.transport?.toUpperCase() ?? 'No transport'}</small></article>
    <article class="card metric-card"><span>RTT</span><strong>{$connection.metrics ? `${$connection.metrics.rttMs} ms` : '—'}</strong><small>Measured only from engine feedback</small></article>
    <article class="card metric-card"><span>Frame rate</span><strong>{$connection.metrics ? `${$connection.metrics.fps} FPS` : '—'}</strong><small>{$connection.metrics ? `${$connection.metrics.droppedFrames} dropped` : 'No media metrics'}</small></article>
  </div>

  <article class="card card-body stack">
    <div><h2 class="card-title">Internet direct route</h2><p class="card-subtitle">Updated during the latest P2P attempt. The Windows host uses a stable UDP port for UPnP or manual forwarding.</p></div>
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
    <div class="notice warning">If STUN works but direct connection still times out, forward UDP {$preferences.host.directUdpPort} on the home router to the Windows PC. Global IPv6 is tried first when both devices expose a validated IPv6 candidate; IPv4/STUN/UPnP/manual-forward remains the automatic fallback.</div>
  </article>

  <div class="grid two diagnostics-grid">
    <article class="card card-body stack">
      <div><h2 class="card-title">Runtime capabilities</h2><p class="card-subtitle">Availability comes from the packaged shell and installed sidecars.</p></div>
      <CapabilityNotice title="Desktop shell" capability={runtime.capabilities.desktopShell} />
      <CapabilityNotice title="Secure storage" capability={runtime.capabilities.secureStorage} />
      <CapabilityNotice title="Windows host" capability={runtime.capabilities.hostEngine} />
      <CapabilityNotice title="macOS client" capability={runtime.capabilities.clientEngine} />
      <CapabilityNotice title="WebRTC" capability={runtime.capabilities.webRtc} />
      <CapabilityNotice title="Native direct" capability={runtime.capabilities.nativeDirect} />
      <CapabilityNotice title="SNV2" capability={runtime.capabilities.nativeSnv2} />
      <CapabilityNotice title="P2P Transport" capability={runtime.capabilities.p2pV2} />
    </article>

    <article class="card card-body stack">
      <div><h2 class="card-title">Native processes</h2><p class="card-subtitle">Process identifiers are shown only inside the local app.</p></div>
      <div class="engine-list">
        {#each runtime.engines.filter((engine) => !isLegacyLocalServer(engine)) as engine}
          <div><div><strong>{engine.kind}</strong><span>{engine.lastError ?? (engine.installed ? 'Sidecar installed' : 'Sidecar not bundled')}</span></div><StatusPill state={engine.running ? 'online' : engine.installed ? 'neutral' : 'offline'} label={engine.running ? `Running · PID ${engine.processId ?? '—'}` : engine.installed ? 'Stopped' : 'Missing'} /></div>
        {/each}
      </div>
      <div class="advanced-metrics">
        {#each ['Capture latency', 'Convert latency', 'Encode latency', 'Send queue', 'Reassembly', 'Decode latency', 'Render latency', 'Input injection', 'Audio buffer', 'CPU / GPU / RAM'] as label}
          <div><span>{label}</span><strong>Not reported</strong></div>
        {/each}
      </div>
    </article>
  </div>

  <article class="card card-body diagnostic-events">
    <div class="event-header"><div><h2 class="card-title">Event buffer</h2><p class="card-subtitle">Latest 300 lifecycle events; no per-frame or per-mouse-move logging.</p></div><div class="filter-tabs">{#each ['all', 'info', 'warn', 'error'] as level}<button class:active={filter === level} onclick={() => (filter = level as typeof filter)}>{level}</button>{/each}</div></div>
    {#if events.length === 0}<div class="empty compact-empty"><div><h3>No diagnostic events</h3><p>Events appear when auth, network, session or engine state changes.</p></div></div>{:else}<div class="event-list">{#each events as event (event.id)}<div class="event-row"><span class="event-level {event.level}">{event.level}</span><time datetime={event.timestamp}>{new Date(event.timestamp).toLocaleTimeString()}</time><strong>{event.category}</strong><p>{event.message}</p>{#if event.details}<code>{JSON.stringify(event.details)}</code>{/if}</div>{/each}</div>{/if}
  </article>
</section>
