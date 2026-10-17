import { cleanup, render, screen, within } from '@testing-library/svelte';
import { afterEach, expect, it } from 'vitest';
import NativePerformance from './NativePerformance.svelte';
import { nativePerformance } from '../stores/nativePerformance';

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
