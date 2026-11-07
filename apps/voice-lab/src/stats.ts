export interface StatsSource { key: object; direction: 'receive' | 'send'; records: Iterable<Record<string, unknown>> }
const counters = {
  packetsReceived: 'receivedPackets', packetsLost: 'lostPackets', concealedSamples: 'concealedSamples',
  concealmentEvents: 'concealmentEvents', totalSamplesReceived: 'receivedSamples',
  insertedSamplesForDeceleration: 'insertedSamples', removedSamplesForAcceleration: 'removedSamples'
} as const;
export interface Observation {
  elapsedMs: number;
  receivers: number;
  micPublished: boolean;
  metrics: Partial<Record<'receiverJitterMaxMs' | 'senderRemoteRttMaxMs' | 'jitterBufferResidenceAvgMs', number>>;
  deltas: Partial<Record<typeof counters[keyof typeof counters], number>>;
  baselines: number;
}
const finite = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v) && v >= 0 && v <= 1e12;
const whole = (v: unknown): v is number => finite(v) && Number.isSafeInteger(v);
interface Previous { at: number; values: Record<string, number> }
// At most four tracks x eight audio records; identifiers are private keys only.
export class VoiceMeasurements {
  private previous = new Map<object, Map<string, Previous>>();
  private samples: Observation[] = [];
  private start?: number;
  private last?: number;
  observe(sources: StatsSource[], atMs: number, micPublished: boolean): void {
    if (!finite(atMs) || (this.last !== undefined && atMs <= this.last)) return;
    this.start ??= atMs;this.last = atMs;
    const sample: Observation = { elapsedMs: atMs - this.start, receivers: 0, micPublished, metrics: {}, deltas: {}, baselines: 0 };
    const seen = new Set<object>();let bufferDelay = 0, bufferEmitted = 0;
    for (const source of sources.slice(0, 4)) {
      if (seen.has(source.key)) continue;seen.add(source.key);
      const oldRecords = this.previous.get(source.key) ?? new Map<string, Previous>();
      const nextRecords = new Map<string, Previous>();
      let scanned = 0;
      for (const record of source.records) {
        if (++scanned > 256) break;
        if (record.kind !== 'audio' && record.mediaType !== 'audio') continue;
        if (source.direction === 'send' && record.type === 'remote-inbound-rtp' && finite(record.roundTripTime) && record.roundTripTime <= 60) {
          sample.metrics.senderRemoteRttMaxMs = Math.max(sample.metrics.senderRemoteRttMaxMs ?? 0, record.roundTripTime * 1000);
        }
        if (source.direction !== 'receive' || record.type !== 'inbound-rtp' || typeof record.id !== 'string' || nextRecords.size >= 8 || nextRecords.has(record.id)) continue;
        ++sample.receivers;
        if (finite(record.jitter) && record.jitter <= 60) sample.metrics.receiverJitterMaxMs = Math.max(sample.metrics.receiverJitterMaxMs ?? 0, record.jitter * 1000);
        const values: Record<string, number> = {};
        for (const field of [...Object.keys(counters), 'jitterBufferEmittedCount']) if (whole(record[field])) values[field] = record[field];
        if (finite(record.jitterBufferDelay)) values.jitterBufferDelay = record.jitterBufferDelay;
        const old = oldRecords.get(record.id);
        const validInterval = old && atMs - old.at <= 5000;
        let baseline = !validInterval;
        const delta = (field: string): number | undefined => {
          const now = values[field], before = old?.values[field];
          if (!validInterval || now === undefined || before === undefined) return;
          if (now < before) { baseline = true;return; }
          return now - before;
        };
        for (const [field, name] of Object.entries(counters) as [string, typeof counters[keyof typeof counters]][]) {
          const change = delta(field);if (change !== undefined) sample.deltas[name] = (sample.deltas[name] ?? 0) + change;
        }
        const emitted = delta('jitterBufferEmittedCount'), delay = delta('jitterBufferDelay');
        if (emitted !== undefined && emitted > 0 && delay !== undefined) { bufferDelay += delay;bufferEmitted += emitted; }
        if (baseline) ++sample.baselines;
        nextRecords.set(record.id, { at: atMs, values });
      }
      this.previous.set(source.key, nextRecords);
    }
    for (const key of this.previous.keys()) if (!seen.has(key)) this.previous.delete(key);
    const residence = bufferEmitted ? bufferDelay / bufferEmitted * 1000 : undefined;
    if (residence !== undefined && finite(residence) && residence <= 60_000) sample.metrics.jitterBufferResidenceAvgMs = residence;
    if (this.samples.length === 120) this.samples.shift();this.samples.push(sample);
  }
  export(): object {
    const fields = ['receiverJitterMaxMs', 'senderRemoteRttMaxMs', 'jitterBufferResidenceAvgMs'] as const;
    const metrics = Object.fromEntries(fields.map((field) => {
      const values = this.samples.map((sample) => sample.metrics[field]).filter((n): n is number => n !== undefined).sort((a, b) => a - b);
      const percentile = (p: number) => values[Math.ceil(values.length * p) - 1];
      return [field, values.length ? { samples: values.length, p50Ms: percentile(.5), p95Ms: percentile(.95), maxMs: values.at(-1) } : null];
    }));
    return { schemaVersion: 1, provider: 'livekit-browser', sdkVersion: '2.23.0', metrics,
      samples: structuredClone(this.samples), limitations: [
        'Up to 120 periodic browser observations; no raw RTC reports, tokens, room/participant/device identifiers or media.',
        'Receiver jitter is the per-poll maximum reported RTP jitter. Sender RTT is to the LiveKit SFU, not peer-to-peer or end-to-end voice latency.',
        'Jitter buffer residence is a delta-based WebRTC receiver average, not screen/audio end-to-end latency.',
        'Counter deltas exclude new/reset fields and gaps over 5 seconds; not a complete-session loss percentage. Participants/tracks can change between samples.',
        'Browser concealed samples, PCM underflow calls and native Opus PLC packets have different units and must not be treated as equivalent.',
        'This is an isolated prototype, not a Sanser native performance report or proof of improved audio quality.'
      ] };
  }
}
