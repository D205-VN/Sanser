<script lang="ts">
  import { t } from '../lib/i18n';
  import { onMount } from 'svelte';
  import CapabilityNotice from '../components/CapabilityNotice.svelte';
  import StatusPill from '../components/StatusPill.svelte';
  import NativePerformance from '../components/NativePerformance.svelte';
  import { nativePerformance, metricIsFresh } from '../stores/nativePerformance';
  import { runtimeStatus } from '../lib/platform';
  import type { RuntimeStatus } from '../lib/types';
  import { connection } from '../stores/connection';
  import { fecHealth } from '../stores/fecHealth';
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
    catch (error) { message = error instanceof Error ? error.message : $t('Unable to refresh runtime status'); }
    finally { runtimeBusy = false; }
  }
  onMount(() => { void refreshRuntime(); const timer=window.setInterval(() => { now=Date.now(); },1_000); return () => window.clearInterval(timer); });
</script>

<section class="stack">
  <header class="page-header">
    <div><h3>{$t('Connection details')}</h3><p>{$t('Connection status and recent errors.')}</p></div>
    <button class="button" disabled={runtimeBusy} onclick={refreshRuntime}>{runtimeBusy ? $t('Refreshing…') : $t('Refresh status')}</button>
  </header>
  {#if message}<div class="notice">{message}</div>{/if}
  <p class="muted">{$t('Connection checks run automatically. The latest 20 native connection reports are saved on this computer and included in Export diagnostics. Reports contain measured performance and the Direct/Relay route, without screen content or credentials. Missing measurements are not treated as a healthy connection.')}</p>

  <div class="diagnostic-summary grid three">
    <article class="card metric-card"><span>{$t('Connection')}</span><strong>{$connection.engineRunning ? $t('Window open') : $connection.session?.status ?? $t('Idle')}</strong><small>{$connection.session?.transport?.toUpperCase() ?? $t('No transport')}</small></article>
    <article class="card metric-card"><span>{$t('Control RTT')}</span><strong>{!$nativePerformance.client?.ended && metricIsFresh(controlRtt,now) ? `${controlRtt.latestMs.toFixed(1)} ms` : '—'}</strong><small>{$t('Application round trip, including queues')}</small></article>
    <article class="card metric-card"><span>{$t('Frame rate')}</span><strong>{$connection.metrics ? `${$connection.metrics.fps} FPS` : '—'}</strong><small>{$connection.metrics ? `${$connection.metrics.droppedFrames} dropped` : $t('No media metrics')}</small></article>
  </div>

  <NativePerformance />

  <article class="card card-body stack">
    <div><h2 class="card-title">{$t('Internet direct route')}</h2><p class="card-subtitle">{$t('Updated during the latest P2P attempt. The host uses a stable UDP port for PCP, NAT-PMP, UPnP or manual forwarding.')}</p></div>
    <div class="advanced-metrics">
      <div><span>{$t('Host UDP port')}</span><strong>{$preferences.host.directUdpPort}</strong></div>
      <div><span>{$t('Last bound port')}</span><strong>{$networkDiagnostics.localPort ?? $t('Not tested')}</strong></div>
      <div><span>{$t('STUN public route')}</span><strong>{$networkDiagnostics.stunSucceeded === null ? $t('Not tested') : $networkDiagnostics.stunSucceeded ? $t('Detected') : $t('Unavailable')}</strong></div>
      <div><span>{$t('Router port mapping')}</span><strong>{$networkDiagnostics.portMappingSucceeded === null ? $t('Not tested') : $networkDiagnostics.portMappingSucceeded ? $t('Mapped') : $t('Not mapped')}</strong></div>
      <div><span>{$t('Public endpoint')}</span><strong>{$networkDiagnostics.publicEndpoint ?? $t('Not detected')}</strong></div>
      <div><span>{$t('Global IPv6')}</span><strong>{$networkDiagnostics.ipv6Available === null ? $t('Not tested') : $networkDiagnostics.ipv6Available ? $t('Detected') : $t('Not detected')}</strong></div>
      <div><span>{$t('Candidates')}</span><strong>{$networkDiagnostics.candidateCount || '—'}</strong></div>
      <div><span>{$t('Gathering time')}</span><strong>{$networkDiagnostics.gatheringDurationMs === null ? '—' : `${$networkDiagnostics.gatheringDurationMs} ms`}</strong></div>
    </div>
    <div class="notice warning">{$t('If STUN works but direct connection still times out, forward UDP {port} on the home router to the host computer. The current media engine requires IPv4. Auto mode tries authenticated direct connectivity and then the encrypted relay.', { port: $preferences.host.directUdpPort })}</div>
  </article>

  {#if $networkDiagnostics.selection}
    {@const selected = $networkDiagnostics.selection}
    <article class="card card-body stack">
      <div><h2 class="card-title">{$t('Selected Direct candidate')}</h2><p class="card-subtitle">{$t(selected.reason)}</p></div>
      <div class="advanced-metrics">
        <div><span>{$t('Local advertised')} · {selected.localCandidateType}</span><strong>{selected.localCandidateEndpoint}</strong></div>
        <div><span>{$t('Advertised local interface · name-based classification')}</span><strong>{selected.localCandidateInterface ? `${selected.localCandidateInterface.name} · ${selected.localCandidateInterface.kind}` : $t('Unknown (not verified from the candidate)')}</strong></div>
        <div><span>{$t('OS route lookup · separate socket')}</span><strong>{selected.probeSourceEndpoint ?? $t('Not measured')}</strong></div>
        <div><span>{$t('OS-selected source interface · name-based classification')}</span><strong>{selected.probeSourceInterface ? `${selected.probeSourceInterface.name} · ${selected.probeSourceInterface.kind}` : $t('Unknown')}</strong></div>
        <div><span>{$t('Router mapping · local / remote')}</span><strong>{selected.localMappingProtocol ?? $t('Unknown')} / {selected.remoteMappingProtocol ?? $t('Unknown')}</strong></div>
        <div><span>{$t('Remote advertised')} · {selected.remoteCandidateType}</span><strong>{selected.remoteCandidateEndpoint}</strong></div>
        <div><span>{$t('Authenticated peer endpoint')}</span><strong>{selected.verifiedRemoteEndpoint}</strong></div>
        <div><span>{$t('Probe median / min / max')}</span><strong>{selected.probe ? `${selected.probe.medianMs.toFixed(1)} / ${selected.probe.minMs.toFixed(1)} / ${selected.probe.maxMs.toFixed(1)} ms` : $t('Not measured')}</strong></div>
        <div><span>{$t('Probe samples / loss')}</span><strong>{selected.probe ? `${selected.probe.samples} / ${selected.probe.lossPercent.toFixed(0)}%` : '—'}</strong></div>
        <div><span>{$t('Latest same-socket UDP echo RTT')}</span><strong>{$networkDiagnostics.routeHealth?.echoRttMs != null ? `${$networkDiagnostics.routeHealth.echoRttMs.toFixed(1)} ms` : $t('Not measured')}</strong></div>
        <div><span>{$t('Echo host hold')}</span><strong>{$networkDiagnostics.routeHealth?.echoHostHoldMs != null ? `${$networkDiagnostics.routeHealth.echoHostHoldMs.toFixed(1)} ms` : '—'}</strong></div>
        <div><span>{$t('Echo measured at')}</span><strong>{$networkDiagnostics.routeHealth?.echoUpdatedAtMs ? new Date($networkDiagnostics.routeHealth.echoUpdatedAtMs).toLocaleTimeString() : $t('Awaiting a supporting host')}</strong></div>
        <div><span>{$t('Live Wire RTT estimate')}</span><strong>{$networkDiagnostics.routeHealth?.liveWireRttMs != null ? `${$networkDiagnostics.routeHealth.liveWireRttMs.toFixed(1)} ms` : $t('Awaiting media measurements')}</strong></div>
      </div>
      <p class="muted">{$t('Candidate addresses are advertised endpoints. The route lookup uses a separate socket and does not prove which physical interface every media packet uses. UDP echo bypasses control/decode queues but still includes OS/socket scheduling and responder work. Probe RTT includes probe handling; live Wire RTT still includes OS queues. Fewer than 3 probe samples means limited confidence. This is the latest nomination, not a guarantee that a session is still active.')}</p>
      {#if $networkDiagnostics.routeHealth?.status === 'elevated-after-media'}
        <div class="notice warning">{$t('RTT increased substantially in at least three measurements after media started. The initial probe was faster; investigate the media path and OS/network queues. Live standby measurements determine whether a better path is available.')}</div>
      {:else if $networkDiagnostics.routeHealth?.status === 'slow-at-nomination' && !$networkDiagnostics.livePaths?.snapshot.switches}
        <div class="notice warning">{$t('This Direct path was already slow before video started. Compare the selected endpoints, router mapping and OS-selected source interface. A Direct connection can still use a VPN or overlay. No faster route has been demonstrated.')}</div>
      {/if}
    </article>
  {/if}

  {#if $networkDiagnostics.livePaths}
    {@const live = $networkDiagnostics.livePaths}
    {@const fresh = now - live.receivedAtMs < 5_000 && now >= live.receivedAtMs}
    <article class="card card-body stack">
      <div><h2 class="card-title">{$t('Live Direct paths')}</h2><p class="card-subtitle">{fresh ? live.snapshot.pending ? $t('Confirming a faster path with the peer…') : $t('Measuring the active and standby UDP paths') : $t('Historical measurements · monitor is no longer reporting')}</p></div>
      <p>{$t('Active:')} <strong>{live.snapshot.activeEndpoint}</strong> · {$t('{count} switches', { count: live.snapshot.switches })}</p>
      <div class="advanced-metrics">
        {#each live.snapshot.paths as path (path.endpoint)}
          <div><span>{path.endpoint}{path.endpoint === live.snapshot.activeEndpoint ? $t(' · active') : $t(' · standby')}</span>
            <strong>{fresh && path.samples >= 3 && path.lastResponseAgeMs !== null && path.lastResponseAgeMs <= 8_000 && path.medianMs !== null ? `${path.medianMs.toFixed(1)} ms median · ${path.jitterMs?.toFixed(1) ?? '—'} ms jitter · ${path.lossPercent.toFixed(0)}% probe loss` : $t('Insufficient fresh authenticated samples')}</strong>
          </div>
        {/each}
      </div>
      <p class="muted">{$t('Only a sustained improvement confirmed by several fresh probes can replace a responding route. Both peers confirm the new path; a 15-second cooldown prevents repeated switching. Probe RTT includes OS scheduling. No faster path is guaranteed. Unanswered probes may also mean the other app does not support live path checks.')}</p>
    </article>
  {/if}

  {#if $fecHealth.length}
    <article class="card card-body stack">
      <div><h2 class="card-title">{$t('Adaptive video FEC')}</h2><p class="card-subtitle">{$t('Optional recovery for short UDP losses. Native retransmission still handles losses beyond one packet per block.')}</p></div>
      {#each $fecHealth as health (`${health.kind}:${health.sessionId}`)}
        {@const fresh = now >= health.receivedAtMs && now - health.receivedAtMs < 5_000}
        <div class="advanced-metrics">
          <div><span>{health.kind === 'host' ? $t('Sending') : $t('Receiving')} · {health.transport === 'direct' ? $t('Direct') : $t('UDP Relay')}</span><strong>{!fresh ? $t('Historical measurements') : !health.stats.negotiated ? $t('Awaiting peer capability confirmation') : $t('Peer capability confirmed')}</strong></div>
          <div><span>{$t('Local send parity budget')}</span><strong>{fresh && health.stats.negotiated ? health.stats.videoBytes > 0 ? `${health.stats.overheadPercent}%` : $t('Not sending video') : '—'}</strong></div>
          <div><span>{$t('Parity packets / reconstructed packets')}</span><strong>{health.stats.parityPackets} / {health.stats.recovered}</strong></div>
          <div><span>{$t('Expired recovery blocks / rejected records')}</span><strong>{health.stats.expired} / {health.stats.invalid}</strong></div>
          <div><span>{$t('Budget skips / congestion pauses')}</span><strong>{health.stats.budgetSkipped} / {health.stats.congestionPauses}</strong></div>
        </div>
      {/each}
      <p class="muted">{$t('The sender budget stays at or below 15% of video wire bytes. FEC adds no waiting time to original video. It cannot repair multiple losses in one block or remove network RTT. Counters describe reconstruction, including useful early recovery during reordering. WSS/TCP does not use FEC.')}</p>
    </article>
  {/if}

  <div class="grid two diagnostics-grid">
    <article class="card card-body stack">
      <div><h2 class="card-title">{$t('Runtime capabilities')}</h2><p class="card-subtitle">{$t('Availability comes from the packaged shell and installed sidecars.')}</p></div>
      <CapabilityNotice title={$t('Desktop shell')} capability={displayedRuntime.capabilities.desktopShell} />
      <CapabilityNotice title={$t('Secure storage')} capability={displayedRuntime.capabilities.secureStorage} />
      <CapabilityNotice title={$t('Host engine')} capability={displayedRuntime.capabilities.hostEngine} />
      <CapabilityNotice title={$t('Client engine')} capability={displayedRuntime.capabilities.clientEngine} />
      <CapabilityNotice title={$t('WebRTC')} capability={displayedRuntime.capabilities.webRtc} />
      <CapabilityNotice title={$t('Native direct')} capability={displayedRuntime.capabilities.nativeDirect} />
      <CapabilityNotice title={$t('SNV2')} capability={displayedRuntime.capabilities.nativeSnv2} />
      <CapabilityNotice title={$t('P2P Transport')} capability={displayedRuntime.capabilities.p2pV2} />
    </article>

    <article class="card card-body stack">
      <div><h2 class="card-title">{$t('Native processes')}</h2><p class="card-subtitle">{$t('Process identifiers are shown only inside the local app.')}</p></div>
      <div class="engine-list">
        {#each displayedRuntime.engines.filter((engine) => !isLegacyLocalServer(engine)) as engine}
          <div><div><strong>{engine.kind}</strong><span>{engine.lastError ?? (engine.installed ? $t('Sidecar installed') : $t('Sidecar not bundled'))}</span></div><StatusPill state={engine.running ? 'online' : engine.installed ? 'neutral' : 'offline'} label={engine.running ? `Running · PID ${engine.processId ?? '—'}` : engine.installed ? $t('Stopped') : $t('Missing')} /></div>
        {/each}
      </div>

    </article>
  </div>

  <article class="card card-body diagnostic-events">
    <div class="event-header"><div><h2 class="card-title">{$t('Event buffer')}</h2><p class="card-subtitle">{$t('Latest 300 lifecycle events; no per-frame or per-mouse-move logging.')}</p></div><div class="filter-tabs">{#each ['all', 'info', 'warn', 'error'] as level}<button class:active={filter === level} aria-pressed={filter === level} onclick={() => (filter = level as typeof filter)}>{level}</button>{/each}</div></div>
    {#if events.length === 0}<div class="empty compact-empty"><div><h3>{filter === 'all' ? $t('No diagnostic events') : `No ${filter} events`}</h3><p>{$t('Events appear when auth, network, session or engine state changes.')}</p></div></div>{:else}<div class="event-list">{#each events as event (event.id)}<div class="event-row"><span class="event-level {event.level}">{event.level}</span><time datetime={event.timestamp}>{new Date(event.timestamp).toLocaleTimeString()}</time><strong>{event.category}</strong><p>{event.message}</p>{#if event.details}<code>{JSON.stringify(event.details)}</code>{/if}</div>{/each}</div>{/if}
  </article>
</section>
