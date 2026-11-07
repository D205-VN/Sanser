<script lang="ts">
  import { onMount } from 'svelte';
  import { nativePerformance, metricIsFresh, audioCounterIsFresh } from '../stores/nativePerformance';
  let now = $state(Date.now());
  onMount(() => { const timer = window.setInterval(() => { now = Date.now(); }, 1_000); return () => window.clearInterval(timer); });
  const labels: Record<string, string> = {
    voiceWorkerGap: 'Voice worker wake gap (window maximum)',
    controlAppRtt: 'Control application RTT', controlAppRttFallback: 'Control-pong RTT',
    controlSilence: 'Authenticated control silence',
    socketRtt: 'Socket RTT', wireRttEstimate: 'Wire RTT estimate', udpEchoRtt: 'UDP echo RTT',
    echoHostHold: 'Echo host hold', inputAckRtt: 'Input ACK RTT', hostInput: 'Host input work',
    inputSendQueue: 'Input send queue', hostInputQueue: 'Host input queue', hostInputQueueMax: 'Host input queue maximum',
    clientSendQueue: 'Client send queue', hostReceiveQueue: 'Host receive queue',
    hostControlWork: 'Host control work', hostSendLock: 'Host send lock', hostSendCall: 'Host send call',
    clientReceiveQueue: 'Client receive queue', udpReceiveQueue: 'UDP receive queue',
    captureWork: 'Capture work', captureWait: 'Capture wait', encodeWork: 'Encode work',
    encodeQueue: 'Encode queue', encodeAndPublish: 'Encode + publish', sendTotal: 'Send total',
    pacingWait: 'Pacing wait', socketSend: 'Socket send', arrivalJitter: 'Arrival jitter',
    decodeQueue: 'Decode queue', decodeQueueMax: 'Decode queue maximum', decodeWork: 'Decode work', colorConversion: 'Color conversion',
    frameQueue: 'Frame queue', renderGpu: 'Render GPU', paintWork: 'Paint work', decodedToPaint: 'Decoded → paint'
  };
  const audioLabels: Record<string, string> = {
    voiceCapturedFrames: 'Voice captured PCM frames', voiceDecodedPackets: 'Voice decoded packets',
    voiceConcealedPackets: 'Voice concealment packets', voiceFecAttempts: 'Voice FEC attempts',
    voiceDroppedPackets: 'Voice stale/queue packet drops', voiceLatePackets: 'Voice late/duplicate packets',
    voiceInvalidPackets: 'Voice invalid packets', voiceRawClippedFrames: 'Mic near-full-scale frames',
    voiceCleanClippedFrames: 'Processed mic near-full-scale frames', voiceLimitedFrames: 'Voice overload-limited frames',
    voiceCaptureQueueDrops: 'Mic capture queue drops', voiceIncomingQueueDrops: 'Voice incoming queue drops',
    voiceStaleCaptureBlocks: 'Mic stale capture blocks', voicePacketsSent: 'Voice packets sent',
    voicePcmUnderflows: 'Voice PCM underflows', voicePcmCapacityDrops: 'Voice PCM capacity drops',
    voicePcmStaleDrops: 'Voice PCM stale drops', voicePcmReprimes: 'Voice PCM reprimes',
    voiceAecBlocks: 'AEC processed blocks', voiceAecMissingReference: 'AEC missing reference blocks',
    voiceAecDryBlocks: 'AEC dry blocks', voiceAecResets: 'AEC resets',
    systemDecodedPackets: 'System sound decoded packets', systemConcealedPackets: 'System sound concealment packets',
    systemDecodeErrors: 'System sound decode errors', systemPcmUnderflows: 'System sound PCM underflows',
    systemPcmCapacityDrops: 'System sound PCM capacity drops', systemPcmStaleDrops: 'System sound PCM stale drops',
    systemPcmReprimes: 'System sound PCM reprimes', systemPcmResets: 'System sound PCM resets',
    systemJitterUnderflows: 'System sound jitter underflows', systemJitterQueueDrops: 'System sound jitter queue drops',
    systemOutputErrors: 'System sound output errors', systemOutputQueueDrops: 'System sound output queue drops',
    systemOutputRecoveries: 'System sound output recoveries', systemOutputRouteChanges: 'System sound route changes',
    systemConverterErrors: 'System sound converter errors', systemClockStalls: 'System sound clock stalls'
  };
  const primary = ['controlAppRtt', 'wireRttEstimate', 'inputAckRtt', 'hostInput', 'decodeQueue', 'decodeWork', 'frameQueue'];
  const messages: Record<string, string> = {
    'voice-playout-underflow': 'Voice playback ran short of PCM in a recent observed interval.',
    'voice-capture-drops': 'Microphone blocks were dropped before encoding.',
    'voice-packet-concealment': 'Voice used missing-packet concealment. This alone does not identify the network as the cause.',
    'voice-signal-clipping': 'Microphone PCM reached near-full-scale. Check microphone gain and the audio path.',
    'system-audio-underflow': 'Shared system sound ran short of buffered audio in a recent observed interval.',
    'system-audio-concealment': 'Shared system sound used missing-packet concealment.',
    'path-or-os-delay': 'The path or OS/socket scheduling has high delay; this does not identify the router or ISP as the cause.',
    'control-round-trip-high': 'Control RTT is high. It includes application queues and peer processing; network delay has not been isolated.',
    'input-delivery-overhead': 'Input ACK is slower than control RTT. Inspect input handling and retries.',
    'input-send-queue-high': 'Input is waiting in the native sender queue.',
    'host-input-queue-high': 'Input is waiting for the host to execute earlier actions.',
    'host-input-work-high': 'The host operating system is taking time to execute input.',
    'capture-over-budget': 'Capture work exceeds the requested frame budget.',
    'encode-over-budget': 'Encoding exceeds the requested frame budget.',
    'encode-publish-over-budget': 'Encode + publish exceeds the frame budget. This combined measurement does not isolate the encoder.',
    'decode-over-budget': 'The decode call exceeds the requested frame budget.',
    'decode-queue-high': 'Encoded frames are waiting in the decode queue.',
    'frame-queue-high': 'Decoded frames are waiting for presentation.',
    'receive-queue-high': 'Received UDP packets are waiting for processing.'
  };
