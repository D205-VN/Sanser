import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiClient } from './api';
import { coordinateP2pConnection } from './p2pSignaling';
import { diagnostics } from '../stores/diagnostics';

type Invoke = (command: string, args?: unknown) => Promise<unknown>;

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn<Invoke>()
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock
}));

interface SentSignal {
  sessionId: string;
  targetDeviceId: string;
  type: string;
  payload: Record<string, unknown>;
}

const attemptId = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';

class FakeWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  static readonly instances: FakeWebSocket[] = [];

  readonly url: string;
  readonly protocols: string[];
  readonly sent: string[] = [];
  readyState = FakeWebSocket.CONNECTING;
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: ((event: { code: number; reason: string; wasClean: boolean }) => void) | null = null;

  constructor(url: string | URL, protocols?: string | string[]) {
    this.url = String(url);
    this.protocols = typeof protocols === 'string' ? [protocols] : (protocols ?? []);
    FakeWebSocket.instances.push(this);
  }

  open(): void {
    this.readyState = FakeWebSocket.OPEN;
    this.onopen?.();
  }

  receive(message: unknown): void {
    this.onmessage?.({ data: JSON.stringify(message) });
  }

  send(data: string): void {
    this.sent.push(data);
  }

  close(): void {
    if (this.readyState === FakeWebSocket.CLOSED) return;
    this.readyState = FakeWebSocket.CLOSED;
    this.onclose?.({ code: 1000, reason: '', wasClean: true });
  }

  failConnection(code = 1006, reason = ''): void {
    this.onerror?.();
    this.readyState = FakeWebSocket.CLOSED;
    this.onclose?.({ code, reason, wasClean: false });
  }
}

const sessionId = 'session-123';
const localDeviceId = 'local-device';
const peerDeviceId = 'peer-device';
const sessionCredential = 'ephemeral-session-secret';
const configuredStunServer = 'stun:stun.sanser.example:3478';
const localCandidates = [{ id: 'local-candidate', address: '192.0.2.10', port: 50_000 }];
const remoteCandidates = [{ id: 'remote-candidate', address: '198.51.100.20', port: 50_001 }];
const punchResult = { localPort: 50_000, remoteAddress: '198.51.100.20', remotePort: 50_001 };
const gatherResult = {
  candidates: localCandidates,
  localPort: 50_000,
  stunSucceeded: true,
  portMappingSucceeded: false,
  ipv6Available: false,
  publicEndpoint: '198.51.100.10:50000',
  durationMs: 25
};

function createClient() {
  const client = new ApiClient('https://sanser.example', () => Promise.resolve('access-token'));
  const iceConfigurationMock = vi.spyOn(client, 'iceConfiguration').mockResolvedValue({
    iceServers: [
      { urls: ['turns:relay.sanser.example:5349'], username: 'user', credential: 'credential' },
      { urls: [configuredStunServer, 'stun:backup.sanser.example:3478'] }
    ],
    networkMode: 'auto',
    iceTransportPolicy: 'all'
  });
  return { client, iceConfigurationMock };
}

function signals(socket: FakeWebSocket): SentSignal[] {
  return socket.sent.map((message) => JSON.parse(message) as SentSignal);
}

async function flushSetup(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal('WebSocket', FakeWebSocket);
  vi.stubGlobal('crypto', { randomUUID: () => attemptId });
  FakeWebSocket.instances.length = 0;
  invokeMock.mockReset();
  invokeMock.mockImplementation((command) => {
    if (command === 'p2p_gather') return Promise.resolve(gatherResult);
    if (command === 'p2p_punch') return Promise.resolve(punchResult);
    if (command === 'p2p_stop') return Promise.resolve(undefined);
    return Promise.reject(new Error(`Unexpected native command: ${command}`));
  });
});

