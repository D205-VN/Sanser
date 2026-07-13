import { invoke } from '@tauri-apps/api/core';
import type { ApiClient } from './api';
import { diagnostics } from '../stores/diagnostics';

export interface P2pPunchResult {
  localPort: number;
  remoteAddress: string;
  remotePort: number;
}

export async function coordinateP2pConnection(
  client: ApiClient,
  sessionId: string,
  localDeviceId: string,
  peerDeviceId: string,
  controlling: boolean
): Promise<P2pPunchResult> {
  diagnostics.add({
    level: 'info',
    category: 'network',
    message: `[P2P] Starting coordination: controlling=${controlling}`
  });

  // 1. Gather local candidates
  const stunServer = 'stun:stun.l.google.com:19302';
  diagnostics.add({
    level: 'info',
    category: 'network',
    message: `[P2P] Gathering candidates from: ${stunServer}`
  });
  
  let localCandidates: Record<string, unknown>[] = [];
  try {
    const gatherResult = await invoke<{ candidates: Record<string, unknown>[] }>('p2p_gather', { stunServer });
    localCandidates = gatherResult.candidates;
    diagnostics.add({
      level: 'info',
      category: 'network',
      message: `[P2P] Gathered ${localCandidates.length} local candidates`
    });
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    diagnostics.add({
      level: 'error',
      category: 'network',
      message: `[P2P] Candidate gathering failed: ${msg}`
    });
    throw err;
  }

  // 2. Connect to the WebSocket signaling server
  const wsBaseUrl = client.serverUrl.replace(/^http/, 'ws');
  const token = await client.getAccessToken();
  if (!token) {
    throw new Error('Unauthenticated: Access token is missing');
  }
  const wsUrl = `${wsBaseUrl}/api/v2/signaling?deviceId=${localDeviceId}`;
  diagnostics.add({
    level: 'info',
    category: 'network',
    message: `[P2P] Connecting WebSocket to signaling server`
  });
  
  return new Promise<P2pPunchResult>((resolve, reject) => {
    let ws: WebSocket;
    try {
      ws = new WebSocket(wsUrl, ['sanser-v2', 'bearer.' + token]);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      diagnostics.add({
        level: 'error',
        category: 'network',
        message: `[P2P] WebSocket init failed: ${msg}`
      });
      reject(err);
      return;
    }

    let remoteCandidates: Record<string, unknown>[] = [];
    let checkInProgress = false;

    const timeout = setTimeout(() => {
      diagnostics.add({
        level: 'error',
        category: 'network',
        message: '[P2P] Timeout reached'
      });
      ws.close();
      reject(new Error('P2P connection negotiation timed out'));
    }, 15000);

    ws.onopen = () => {
      diagnostics.add({
        level: 'info',
        category: 'network',
        message: '[P2P] WebSocket connected. Sending candidates...'
      });
      // Send candidates
      const msg = {
        sessionId,
        targetDeviceId: peerDeviceId,
        type: 'p2p.candidates',
        payload: {
          generation: 1,
          candidates: localCandidates
        }
      };
      ws.send(JSON.stringify(msg));
    };

    ws.onmessage = async (event) => {
      try {
        const data = JSON.parse(event.data) as {
          type?: string;
          payload?: { candidates?: Record<string, unknown>[] };
        };
        diagnostics.add({
          level: 'info',
          category: 'network',
          message: `[P2P] Message received: type=${data.type}`
        });

        if (data.type === 'p2p.candidates' && data.payload?.candidates) {
          remoteCandidates = data.payload.candidates;
          diagnostics.add({
            level: 'info',
            category: 'network',
            message: `[P2P] Received ${remoteCandidates.length} remote candidates`
          });
          
          if (!checkInProgress) {
            checkInProgress = true;
            ws.close(); // We don't need signaling anymore once we have remote candidates
            clearTimeout(timeout);
            
            // Perform connectivity checks (UDP hole punching)
            diagnostics.add({
              level: 'info',
              category: 'network',
              message: '[P2P] Initiating UDP hole punching checks'
            });

            const punchResult = await invoke<P2pPunchResult>('p2p_punch', {
              request: {
                sessionId,
                localCandidates,
                remoteCandidates,
                controlling
              }
            });

            diagnostics.add({
              level: 'info',
              category: 'network',
              message: `[P2P] Punch succeeded: localPort=${punchResult.localPort}, remote=${punchResult.remoteAddress}:${punchResult.remotePort}`
            });

            resolve(punchResult);
          }
        }
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        diagnostics.add({
          level: 'error',
          category: 'network',
          message: `[P2P] Message processing error: ${msg}`
        });
        clearTimeout(timeout);
        ws.close();
        reject(err instanceof Error ? err : new Error(msg));
      }
    };

    ws.onerror = () => {
      diagnostics.add({
        level: 'error',
        category: 'network',
        message: '[P2P] WebSocket error'
      });
      clearTimeout(timeout);
      ws.close();
      reject(new Error('WebSocket signaling connection error'));
    };
  });
}
