<script lang="ts">
  import { onMount } from 'svelte';
  import { isTauri } from '@tauri-apps/api/core';
  import { check, type Update } from '@tauri-apps/plugin-updater';
  import { relaunch } from '@tauri-apps/plugin-process';
  import { connection } from '../stores/connection';
  import { host as hostStore } from '../stores/host';
  import { diagnostics } from '../stores/diagnostics';

  let dialog = $state<HTMLDialogElement>();
  let manifest = $state<Update | null>(null);
  let status = $state<'idle' | 'downloading' | 'installing' | 'done' | 'error'>('idle');
  let progress = $state(0);
  let totalSize = $state(0);
  let downloadedSize = $state(0);
  let error = $state('');
  const updating = $derived(status === 'downloading' || status === 'installing');
  const activeSession = $derived(
    $connection.engineRunning || $connection.busy ||
    ($connection.session !== null && ['pending', 'accepted', 'connecting', 'connected'].includes($connection.session.status)) ||
    $hostStore.engineRunning || $hostStore.launchingSessionId !== null ||
    $hostStore.sessions.some((item) => item.status === 'pending' || item.status === 'accepted')
  );

  onMount(() => {
    if (!isTauri()) return;
    let mounted = true;
    const timer = window.setTimeout(() => {
      void check().then((update) => {
        if (mounted) manifest = update;
        else if (update) void update.close();
      }).catch((caught: unknown) => {
        diagnostics.add({ level: 'warn', category: 'app', message: caught instanceof Error ? caught.message : 'Unable to check for updates' });
      });
    }, 5_000);
    return () => {
      mounted = false;
      window.clearTimeout(timer);
      if (manifest) void manifest.close().catch(() => undefined);
    };
  });

  $effect(() => {
    if (manifest && dialog && !dialog.open && !activeSession) dialog.showModal();
  });

  async function install(): Promise<void> {
    if (!manifest || updating || activeSession) return;
    status = 'downloading';
    progress = 0;
    totalSize = 0;
    downloadedSize = 0;
    error = '';
    try {
      await manifest.downloadAndInstall((event) => {
        if (event.event === 'Started') totalSize = event.data.contentLength ?? 0;
        if (event.event === 'Progress') {
          downloadedSize += event.data.chunkLength;
          if (totalSize) progress = Math.min(100, Math.round(downloadedSize / totalSize * 100));
        }
        if (event.event === 'Finished') status = 'installing';
      });
      status = 'done';
    } catch (caught) {
      status = 'error';
      error = caught instanceof Error ? caught.message : 'Unable to install the update. Please try again.';
    }
  }

  async function restart(): Promise<void> {
    try { await relaunch(); }
    catch { error = 'Unable to restart automatically. Quit and reopen Sanser to finish updating.'; }
  }

  function dismiss(): void {
    if (updating) return;
    dialog?.close();
    const update = manifest;
    manifest = null;
    if (update) void update.close().catch(() => undefined);
  }
</script>

<dialog bind:this={dialog} class="update-dialog" aria-labelledby="update-title" oncancel={(event) => { event.preventDefault(); dismiss(); }}>
  {#if manifest}
    <div class="update-heading"><span class="section-eyebrow">SANSER UPDATE</span><h2 id="update-title">{status === 'done' ? 'Your update is ready' : 'A better workspace awaits'}</h2><span class="status available">Version {manifest.version}</span></div>
    <div aria-live="polite">
      {#if status === 'idle' || status === 'error'}
        <p>Get the latest improvements and fixes. Sanser will ask you to restart when installation is complete.</p>
        {#if manifest.body}<div class="update-notes">{manifest.body}</div>{/if}
      {:else if status === 'downloading'}
        <p>{totalSize ? `Downloading update · ${progress}%` : `Downloading update · ${(downloadedSize / 1_048_576).toFixed(1)} MB`}</p>
        <progress max="100" value={totalSize ? progress : undefined} aria-label="Download progress"></progress>
      {:else if status === 'installing'}<p>Installing the update. Please keep Sanser open.</p>
      {:else}<p>Restart Sanser to start using the new version.</p>{/if}
    </div>
    {#if error}<div class="notice error" role="alert">{error}</div>{/if}
    {#if activeSession}<div class="notice warning">Finish your remote session before updating or restarting.</div>{/if}
    <div class="update-actions">
      <button class="button ghost" disabled={updating} onclick={dismiss}>{status === 'done' ? 'Restart later' : 'Not now'}</button>
      {#if status === 'done'}<button class="button primary" disabled={activeSession} onclick={restart}>Restart Sanser</button>
      {:else}<button class="button primary" disabled={updating || activeSession} onclick={install}>{updating ? 'Updating…' : status === 'error' ? 'Try again' : 'Install update'}</button>{/if}
    </div>
  {/if}
</dialog>

<style>
  .update-dialog { width: min(480px, calc(100vw - 32px)); max-height: calc(100vh - 48px); padding: 28px; border: 1px solid var(--border-strong); border-radius: 20px; color: var(--text); background: var(--surface-solid); box-shadow: var(--shadow); }
  .update-dialog::backdrop { background: #05090fc9; backdrop-filter: blur(6px); }
  .update-heading h2 { margin: 0 0 16px; font-size: 25px; letter-spacing: -.03em; }
  p { color: var(--muted); font-size: 14px; line-height: 1.7; }
  .update-notes { max-height: 180px; overflow: auto; margin-block: 16px; padding: 16px; border-radius: 10px; background: var(--bg); color: var(--text-soft); font-size: 13px; white-space: pre-wrap; overflow-wrap: anywhere; }
  progress { width: 100%; height: 8px; accent-color: var(--accent); }
  .update-actions { display: flex; justify-content: flex-end; flex-wrap: wrap; gap: 10px; padding-top: 20px; border-top: 1px solid var(--border); }
</style>
