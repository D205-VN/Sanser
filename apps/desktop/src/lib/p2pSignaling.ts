import { invoke } from '@tauri-apps/api/core';
import type { ApiClient } from './api';
import { diagnostics, networkDiagnostics } from '../stores/diagnostics';

export interface P2pPunchResult {
  localPort: number;
  remoteAddress: string;
  remotePort: number;
}

interface P2pGatherResult {
  candidates: Record<string, unknown>[];
  localPort: number;
  stunSucceeded: boolean;
  portMappingSucceeded: boolean;
  ipv6Available: boolean;
  publicEndpoint: string | null;
  durationMs: number;
}

interface SignalingMessage {
  sessionId?: string;
  senderDeviceId?: string;
  type?: string;
  payload?: {
    generation?: number;
    candidates?: Record<string, unknown>[];
  };
  error?: {
    code?: string;
    message?: string;
  };
}

const CANDIDATE_GENERATION = 1;
const NEGOTIATION_TIMEOUT_MS = 20_000;
const RETRY_DELAYS_MS = [250, 500, 1_000, 2_000] as const;
const TRANSIENT_SIGNAL_ERRORS = new Set([
  'target_offline',
  'target_backpressure'
]);

function websocketUrl(serverUrl: string, deviceId: string): string {
  const url = new URL(serverUrl);
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  url.pathname = `${url.pathname.replace(/\/$/, '')}/api/v2/signaling`;
  url.search = new URLSearchParams({ deviceId }).toString();
  return url.toString();
}

