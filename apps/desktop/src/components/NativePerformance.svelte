<script lang="ts">
  import { onMount } from 'svelte';
  import { nativePerformance, metricIsFresh } from '../stores/nativePerformance';
  let now = $state(Date.now());
  onMount(() => { const timer = window.setInterval(() => { now = Date.now(); }, 1_000); return () => window.clearInterval(timer); });
  const labels: Record<string, string> = {
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
  const primary = ['controlAppRtt', 'wireRttEstimate', 'inputAckRtt', 'hostInput', 'decodeQueue', 'decodeWork', 'frameQueue'];
  const messages: Record<string, string> = {
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
      {@const fresh = !update.ended && Object.values(metrics).some((metric) => metricIsFresh(metric, now))}
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
