<script lang="ts">
  import { t, tk } from '../lib/i18n';
  import { onMount } from 'svelte';
  import Icon from '../components/Icon.svelte';
  import GroupVoice from '../components/GroupVoice.svelte';
  import SessionTextTools from '../components/SessionTextTools.svelte';
  import SessionFiles from '../components/SessionFiles.svelte';
  import SessionDisplays from '../components/SessionDisplays.svelte';
  import SessionChat from '../components/SessionChat.svelte';
  import { sharing } from '../stores/sharing';
  import StatusPill from '../components/StatusPill.svelte';
  import { engineStatus, launchEngine, startRelay, stopEngine, stopRelay, routeRecoveryPlan, type RouteRecoveryEvidence } from '../lib/platform';
  import { resolveStreamProfile, resolveStreamSize } from '../lib/streamProfile';
  import { ApiError } from '../lib/api';
  import { coordinateP2pConnection, directCheckFailure, releaseP2pConnection } from '../lib/p2pSignaling';
  import type { ConnectionSession, Page, RuntimeStatus } from '../lib/types';
  import { connection } from '../stores/connection';
  import { diagnostics, networkDiagnostics } from '../stores/diagnostics';
  import { nativePerformance } from '../stores/nativePerformance';
  import { preferences } from '../stores/preferences';
  import { presence } from '../stores/presence';
  import { session } from '../stores/session';
  import { viewerAudio, prepareViewerAudio, muteViewerAudio, forgetViewerAudio } from '../stores/viewerAudio';

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
  const phaseLabels = { idle: tk('Waiting for approval'), authorizing: tk('Preparing connection'), direct: tk('Connecting directly'), relay: tk('Connecting through relay'), launching: tk('Opening the remote window') };
  const activeRoute = $derived($connection.nativeRoute);

  const sessionState = $derived($connection.session);
  const metrics = $derived($connection.metrics);
  let changingSound = $state(false);
  const groupSound = $derived(sessionState?.wireProtocol === 'snv2' && !!sessionState.sharingId);
  const liveSoundControl = $derived(sessionState?.wireProtocol === 'snv2' && runtime.capabilities.clientAudio === true && sessionState.id in $viewerAudio);
  const soundUnavailable = $derived(sessionState?.wireProtocol === 'snv2' && runtime.capabilities.clientAudio !== true);
  async function toggleSound(): Promise<void> {
    if (!sessionState || changingSound || !liveSoundControl || !$connection.engineRunning || disconnecting || recoveryInFlight) return;
    const id = sessionState.id;
    changingSound = true;
    try { await muteViewerAudio(id, !($viewerAudio[id] ?? false)); }
    catch (error) { if ($connection.session?.id === id) localError = error instanceof Error ? error.message : $t('Unable to change sound'); }
    finally { changingSound = false; }
  }
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
      if (!localDeviceId || !peerDeviceId) throw new Error($t('Missing device IDs for P2P connection'));

      diagnostics.add({ level: 'info', category: 'session', message: 'Starting P2P NAT Traversal...' });
      const client = session.client();
      if (!client) throw new Error($t('Api client is unavailable'));
      let p2pResult: Awaited<ReturnType<typeof coordinateP2pConnection>> | null = null;
      let directCheck: import('../lib/types').LaunchEngineRequest['directCheck'] = 'passed';
      let relayResult: Awaited<ReturnType<typeof startRelay>> | null = null;
      try {
        phase = 'direct';
        if (target.networkMode === 'relay') throw new Error($t('Relay-only mode selected'));
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
        if (!accessToken) throw new Error($t('Relay fallback requires an authenticated access token'));
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

      if (signal.aborted || !componentActive || $connection.session?.id !== target.id || $session.mode === 'signedOut') {
        if (selectedP2p) await releaseP2pConnection(selectedP2p).catch(() => undefined);
        if (relayResult) await stopRelay('client', target.id).catch(() => undefined);
        return false;
      }
      phase = 'launching';
      connection.markNativeLaunch();
      const audioEnabled = (!!target.sharingId || $preferences.host.audioEnabled) &&
        (target.wireProtocol !== 'snv2' || runtime.capabilities.clientAudio === true);
      const audioMuted = target.wireProtocol === 'snv2' && audioEnabled
        ? prepareViewerAudio(target.id, !!target.sharingId && !$preferences.host.audioEnabled) : false;
      if (target.wireProtocol !== 'snv2' || !audioEnabled) forgetViewerAudio(target.id);
      await launchEngine({
        ultraLowLatency: stream.profile === 'competitive',
        directCheck,
        kind: 'client',
        wireProtocol: target.wireProtocol,
        sessionId: target.id,
        sharingId: target.sharingId,
        address: relayResult ? '127.0.0.1' : p2pResult?.remoteAddress,
        port: relayResult?.enginePort ?? p2pResult?.localPort,
        codec: target.requestedCodec,
        fps: stream.fps,
        bitrateKbps: Math.round(stream.bitrateMbps * 1_000),
        width: size.width,
        height: size.height,
        networkMode: target.networkMode,
        audioEnabled,
        audioMuted,
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
      const stillCurrent = () => !signal.aborted && componentActive && $connection.session?.id === target.id;
      if (!stillCurrent()) return false;
      if (phase === 'launching') {
        const status = await engineStatus('client').catch(() => null);
        if (!stillCurrent()) return false;
        if (status?.userClosed && status.sessionId === target.id) {
          closedViewerSessionId = target.id;
          await disconnect(); return false;
        }
      }
      const message = error instanceof Error ? error.message : $t('Unable to start native client');
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
      if (isAborted(controller.signal) || !componentActive || $connection.session?.id !== target.id || $session.mode === 'signedOut') return;
      if ($connection.nativeRecoveryPending && Date.now() > $connection.nativeRecoveryRetryAt + 30_000) {
        connection.blockNativeRetry();
        connection.setError($t('The server did not recover in time. Retry the connection manually.'));
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
        localError = $t('Waiting for the server to recover before authorizing a fresh connection…');
        connection.setError(localError);
        return;
      }
      const exhausted = p2pRetryCount >= 3 || $connection.nativeStartupRetries > 0 || $connection.nativeRecoveryPending;
      failedP2pSessionId = exhausted ? target.id : null;
      if (exhausted) connection.blockNativeRetry();
      const message = error instanceof Error ? error.message : $t('Unable to prepare a fresh native session');
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
    const paths = $networkDiagnostics.livePaths;
    // A live same-session router owns latency-only path selection. Do not tear
    // down a healthy window simply to race the same candidates again.
    if (update.route === 'direct' && paths?.sessionId === current.id && paths.receivedAtMs >= launched && now >= paths.receivedAtMs && now - paths.receivedAtMs < 5_000) return null;
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
    const ownsSession = () => !disconnecting && $session.mode !== 'signedOut' && $connection.session?.id === target.id;
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
      diagnostics.add({ level: 'warn', category: 'network', message: $t('Reconnecting the accepted session with fresh credentials and candidates; the remote window will reopen.'),
        details: { reason: plan.reason, automatic: !manual, attempt: $connection.nativeRouteRecoveries } });
      connection.setBusy(false);
      if (manual && componentActive && confirmed) await startNativeClient(confirmed);
      return true;
    } catch (error) {
      if (ownsSession()) {
        connection.blockNativeRetry();
        localError = error instanceof Error ? error.message : $t('Unable to reconnect; retry manually');
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
      const message = error instanceof Error ? error.message : $t('Unable to disconnect');
      localError = message;
      connection.setError(message);
    } finally {
      disconnecting = false;
    }
  }

  const endedStatuses: ConnectionSession['status'][] = ['rejected', 'disconnected', 'expired', 'closed', 'failed'];

  // The server already ended this viewer's session; close the view locally.
  async function leaveEndedSharing(targetId: string): Promise<void> {
    if (disconnecting) return;
    disconnecting = true;
    connection.blockNativeRetry();
    negotiationController?.abort();
    try {
      if (runtime.capabilities.desktopShell.state === 'available') {
        await Promise.allSettled([stopEngine('client', targetId), stopRelay('client', targetId)]);
      }
      if ($connection.session?.id !== targetId) return;
      failedP2pSessionId = null;
      p2pRetryCount = 0;
      p2pRetryAfter = 0;
      p2pRetrySessionId = null;
      localError = null;
      closedViewerSessionId = null;
      connection.clear();
      diagnostics.add({ level: 'info', category: 'session', message: 'The host stopped sharing; the shared screen was closed' });
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
      // A shared screen ends for every viewer when the host stops sharing or
      // removes this viewer. Check before engine recovery, so a vanished host
      // is not mistaken for a broken route and retried.
      let sharingRefresh: ConnectionSession | null = null;
      if (current.sharingId) {
        sharingRefresh = await connection.refresh(client);
        // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
        if (!componentActive || $connection.session?.id !== current.id) return;
        if (sharingRefresh && endedStatuses.includes(sharingRefresh.status)) { await leaveEndedSharing(current.id); return; }
      }
      if ($connection.nativeRecoveryPending && Date.now() > $connection.nativeRecoveryRetryAt + 30_000) {
        connection.blockNativeRetry();
        connection.setError($t('The server did not recover in time. Retry the connection manually.'));
        return;
      }
      if ($connection.engineRunning) {
        const status = await engineStatus('client');
        if (status.sessionId && status.sessionId !== current.id) return;
        // Window close is terminal even if native audio/device teardown is
        // still running. Do not wait for process exit or trigger route recovery.
        if (status.userClosed && status.sessionId === current.id) { closedViewerSessionId = current.id; await disconnect(); return; }
        if (status.running && status.videoReceived && current.sharingId && current.requesterReadyAt && localDeviceId) {
          await client.markSharingStarted(current.id, localDeviceId, current.requesterReadyAt);
        }
        // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
        if (!componentActive || $connection.session?.id !== current.id) return;
        if (!status.running) {
          // A deliberate viewer X closes only its server session. This page
          // remains mounted across navigation; closing the main shell to tray
          // never emits the native viewer marker.
          const message = status.lastError ?? $t('The native client stopped unexpectedly');
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
          localError = retry ? $t('{message} Retrying once with a fresh connection…', { message })
            : status.routeFailure && $connection.nativeRouteRecoveries >= 2 ? $t('{message} Automatic reconnection stopped after two attempts. Retry manually.', { message }) : message;
          connection.setError(localError);
          diagnostics.add({
            level: 'warn',
            category: 'engine',
            message: retry ? 'Retrying native video startup once with fresh credentials and endpoints' : `${message}; the accepted session remains available for retry`
          });
          return;
        }
      }

      const refreshed = sharingRefresh ?? await connection.refresh(client);
      // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
      if (!componentActive || $connection.session?.id !== current.id) return;
      if (
        refreshed &&
        endedStatuses.includes(refreshed.status) &&
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
      connection.setError(error instanceof Error ? error.message : $t('Unable to refresh the native session'));
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
  <header class="page-header"><div><h1>{$t('Active session')}</h1><p>{sessionState?.accessMode === 'view-only' ? $t('Watch the shared screen in its own window.') : $t('Your connection and remote desktop, in one place.')}</p></div></header>
  {#if sessionState}<div class="session-toolbar">
    <div class="session-identity">
      <StatusPill state={$connection.engineRunning ? 'online' : 'warning'} label={$connection.nativeRecoveryPending ? $t('Reconnecting') : $connection.engineRunning ? $t('Window open') : sessionState.status} />
      {#if activeRoute}<span class="session-route"><Icon name="link" />{activeRoute === 'direct' ? $t('Direct connection') : $t('Relay connection')}</span>{/if}
      <span class="session-access" title={$t('Session access mode')}><Icon name={sessionState.accessMode === 'view-only' ? 'eye' : 'pointer'} />{sessionState.accessMode === 'view-only' ? $t('View only') : $t('Remote control')}</span>
    </div>
    <div class="session-actions" role="group" aria-label={$t('Session actions')}>
      {#if liveSoundControl && $connection.engineRunning}
        <button class="button small" disabled={changingSound || disconnecting || recoveryInFlight} aria-pressed={$viewerAudio[sessionState.id] ?? false} title={$t('Mute or unmute only this viewer')} onclick={toggleSound}><Icon name={($viewerAudio[sessionState.id] ?? false) ? 'volume-off' : 'volume'} />{($viewerAudio[sessionState.id] ?? false) ? $t('Unmute sound') : $t('Mute sound')}</button>
      {/if}
      {#if $connection.engineRunning}<button class="button small" title={$t('Close and reopen the remote window')} disabled={$connection.busy || nativePreparationInFlight || recoveryInFlight || disconnecting} onclick={() => recoverRoute(sessionState, null, true)}><Icon name="refresh" />{$t('Reconnect')}</button>{/if}
      {#if $session.mode !== 'guest'}<button class="button small" onclick={() => navigate('settings')}><Icon name="settings" />{$t('Session settings')}</button>
      <button class="button small" onclick={() => navigate('diagnostics')}><Icon name="diagnostics" />{$t('Connection details')}</button>{/if}
      <button class="button small danger" disabled={disconnecting} onclick={disconnect}><Icon name="leave" />{disconnecting ? $t('Disconnecting…') : nativePreparationInFlight ? $t('Cancel connection') : $connection.nativeRecoveryPending ? $t('Cancel reconnection') : $t('Disconnect')}</button>
    </div>
  </div>{/if}

  {#if sessionState}
    <ol class="connection-steps" aria-label={$t('Connection progress')}>
      <li class:complete={['accepted', 'connecting', 'connected'].includes(sessionState.status)}><span>01</span><div><strong>{$t('Approval')}</strong><small>{sessionState.status === 'pending' ? $t('Waiting for the host') : sessionState.status === 'accepted' ? $t('Host accepted') : sessionState.status}</small></div></li>
      <li class:complete={activeRoute !== null} class:current={nativePreparationInFlight}><span>02</span><div><strong>{$t('Connection')}</strong><small>{nativePreparationInFlight ? $t(phaseLabels[phase]) : activeRoute === 'relay' ? $t('Relay connection') : activeRoute === 'direct' ? $t('Direct connection') : $t('Waiting')}</small></div></li>
      <li class:complete={metrics !== null}><span>03</span><div><strong>{$t('Remote desktop')}</strong><small>{metrics ? $t('Screen data received') : $connection.engineRunning ? $t('Check the remote window') : $t('Not started')}</small></div></li>
    </ol>
  {/if}
  {#if sessionState?.sharingId && sessionState.status === 'accepted' && $connection.engineRunning}
    <GroupVoice kind="client" sharingId={sessionState.sharingId} sessionId={sessionState.id} />
  {/if}
  {#if sessionState?.wireProtocol === 'snv2' && sessionState.status === 'accepted' && $connection.engineRunning}
    {#key sessionState.id}<SessionChat kind="client" sessionId={sessionState.id} selfName={$sharing.devices.find((device) => device.id === $presence.deviceId)?.name ?? ''} />{/key}
  {/if}

  {#if soundUnavailable}
    <div class="notice">{$t('This viewer does not support sound for this connection yet.')}</div>
  {:else if groupSound}
    <div class="notice">{$t('Sound is available when the host shares system audio. Muting here affects only this viewer.')}</div>
  {:else if sessionState?.wireProtocol === 'snv2' && $connection.engineRunning && !liveSoundControl}
    <div class="notice">{$t('To enable live sound controls, turn on sound in Settings and reconnect.')}</div>
  {/if}
  {#if sessionState && !sessionState.sharingId && $connection.engineRunning}
    {#key sessionState.id}<SessionDisplays sessionId={sessionState.id} legacy={sessionState.wireProtocol !== 'snv2'} />{/key}
  {/if}
  {#if sessionState?.wireProtocol === 'snv2' && sessionState.accessMode !== 'view-only' && !sessionState.sharingId && $connection.engineRunning}
    {#key sessionState.id}<SessionTextTools sessionId={sessionState.id} /><SessionFiles sessionId={sessionState.id} />{/key}
  {/if}
  {#if sessionState?.wireProtocol === 'snv2' && sessionState.accessMode !== 'view-only' && $preferences.input.mouseMode === 'relative'}
    <div class="notice">{$t('This connection uses absolute mouse control. Relative mouse capture is not supported yet.')}</div>
  {/if}
  <div class="stream-stage" class:has-metrics={metrics !== null}>
    {#if $connection.engineRunning}
      <div class="native-stage-message"><span class="capture-dot"></span><h2>{$t('Remote window opened')}</h2><p>{sessionState?.accessMode === 'view-only' ? $t('Switch to the remote window to view the shared screen. If the screen stays blank, check connection details.') : $t('Switch to the remote window to view and control the computer. If the screen stays blank, check connection details.')}</p><button class="button" onclick={() => navigate('diagnostics')}>{$t('View connection details')}</button></div>
    {:else if sessionState}
      <div class="native-stage-message">
        <span class="capture-dot waiting"></span>
        <h2>{nativePreparationInFlight ? $t(phaseLabels[phase]) : $connection.nativeRecoveryPending ? $t('Reconnecting') : sessionState.status === 'pending' ? $t('Waiting for host approval') : sessionState.status === 'accepted' ? $t('Host accepted the session') : `Session ${sessionState.status}`}</h2>
        <p>{$connection.nativeRecoveryPending ? $t('Checking fresh connection routes. The remote window will reopen; you can cancel below.') : nativeReady ? $t('Open the remote desktop to begin.') : runtime.capabilities.nativeDirect.reason ?? runtime.capabilities.clientEngine.reason ?? $t('Waiting for the other computer.')}</p>
        <button class="button primary" disabled={!nativeReady || $connection.busy || nativePreparationInFlight} onclick={retryNativeClient}>{nativePreparationInFlight ? $t('Connecting…') : failedP2pSessionId === sessionState.id || $connection.nativeRetryBlocked ? $t('Retry connection') : $t('Open remote desktop')}</button>
        {#if !nativeReady && sessionState.status === 'accepted'}<span class="planned-inline">{$t('This connection is unavailable. Check Settings → Diagnostics for details.')}</span>{/if}
      </div>
    {:else}
      <div class="native-stage-message"><div class="session-empty-icon"><Icon name="monitor" /></div><h2>{$t('No active session')}</h2><p>{$t('Choose an online computer to get started.')}</p><button class="button primary" onclick={() => navigate('computers')}>{$t('Browse computers →')}</button></div>
    {/if}

    {#if metrics}<div class="stream-stats" aria-label={$t('Connection statistics')}>
      <div><span>{$t('Frame rate')}</span><strong>{metrics.fps} FPS</strong></div>
      <div><span>{$t('Bitrate')}</span><strong>{metrics.bitrateMbps.toFixed(1)} Mb/s</strong></div>
      <div><span>{$t('Latency')}</span><strong>{metrics.rttMs} ms</strong></div>
      <div><span>{$t('Packet loss')}</span><strong>{metrics.packetLossPercent.toFixed(2)}%</strong></div>
    </div>{/if}
  </div>

  {#if localError ?? $connection.error}<div class="session-error notice error" role="alert">{$t(localError ?? $connection.error ?? '')}</div>{/if}
</section>
