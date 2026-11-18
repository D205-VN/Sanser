<script lang="ts">
  import { t, tk } from '../lib/i18n';
  import { onMount } from 'svelte';
  import { nativePerformance, metricIsFresh, audioCounterIsFresh } from '../stores/nativePerformance';
  let now = $state(Date.now());
  onMount(() => { const timer = window.setInterval(() => { now = Date.now(); }, 1_000); return () => window.clearInterval(timer); });
  const labels: Record<string, string> = {
    voiceWorkerGap: tk('Voice worker wake gap (window maximum)'),
    controlAppRtt: tk('Control application RTT'), controlAppRttFallback: tk('Control-pong RTT'),
    controlSilence: tk('Authenticated control silence'),
    socketRtt: tk('Socket RTT'), wireRttEstimate: tk('Wire RTT estimate'), udpEchoRtt: tk('UDP echo RTT'),
    echoHostHold: tk('Echo host hold'), inputAckRtt: tk('Input ACK RTT'), hostInput: tk('Host input work'),
    inputSendQueue: tk('Input send queue'), hostInputQueue: tk('Host input queue'), hostInputQueueMax: tk('Host input queue maximum'),
    clientSendQueue: tk('Client send queue'), hostReceiveQueue: tk('Host receive queue'),
    hostControlWork: tk('Host control work'), hostSendLock: tk('Host send lock'), hostSendCall: tk('Host send call'),
    clientReceiveQueue: tk('Client receive queue'), udpReceiveQueue: tk('UDP receive queue'),
    captureWork: tk('Capture work'), captureWait: tk('Capture wait'), encodeWork: tk('Encode work'),
    encodeQueue: tk('Encode queue'), encodeAndPublish: tk('Encode + publish'), sendTotal: tk('Send total'),
    pacingWait: tk('Pacing wait'), socketSend: tk('Socket send'), arrivalJitter: tk('Arrival jitter'),
    decodeQueue: tk('Decode queue'), decodeQueueMax: tk('Decode queue maximum'), decodeWork: tk('Decode work'), colorConversion: tk('Color conversion'),
    frameQueue: tk('Frame queue'), renderGpu: tk('Render GPU'), paintWork: tk('Paint work'), decodedToPaint: tk('Decoded → paint')
  };
  const audioLabels: Record<string, string> = {
    voiceCapturedFrames: tk('Voice captured PCM frames'), voiceDecodedPackets: tk('Voice decoded packets'),
    voiceConcealedPackets: tk('Voice concealment packets'), voiceFecAttempts: tk('Voice FEC attempts'),
    voiceDroppedPackets: tk('Voice stale/queue packet drops'), voiceLatePackets: tk('Voice late/duplicate packets'),
    voiceInvalidPackets: tk('Voice invalid packets'), voiceRawClippedFrames: tk('Mic near-full-scale frames'),
    voiceCleanClippedFrames: tk('Processed mic near-full-scale frames'), voiceLimitedFrames: tk('Voice overload-limited frames'),
    voiceCaptureQueueDrops: tk('Mic capture queue drops'), voiceIncomingQueueDrops: tk('Voice incoming queue drops'),
    voiceStaleCaptureBlocks: tk('Mic stale capture blocks'), voicePacketsSent: tk('Voice packets sent'),
    voicePcmUnderflows: tk('Voice PCM underflows'), voicePcmCapacityDrops: tk('Voice PCM capacity drops'),
    voicePcmStaleDrops: tk('Voice PCM stale drops'), voicePcmReprimes: tk('Voice PCM reprimes'),
    voiceAecBlocks: tk('AEC processed blocks'), voiceAecMissingReference: tk('AEC missing reference blocks'),
    voiceAecDryBlocks: tk('AEC dry blocks'), voiceAecResets: tk('AEC resets'),
    systemDecodedPackets: tk('System sound decoded packets'), systemConcealedPackets: tk('System sound concealment packets'),
    systemDecodeErrors: tk('System sound decode errors'), systemPcmUnderflows: tk('System sound PCM underflows'),
    systemPcmCapacityDrops: tk('System sound PCM capacity drops'), systemPcmStaleDrops: tk('System sound PCM stale drops'),
    systemPcmReprimes: tk('System sound PCM reprimes'), systemPcmResets: tk('System sound PCM resets'),
    systemJitterUnderflows: tk('System sound jitter underflows'), systemJitterQueueDrops: tk('System sound jitter queue drops'),
    systemOutputErrors: tk('System sound output errors'), systemOutputQueueDrops: tk('System sound output queue drops'),
    systemOutputRecoveries: tk('System sound output recoveries'), systemOutputRouteChanges: tk('System sound route changes'),
    systemConverterErrors: tk('System sound converter errors'), systemClockStalls: tk('System sound clock stalls')
  };
  const primary = ['controlAppRtt', 'wireRttEstimate', 'inputAckRtt', 'hostInput', 'decodeQueue', 'decodeWork', 'frameQueue'];
  const messages: Record<string, string> = {
    'voice-playout-underflow': tk('Voice playback ran short of PCM in a recent observed interval.'),
    'voice-capture-drops': tk('Microphone blocks were dropped before encoding.'),
    'voice-packet-concealment': tk('Voice used missing-packet concealment. This alone does not identify the network as the cause.'),
    'voice-signal-clipping': tk('Microphone PCM reached near-full-scale. Check microphone gain and the audio path.'),
    'system-audio-underflow': tk('Shared system sound ran short of buffered audio in a recent observed interval.'),
    'system-audio-concealment': tk('Shared system sound used missing-packet concealment.'),
    'path-or-os-delay': tk('The path or OS/socket scheduling has high delay; this does not identify the router or ISP as the cause.'),
    'control-round-trip-high': tk('Control RTT is high. It includes application queues and peer processing; network delay has not been isolated.'),
    'input-delivery-overhead': tk('Input ACK is slower than control RTT. Inspect input handling and retries.'),
    'input-send-queue-high': tk('Input is waiting in the native sender queue.'),
    'host-input-queue-high': tk('Input is waiting for the host to execute earlier actions.'),
    'host-input-work-high': tk('The host operating system is taking time to execute input.'),
    'capture-over-budget': tk('Capture work exceeds the requested frame budget.'),
    'encode-over-budget': tk('Encoding exceeds the requested frame budget.'),
    'encode-publish-over-budget': tk('Encode + publish exceeds the frame budget. This combined measurement does not isolate the encoder.'),
    'decode-over-budget': tk('The decode call exceeds the requested frame budget.'),
    'decode-queue-high': tk('Encoded frames are waiting in the decode queue.'),
    'frame-queue-high': tk('Decoded frames are waiting for presentation.'),
    'receive-queue-high': tk('Received UDP packets are waiting for processing.')
  };