afterEach(() => {
  diagnostics.clear();
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('coordinateP2pConnection', () => {
  it('retries an offline peer, filters foreign messages, acknowledges candidates and authenticates punch', async () => {
    const { client, iceConfigurationMock } = createClient();
    const negotiation = coordinateP2pConnection(
      client,
      sessionId,
      localDeviceId,
      peerDeviceId,
      true,
      sessionCredential,
      50_000
    );
    await flushSetup();

    expect(iceConfigurationMock).toHaveBeenCalledOnce();
    expect(invokeMock).toHaveBeenCalledWith('p2p_gather', {
      sessionId,
      attemptId,
      stunServers: [configuredStunServer, 'stun:backup.sanser.example:3478'],
      preferredLocalPort: 50_000
    });
    const socket = FakeWebSocket.instances[0];
    expect(socket).toBeDefined();
    if (!socket) throw new Error('Signaling WebSocket was not created');
    expect(socket.url).toBe(`wss://sanser.example/api/v2/signaling?deviceId=${localDeviceId}`);
    expect(socket.protocols).toEqual(['sanser-v2', 'bearer.access-token']);

    socket.open();
    expect(signals(socket).filter((message) => message.type === 'p2p.candidates')).toHaveLength(1);

    socket.receive({
      type: 'error',
      error: { code: 'target_offline', message: 'Peer signaling socket is not ready' }
    });
    await vi.advanceTimersByTimeAsync(499);
    expect(signals(socket).filter((message) => message.type === 'p2p.candidates')).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(signals(socket).filter((message) => message.type === 'p2p.candidates')).toHaveLength(2);

    socket.receive({
      sessionId: 'foreign-session',
      senderDeviceId: peerDeviceId,
      type: 'p2p.candidates',
      payload: { generation: 1, candidates: remoteCandidates }
    });
    socket.receive({
      sessionId,
      senderDeviceId: 'foreign-device',
      type: 'p2p.candidates',
      payload: { generation: 1, candidates: remoteCandidates }
    });
    expect(signals(socket)).toHaveLength(2);

    socket.receive({
      sessionId,
      senderDeviceId: peerDeviceId,
      type: 'p2p.candidates',
      payload: { generation: 1, candidates: remoteCandidates }
    });
    const receipts = signals(socket).slice(2);
    expect(receipts).toEqual([
      {
        sessionId,
        targetDeviceId: peerDeviceId,
        type: 'p2p.candidatesAck',
        payload: { generation: 1 }
      },
      {
        sessionId,
        targetDeviceId: peerDeviceId,
        type: 'p2p.gatheringComplete',
        payload: { generation: 1 }
      }
    ]);
    expect(receipts.every((message) => !('attemptId' in message))).toBe(true);
    expect(invokeMock).not.toHaveBeenCalledWith('p2p_punch', expect.anything());

    socket.receive({
      sessionId: 'foreign-session',
      senderDeviceId: peerDeviceId,
      type: 'p2p.candidatesAck',
      payload: { generation: 1 }
    });
    socket.receive({
      sessionId,
      senderDeviceId: 'foreign-device',
      type: 'p2p.candidatesAck',
      payload: { generation: 1 }
    });
    expect(invokeMock).not.toHaveBeenCalledWith('p2p_punch', expect.anything());

    socket.receive({
      sessionId,
      senderDeviceId: peerDeviceId,
      type: 'p2p.candidatesAck',
      payload: { generation: 1 }
    });

    await expect(negotiation).resolves.toEqual(punchResult);
    expect(invokeMock).toHaveBeenCalledWith('p2p_punch', {
      request: {
        sessionId,
        attemptId,
        remoteCandidates,
        controlling: true,
        localDeviceId,
        peerDeviceId,
        sessionCredential
      }
    });
    expect(invokeMock).not.toHaveBeenCalledWith('p2p_stop');
    expect(socket.readyState).toBe(FakeWebSocket.CLOSED);
  });

  it('releases the reserved P2P socket after a fatal signaling failure', async () => {
    const { client, iceConfigurationMock } = createClient();
    const negotiation = coordinateP2pConnection(
      client,
      sessionId,
      localDeviceId,
      peerDeviceId,
      false,
      sessionCredential
    );
    const rejection = expect(negotiation).rejects.toThrow('Session expired (invalid_session)');
    await flushSetup();

    const socket = FakeWebSocket.instances[0];
    expect(socket).toBeDefined();
    if (!socket) throw new Error('Signaling WebSocket was not created');
    socket.open();
    socket.receive({
      type: 'error',
      error: { code: 'invalid_session', message: 'Session expired' }
    });

    await rejection;
    expect(iceConfigurationMock).toHaveBeenCalledOnce();
    expect(invokeMock).toHaveBeenCalledTimes(2);
    expect(invokeMock).toHaveBeenNthCalledWith(1, 'p2p_gather', {
      sessionId,
      attemptId,
      stunServers: [configuredStunServer, 'stun:backup.sanser.example:3478'],
      preferredLocalPort: undefined
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, 'p2p_stop', { attemptId });
    expect(invokeMock).not.toHaveBeenCalledWith('p2p_punch', expect.anything());
    expect(socket.readyState).toBe(FakeWebSocket.CLOSED);
  });

  it('reports the close code and endpoint after a WebSocket handshake failure', async () => {
    const { client } = createClient();
    const negotiation = coordinateP2pConnection(
      client,
      sessionId,
      localDeviceId,
      peerDeviceId,
      true,
      sessionCredential,
      50_000
    );
    const rejection = expect(negotiation).rejects.toThrow(
      'WebSocket signaling closed before candidate exchange (code 1006; endpoint wss://sanser.example)'
    );
    await flushSetup();

    const socket = FakeWebSocket.instances[0];
    if (!socket) throw new Error('Signaling WebSocket was not created');
    socket.failConnection();

    await rejection;
    expect(invokeMock).toHaveBeenLastCalledWith('p2p_stop', { attemptId });
  });
});
