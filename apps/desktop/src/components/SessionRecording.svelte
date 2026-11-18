<script lang="ts">
  import { onDestroy } from 'svelte';
  import { t } from '../lib/i18n';
  import { formatDuration, openRecordingsFolder, stopReasonMessage } from '../lib/recording';
  import { viewerRecording } from '../stores/recording';

  let { sessionId }: { sessionId: string } = $props();
  const busy = $derived($viewerRecording.phase === 'asking' || $viewerRecording.phase === 'starting' || $viewerRecording.phase === 'stopping');
  const active = $derived($viewerRecording.sessionId === sessionId && $viewerRecording.phase !== 'idle');
  const live = $derived(active && $viewerRecording.status.state === 'recording');
  const message = $derived($viewerRecording.sessionId === sessionId ? $viewerRecording.message : null);

  async function toggle(): Promise<void> {
    if (active) await viewerRecording.stop();
    else await viewerRecording.start(sessionId);
  }
  // Leaving the session page keeps recording; closing the session stops it.
  onDestroy(() => { if ($viewerRecording.sessionId !== sessionId) viewerRecording.reset(); });
</script>

<div class="session-recording card card-body stack">
  <div class="recording-row">
    {#if live}
      <span class="recording-indicator" role="status"><span class="recording-dot" aria-hidden="true"></span>{$t('REC')} {formatDuration($viewerRecording.status.durationMs)}{$viewerRecording.status.audio ? ` · ${$t('with sound')}` : ''}</span>
    {:else if active}
      <span class="recording-indicator pending" role="status">{$viewerRecording.phase === 'asking' ? $t('Asking the host…') : $viewerRecording.phase === 'stopping' ? $t('Saving…') : $t('Waiting for a keyframe…')}</span>
    {/if}
    <button class="button small" class:danger={active} disabled={busy && $viewerRecording.phase !== 'starting'} onclick={toggle}>{active ? $t('Stop recording') : $t('Record')}</button>
    <button class="button small ghost" onclick={() => openRecordingsFolder()}>{$t('Open recordings folder')}</button>
  </div>
  <p class="muted">{$t('Recordings are saved on this computer in Downloads/Sanser/Recordings as MP4 (H.264, system sound as Opus). The host is told that you are recording and can refuse. Recording stops after 2 hours or 3.8 GB.')}</p>
  {#if message}<div class="notice" class:error={$viewerRecording.error} role="status">{$t(stopReasonMessage(message))}<button class="link-button" onclick={() => viewerRecording.dismiss()}>{$t('Dismiss')}</button></div>{/if}
</div>

<style>
  .recording-row { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
  .recording-indicator { display: inline-flex; gap: 6px; align-items: center; font-weight: 600; color: var(--danger, #d92d20); font-variant-numeric: tabular-nums; }
  .recording-indicator.pending { color: inherit; font-weight: 500; }
  .recording-dot { width: 10px; height: 10px; border-radius: 50%; background: currentColor; animation: recording-pulse 1.2s ease-in-out infinite; }
  @keyframes recording-pulse { 50% { opacity: 0.35; } }
  @media (prefers-reduced-motion: reduce) { .recording-dot { animation: none; } }
</style>
