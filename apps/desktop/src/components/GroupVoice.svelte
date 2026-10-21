<script lang="ts">
  import { untrack } from 'svelte';
  import { createGroupVoice } from '../stores/groupVoice';
  import type { GroupVoiceScope } from '../lib/platform';
  let { kind, sharingId, sessionId }: GroupVoiceScope = $props();
  const voice = createGroupVoice();
  $effect(() => {
    const target = { kind, sharingId, sessionId };
    return untrack(() => {
      voice.attach(target); void voice.refresh();
      const timer = window.setInterval(() => void voice.refresh(), 250);
      return () => { window.clearInterval(timer); void voice.detach(); };
    });
  });
  const notice = $derived($voice.code === 'permission-denied' ? 'Microphone access was denied. Allow Sanser in system privacy settings, then unmute again.' : $voice.code === 'no-device' ? 'No microphone is available. Connect a microphone and unmute again.' : $voice.code === 'unavailable' ? 'Voice capture is unavailable on this computer.' : $voice.code === 'unsupported' ? 'Update Sanser on this computer to enable group voice. Screen sharing continues normally.' : $voice.code === 'device-changed' ? 'The microphone changed or disconnected. Reconnecting to your system microphone…' : $voice.code === 'permission-requested' ? 'Allow microphone access in the system permission dialog.' : null);
</script>
<section class="group-voice" aria-label="Group voice">
  <div><strong>Group voice</strong><p class="muted">Talk to the host and other viewers. Joining listens only; unmute to allow microphone access. Use headphones to reduce echo.</p></div>
  <div class="button-row">
    {#if !$voice.joined}
      <button class="button small" disabled={$voice.busy || !$voice.available} onclick={() => void voice.join()}>Join voice</button>
      {#if !$voice.available && $voice.code !== 'unsupported'}<span class="muted">Available when the sharing connection is ready.</span>{/if}
    {:else}
      <button class="button small" disabled={$voice.busy} onclick={() => void voice.toggleMicrophone()}>{$voice.microphone ? 'Mute mic' : 'Unmute mic'}</button>
      <button class="button small" disabled={$voice.busy} onclick={() => void voice.toggleDeafen()}>{$voice.deafened ? 'Hear voice' : 'Deafen voice'}</button>
      <button class="button small" disabled={$voice.busy} onclick={() => void voice.leave()}>Leave voice</button>
      <span role="status" class="muted">{$voice.code === 'joining' ? 'Joining voice…' : $voice.code === 'leaving' ? 'Leaving voice…' : $voice.code === 'changing' ? 'Updating microphone…' : $voice.capture ? 'Microphone on' : ($voice.code === 'starting' || $voice.code === 'permission-requested' || $voice.code === 'device-changed') ? 'Opening microphone…' : $voice.deafened ? 'Voice deafened · microphone off' : 'Listening · microphone off'}</span>
    {/if}
  </div>
  {#if kind === 'host'}<p class="muted">On Windows versions that cannot exclude Sanser from audio capture, shared system sound pauses while the host is in voice to prevent echo. It resumes after leaving voice.</p>{/if}
  {#if notice}<p role="status">{notice}</p>{/if}
  {#if $voice.error}<p class="notice error" role="alert">{$voice.error}</p>{/if}
</section>
<style>
  .group-voice { border-top: 1px solid var(--border); padding-top: 16px; margin-top: 16px; }
  p { margin: 6px 0 12px; font-size: 13px; }
</style>
