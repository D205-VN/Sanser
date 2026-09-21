<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '../components/Icon.svelte';
  import StatusPill from '../components/StatusPill.svelte';
  import { engineStatus, launchEngine, startRelay, stopEngine, stopRelay } from '../lib/platform';
  import { resolveStreamProfile } from '../lib/streamProfile';
  import { coordinateP2pConnection } from '../lib/p2pSignaling';
  import type { ConnectionSession, Page, RuntimeStatus } from '../lib/types';
  import { connection } from '../stores/connection';
  import { diagnostics } from '../stores/diagnostics';
  import { preferences } from '../stores/preferences';
  import { presence } from '../stores/presence';
  import { session } from '../stores/session';

  let { runtime, navigate }: { runtime: RuntimeStatus; navigate: (page: Page) => void } = $props();
  let localError = $state<string | null>(null);
  let failedP2pSessionId = $state<string | null>(null);
  let p2pRetryCount = 0;
  let p2pRetryAfter = 0;
  let p2pRetrySessionId: string | null = null;
  let componentActive = false;
  let refreshInFlight = false;
  let nativePreparationInFlight = $state(false);
  let negotiationController: AbortController | null = null;
  let disconnecting = $state(false);
  let phase = $state<'idle' | 'authorizing' | 'direct' | 'relay' | 'launching'>('idle');
  const phaseLabels = { idle: 'Waiting for approval', authorizing: 'Preparing connection', direct: 'Connecting directly', relay: 'Connecting through relay', launching: 'Opening the remote window' };
  let activeRoute = $state<'direct' | 'relay' | null>(null);

  const sessionState = $derived($connection.session);
  const metrics = $derived($connection.metrics);
  const nativeReady = $derived(
    sessionState?.status === 'accepted' &&
      runtime.capabilities.clientEngine.state === 'available' &&
      runtime.capabilities.nativeDirect.state === 'available' &&
      sessionState.transport === 'native'
  );

  function resolutionSize(resolution: string): { width: number; height: number } {
    switch (resolution) {
      case '720p': return { width: 1280, height: 720 };
      case '1440p': return { width: 2560, height: 1440 };
      case '2160p': return { width: 3840, height: 2160 };
      default: return { width: 1920, height: 1080 };
    }
  }

  function isAborted(signal: AbortSignal): boolean { return signal.aborted; }

  function udpEndpoint(address: string, port: number): string {
    return address.includes(':') ? `[${address}]:${port}` : `${address}:${port}`;
  }

  async function launchNativeClient(target: ConnectionSession, signal: AbortSignal): Promise<boolean> {
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
    const stream = resolveStreamProfile($preferences.stream, target.qualityProfile);
    const size = resolutionSize(stream.resolution);
    connection.setBusy(true);
    localError = null;
    try {
      const localDeviceId = $presence.deviceId;
      const peerDeviceId = target.hostDeviceId;
      if (!localDeviceId || !peerDeviceId) throw new Error('Missing device IDs for P2P connection');

      diagnostics.add({ level: 'info', category: 'session', message: 'Starting P2P NAT Traversal...' });
      const client = session.client();
      if (!client) throw new Error('Api client is unavailable');
      let p2pResult: Awaited<ReturnType<typeof coordinateP2pConnection>> | null = null;
      let relayResult: Awaited<ReturnType<typeof startRelay>> | null = null;
      try {
        phase = 'direct';
        if (target.networkMode === 'relay') throw new Error('Relay-only mode selected');
        p2pResult = await coordinateP2pConnection(
          client,
          target.id,
          localDeviceId,
          peerDeviceId,
          false, // client is controlled
          target.sessionToken,
          undefined,
          signal
        );
        diagnostics.add({ level: 'info', category: 'session', message: `P2P Hole Punching success! Local port: ${p2pResult.localPort}` });
      } catch (directError) {
        if (signal.aborted || target.networkMode === 'direct') throw directError;
        phase = 'relay';
        const accessToken = await client.getAccessToken();
        if (isAborted(signal)) return false;
        if (!accessToken) throw new Error('Relay fallback requires an authenticated access token');
        diagnostics.add({
          level: 'warn',
          category: 'network',
          message: `Direct P2P failed; switching to encrypted Sanser relay: ${directError instanceof Error ? directError.message : String(directError)}`
        });
        relayResult = await startRelay({
          kind: 'client',
          serverUrl: client.serverUrl,
          sessionId: target.id,
          deviceId: localDeviceId,
          peerDeviceId,
          accessToken,
          sessionCredential: target.sessionToken
        });
        diagnostics.add({ level: 'info', category: 'network', message: 'Encrypted relay route is ready' });
      }

      if (signal.aborted || !componentActive || $connection.session?.id !== target.id || $session.mode !== 'cloud') {
        if (relayResult) await stopRelay('client').catch(() => undefined);
        return false;
      }
      phase = 'launching';
      await launchEngine({
        kind: 'client',
        wireProtocol: target.wireProtocol,
        sessionId: target.id,
        address: relayResult ? '127.0.0.1' : p2pResult?.remoteAddress,
        port: relayResult?.enginePort ?? p2pResult?.localPort,
        codec: target.requestedCodec,
        fps: stream.fps,
        bitrateKbps: Math.round(stream.bitrateMbps * 1_000),
        width: size.width,
        height: size.height,
        networkMode: target.networkMode,
        audioEnabled: target.wireProtocol !== 'snv2' && $preferences.host.audioEnabled,
        inputEnabled: $preferences.host.inputEnabled,
        relativeMouse: target.wireProtocol !== 'snv2' && $preferences.input.mouseMode === 'relative',
        sessionToken: target.sessionToken,
        udpConnect: relayResult
          ? `127.0.0.1:${relayResult.proxyPort}`
          : p2pResult
            ? udpEndpoint(p2pResult.remoteAddress, p2pResult.remotePort)
            : undefined,
        relay: relayResult !== null
      });
      // Async engine startup can outlive sign-out. Recheck the current session.
      // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
      if (signal.aborted || !componentActive || $connection.session?.id !== target.id) {
        await stopEngine('client').catch(() => undefined);
        return false;
      }
      connection.setEngineRunning(true);
      activeRoute = relayResult ? 'relay' : 'direct';
      failedP2pSessionId = null;
      p2pRetryCount = 0;
      p2pRetryAfter = 0;
      p2pRetrySessionId = target.id;
      diagnostics.add({ level: 'info', category: 'engine', message: 'Native client started' });
      return true;
    } catch (error) {
      await stopRelay('client').catch(() => undefined);
      if (signal.aborted || $connection.session?.id !== target.id) return false;
      const message = error instanceof Error ? error.message : 'Unable to start native client';
      p2pRetryCount += 1;
      p2pRetryAfter = Date.now() + Math.min(1_500 * p2pRetryCount, 5_000);
      failedP2pSessionId = p2pRetryCount >= 3 ? target.id : null;
      localError = message;
      connection.setError(message);
      return false;
    } finally {
      if ($connection.session?.id === target.id && !signal.aborted) connection.setBusy(false);
    }
  }

  async function startNativeClient(target = sessionState): Promise<void> {
    const client = session.client();
    const localDeviceId = $presence.deviceId;
    if (!target || !client || !localDeviceId || nativePreparationInFlight || $connection.engineRunning) return;
    if (p2pRetrySessionId !== target.id) {
      p2pRetrySessionId = target.id;
      p2pRetryCount = 0;
      p2pRetryAfter = 0;
      failedP2pSessionId = null;
    }
    nativePreparationInFlight = true;
    connection.setPreparing(true);
    const controller = new AbortController();
    negotiationController = controller;
    phase = 'authorizing';
    try {
      const ready = await client.markNativeReady(target.id, localDeviceId);
      if (controller.signal.aborted) return;
      const credentials = await client.sessionCredentials(target.id, localDeviceId);
      if (isAborted(controller.signal) || !componentActive || $connection.session?.id !== target.id || $session.mode !== 'cloud') return;
      connection.authorizeNative(credentials);
      diagnostics.add({
        level: 'info',
        category: 'session',
        message: 'The client opened a fresh native negotiation generation',
        details: { sessionId: target.id, requesterReadyAt: ready.requesterReadyAt ?? null }
      });
      await launchNativeClient({
        ...target,
        requesterReadyAt: ready.requesterReadyAt,
        address: credentials.peerRouteAddress,
        port: credentials.basePort,
        sessionToken: credentials.sessionToken,
        wireProtocol: credentials.wireProtocol,
        credentialExpiresAt: credentials.expiresAt
      }, controller.signal);
    } catch (error) {
      if (isAborted(controller.signal) || $connection.session?.id !== target.id) return;
      p2pRetryCount += 1;
      p2pRetryAfter = Date.now() + Math.min(1_500 * p2pRetryCount, 5_000);
      failedP2pSessionId = p2pRetryCount >= 3 ? target.id : null;
      const message = error instanceof Error ? error.message : 'Unable to prepare a fresh native session';
      localError = message;
      connection.setError(message);
    } finally {
      nativePreparationInFlight = false;
      connection.setPreparing(false);
      if (negotiationController === controller) negotiationController = null;
      phase = 'idle';
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
    if (!sessionState || disconnecting) return;
    const targetId = sessionState.id;
    disconnecting = true;
    failedP2pSessionId = targetId;
    p2pRetryCount = 3;
    negotiationController?.abort();
    connection.setBusy(true);
    localError = null;
    try {
      if (runtime.capabilities.desktopShell.state === 'available') {
        await stopEngine('client');
        await stopRelay('client');
      }
      connection.setEngineRunning(false);
      activeRoute = null;
      const client = session.client();
      if (client) await client.disconnectSession(targetId);
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
    } finally {
      disconnecting = false;
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
          const message = status.lastError ?? 'The native client stopped unexpectedly';
          await stopEngine('client').catch(() => undefined);
          connection.setEngineRunning(false);
          activeRoute = null;
          failedP2pSessionId = current.id;
          p2pRetrySessionId = current.id;
          p2pRetryCount = Math.max(p2pRetryCount, 3);
          connection.setError(message);
          diagnostics.add({
            level: 'warn',
            category: 'engine',
            message: `${message}; the accepted session remains available for retry`
          });
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
        activeRoute = null;
        return;
      }
      if (
        refreshed?.status === 'accepted' &&
        refreshed.transport === 'native' &&
        localDeviceId &&
        !$connection.engineRunning &&
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
      if (current && ['pending', 'accepted', 'connecting', 'connected'].includes(current.status)) void refreshSession();
    }, 2_000);
    return () => {
      componentActive = false;
      negotiationController?.abort();
      window.clearInterval(timer);
    };
  });
