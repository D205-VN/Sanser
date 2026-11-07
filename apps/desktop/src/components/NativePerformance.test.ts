import { cleanup, render, screen, within } from '@testing-library/svelte';
import { afterEach, expect, it } from 'vitest';
import NativePerformance from './NativePerformance.svelte';
import { nativePerformance } from '../stores/nativePerformance';

function audioRow(label: string): HTMLTableRowElement {
  const row = screen.getByText(label).closest('tr');
  if (!row) throw new Error(`Missing audio counter row: ${label}`);
  return row;
}

afterEach(() => { cleanup(); nativePerformance.clear(); });
it('shows missing wire RTT honestly and keeps distinct input/control measurements', () => {
  const metric = (value: number) => ({ latestMs: value, p50Ms: value, p95Ms: value, maxMs: value, samples: 1, updatedAtMs: Date.now(), source: 'snv2-control' });
  nativePerformance.update({ reportId: 'viewer', kind: 'client', wireProtocol: 'snv2', route: 'direct', relayTransport: null,
    startedAtMs: 1, ended: false, performance: { metrics: { controlAppRtt: metric(12), inputAckRtt: metric(190), decodeWork: metric(0) } },
    findings: ['input-delivery-overhead'] });
  render(NativePerformance);
  expect(screen.getByText(/Input ACK is slower/)).toBeInTheDocument();
  const wire = screen.getByRole('row', { name: /Wire RTT estimate/ });
  expect(within(wire).getByText('Not measured')).toBeInTheDocument();
  const decode = screen.getByRole('row', { name: /Decode work/ });
  expect(within(decode).getAllByText('0.0 ms')).toHaveLength(3);
  expect(screen.getByText(/periodic observations/)).toBeInTheDocument();
});
it('marks ended observations stale instead of displaying old latency as live', () => {
  nativePerformance.update({ reportId: 'ended', kind: 'host', wireProtocol: 'snv2', route: 'relay', relayTransport: 'udp',
    startedAtMs: 1, ended: true, performance: { metrics: { encodeQueue: { latestMs: 100, p50Ms: 50, p95Ms: 100, maxMs: 100, samples: 2, updatedAtMs: Date.now(), source: 'snv2-host' } } }, findings: ['decode-queue-high'] });
  render(NativePerformance);
  expect(screen.getByText(/Hosting.*Ended/)).toBeInTheDocument();
  expect(screen.queryByText(/Encoded frames are waiting/)).not.toBeInTheDocument();
  expect(within(screen.getByRole('row', { name: /Encode queue/ })).getByText('Stale')).toBeInTheDocument();
});
it('distinguishes receipt ACK from queued host execution', () => {
  const metric = (value: number) => ({ latestMs: value, p50Ms: value, p95Ms: value, maxMs: value, samples: 1, updatedAtMs: Date.now(), source: 'snv2-host-input' });
  nativePerformance.update({ reportId: 'input', kind: 'client', wireProtocol: 'snv2', route: 'direct', relayTransport: null,
    startedAtMs: 1, ended: false, performance: { metrics: { inputAckRtt: metric(12), hostInputQueue: metric(60), hostInput: metric(8) } },
    findings: ['host-input-queue-high', 'host-input-work-high'] });
  render(NativePerformance);
  expect(screen.getByText(/SNV2 input ACK confirms ordered queue acceptance/)).toBeInTheDocument();
  expect(screen.getByText('Input is waiting for the host to execute earlier actions.')).toBeInTheDocument();
  expect(within(screen.getByRole('row', { name: /^Host input queue / })).getAllByText('60.0 ms')).toHaveLength(3);
  expect(within(screen.getByRole('row', { name: /^Host input work / })).getAllByText('8.0 ms')).toHaveLength(3);
});
it('shows voice-only counters as counts and separates baseline, observed increase and stale data', () => {
  const counter = { latest: 120, delta: 3, intervalMs: 1000, observedIncrease: 5, observedDurationMs: 2000,
    resets: 1, samples: 4, updatedAtMs: Date.now(), source: 'native-voice' };
  nativePerformance.update({ reportId: 'voice', kind: 'host', wireProtocol: 'snv2', route: 'direct', relayTransport: null,
    startedAtMs: 1, ended: false, performance: { metrics: {}, audio: { counters: {
      voicePcmUnderflows: counter, voiceCaptureQueueDrops: { ...counter, delta: null, intervalMs: null },
      systemPcmUnderflows: { ...counter, updatedAtMs: 1 }
    } } }, findings: ['voice-playout-underflow'] });
  render(NativePerformance);
  expect(screen.getByText(/Hosting.*Receiving measurements/)).toBeInTheDocument();
  expect(screen.getByText(/Voice playback ran short/)).toBeInTheDocument();
  const row = audioRow('Voice PCM underflows');
  expect(within(row).getByText('+3 / 1.0 s')).toBeInTheDocument();
  expect(within(row).queryByText(/ms/)).not.toBeInTheDocument();
  expect(within(audioRow('Mic capture queue drops')).getByText('Baseline')).toBeInTheDocument();
  expect(within(audioRow('System sound PCM underflows')).getByText('Stale')).toBeInTheDocument();
});
it('marks ended voice counters stale and suppresses their findings', () => {
  nativePerformance.update({ reportId: 'voice-ended', kind: 'client', wireProtocol: 'snv2', route: 'relay', relayTransport: 'wss',
    startedAtMs: 1, ended: true, performance: { metrics: {}, audio: { counters: { voicePcmUnderflows: {
      latest: 2, delta: 1, intervalMs: 1000, observedIncrease: 1, observedDurationMs: 1000, resets: 0, samples: 2,
      updatedAtMs: Date.now(), source: 'native-voice'
    } } } }, findings: ['voice-playout-underflow'] });
  render(NativePerformance);
  expect(screen.queryByText(/Voice playback ran short/)).not.toBeInTheDocument();
  expect(within(audioRow('Voice PCM underflows')).getByText('Stale')).toBeInTheDocument();
});
