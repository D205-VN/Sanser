<script lang="ts">
  import { t } from '../lib/i18n';
  import { onMount } from 'svelte';
  import Toggle from './Toggle.svelte';
  import { windowsUacStatus } from '../lib/platform';
  import { preferences } from '../stores/preferences';
  import type { RuntimeStatus } from '../lib/types';

  // Host-only: these values come from this computer's preferences and are
  // passed to the host engine at launch. Nothing a viewer sends can change them.
  let { runtime, onerror }: { runtime: RuntimeStatus; onerror?: (message: string) => void } = $props();
  const windows = $derived(runtime.platform.toLowerCase().includes('win'));
  const mac = $derived(runtime.platform.toLowerCase().includes('mac'));
  const disabled = $derived(runtime.capabilities.hostEngine.state !== 'available');
  let helperReady = $state(false);
  onMount(() => {
    if (!runtime.platform.toLowerCase().includes('win')) return;
    void windowsUacStatus().then((status) => { helperReady = status.ready; }).catch(() => { helperReady = false; });
  });
  const blockAvailable = $derived(windows && helperReady);
  async function save(update: Partial<typeof $preferences.host>): Promise<void> {
    try { await preferences.save({ ...$preferences, host: { ...$preferences.host, ...update } }); }
    catch (error) { onerror?.(error instanceof Error ? error.message : $t('Unable to save host settings')); }
  }
</script>

<Toggle checked={$preferences.host.privacyScreen} {disabled} label={$t('Privacy screen')}
  description={$t('Black out this computer\'s monitors while someone controls it. The viewer still sees your desktop. Only you can turn this on, here. Applies from the next connection.')}
  onchange={(value) => save(value ? { privacyScreen: true } : { privacyScreen: false, privacyBlockInput: false })} />
{#if $preferences.host.privacyScreen}
  <Toggle checked={$preferences.host.privacyBlockInput && blockAvailable} disabled={disabled || !blockAvailable} label={$t('Block local keyboard and mouse')}
    description={blockAvailable ? $t('Ignore this computer\'s own keyboard and mouse while connected. Press Ctrl+Alt+Del to get them back at any time.')
      : windows ? $t('Unavailable: turn on Windows administrator prompts support first.') : $t('Unavailable on this operating system.')}
    onchange={(value) => save({ privacyBlockInput: value })} />
  <p class="muted">{windows ? $t('To remove the black screen on this computer during a session, press Ctrl+Alt+Shift+P. It also disappears when the session ends.')
    : mac ? $t('To remove the black screen on this computer during a session, press Control+Option+Command+P. It also disappears when the session ends.')
    : $t('The black screen disappears when the session ends.')}</p>
{/if}
{#if windows || mac}
  <p class="muted control-cutoff">{windows ? $t('Stop remote control: press Ctrl+Alt+K on this computer during a session. The viewer keeps seeing the screen but can no longer use the mouse or keyboard until the session ends.')
    : $t('Stop remote control: press Control+Command+K on this computer during a session. The viewer keeps seeing the screen but can no longer use the mouse or keyboard until the session ends.')}</p>
{/if}