</script>

<article class="card card-body stack">
  <div><h2 class="card-title">Measured native performance</h2><p class="card-subtitle">One measurement view for Legacy and SNV2.</p></div>
  {#if !$nativePerformance.client && !$nativePerformance.host}
    <p class="muted">Awaiting native measurements. No measured latency is available yet.</p>
  {/if}
  {#each ['client', 'host'] as kind}
    {@const update = $nativePerformance[kind as 'client' | 'host']}
    {#if update}
      {@const metrics = update.performance.metrics}
      {@const audio = update.performance.audio?.counters ?? {}}
      {@const fresh = !update.ended && (Object.values(metrics).some((metric) => metricIsFresh(metric, now)) || Object.values(audio).some((counter) => audioCounterIsFresh(counter, now)))}
      <div class="stack">
        <strong>{kind === 'client' ? 'Viewing' : 'Hosting'} · {update.wireProtocol.toUpperCase()} · {update.route}{update.relayTransport ? ` / ${update.relayTransport.toUpperCase()}` : ''} · {update.ended ? 'Ended' : fresh ? 'Receiving measurements' : 'Awaiting fresh measurements'}</strong>
        {#if fresh}
          {#each update.findings.filter((finding) => messages[finding]) as finding}<p class="notice warning">{messages[finding]}</p>{/each}
        {/if}
        <div class="performance-table">
          <table>
            <thead><tr><th>Stage</th><th>Latest</th><th>P50</th><th>P95</th><th>Samples</th></tr></thead>
            <tbody>
              {#each [...new Set([...primary, ...Object.keys(metrics).filter((key) => labels[key])])] as key}
                {@const metric = metrics[key]}
                <tr><th>{labels[key]}</th><td>{metric ? !update.ended && metricIsFresh(metric, now) ? `${metric.latestMs.toFixed(1)} ms` : 'Stale' : 'Not measured'}</td><td>{metric ? `${metric.p50Ms.toFixed(1)} ms` : '—'}</td><td>{metric ? `${metric.p95Ms.toFixed(1)} ms` : '—'}</td><td>{metric?.samples ?? '—'}</td></tr>
              {/each}
            </tbody>
          </table>
        </div>
        <details>
          <summary>Audio and voice counters</summary>
          {#if Object.keys(audio).length === 0}
            <p class="muted">No audio measurements received. This does not confirm working sound.</p>
          {:else}
            <div class="performance-table">
              <table aria-label="Audio and voice counters">
                <thead><tr><th>Counter</th><th>Latest interval increase</th><th>Observed increase</th><th>Observed time</th><th>Counter/voice resets</th></tr></thead>
                <tbody>
                  {#each Object.entries(audio).filter(([key]) => audioLabels[key]) as [key, counter]}
                    <tr><th>{audioLabels[key]}</th><td>{update.ended || !audioCounterIsFresh(counter, now) ? 'Stale' : counter.delta === null ? 'Baseline' : `+${counter.delta} / ${((counter.intervalMs ?? 0) / 1_000).toFixed(1)} s`}</td><td>{counter.observedIncrease}</td><td>{(counter.observedDurationMs / 1_000).toFixed(1)} s</td><td>{counter.resets}</td></tr>
                  {/each}
                </tbody>
              </table>
            </div>
            <p class="muted">Counts are not milliseconds or packet-loss percentages. The first sample, counter reset or gap over 5 seconds establishes a baseline; those gaps are excluded from observed increases. Device, consent and voice changes can reset counters. FEC attempts do not confirm recovery, and AEC dry blocks do not prove echo.</p>
          {/if}
        </details>
      </div>
    {/if}
  {/each}
  <p class="muted">RTT is round trip, not one-way or screen-to-screen delay. SNV2 input ACK confirms ordered queue acceptance; host input queue and work measure execution separately. P50/P95 summarize up to 120 periodic observations; some are interval averages, not individual frames. Combined stages can overlap and must not be added together. Missing or stale data does not mean zero latency.</p>
</article>

<style>
  .performance-table { overflow-x: auto; }
  table { width: 100%; border-collapse: collapse; font-size: 0.875rem; }
  th, td { padding: 0.65rem; text-align: right; white-space: nowrap; border-bottom: 1px solid var(--border, #303744); }
  th:first-child { text-align: left; }
  tbody th { font-weight: 500; }
</style>