export async function coordinateP2pConnection(
  client: ApiClient,
  sessionId: string,
  localDeviceId: string,
  peerDeviceId: string,
  controlling: boolean,
  sessionCredential: string,
  preferredLocalPort?: number
): Promise<P2pPunchResult> {
  const attemptId = crypto.randomUUID();
  if (!sessionCredential) throw new Error('P2P session credential is missing');
  const token = await client.getAccessToken();
  if (!token) throw new Error('Unauthenticated: Access token is missing');

  diagnostics.add({
    level: 'info',
    category: 'network',
    message: `[P2P] Starting authenticated coordination: controlling=${controlling}`
  });

  let stunServers: string[] = [];
  try {
    const ice = await client.iceConfiguration();
    stunServers = [...new Set(
      ice.iceServers
        .flatMap((server) => server.urls)
        .filter((url) => url.toLowerCase().startsWith('stun:'))
    )].slice(0, 3);
  } catch (error) {
    diagnostics.add({
      level: 'warn',
      category: 'network',
      message: `[P2P] STUN configuration is unavailable; trying LAN candidates only: ${error instanceof Error ? error.message : String(error)}`
    });
  }
  let localCandidates: Record<string, unknown>[];
  try {
    const gathered = await invoke<P2pGatherResult>('p2p_gather', {
      sessionId,
      attemptId,
      stunServers,
      preferredLocalPort
    });
    localCandidates = gathered.candidates;
    if (localCandidates.length === 0) throw new Error('No local P2P candidate was gathered');
    networkDiagnostics.updateGather(gathered, preferredLocalPort !== undefined);
    diagnostics.add({
      level: 'info',
      category: 'network',
      message: `[P2P] Gathered ${localCandidates.length} candidates on reserved UDP port ${gathered.localPort}`,
      details: {
        localPort: gathered.localPort,
        stun: gathered.stunSucceeded,
        upnp: gathered.portMappingSucceeded,
        ipv6Available: gathered.ipv6Available,
        durationMs: gathered.durationMs
      }
    });
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    diagnostics.add({ level: 'error', category: 'network', message: `[P2P] Gathering failed: ${message}` });
    throw error;
  }

  return new Promise<P2pPunchResult>((resolve, reject) => {
    let websocket: WebSocket;
    const signalingEndpoint = websocketUrl(client.serverUrl, localDeviceId);
    let settled = false;
    let checkInProgress = false;
    let candidatesAcknowledged = false;
    let remoteCandidates: Record<string, unknown>[] | null = null;
    let retryIndex = 0;
    let retryTimer: number | undefined;

    const clearTimers = (): void => {
      window.clearTimeout(timeoutTimer);
      if (retryTimer !== undefined) window.clearTimeout(retryTimer);
      retryTimer = undefined;
    };

    const fail = (error: Error): void => {
      if (settled) return;
      settled = true;
      clearTimers();
      websocket.close();
      void invoke('p2p_stop', { attemptId })
        .catch(() => undefined)
        .finally(() => reject(error));
    };

    const succeed = (result: P2pPunchResult): void => {
      if (settled) return;
      settled = true;
      clearTimers();
      // The selected socket remains reserved in Rust until launch_engine
      // releases it immediately before the native sidecar binds the same port.
      websocket.close();
      resolve(result);
    };

    const send = (type: string, payload: Record<string, unknown>): boolean => {
      if (websocket.readyState !== WebSocket.OPEN) return false;
      websocket.send(
        JSON.stringify({
          sessionId,
          targetDeviceId: peerDeviceId,
          type,
          payload
        })
      );
      return true;
    };

    const scheduleCandidateRetry = (): void => {
      if (settled || candidatesAcknowledged || websocket.readyState !== WebSocket.OPEN) return;
      if (retryTimer !== undefined) window.clearTimeout(retryTimer);
      const delay = RETRY_DELAYS_MS[Math.min(retryIndex, RETRY_DELAYS_MS.length - 1)];
      retryIndex += 1;
      retryTimer = window.setTimeout(sendCandidates, delay);
    };

    const sendCandidates = (): void => {
      retryTimer = undefined;
      if (settled || candidatesAcknowledged) return;
      if (
        send('p2p.candidates', {
          generation: CANDIDATE_GENERATION,
          candidates: localCandidates
        })
      ) {
        scheduleCandidateRetry();
      }
    };

    const acknowledgeCandidates = (): void => {
      const payload = { generation: CANDIDATE_GENERATION };
      // candidatesAck is explicit in current servers. gatheringComplete is a
      // compatibility receipt for servers deployed before this fix.
      send('p2p.candidatesAck', payload);
      send('p2p.gatheringComplete', payload);
    };

    const maybeStartConnectivityCheck = (): void => {
      if (
        settled ||
        checkInProgress ||
        !candidatesAcknowledged ||
        remoteCandidates === null
      ) return;
      checkInProgress = true;
      if (retryTimer !== undefined) window.clearTimeout(retryTimer);
      retryTimer = undefined;
      diagnostics.add({
        level: 'info',
        category: 'network',
        message: '[P2P] Candidate exchange acknowledged; starting UDP connectivity checks'
      });
      void invoke<P2pPunchResult>('p2p_punch', {
        request: {
          sessionId,
          attemptId,
          remoteCandidates,
          controlling,
          localDeviceId,
          peerDeviceId,
          sessionCredential
        }
      })
        .then((result) => {
          diagnostics.add({
            level: 'info',
            category: 'network',
            message: `[P2P] Direct route selected on reserved local port ${result.localPort}`
          });
          succeed(result);
        })
        .catch((error: unknown) => {
          const message = error instanceof Error ? error.message : String(error);
          fail(new Error(`P2P connectivity check failed: ${message}`));
        });
    };

    const timeoutTimer = window.setTimeout(() => {
      fail(new Error('P2P connection negotiation timed out while waiting for the peer'));
    }, NEGOTIATION_TIMEOUT_MS);

    try {
      websocket = new WebSocket(signalingEndpoint, [
        'sanser-v2',
        `bearer.${token}`
      ]);
    } catch (error) {
      clearTimers();
      void invoke('p2p_stop', { attemptId })
        .catch(() => undefined)
        .finally(() => reject(error instanceof Error ? error : new Error(String(error))));
      return;
    }

    websocket.onopen = () => {
      diagnostics.add({ level: 'info', category: 'network', message: '[P2P] Signaling connected' });
      sendCandidates();
    };

    websocket.onmessage = (event) => {
      let message: SignalingMessage;
      try {
        message = JSON.parse(String(event.data)) as SignalingMessage;
      } catch {
        fail(new Error('Signaling server returned invalid JSON'));
        return;
      }

      if (message.type === 'error') {
        const code = message.error?.code ?? 'signaling_error';
        const detail = message.error?.message ?? 'Signaling request failed';
        if (TRANSIENT_SIGNAL_ERRORS.has(code)) {
          diagnostics.add({
            level: 'warn',
            category: 'network',
            message: `[P2P] Peer is not ready (${code}); candidate retry remains active`
          });
          scheduleCandidateRetry();
          return;
        }
        // Older deployed servers do not know candidatesAck, but do know the
        // gatheringComplete compatibility receipt sent beside it.
        if (code === 'invalid_signal_type') return;
        fail(new Error(`${detail} (${code})`));
        return;
      }

      if (message.sessionId !== sessionId || message.senderDeviceId !== peerDeviceId) return;
      if (
        message.type === 'p2p.candidates' &&
        message.payload?.generation === CANDIDATE_GENERATION &&
        Array.isArray(message.payload.candidates) &&
        message.payload.candidates.length > 0
      ) {
        remoteCandidates = message.payload.candidates;
        diagnostics.add({
          level: 'info',
          category: 'network',
          message: `[P2P] Received ${remoteCandidates.length} remote candidates`
        });
        acknowledgeCandidates();
        maybeStartConnectivityCheck();
        return;
      }
      if (
        (message.type === 'p2p.candidatesAck' || message.type === 'p2p.gatheringComplete') &&
        message.payload?.generation === CANDIDATE_GENERATION
      ) {
        candidatesAcknowledged = true;
        if (retryTimer !== undefined) window.clearTimeout(retryTimer);
        retryTimer = undefined;
        maybeStartConnectivityCheck();
      }
    };

    websocket.onerror = () => {
      if (checkInProgress) {
        diagnostics.add({
          level: 'warn',
          category: 'network',
          message: '[P2P] Signaling closed after candidate exchange; connectivity check continues'
        });
        return;
      }
      // Browsers intentionally hide HTTP handshake details from `onerror`.
      // Wait for `onclose` so its code/reason can be surfaced. The overall
      // negotiation timer remains the final guard if a WebView omits close.
      diagnostics.add({
        level: 'warn',
        category: 'network',
        message: `[P2P] Signaling transport error at ${new URL(signalingEndpoint).origin}; waiting for close details`
      });
    };

    websocket.onclose = (event) => {
      if (!settled && !checkInProgress) {
        const reason = event.reason.trim().slice(0, 160);
        const detail = reason ? `, reason: ${reason}` : '';
        fail(new Error(
          `WebSocket signaling closed before candidate exchange (code ${event.code}${detail}; endpoint ${new URL(signalingEndpoint).origin})`
        ));
      }
    };
  });
}