</script>

<section class="session-page">
  {#if sessionState}<div class="session-toolbar">
    <div class="session-identity">
      <StatusPill state={$connection.engineRunning ? 'online' : 'warning'} label={$connection.engineRunning ? 'Window open' : sessionState.status} />

    </div>
    <div class="session-actions">

      <button class="button small danger" disabled={disconnecting} onclick={disconnect}>{disconnecting ? 'Disconnecting…' : nativePreparationInFlight ? 'Cancel connection' : 'Disconnect'}</button>
    </div>
  </div>{/if}

  {#if sessionState}
    <ol class="connection-steps" aria-label="Connection progress">
      <li class:complete={['accepted', 'connecting', 'connected'].includes(sessionState.status)}><span>01</span><div><strong>Approval</strong><small>{sessionState.status === 'pending' ? 'Waiting for the host' : sessionState.status === 'accepted' ? 'Host accepted' : sessionState.status}</small></div></li>
      <li class:complete={activeRoute !== null} class:current={nativePreparationInFlight}><span>02</span><div><strong>Connection</strong><small>{nativePreparationInFlight ? phaseLabels[phase] : activeRoute === 'relay' ? 'Relay connection' : activeRoute === 'direct' ? 'Direct connection' : 'Waiting'}</small></div></li>
      <li class:complete={metrics !== null}><span>03</span><div><strong>Remote desktop</strong><small>{metrics ? 'Screen data received' : $connection.engineRunning ? 'Check the remote window' : 'Not started'}</small></div></li>
    </ol>
  {/if}
  {#if sessionState?.wireProtocol === 'snv2'}
    <div class="notice">Sound is unavailable for this connection.{#if $preferences.input.mouseMode === 'relative'} Relative mouse capture is also unavailable.{/if}</div>
  {/if}
  <div class="stream-stage" class:has-metrics={metrics !== null}>
    {#if $connection.engineRunning}
      <div class="native-stage-message"><span class="capture-dot"></span><h2>Remote window opened</h2><p>Switch to the remote window to view and control the computer. If the screen stays blank, check connection details.</p><button class="button" onclick={() => navigate('diagnostics')}>View connection details</button></div>
    {:else if sessionState}
      <div class="native-stage-message">
        <span class="capture-dot waiting"></span>
        <h2>{nativePreparationInFlight ? phaseLabels[phase] : sessionState.status === 'pending' ? 'Waiting for host approval' : sessionState.status === 'accepted' ? 'Host accepted the session' : `Session ${sessionState.status}`}</h2>
        <p>{nativeReady ? 'Open the remote desktop to begin.' : runtime.capabilities.nativeDirect.reason ?? runtime.capabilities.clientEngine.reason ?? 'Waiting for the other computer.'}</p>
        <button class="button primary" disabled={!nativeReady || $connection.busy || nativePreparationInFlight} onclick={retryNativeClient}>{nativePreparationInFlight ? 'Connecting…' : failedP2pSessionId === sessionState.id ? 'Retry connection' : 'Open remote desktop'}</button>
        {#if !nativeReady && sessionState.status === 'accepted'}<span class="planned-inline">This connection is unavailable. Check Settings → Diagnostics for details.</span>{/if}
      </div>
    {:else}
      <div class="native-stage-message"><div class="session-empty-icon"><Icon name="monitor" /></div><h2>No active session</h2><p>Choose an online computer to get started.</p><button class="button primary" onclick={() => navigate('computers')}>Browse computers →</button></div>
    {/if}

    {#if metrics}<div class="stream-stats" aria-label="Connection statistics">
      <div><span>Frame rate</span><strong>{metrics.fps} FPS</strong></div>
      <div><span>Bitrate</span><strong>{metrics.bitrateMbps.toFixed(1)} Mb/s</strong></div>
      <div><span>Latency</span><strong>{metrics.rttMs} ms</strong></div>
      <div><span>Packet loss</span><strong>{metrics.packetLossPercent.toFixed(2)}%</strong></div>
    </div>{/if}
  </div>

  {#if localError ?? $connection.error}<div class="session-error notice error" role="alert">{localError ?? $connection.error}</div>{/if}
</section>
