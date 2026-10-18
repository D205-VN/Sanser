<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '../components/Icon.svelte';
  import StatusPill from '../components/StatusPill.svelte';
  import { engineStatus, launchEngine, startRelay, stopEngine, stopRelay, routeRecoveryPlan, type RouteRecoveryEvidence } from '../lib/platform';
  import { resolveStreamProfile, resolveStreamSize } from '../lib/streamProfile';
  import { ApiError } from '../lib/api';
  import { coordinateP2pConnection, directCheckFailure, releaseP2pConnection } from '../lib/p2pSignaling';
  import type { ConnectionSession, Page, RuntimeStatus } from '../lib/types';
  import { connection } from '../stores/connection';
  import { diagnostics } from '../stores/diagnostics';
  import { nativePerformance } from '../stores/nativePerformance';
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
  let closedViewerSessionId: string | null = null;
  let recoveryInFlight = $state(false);
  let phase = $state<'idle' | 'authorizing' | 'direct' | 'relay' | 'launching'>('idle');
  const phaseLabels = { idle: 'Waiting for approval', authorizing: 'Preparing connection', direct: 'Connecting directly', relay: 'Connecting through relay', launching: 'Opening the remote window' };
  const activeRoute = $derived($connection.nativeRoute);

  const sessionState = $derived($connection.session);
  const metrics = $derived($connection.metrics);
  const sharingWithoutSound = $derived(sessionState?.wireProtocol === 'snv2' && !!sessionState.sharingId);
  const soundUnavailable = $derived(sessionState?.wireProtocol === 'snv2' && !sharingWithoutSound && runtime.capabilities.clientAudio !== true);
  const nativeReady = $derived(
    sessionState?.status === 'accepted' &&
      runtime.capabilities.clientEngine.state === 'available' &&
      runtime.capabilities.nativeDirect.state === 'available' &&
      sessionState.transport === 'native'
  );

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
    const size = resolveStreamSize(stream.resolution, target.wireProtocol);
    connection.setBusy(true);
    localError = null;
    let selectedP2p: Awaited<ReturnType<typeof coordinateP2pConnection>> | null = null;
    let engineLaunched = false;
    try {
      const localDeviceId = $presence.deviceId;
      const peerDeviceId = target.hostDeviceId;
      if (!localDeviceId || !peerDeviceId) throw new Error('Missing device IDs for P2P connection');

      diagnostics.add({ level: 'info', category: 'session', message: 'Starting P2P NAT Traversal...' });
      const client = session.client();
      if (!client) throw new Error('Api client is unavailable');
      let p2pResult: Awaited<ReturnType<typeof coordinateP2pConnection>> | null = null;
      let directCheck: import('../lib/types').LaunchEngineRequest['directCheck'] = 'passed';
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
          signal,
          target.requesterReadyAt
        );
        selectedP2p = p2pResult;
        diagnostics.add({ level: 'info', category: 'session', message: `P2P Hole Punching success! Local port: ${p2pResult.localPort}` });
      } catch (directError) {
        directCheck = directCheckFailure(directError, target.networkMode === 'relay');
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
        diagnostics.add({ level: 'info', category: 'network', message: `Encrypted relay route is ready (${relayResult.transport === 'udp' ? 'UDP datagrams' : 'WSS/TCP fallback'})` });
      }

      if (signal.aborted || !componentActive || $connection.session?.id !== target.id || $session.mode !== 'cloud') {
        if (selectedP2p) await releaseP2pConnection(selectedP2p).catch(() => undefined);
        if (relayResult) await stopRelay('client', target.id).catch(() => undefined);
        return false;
      }
      phase = 'launching';
      connection.markNativeLaunch();
      await launchEngine({
        ultraLowLatency: stream.profile === 'competitive',
        directCheck,
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
        audioEnabled: $preferences.host.audioEnabled &&
          !target.sharingId &&
          (target.wireProtocol !== 'snv2' || runtime.capabilities.clientAudio === true),
        inputEnabled: target.accessMode !== 'view-only' && $preferences.host.inputEnabled,
        relativeMouse: target.accessMode !== 'view-only' && target.wireProtocol !== 'snv2' && $preferences.input.mouseMode === 'relative',
        sessionToken: target.sessionToken,
        udpConnect: relayResult
          ? `127.0.0.1:${relayResult.proxyPort}`
          : p2pResult
            ? udpEndpoint(p2pResult.remoteAddress, p2pResult.remotePort)
            : undefined,
        relay: relayResult !== null
      });
      engineLaunched = true;
      // Async engine startup can outlive sign-out. Recheck the current session.
      // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
      if (signal.aborted || !componentActive || $connection.session?.id !== target.id) {
        await stopEngine('client', target.id).catch(() => undefined);
        await stopRelay('client', target.id).catch(() => undefined);
        return false;
      }
      connection.setEngineRunning(true);
      connection.setNativeRoute(relayResult ? 'relay' : 'direct');
      failedP2pSessionId = null;
      p2pRetryCount = 0;
      p2pRetryAfter = 0;
      p2pRetrySessionId = target.id;
      diagnostics.add({ level: 'info', category: 'engine', message: 'Native client started' });
      return true;
    } catch (error) {
      if (selectedP2p && !engineLaunched) await releaseP2pConnection(selectedP2p).catch(() => undefined);
      await stopRelay('client', target.id).catch(() => undefined);
      if (signal.aborted || $connection.session?.id !== target.id) return false;
      const message = error instanceof Error ? error.message : 'Unable to start native client';
      p2pRetryCount += 1;
      p2pRetryAfter = Date.now() + Math.min(1_500 * p2pRetryCount, 5_000);
      const exhausted = p2pRetryCount >= 3 || $connection.nativeStartupRetries > 0 || $connection.nativeRecoveryPending;
      failedP2pSessionId = exhausted ? target.id : null;
      if (exhausted) connection.blockNativeRetry();
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
    if (!target || !client || !localDeviceId || nativePreparationInFlight || $connection.busy || $connection.engineRunning || $connection.nativeRetryBlocked || Date.now() < $connection.nativeRecoveryRetryAt) return;
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
      if ($connection.nativeRecoveryPending && Date.now() > $connection.nativeRecoveryRetryAt + 30_000) {
        connection.blockNativeRetry();
        connection.setError('The server did not recover in time. Retry the connection manually.');
        return;
      }
      connection.authorizeNative(credentials, ready.requesterReadyAt);
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
        credentialExpiresAt: credentials.expiresAt,
        accessMode: target.accessMode === 'view-only' ? 'view-only' : credentials.accessMode ?? target.accessMode
      }, controller.signal);
    } catch (error) {
      if (isAborted(controller.signal) || $connection.session?.id !== target.id) return;
      p2pRetryCount += 1;
      p2pRetryAfter = Date.now() + Math.min(1_500 * p2pRetryCount, 5_000);
      if ($connection.nativeRecoveryPending && p2pRetryCount < 3 &&
          Date.now() <= $connection.nativeRecoveryRetryAt + 30_000 &&
          error instanceof ApiError && [0, 408, 429, 502, 503, 504].includes(error.status)) {
        localError = 'Waiting for the server to recover before authorizing a fresh connection…';
        connection.setError(localError);
        return;
      }
      const exhausted = p2pRetryCount >= 3 || $connection.nativeStartupRetries > 0 || $connection.nativeRecoveryPending;
      failedP2pSessionId = exhausted ? target.id : null;
      if (exhausted) connection.blockNativeRetry();
      const message = error instanceof Error ? error.message : 'Unable to prepare a fresh native session';
      localError = message;
      connection.setError(message);
    } finally {
      nativePreparationInFlight = false;
      connection.setPreparing(false);
      if (controller.signal.aborted && !disconnecting && $connection.session?.id === target.id) {
        connection.setBusy(false);
      }
      if (negotiationController === controller) negotiationController = null;
      phase = 'idle';
    }
  }

  async function retryNativeClient(): Promise<void> {
    connection.resetNativeRetry();
    failedP2pSessionId = null;
    p2pRetryCount = 0;
    p2pRetryAfter = 0;
    p2pRetrySessionId = sessionState?.id ?? null;
    localError = null;
    connection.setError(null);
    await startNativeClient();
  }

  function liveRecoveryEvidence(): Omit<RouteRecoveryEvidence, 'automaticAttempts'> | null {
    const update = $nativePerformance.client;
    const current = $connection.session;
    const launched = $connection.nativeLaunchStartedAtMs;
    if (!current || launched === null || !update || update.ended || update.sessionId !== current.id || update.startedAtMs < launched) return null;
    const now = Date.now();
    const stableMs = Math.max(0, now - launched);
    const silence = update.performance.metrics.controlSilence;
    if (silence && silence.latestMs >= 15_000 && now >= silence.updatedAtMs) {
      return { cause: 'control-unresponsive', stableMs, evidenceAgeMs: now - silence.updatedAtMs, controlSilenceMs: silence.latestMs };
    }
    const wire = update.performance.metrics.wireRttEstimate;
    const health = update.routeHealth;
    if (update.route === 'direct' && health?.status === 'elevated-after-media' && wire && now >= wire.updatedAtMs) {
      return { cause: 'sustained-wire-delay', stableMs, evidenceAgeMs: now - wire.updatedAtMs,
        probeSamples: health.probe.samples, probeMedianMs: health.probe.medianMs,
        wireRttMs: wire.latestMs, elevatedSamples: health.consecutiveElevated };
    }
    return null;
  }

  async function recoverRoute(target: ConnectionSession, evidence: Omit<RouteRecoveryEvidence, 'automaticAttempts'> | null, manual = false): Promise<boolean> {
    if (recoveryInFlight || nativePreparationInFlight || $connection.busy || disconnecting || (!manual && ($connection.nativeRetryBlocked || $connection.nativeRecoveryPending || $connection.nativeRouteRecoveries >= 2))) return false;
    const api = session.client();
    const ownsSession = () => !disconnecting && $session.mode === 'cloud' && $connection.session?.id === target.id;
    const stillCurrent = () => componentActive && ownsSession();
    if (!api || !stillCurrent()) return false;
    recoveryInFlight = true;
    let cleanupOwned = false;
    try {
      // Keep a working route until consent is confirmed. A dead route may wait
      // through a brief server restart; no engine is relaunched before fresh
      // markNativeReady/credentials calls succeed. Never create/accept a session.
      const confirmed = await connection.refresh(api);
      const lost = evidence?.cause === 'relay-lost' || evidence?.cause === 'peer-timeout';
      if (!stillCurrent() || (confirmed && confirmed.status !== 'accepted') || (!confirmed && (!lost || manual))) return false;
      const plan = manual ? { action: 'reconnect' as const, delayMs: 0, reason: 'manual-reconnect' }
        : evidence ? await routeRecoveryPlan({ ...evidence, automaticAttempts: $connection.nativeRouteRecoveries }) : null;
      if (!stillCurrent() || (!manual && $connection.nativeRetryBlocked) || plan?.action !== 'reconnect') return false;
      if (manual) connection.resetNativeRetry();
      if (!connection.scheduleRouteRecovery(plan.delayMs, plan.reason, !manual)) return false;
      connection.setBusy(true);
      cleanupOwned = true;
      localError = null;
      await stopEngine('client', target.id);
      if (!ownsSession()) return false;
      await stopRelay('client', target.id);
      if (!ownsSession()) return false;
      connection.setEngineRunning(false);
      connection.setNativeRoute(null);
      p2pRetrySessionId = target.id; p2pRetryCount = 0; failedP2pSessionId = null;
      p2pRetryAfter = $connection.nativeRecoveryRetryAt;
      diagnostics.add({ level: 'warn', category: 'network', message: 'Reconnecting the accepted session with fresh credentials and candidates; the remote window will reopen.',
        details: { reason: plan.reason, automatic: !manual, attempt: $connection.nativeRouteRecoveries } });
      connection.setBusy(false);
      if (manual && componentActive && confirmed) await startNativeClient(confirmed);
      return true;
    } catch (error) {
      if (ownsSession()) {
        connection.blockNativeRetry();
        localError = error instanceof Error ? error.message : 'Unable to reconnect; retry manually';
        connection.setError(localError);
      }
      return false;
    } finally {
      recoveryInFlight = false;
      if (cleanupOwned && ownsSession() && !$connection.preparing) connection.setBusy(false);
    }
  }

  async function disconnect(): Promise<void> {
    if (!sessionState || disconnecting) return;
    const targetId = sessionState.id;
    disconnecting = true;
    connection.blockNativeRetry();
    failedP2pSessionId = targetId;
    p2pRetryCount = 3;
    negotiationController?.abort();
    connection.setBusy(true);
    localError = null;
    try {
      if (runtime.capabilities.desktopShell.state === 'available') {
        // A failed local cleanup must not prevent revoking server consent.
        await Promise.allSettled([stopEngine('client', targetId), stopRelay('client', targetId)]);
      }
      if ($connection.session?.id !== targetId) {
        const client = session.client();
        if (client) await client.disconnectSession(targetId);
        return;
      }
      connection.setEngineRunning(false);
      connection.setNativeRoute(null);
      const client = session.client();
      if (client) await client.disconnectSession(targetId);
      // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
      if ($connection.session?.id !== targetId) return;
      failedP2pSessionId = null;
      p2pRetryCount = 0;
      p2pRetryAfter = 0;
      p2pRetrySessionId = null;
      connection.clear();
      closedViewerSessionId = null;
      diagnostics.add({ level: 'info', category: 'session', message: 'Session disconnected' });
    } catch (error) {
      if ($connection.session?.id !== targetId) return;
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
      if (closedViewerSessionId === current.id) { await disconnect(); return; }
      if ($connection.nativeRecoveryPending && Date.now() > $connection.nativeRecoveryRetryAt + 30_000) {
        connection.blockNativeRetry();
        connection.setError('The server did not recover in time. Retry the connection manually.');
        return;
      }
      if ($connection.engineRunning) {
        const status = await engineStatus('client');
        if (status.sessionId && status.sessionId !== current.id) return;
        if (status.running && status.videoReceived && current.sharingId && current.requesterReadyAt && localDeviceId) {
          await client.markSharingStarted(current.id, localDeviceId, current.requesterReadyAt);
        }
        // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
        if (!componentActive || $connection.session?.id !== current.id) return;
        if (!status.running) {
          // A deliberate viewer X closes only its server session. This page
          // remains mounted across navigation; closing the main shell to tray
          // never emits the native viewer marker.
          if (status.userClosed && status.sessionId === current.id) { closedViewerSessionId = current.id; await disconnect(); return; }
          const message = status.lastError ?? 'The native client stopped unexpectedly';
          const stillCurrent = () => componentActive && !disconnecting && $connection.session?.id === current.id;
          if (status.routeFailure && status.startupFailure !== 'auth-rejected' && status.startupFailure !== 'decode-failed' &&
            await recoverRoute(current, { cause: status.routeFailure, stableMs: 0, evidenceAgeMs: 0 })) return;
          // Recheck after asynchronous recovery policy/consent validation.
          if (!stillCurrent()) return;
          await stopEngine('client', current.id);
          if (!stillCurrent()) return;
          await stopRelay('client', current.id);
          // Cleanup can outlive Disconnect or navigation to another session.
          if (!stillCurrent()) return;
          connection.setEngineRunning(false);
          connection.setNativeRoute(null);
          p2pRetrySessionId = current.id;
          const retryable = status.startupFailure !== undefined && status.startupFailure !== null &&
            ['no-udp', 'unexpected-peer', 'no-video', 'incomplete-video'].includes(status.startupFailure);
          const retry = retryable && connection.takeNativeStartupRetry();
          if (retry) {
            failedP2pSessionId = null;
            p2pRetryCount = 0;
            p2pRetryAfter = Date.now() + 1_500;
          } else {
            connection.blockNativeRetry();
            failedP2pSessionId = current.id;
            p2pRetryCount = Math.max(p2pRetryCount, 3);
          }
          localError = retry ? `${message} Retrying once with a fresh connection…`
            : status.routeFailure && $connection.nativeRouteRecoveries >= 2 ? `${message} Automatic reconnection stopped after two attempts. Retry manually.` : message;
          connection.setError(localError);
          diagnostics.add({
            level: 'warn',
            category: 'engine',
            message: retry ? 'Retrying native video startup once with fresh credentials and endpoints' : `${message}; the accepted session remains available for retry`
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
        const stillCurrent = () => componentActive && !disconnecting && $connection.session?.id === current.id;
        await stopEngine('client', current.id);
        if (!stillCurrent()) return;
        await stopRelay('client', current.id);
        if (!stillCurrent()) return;
        connection.setEngineRunning(false);
        connection.setNativeRoute(null);
        return;
      }
      if (refreshed?.status === 'accepted' && $connection.engineRunning) {
        const evidence = liveRecoveryEvidence();
        if (evidence && await recoverRoute(refreshed, evidence)) return;
      }
      if (
        refreshed?.status === 'accepted' &&
        refreshed.transport === 'native' &&
        localDeviceId &&
        !$connection.engineRunning &&
        !$connection.nativeRetryBlocked &&
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
  <header class="page-header"><div><h1>Active session</h1><p>{sessionState?.accessMode === 'view-only' ? 'Watch the shared screen in its own window.' : 'Your connection and remote desktop, in one place.'}</p></div></header>
  {#if sessionState}<div class="session-toolbar">
    <div class="session-identity">
      <StatusPill state={$connection.engineRunning ? 'online' : 'warning'} label={$connection.nativeRecoveryPending ? 'Reconnecting' : $connection.engineRunning ? 'Window open' : sessionState.status} />
      {#if activeRoute}<span class="session-route"><Icon name="link" />{activeRoute === 'direct' ? 'Direct connection' : 'Relay connection'}</span>{/if}
    </div>
    <div class="session-actions">
      {#if $connection.engineRunning}<button class="button small" title="Close and reopen the remote window" disabled={$connection.busy || nativePreparationInFlight || recoveryInFlight || disconnecting} onclick={() => recoverRoute(sessionState, null, true)}>Reconnect</button>{/if}
      <button class="button small danger" disabled={disconnecting} onclick={disconnect}>{disconnecting ? 'Disconnecting…' : nativePreparationInFlight ? 'Cancel connection' : $connection.nativeRecoveryPending ? 'Cancel reconnection' : 'Disconnect'}</button>
    </div>
  </div>{/if}

  {#if sessionState}
    <ol class="connection-steps" aria-label="Connection progress">
      <li class:complete={['accepted', 'connecting', 'connected'].includes(sessionState.status)}><span>01</span><div><strong>Approval</strong><small>{sessionState.status === 'pending' ? 'Waiting for the host' : sessionState.status === 'accepted' ? 'Host accepted' : sessionState.status}</small></div></li>
      <li class:complete={activeRoute !== null} class:current={nativePreparationInFlight}><span>02</span><div><strong>Connection</strong><small>{nativePreparationInFlight ? phaseLabels[phase] : activeRoute === 'relay' ? 'Relay connection' : activeRoute === 'direct' ? 'Direct connection' : 'Waiting'}</small></div></li>
      <li class:complete={metrics !== null}><span>03</span><div><strong>Remote desktop</strong><small>{metrics ? 'Screen data received' : $connection.engineRunning ? 'Check the remote window' : 'Not started'}</small></div></li>
    </ol>
  {/if}
  {#if sharingWithoutSound}
    <div class="notice">Multi-computer sharing currently sends video only. Sound is not included.</div>
  {:else if soundUnavailable}
    <div class="notice">This viewer does not support sound for this connection yet.</div>
  {/if}
  {#if sessionState?.wireProtocol === 'snv2' && sessionState.accessMode !== 'view-only' && $preferences.input.mouseMode === 'relative'}
    <div class="notice">This connection uses absolute mouse control. Relative mouse capture is not supported yet.</div>
  {/if}
  <div class="stream-stage" class:has-metrics={metrics !== null}>
    {#if $connection.engineRunning}
      <div class="native-stage-message"><span class="capture-dot"></span><h2>Remote window opened</h2><p>Switch to the remote window to {sessionState?.accessMode === 'view-only' ? 'view the shared screen' : 'view and control the computer'}. If the screen stays blank, check connection details.</p><button class="button" onclick={() => navigate('diagnostics')}>View connection details</button></div>
    {:else if sessionState}
      <div class="native-stage-message">
        <span class="capture-dot waiting"></span>
        <h2>{nativePreparationInFlight ? phaseLabels[phase] : $connection.nativeRecoveryPending ? 'Reconnecting' : sessionState.status === 'pending' ? 'Waiting for host approval' : sessionState.status === 'accepted' ? 'Host accepted the session' : `Session ${sessionState.status}`}</h2>
        <p>{$connection.nativeRecoveryPending ? 'Checking fresh connection routes. The remote window will reopen; you can cancel below.' : nativeReady ? 'Open the remote desktop to begin.' : runtime.capabilities.nativeDirect.reason ?? runtime.capabilities.clientEngine.reason ?? 'Waiting for the other computer.'}</p>
        <button class="button primary" disabled={!nativeReady || $connection.busy || nativePreparationInFlight} onclick={retryNativeClient}>{nativePreparationInFlight ? 'Connecting…' : failedP2pSessionId === sessionState.id || $connection.nativeRetryBlocked ? 'Retry connection' : 'Open remote desktop'}</button>
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
