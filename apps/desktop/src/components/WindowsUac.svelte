<script lang="ts">
  import { onMount } from 'svelte';
  import { configureWindowsUac, windowsUacStatus, type WindowsUacStatus } from '../lib/platform';
  let { sessionRunning = false }: { sessionRunning?: boolean } = $props();
  let status = $state<WindowsUacStatus | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);
  onMount(() => { void windowsUacStatus().then((value) => { status = value; }).catch(() => { error = 'Unable to check UAC support.'; }); });
  async function configure(enabled: boolean): Promise<void> {
    busy = true; error = null;
    try { status = await configureWindowsUac(enabled); }
    catch (cause) { error = cause instanceof Error ? cause.message : String(cause); }
    finally { busy = false; }
  }
</script>

<article class="card card-body stack">
  <div><h2 class="card-title">Windows administrator prompts</h2><p class="card-subtitle">Keep sharing and use the mouse and keyboard when an installer opens a UAC prompt.</p></div>
  {#if status?.ready}
    <p>UAC support is enabled. Your next connection will use it automatically.</p>
  {:else if status?.enabled}
    <p>Update UAC support to match this version of Sanser.</p>
  {:else}
    <p>Enable the Windows helper once with administrator approval on this computer. It runs as a Windows service; your screen is shared only through an accepted Sanser connection.</p>
  {/if}
  {#if sessionRunning}<p>Stop the current connection or group sharing before changing this setting.</p>{/if}
  {#if error}<div class="notice error" role="alert">{error}</div>{/if}
  <div class="button-row">
    {#if !status?.ready}
      <button class="button primary" disabled={busy || sessionRunning || !status?.available} onclick={() => configure(true)}>{busy ? 'Waiting for Windows…' : status?.enabled ? 'Update UAC support' : 'Enable UAC support'}</button>
    {/if}
    {#if status?.enabled}<button class="button danger" disabled={busy || sessionRunning} onclick={() => configure(false)}>Disable UAC support</button>{/if}
  </div>
  {#if status && !status.available}<p>The UAC helper is missing from this installation. Install a Sanser build that includes it.</p>{/if}
</article>
