<script lang="ts">
  import { onMount } from 'svelte';
  import StatusPill from '../components/StatusPill.svelte';
  import { engineStatus, launchEngine, stopEngine } from '../lib/platform';
  import { coordinateP2pConnection } from '../lib/p2pSignaling';
  import type { ConnectionSession, RuntimeStatus } from '../lib/types';
  import { connection } from '../stores/connection';
  import { diagnostics } from '../stores/diagnostics';
  import { preferences } from '../stores/preferences';
  import { presence } from '../stores/presence';
  import { session } from '../stores/session';

  let { runtime }: { runtime: RuntimeStatus } = $props();
  let localError = $state<string | null>(null);
  let failedP2pSessionId = $state<string | null>(null);
  let p2pRetryCount = 0;
  let p2pRetryAfter = 0;
  let p2pRetrySessionId: string | null = null;
  let componentActive = false;
  let refreshInFlight = false;
  let nativePreparationInFlight = $state(false);

  const sessionState = $derived($connection.session);
  const metrics = $derived($connection.metrics);
  const nativeReady = $derived(
    sessionState?.status === 'accepted' &&
      runtime.capabilities.clientEngine.state === 'available' &&
      runtime.capabilities.nativeDirect.state === 'available' &&
      sessionState.transport === 'native' &&
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

  function udpEndpoint(address: string, port: number): string {
    return address.includes(':') ? `[${address}]:${port}` : `${address}:${port}`;
  }

  async function launchNativeClient(target: ConnectionSession): Promise<boolean> {
    if (
      $connection.engineRunning ||
      $connection.busy ||
      target.status !== 'accepted' ||
      target.transport !== 'native' ||
      !target.address ||
      !target.port ||
      !target.sessionToken
    ) return false;
    if (p2pRetrySessionId !== target.id) {
      p2pRetrySessionId = target.id;
      p2pRetryCount = 0;
      p2pRetryAfter = 0;
      failedP2pSessionId = null;
    }
    const size = resolutionSize();
    connection.setBusy(true);
    localError = null;
    try {
      const localDeviceId = $presence.deviceId;
      const peerDeviceId = target.hostDeviceId;
      if (!localDeviceId || !peerDeviceId) throw new Error('Missing device IDs for P2P connection');

      diagnostics.add({ level: 'info', category: 'session', message: 'Starting P2P NAT Traversal...' });
      const client = session.client();
      if (!client) throw new Error('Api client is unavailable');
      const p2pResult = await coordinateP2pConnection(
        client,
        target.id,
        localDeviceId,
        peerDeviceId,
        false, // client is controlled
        target.sessionToken
      );
      diagnostics.add({ level: 'info', category: 'session', message: `P2P Hole Punching success! Local port: ${p2pResult.localPort}` });

      await launchEngine({
        kind: 'client',
        sessionId: target.id,
        address: p2pResult.remoteAddress,
        port: p2pResult.localPort,
        codec: $preferences.stream.codec,
        fps: $preferences.stream.fps,
        bitrateKbps: Math.round($preferences.stream.bitrateMbps * 1_000),
        width: size.width,
        height: size.height,
        networkMode: target.networkMode,
        audioEnabled: $preferences.host.audioEnabled,
        inputEnabled: $preferences.host.inputEnabled,
        relativeMouse: $preferences.input.mouseMode === 'relative',
        sessionToken: target.sessionToken,
        udpConnect: udpEndpoint(p2pResult.remoteAddress, p2pResult.remotePort)
      });
      if (!componentActive || $connection.session?.id !== target.id || $session.mode === 'signedOut') {
        await stopEngine('client').catch(() => undefined);
        return false;
      }
      connection.setEngineRunning(true);
      failedP2pSessionId = null;
      p2pRetryCount = 0;
      p2pRetryAfter = 0;
      p2pRetrySessionId = target.id;
      diagnostics.add({ level: 'info', category: 'engine', message: 'Native macOS client started' });
      return true;
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Unable to start native client';
      p2pRetryCount += 1;
      p2pRetryAfter = Date.now() + Math.min(1_500 * p2pRetryCount, 5_000);
      failedP2pSessionId = p2pRetryCount >= 3 ? target.id : null;
      localError = message;
      connection.setError(message);
      return false;
    } finally {
      connection.setBusy(false);
    }
  }

  async function startNativeClient(target = sessionState): Promise<void> {
    const client = session.client();
    const localDeviceId = $presence.deviceId;
    if (!target || !client || !localDeviceId || nativePreparationInFlight) return;
    nativePreparationInFlight = true;
    try {
      const ready = await client.markNativeReady(target.id, localDeviceId);
      const credentials = await client.sessionCredentials(target.id, localDeviceId);
      connection.authorizeNative(credentials);
      diagnostics.add({
        level: 'info',
        category: 'session',
        message: 'macOS opened a fresh native negotiation generation',
        details: { sessionId: target.id, requesterReadyAt: ready.requesterReadyAt ?? null }
      });
      await launchNativeClient({
        ...target,
        requesterReadyAt: ready.requesterReadyAt,
        address: credentials.peerRouteAddress,
        port: credentials.basePort,
        sessionToken: credentials.sessionToken,
        credentialExpiresAt: credentials.expiresAt
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Unable to prepare a fresh native session';
      localError = message;
      connection.setError(message);
    } finally {
      nativePreparationInFlight = false;
    }
  }

  async function retryNativeClient(): Promise<void> {
    failedP2pSessionId = null;
    p2pRetryCount = 0;
    p2pRetryAfter = 0;
    p2pRetrySessionId = sessionState?.id ?? null;
    localError = null;
    connection.setError(null);
    await startNativeClient();
  }

  async function disconnect(): Promise<void> {
    if (!sessionState) return;
    connection.setBusy(true);
    localError = null;
    try {
      if ($connection.engineRunning) await stopEngine('client');
      const client = session.client();
      if (client) await client.disconnectSession(sessionState.id);
      failedP2pSessionId = null;
      p2pRetryCount = 0;
      p2pRetryAfter = 0;
      p2pRetrySessionId = null;
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

  async function refreshSession(): Promise<void> {
    if (!componentActive || refreshInFlight) return;
    const client = session.client();
    const current = $connection.session;
    const localDeviceId = $presence.deviceId;
    if (!client || !current || $connection.busy) return;
    refreshInFlight = true;
    try {
      if ($connection.engineRunning) {
        const status = await engineStatus('client');
        // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
        if (!componentActive || $connection.session?.id !== current.id) return;
        if (!status.running) {
          const message = status.lastError ?? 'The native macOS client stopped unexpectedly';
          connection.setEngineRunning(false);
          connection.setError(message);
          await client.disconnectSession(current.id).catch(() => undefined);
          diagnostics.add({ level: 'warn', category: 'engine', message });
          return;
        }
      }

      const refreshed = await connection.refresh(client);
      // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
      if (!componentActive || $connection.session?.id !== current.id) return;
      if (
        refreshed &&
        ['rejected', 'disconnected', 'expired', 'closed', 'failed'].includes(refreshed.status) &&
        $connection.engineRunning
      ) {
        await stopEngine('client').catch(() => undefined);
        connection.setEngineRunning(false);
        return;
      }
      if (
        refreshed?.status === 'accepted' &&
        refreshed.transport === 'native' &&
        localDeviceId &&
        failedP2pSessionId !== refreshed.id &&
        Date.now() >= p2pRetryAfter
      ) {
        await startNativeClient(refreshed);
      }
    } catch (error) {
      connection.setError(error instanceof Error ? error.message : 'Unable to refresh the native session');
    } finally {
      refreshInFlight = false;
    }
  }

  onMount(() => {
    componentActive = true;
    const timer = window.setInterval(() => {
      const current = $connection.session;
      if (current && ['pending', 'accepted', 'connecting'].includes(current.status)) void refreshSession();
    }, 2_000);
    return () => {
      componentActive = false;
      window.clearInterval(timer);
    };
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
      <button class="button small danger" disabled={!sessionState || $connection.busy || nativePreparationInFlight} onclick={disconnect}>Disconnect</button>
    </div>
  </div>

  <div class="stream-stage">
    {#if $connection.engineRunning}
      <div class="native-stage-message"><span class="capture-dot"></span><h2>Native stream window is active</h2><p>Input and video are handled outside the webview by the macOS engine.</p></div>
    {:else if sessionState}
      <div class="native-stage-message">
        <span class="capture-dot waiting"></span>
        <h2>{sessionState.status === 'pending' ? 'Waiting for host approval' : sessionState.status === 'accepted' ? 'Host accepted the session' : `Session ${sessionState.status}`}</h2>
        <p>{nativeReady ? 'Ready to create a fresh authenticated native endpoint.' : sessionState.networkMode === 'relay' ? 'Relay requires the planned native WebRTC engine; native direct never runs through TURN.' : runtime.capabilities.nativeDirect.reason ?? runtime.capabilities.clientEngine.reason ?? 'Waiting for a negotiated media endpoint.'}</p>
        <button class="button primary" disabled={!nativeReady || $connection.busy || nativePreparationInFlight} onclick={retryNativeClient}>{nativePreparationInFlight ? 'Preparing fresh session…' : failedP2pSessionId === sessionState.id ? 'Retry native stream' : 'Start native stream'}</button>
        {#if !nativeReady && sessionState.status === 'accepted'}<span class="planned-inline">Native transport is unavailable on one endpoint</span>{/if}
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
      <div><span>Transport</span><strong>{metrics?.transport ?? (sessionState?.transport === 'native' ? 'Native direct' : sessionState?.transport === 'webrtc' ? 'WebRTC' : '—')}</strong></div>
    </div>
  </div>

  {#if localError ?? $connection.error}<div class="session-error notice error" role="alert">{localError ?? $connection.error}</div>{/if}
  <div class="capture-hint">Input capture is released with {$preferences.input.releaseShortcut}. The webview never forwards realtime input packets.</div>
</section>
