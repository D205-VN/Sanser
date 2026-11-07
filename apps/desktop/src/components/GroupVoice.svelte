<script lang="ts">
  import { untrack } from 'svelte';
  import { get } from 'svelte/store';
  import { invoke } from '@tauri-apps/api/core';
  import { session } from '../stores/session';
  import { host } from '../stores/host';
  import { presence } from '../stores/presence';
  import Icon from './Icon.svelte';
  import { createGroupVoice } from '../stores/groupVoice';
  import type { GroupVoiceScope } from '../lib/platform';
  let { kind, sharingId, sessionId }: GroupVoiceScope = $props();
  const voice = createGroupVoice();
  let gateway = $state(false);
  let opening = $state(false);
  let gatewayError = $state<string | null>(null);
  $effect(() => {
    const group = sharingId;
    let live = true;
    gateway = false;
    const api = untrack(() => session.client());
    if (group && api && typeof api.voiceGatewayAvailable === 'function') void api.voiceGatewayAvailable().then((available) => { if (live) gateway = available; }).catch(() => undefined);
    return () => { live = false; };
  });
  async function enhancedVoice(): Promise<void> {
    const api = session.client();
    const device = kind === 'host' ? get(host).deviceId : get(presence).deviceId;
    if (!api || !device || opening) return;
    opening = true; gatewayError = null;
    try {
      const grant = await api.voiceGatewayCredentials(sharingId, device);
      try { await invoke('open_voice_window', { grant: { ...grant, kind, sharingId, sessionId } }); }
      finally { grant.token = ''; }
    } catch { gatewayError = 'Unable to join enhanced voice. Close other desktop sessions or native voice calls, check the server, and update the native engine.'; }
    finally { opening = false; }
  }
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
  <div class="voice-heading">
    <span class="voice-glyph"><Icon name="headphones" /></span>
    <div><strong>Group voice</strong><p class="muted">Listen together. Turn on your mic when you’re ready to talk.</p></div>
    <span class="voice-state" class:live={$voice.capture} class:pending={$voice.busy} role="status">
      <Icon name={$voice.capture ? 'mic' : 'mic-off'} />
      {$voice.code === 'joining' ? 'Joining voice…' : $voice.code === 'leaving' ? 'Leaving voice…' : $voice.code === 'changing' ? 'Updating microphone…' : $voice.capture ? 'Microphone on' : ($voice.code === 'starting' || $voice.code === 'permission-requested' || $voice.code === 'device-changed') ? 'Opening microphone…' : $voice.deafened ? 'Voice deafened · microphone off' : $voice.joined ? 'Listening · microphone off' : 'Not joined · microphone off'}
    </span>
  </div>
  <div class="button-row voice-actions">
    {#if !$voice.joined}
      {#if gateway}<button class="button small" disabled={opening || $voice.busy || !$voice.available} onclick={() => void enhancedVoice()}><Icon name="headphones" />{$voice.code === 'external' ? 'Open voice controls' : 'Join enhanced voice'}</button>{/if}
      <button class="button small" disabled={$voice.busy || !$voice.available || $voice.code === 'external' || opening} onclick={() => void voice.join()}><Icon name="headphones" />Join voice</button>
      {#if !$voice.available && $voice.code !== 'unsupported'}<span class="muted">Available when the sharing connection is ready.</span>{/if}
    {:else}
      <button class="button small" class:mic-live={$voice.capture} aria-pressed={$voice.microphone} disabled={$voice.busy} onclick={() => void voice.toggleMicrophone()}><Icon name={$voice.microphone ? 'mic' : 'mic-off'} />{$voice.microphone ? 'Mute mic' : 'Unmute mic'}</button>
      <button class="button small" aria-pressed={$voice.deafened} disabled={$voice.busy} onclick={() => void voice.toggleDeafen()}><Icon name={$voice.deafened ? 'volume-off' : 'headphones'} />{$voice.deafened ? 'Hear voice' : 'Deafen voice'}</button>
      <button class="button small" disabled={$voice.busy} onclick={() => void voice.leave()}><Icon name="leave" />Leave voice</button>

    {/if}
  </div>
  {#if kind === 'host'}<p class="muted">On Windows versions that cannot exclude Sanser from audio capture, shared system sound pauses while the host is in voice to prevent echo. It resumes after leaving voice.</p>{/if}
  {#if notice}<p role="status">{notice}</p>{/if}
  {#if $voice.error}<p class="notice error" role="alert">{$voice.error}</p>{/if}
  {#if gatewayError}<p class="notice error" role="alert">{gatewayError}</p>{/if}
  {#if $voice.code === 'external'}<p class="muted">Voice is running in the floating controls. Shared system sound pauses until that voice window closes.</p>{/if}
</section>
<style>
  .group-voice { border: 1px solid var(--border); border-radius: var(--sn-card-radius); padding: 18px; margin-top: 16px; background: var(--sn-background); }
  .voice-heading { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }
  .voice-heading > div { flex: 1; min-width: 180px; }
  .voice-heading strong { font-size: 14px; }
  .voice-heading p { margin: 4px 0 0; }
  .voice-glyph { display: grid; place-items: center; flex-shrink: 0; width: 38px; height: 38px; border: 1px solid var(--border); border-radius: 11px; color: var(--accent); background: var(--surface); }
  .voice-glyph :global(svg) { width: 19px; height: 19px; }
  .voice-state { display: inline-flex; align-items: center; gap: 7px; padding: 7px 10px; border: 1px solid var(--border); border-radius: 999px; color: var(--muted); font-size: 11px; line-height: 1.4; }
  .voice-state :global(svg) { width: 14px; height: 14px; flex-shrink: 0; }
  .voice-state.live { color: var(--green); border-color: var(--green); background: var(--green-soft); }
  .voice-state.pending { color: var(--yellow); }
  .voice-actions { margin-top: 16px; gap: 8px; }
  .voice-actions .button { min-height: 36px; }
  .voice-actions .mic-live { color: var(--green); border-color: var(--green); background: var(--green-soft); }
  p { margin: 6px 0 12px; font-size: 13px; }
</style>
