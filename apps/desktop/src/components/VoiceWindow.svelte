<script lang="ts">
  import { t, tk } from '../lib/i18n';
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import Icon from './Icon.svelte';
  import { VoiceSession } from '../../../voice-lab/src/session';
  import { LiveKitVoiceRoom } from '../../../voice-lab/src/livekit';
  let sink: HTMLDivElement;
  const voice = new VoiceSession(() => new LiveKitVoiceRoom(sink));
  let status = $state(voice.state);
  let error = $state<string | null>(null);
  let host = $state(false);
  const notices: Record<string, string> = {
    'connection-failed': tk('Unable to join voice. Close and try again.'),
    'connection-lost': tk('Voice disconnected. Your microphone is off.'),
    'microphone-denied': tk('Allow microphone access in system settings, then try again.'),
    'microphone-failed': tk('The microphone stopped. Check the device, then unmute again.'),
    'room-capacity': tk('The voice room is full.'), 'invalid-config': tk('Voice configuration is unavailable.')
  };
  async function leave(): Promise<void> {
    voice.leave();
    await getCurrentWindow().close();
  }
  async function stopSharing(): Promise<void> {
    voice.leave();
    try { await invoke('stop_voice_sharing'); await getCurrentWindow().close(); }
    catch { error = $t('Unable to stop sharing. Open Sanser and choose Stop sharing.'); }
  }
  onMount(() => {
    let live = true;
    const isLive = () => live;
    const unsubscribe = voice.subscribe((value) => { status = value; });
    const stop = () => voice.leave();
    window.addEventListener('pagehide', stop);
    void invoke<{ url: string; token: string; kind: string }>('take_voice_window_grant').then(async (grant) => {
      host = grant.kind === 'host';
      try {
        if (isLive()) {
          await voice.join(grant.url, grant.token);
          if (isLive() && voice.state.joined) await invoke('voice_window_ready');
        }
      }
      finally { grant.token = ''; }
    }).catch(() => {
      voice.leave();
      if (live) error = $t('Unable to open voice. Close and join again from Sanser.');
    });
    return () => { live = false; window.removeEventListener('pagehide', stop); voice.leave(); unsubscribe(); };
  });
</script>
<main>
  <header>
    <div class="identity" data-tauri-drag-region>
      <Icon name="headphones" /><div data-tauri-drag-region><strong>{host ? $t('Screen sharing') : $t('Voice chat')}</strong><span data-tauri-drag-region>{status.joined ? `${status.members} in voice` : status.phase === 'joining' ? $t('Connecting…') : $t('Microphone off')}</span></div>
    </div>
    <button class:live={status.microphone} aria-label={status.microphone ? $t('Mute mic') : $t('Unmute mic')} aria-pressed={status.microphone}
      disabled={!status.joined || status.micPending} onclick={() => void voice.setMicrophone(!status.microphone)}>
      <Icon name={status.microphone ? 'mic' : 'mic-off'} />
    </button>
    <button aria-label={status.deafened ? $t('Hear voice') : $t('Deafen voice')} aria-pressed={status.deafened} disabled={!status.joined}
      onclick={() => voice.setDeafened(!status.deafened)}><Icon name={status.deafened ? 'volume-off' : 'headphones'} /></button>
    <button class="leave" aria-label={$t('Leave voice')} onclick={() => void leave()}><Icon name="leave" /></button>
    {#if host}<button class="leave" aria-label={$t('Stop sharing')} onclick={() => void stopSharing()}><Icon name="stop" /></button>{/if}
  </header>
  <p class:failure={error !== null || status.error !== null} role="status">{(error ? $t(error) : null) ?? (status.error ? $t(notices[status.error] ?? status.error) : status.microphone ? $t('Your microphone is on.') : $t('Listening. Unmute when you’re ready to talk.'))}</p>
  {#if status.playbackBlocked}<button class="resume" onclick={() => void voice.resumeAudio()}>{$t('Play voice audio')}</button>{/if}
  {#if host}<small>{$t('Shared system sound pauses during voice to prevent echo.')}</small>{/if}
  <div bind:this={sink} class="audio-sink"></div>
</main>
<style>
  :global(body) { margin: 0; background: var(--sn-background); color: var(--sn-text); font: 13px system-ui,sans-serif; }
  main { padding: 15px; min-height: 160px; box-sizing: border-box; border: 1px solid var(--sn-border); border-radius: 14px; }
  header { display:flex; align-items:center; gap:8px; }
  .identity { display:flex; align-items:center; gap:10px; flex:1; min-width:0; cursor:grab; }
  .identity :global(svg) { width:22px; height:22px; color:var(--sn-success); }
  strong,span { display:block; } span { font-size:12px; color:var(--sn-muted); margin-top:4px; }
  button { display:grid; place-items:center; width:38px; height:36px; padding:0; border:1px solid var(--sn-border); border-radius:9px; background:var(--sn-surface); color:var(--sn-text); cursor:pointer; }
  button :global(svg) { width:18px; height:18px; } button:disabled { opacity:.4; cursor:default; }
  button:focus-visible { outline:2px solid var(--sn-accent); outline-offset:2px; }
  .live { color:var(--sn-success); background:var(--sn-success-soft); } .leave,.failure { color:var(--sn-danger); }
  p { margin:15px 0 8px; line-height:1.4; color:var(--sn-muted); } small { display:block; color:var(--sn-muted); font-size:12px; }
  .resume { display:block; width:auto; padding:5px 10px; height:auto; margin-bottom:7px; }
  .audio-sink { display:none; }
</style>