</script>

<article class="card card-body stack">
  <div><h2 class="card-title">{$t('Measured native performance')}</h2><p class="card-subtitle">{$t('One measurement view for Legacy and SNV2.')}</p></div>
  {#if !$nativePerformance.client && !$nativePerformance.host}
    <p class="muted">{$t('Awaiting native measurements. No measured latency is available yet.')}</p>
  {/if}
  {#each ['client', 'host'] as kind}
    {@const update = $nativePerformance[kind as 'client' | 'host']}
    {#if update}
      {@const metrics = update.performance.metrics}
      {@const audio = update.performance.audio?.counters ?? {}}
      {@const fresh = !update.ended && (Object.values(metrics).some((metric) => metricIsFresh(metric, now)) || Object.values(audio).some((counter) => audioCounterIsFresh(counter, now)))}
      <div class="stack">
        <strong>{kind === 'client' ? $t('Viewing') : $t('Hosting')} · {update.wireProtocol.toUpperCase()} · {update.route}{update.relayTransport ? ` / ${update.relayTransport.toUpperCase()}` : ''} · {update.ended ? $t('Ended') : fresh ? $t('Receiving measurements') : $t('Awaiting fresh measurements')}</strong>
        {#if fresh}
          {#each update.findings.filter((finding) => messages[finding]) as finding}<p class="notice warning">{$t(messages[finding] ?? finding)}</p>{/each}
        {/if}
        <div class="performance-table">
          <table>
            <thead><tr><th>{$t('Stage')}</th><th>{$t('Latest')}</th><th>P50</th><th>P95</th><th>{$t('Samples')}</th></tr></thead>
            <tbody>
              {#each [...new Set([...primary, ...Object.keys(metrics).filter((key) => labels[key])])] as key}
                {@const metric = metrics[key]}
                <tr><th>{$t(labels[key] ?? key)}</th><td>{metric ? !update.ended && metricIsFresh(metric, now) ? `${metric.latestMs.toFixed(1)} ms` : $t('Stale') : $t('Not measured')}</td><td>{metric ? `${metric.p50Ms.toFixed(1)} ms` : '—'}</td><td>{metric ? `${metric.p95Ms.toFixed(1)} ms` : '—'}</td><td>{metric?.samples ?? '—'}</td></tr>
              {/each}
            </tbody>
          </table>
        </div>
        <details>
          <summary>{$t('Audio and voice counters')}</summary>
          {#if Object.keys(audio).length === 0}
            <p class="muted">{$t('No audio measurements received. This does not confirm working sound.')}</p>
          {:else}
            <div class="performance-table">
              <table aria-label={$t('Audio and voice counters')}>
                <thead><tr><th>{$t('Counter')}</th><th>{$t('Latest interval increase')}</th><th>{$t('Observed increase')}</th><th>{$t('Observed time')}</th><th>{$t('Counter/voice resets')}</th></tr></thead>
                <tbody>
                  {#each Object.entries(audio).filter(([key]) => audioLabels[key]) as [key, counter]}
                    <tr><th>{$t(audioLabels[key] ?? key)}</th><td>{update.ended || !audioCounterIsFresh(counter, now) ? $t('Stale') : counter.delta === null ? $t('Baseline') : `+${counter.delta} / ${((counter.intervalMs ?? 0) / 1_000).toFixed(1)} s`}</td><td>{counter.observedIncrease}</td><td>{(counter.observedDurationMs / 1_000).toFixed(1)} s</td><td>{counter.resets}</td></tr>
                  {/each}
                </tbody>
              </table>
            </div>
            <p class="muted">{$t('Counts are not milliseconds or packet-loss percentages. The first sample, counter reset or gap over 5 seconds establishes a baseline; those gaps are excluded from observed increases. Device, consent and voice changes can reset counters. FEC attempts do not confirm recovery, and AEC dry blocks do not prove echo.')}</p>
          {/if}
        </details>
      </div>
    {/if}
  {/each}
  <p class="muted">{$t('RTT is round trip, not one-way or screen-to-screen delay. SNV2 input ACK confirms ordered queue acceptance; host input queue and work measure execution separately. P50/P95 summarize up to 120 periodic observations; some are interval averages, not individual frames. Combined stages can overlap and must not be added together. Missing or stale data does not mean zero latency.')}</p>
</article>

<style>
  .performance-table { overflow-x: auto; }
  table { width: 100%; border-collapse: collapse; font-size: 0.875rem; }
  th, td { padding: 0.65rem; text-align: right; white-space: nowrap; border-bottom: 1px solid var(--border, #303744); }
  th:first-child { text-align: left; }
  tbody th { font-weight: 500; }
</style>
