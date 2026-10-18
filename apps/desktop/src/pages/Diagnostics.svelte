<script lang="ts">
  import { onMount } from 'svelte';
  import CapabilityNotice from '../components/CapabilityNotice.svelte';
  import StatusPill from '../components/StatusPill.svelte';
  import NativePerformance from '../components/NativePerformance.svelte';
  import { nativePerformance, metricIsFresh } from '../stores/nativePerformance';
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
  let now = $state(Date.now());
  const controlRtt = $derived($nativePerformance.client?.performance.metrics.controlAppRtt ?? $nativePerformance.client?.performance.metrics.controlAppRttFallback);
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
  onMount(() => { void refreshRuntime(); const timer=window.setInterval(() => { now=Date.now(); },1_000); return () => window.clearInterval(timer); });
</script>

<section class="stack">
  <header class="page-header">
    <div><h3>Connection details</h3><p>Connection status and recent errors.</p></div>
    <button class="button" disabled={runtimeBusy} onclick={refreshRuntime}>{runtimeBusy ? 'Refreshing…' : 'Refresh status'}</button>
  </header>
  {#if message}<div class="notice">{message}</div>{/if}
  <p class="muted">Connection checks run automatically. The latest 20 native connection reports are saved on this computer and included in Export diagnostics. Reports contain measured performance and the Direct/Relay route, without screen content or credentials. Missing measurements are not treated as a healthy connection.</p>

  <div class="diagnostic-summary grid three">
    <article class="card metric-card"><span>Connection</span><strong>{$connection.engineRunning ? 'Window open' : $connection.session?.status ?? 'Idle'}</strong><small>{$connection.session?.transport?.toUpperCase() ?? 'No transport'}</small></article>
    <article class="card metric-card"><span>Control RTT</span><strong>{!$nativePerformance.client?.ended && metricIsFresh(controlRtt,now) ? `${controlRtt.latestMs.toFixed(1)} ms` : '—'}</strong><small>Application round trip, including queues</small></article>
    <article class="card metric-card"><span>Frame rate</span><strong>{$connection.metrics ? `${$connection.metrics.fps} FPS` : '—'}</strong><small>{$connection.metrics ? `${$connection.metrics.droppedFrames} dropped` : 'No media metrics'}</small></article>
  </div>

  <NativePerformance />

  <article class="card card-body stack">
    <div><h2 class="card-title">Internet direct route</h2><p class="card-subtitle">Updated during the latest P2P attempt. The host uses a stable UDP port for PCP, NAT-PMP, UPnP or manual forwarding.</p></div>
    <div class="advanced-metrics">
      <div><span>Host UDP port</span><strong>{$preferences.host.directUdpPort}</strong></div>
      <div><span>Last bound port</span><strong>{$networkDiagnostics.localPort ?? 'Not tested'}</strong></div>
      <div><span>STUN public route</span><strong>{$networkDiagnostics.stunSucceeded === null ? 'Not tested' : $networkDiagnostics.stunSucceeded ? 'Detected' : 'Unavailable'}</strong></div>
      <div><span>Router port mapping</span><strong>{$networkDiagnostics.portMappingSucceeded === null ? 'Not tested' : $networkDiagnostics.portMappingSucceeded ? 'Mapped' : 'Not mapped'}</strong></div>
      <div><span>Public endpoint</span><strong>{$networkDiagnostics.publicEndpoint ?? 'Not detected'}</strong></div>
      <div><span>Global IPv6</span><strong>{$networkDiagnostics.ipv6Available === null ? 'Not tested' : $networkDiagnostics.ipv6Available ? 'Detected' : 'Not detected'}</strong></div>
      <div><span>Candidates</span><strong>{$networkDiagnostics.candidateCount || '—'}</strong></div>
      <div><span>Gathering time</span><strong>{$networkDiagnostics.gatheringDurationMs === null ? '—' : `${$networkDiagnostics.gatheringDurationMs} ms`}</strong></div>
    </div>
    <div class="notice warning">If STUN works but direct connection still times out, forward UDP {$preferences.host.directUdpPort} on the home router to the host computer. The current media engine requires IPv4. Auto mode tries authenticated direct connectivity and then the encrypted relay.</div>
  </article>

  {#if $networkDiagnostics.selection}
    {@const selected = $networkDiagnostics.selection}
    <article class="card card-body stack">
      <div><h2 class="card-title">Selected Direct candidate</h2><p class="card-subtitle">{selected.reason}</p></div>
      <div class="advanced-metrics">
        <div><span>Local advertised · {selected.localCandidateType}</span><strong>{selected.localCandidateEndpoint}</strong></div>
        <div><span>Advertised local interface · name-based classification</span><strong>{selected.localCandidateInterface ? `${selected.localCandidateInterface.name} · ${selected.localCandidateInterface.kind}` : 'Unknown (not verified from the candidate)'}</strong></div>
        <div><span>OS route lookup · separate socket</span><strong>{selected.probeSourceEndpoint ?? 'Not measured'}</strong></div>
        <div><span>OS-selected source interface · name-based classification</span><strong>{selected.probeSourceInterface ? `${selected.probeSourceInterface.name} · ${selected.probeSourceInterface.kind}` : 'Unknown'}</strong></div>
        <div><span>Router mapping · local / remote</span><strong>{selected.localMappingProtocol ?? 'Unknown'} / {selected.remoteMappingProtocol ?? 'Unknown'}</strong></div>
        <div><span>Remote advertised · {selected.remoteCandidateType}</span><strong>{selected.remoteCandidateEndpoint}</strong></div>
        <div><span>Authenticated peer endpoint</span><strong>{selected.verifiedRemoteEndpoint}</strong></div>
        <div><span>Probe median / min / max</span><strong>{selected.probe ? `${selected.probe.medianMs.toFixed(1)} / ${selected.probe.minMs.toFixed(1)} / ${selected.probe.maxMs.toFixed(1)} ms` : 'Not measured'}</strong></div>
        <div><span>Probe samples / loss</span><strong>{selected.probe ? `${selected.probe.samples} / ${selected.probe.lossPercent.toFixed(0)}%` : '—'}</strong></div>
        <div><span>Latest same-socket UDP echo RTT</span><strong>{$networkDiagnostics.routeHealth?.echoRttMs != null ? `${$networkDiagnostics.routeHealth.echoRttMs.toFixed(1)} ms` : 'Not measured'}</strong></div>
        <div><span>Echo host hold</span><strong>{$networkDiagnostics.routeHealth?.echoHostHoldMs != null ? `${$networkDiagnostics.routeHealth.echoHostHoldMs.toFixed(1)} ms` : '—'}</strong></div>
        <div><span>Echo measured at</span><strong>{$networkDiagnostics.routeHealth?.echoUpdatedAtMs ? new Date($networkDiagnostics.routeHealth.echoUpdatedAtMs).toLocaleTimeString() : 'Awaiting a supporting host'}</strong></div>
        <div><span>Live Wire RTT estimate</span><strong>{$networkDiagnostics.routeHealth?.liveWireRttMs != null ? `${$networkDiagnostics.routeHealth.liveWireRttMs.toFixed(1)} ms` : 'Awaiting media measurements'}</strong></div>
      </div>
      <p class="muted">Candidate addresses are advertised endpoints. The route lookup uses a separate socket and does not prove which physical interface every media packet uses. UDP echo bypasses control/decode queues but still includes OS/socket scheduling and responder work. Probe RTT includes probe handling; live Wire RTT still includes OS queues. Fewer than 3 probe samples means limited confidence. This is the latest nomination, not a guarantee that a session is still active.</p>
      {#if $networkDiagnostics.routeHealth?.status === 'elevated-after-media'}
        <div class="notice warning">RTT increased substantially in at least three measurements after media started. The initial probe was faster; investigate the media path and OS/network queues. Live standby measurements determine whether a better path is available.</div>
      {:else if $networkDiagnostics.routeHealth?.status === 'slow-at-nomination' && !$networkDiagnostics.livePaths?.snapshot.switches}
        <div class="notice warning">This Direct path was already slow before video started. Compare the selected endpoints, router mapping and OS-selected source interface. A Direct connection can still use a VPN or overlay. No faster route has been demonstrated.</div>
      {/if}
    </article>
  {/if}

  {#if $networkDiagnostics.livePaths}
    {@const live = $networkDiagnostics.livePaths}
    {@const fresh = now - live.receivedAtMs < 5_000 && now >= live.receivedAtMs}
    <article class="card card-body stack">
      <div><h2 class="card-title">Live Direct paths</h2><p class="card-subtitle">{fresh ? live.snapshot.pending ? 'Confirming a faster path with the peer…' : 'Measuring the active and standby UDP paths' : 'Historical measurements · monitor is no longer reporting'}</p></div>
      <p>Active: <strong>{live.snapshot.activeEndpoint}</strong> · {live.snapshot.switches} switches</p>
      <div class="advanced-metrics">
        {#each live.snapshot.paths as path (path.endpoint)}
          <div><span>{path.endpoint}{path.endpoint === live.snapshot.activeEndpoint ? ' · active' : ' · standby'}</span>
            <strong>{fresh && path.samples >= 3 && path.lastResponseAgeMs !== null && path.lastResponseAgeMs <= 8_000 && path.medianMs !== null ? `${path.medianMs.toFixed(1)} ms median · ${path.jitterMs?.toFixed(1) ?? '—'} ms jitter · ${path.lossPercent.toFixed(0)}% probe loss` : 'Insufficient fresh authenticated samples'}</strong>
          </div>
        {/each}
      </div>
      <p class="muted">Only a sustained improvement confirmed by several fresh probes can replace a responding route. Both peers confirm the new path; a 15-second cooldown prevents repeated switching. Probe RTT includes OS scheduling. No faster path is guaranteed. Unanswered probes may also mean the other app does not support live path checks.</p>
    </article>
  {/if}

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
