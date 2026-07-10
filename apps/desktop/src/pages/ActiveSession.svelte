<script lang="ts">
  import { onMount } from 'svelte';
  import StatusPill from '../components/StatusPill.svelte';
  import { launchEngine, stopEngine } from '../lib/platform';
  import type { RuntimeStatus } from '../lib/types';
  import { connection } from '../stores/connection';
  import { diagnostics } from '../stores/diagnostics';
  import { preferences } from '../stores/preferences';
  import { session } from '../stores/session';

  let { runtime }: { runtime: RuntimeStatus } = $props();
  let localError = $state<string | null>(null);

  const sessionState = $derived($connection.session);
  const metrics = $derived($connection.metrics);
  const nativeReady = $derived(
    sessionState?.status === 'accepted' &&
      runtime.capabilities.clientEngine.state === 'available' &&
      runtime.capabilities.nativeSnv2.state === 'available' &&
      sessionState.address !== undefined &&
      sessionState.port !== undefined &&
      sessionState.networkMode !== 'relay'
  );

  function resolutionSize(): { width: number; height: number } {
    switch ($preferences.stream.resolution) {
      case '720p': return { width: 1280, height: 720 };
      case '1440p': return { width: 2560, height: 1440 };
      case '2160p': return { width: 3840, height: 2160 };
      default: return { width: 1920, height: 1080 };
    }
  }

  async function startNativeClient(): Promise<void> {
    if (!sessionState || !nativeReady || !sessionState.port) return;
    const size = resolutionSize();
    connection.setBusy(true);
    localError = null;
    try {
      await launchEngine({
        kind: 'client',
        sessionId: sessionState.id,
        address: sessionState.address,
        port: sessionState.port,
        codec: $preferences.stream.codec,
        fps: $preferences.stream.fps,
        bitrateKbps: Math.round($preferences.stream.bitrateMbps * 1_000),
        width: size.width,
        height: size.height,
        networkMode: sessionState.networkMode,
        audioEnabled: $preferences.host.audioEnabled,
        inputEnabled: $preferences.host.inputEnabled,
        relativeMouse: $preferences.input.mouseMode === 'relative',
        sessionToken: undefined
      });
      connection.setEngineRunning(true);
      diagnostics.add({ level: 'info', category: 'engine', message: 'Native macOS client started' });
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Unable to start native client';
      localError = message;
      connection.setError(message);
    } finally {
      connection.setBusy(false);
    }
  }

  async function disconnect(): Promise<void> {
    if (!sessionState) return;
    connection.setBusy(true);
    localError = null;
    try {
      if ($connection.engineRunning) await stopEngine('client');
      const client = session.client();
      if (client) await client.disconnectSession(sessionState.id);
      connection.clear();
      diagnostics.add({ level: 'info', category: 'session', message: 'Session disconnected' });
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Unable to disconnect';
      localError = message;
      connection.setError(message);
    }
  }

  async function fullscreen(): Promise<void> {
    try {
      if (document.fullscreenElement) await document.exitFullscreen();
      else await document.documentElement.requestFullscreen();
    } catch {
      localError = 'Fullscreen is not available in this environment';
    }
  }

  onMount(() => {
    const timer = window.setInterval(() => {
      const client = session.client();
      const current = $connection.session;
      if (client && current && ['pending', 'accepted', 'connecting'].includes(current.status)) void connection.refresh(client);
    }, 2_000);
    return () => window.clearInterval(timer);
  });
</script>

<section class="session-page">
  <div class="session-toolbar">
    <div class="session-identity">
      <StatusPill state={sessionState?.status === 'connected' ? 'online' : sessionState ? 'warning' : 'neutral'} label={sessionState?.status ?? 'No session'} />
      <span>{sessionState ? `Session ${sessionState.id.slice(0, 8)}` : 'Select a computer to begin'}</span>
    </div>
    <div class="session-actions">
      <button class="button small" onclick={fullscreen}>Fullscreen</button>
      <button class="button small" disabled title="Runtime audio control is planned">Audio · Planned</button>
      <button class="button small danger" disabled={!sessionState || $connection.busy} onclick={disconnect}>Disconnect</button>
    </div>
  </div>

  <div class="stream-stage">
    {#if $connection.engineRunning}
      <div class="native-stage-message"><span class="capture-dot"></span><h2>Native stream window is active</h2><p>Input and video are handled outside the webview by the macOS engine.</p></div>
    {:else if sessionState}
      <div class="native-stage-message">
        <span class="capture-dot waiting"></span>
        <h2>{sessionState.status === 'pending' ? 'Waiting for host approval' : sessionState.status === 'accepted' ? 'Host accepted the session' : `Session ${sessionState.status}`}</h2>
        <p>{nativeReady ? 'The authenticated native endpoint is ready.' : sessionState.networkMode === 'relay' ? 'Relay requires the planned native WebRTC engine; SNV2 never runs through TURN.' : runtime.capabilities.clientEngine.reason ?? 'Waiting for a negotiated media endpoint.'}</p>
        <button class="button primary" disabled={!nativeReady || $connection.busy} onclick={startNativeClient}>Start native stream</button>
        {#if !nativeReady && sessionState.status === 'accepted'}<span class="planned-inline">Media negotiation unavailable in this build</span>{/if}
      </div>
    {:else}
      <div class="native-stage-message"><h2>No active session</h2><p>Choose an online computer from Computers. Sanser will request a route without starting capture early.</p></div>
    {/if}

    <div class="stream-stats" aria-label="Connection statistics">
      <div><span>FPS</span><strong>{metrics?.fps ?? '—'}</strong></div>
      <div><span>Bitrate</span><strong>{metrics ? `${metrics.bitrateMbps.toFixed(1)} Mb/s` : '—'}</strong></div>
      <div><span>RTT</span><strong>{metrics ? `${metrics.rttMs} ms` : '—'}</strong></div>
      <div><span>Jitter</span><strong>{metrics ? `${metrics.jitterMs} ms` : '—'}</strong></div>
      <div><span>Loss</span><strong>{metrics ? `${metrics.packetLossPercent.toFixed(2)}%` : '—'}</strong></div>
      <div><span>Input</span><strong>{metrics ? `${metrics.inputLatencyMs} ms` : '—'}</strong></div>
      <div><span>Codec</span><strong>{metrics?.codec === 'hevc' ? 'HEVC' : metrics?.codec === 'h264' ? 'H.264' : metrics?.codec === 'auto' ? 'Auto' : '—'}</strong></div>
      <div><span>Transport</span><strong>{metrics?.transport ?? (sessionState?.transport === 'snv2' ? 'SNV2' : sessionState?.transport === 'webrtc' ? 'WebRTC' : '—')}</strong></div>
    </div>
  </div>

  {#if localError ?? $connection.error}<div class="session-error notice error" role="alert">{localError ?? $connection.error}</div>{/if}
  <div class="capture-hint">Input capture is released with {$preferences.input.releaseShortcut}. The webview never forwards realtime input packets.</div>
</section>
