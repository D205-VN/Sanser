<script lang="ts">
  import { t } from '../lib/i18n';
  import { listRunningApps } from '../lib/platform';
  import { preferences, validBlockedAppId } from '../stores/preferences';
  import type { BlockedApp, RuntimeStatus } from '../lib/types';

  // Host-only: the list is passed to the host engine at launch. Nothing a
  // viewer sends can change it. See docs/blocked-apps.md.
  let { runtime, onerror }: { runtime: RuntimeStatus; onerror?: (message: string) => void } = $props();
  const windows = $derived(runtime.platform.toLowerCase().includes('win'));
  const disabled = $derived(runtime.capabilities.hostEngine.state !== 'available');
  const blocked = $derived($preferences.host.blockedApps);
  let running = $state<BlockedApp[]>([]);
  let loading = $state(false);
  let selected = $state('');
  let manual = $state('');
  let message = $state<string | null>(null);
  const choices = $derived(running.filter((app) => !blocked.some((item) => item.id.toLowerCase() === app.id.toLowerCase())));

  async function save(next: BlockedApp[]): Promise<void> {
    try { await preferences.save({ ...$preferences, host: { ...$preferences.host, blockedApps: next } }); }
    catch (error) { onerror?.(error instanceof Error ? error.message : $t('Unable to save host settings')); }
  }

  async function refresh(): Promise<void> {
    loading = true; message = null;
    try { running = await listRunningApps(); if (running.length === 0) message = 'No running apps were found. Enter the app below instead.'; }
    catch { message = 'Unable to list running apps. Enter the app below instead.'; }
    finally { loading = false; }
  }

  async function add(app: BlockedApp): Promise<void> {
    message = null;
    if (!validBlockedAppId(app.id)) { message = windows ? 'Enter a program file name such as chrome.exe.' : 'Enter a bundle identifier such as com.google.Chrome.'; return; }
    if (blocked.some((item) => item.id.toLowerCase() === app.id.trim().toLowerCase())) return;
    if (blocked.length >= 64) { message = 'At most 64 apps can be blocked.'; return; }
    await save([...blocked, { id: app.id.trim(), name: app.name.trim() || app.id.trim() }]);
  }

  async function addSelected(): Promise<void> {
    const app = choices.find((item) => item.id === selected);
    if (app) { await add(app); selected = ''; }
  }

  async function addManual(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const id = manual.trim();
    if (!id) return;
    await add({ id, name: id });
    if (!message) manual = '';
  }
</script>

<section class="blocked-apps stack" aria-labelledby="blocked-apps-title">
  <div>
    <h3 id="blocked-apps-title">{$t('Blocked apps')}</h3>
    <p class="muted">{$t('While someone is connected, these apps are hidden from the shared screen and the remote person cannot click or type in them. You can still use them on this computer. Applies from the next connection.')}</p>
  </div>
  {#if blocked.length > 0}
    <ul class="blocked-list">
      {#each blocked as app (app.id)}
        <li><div><strong>{app.name}</strong>{#if app.name !== app.id}<span class="muted"> · {app.id}</span>{/if}</div>
          <button class="button small" {disabled} onclick={() => save(blocked.filter((item) => item.id !== app.id))}>{$t('Remove')}</button></li>
      {/each}
    </ul>
  {:else}
    <p class="muted">{$t('No apps are blocked.')}</p>
  {/if}
  <div class="button-row">
    <button class="button small" disabled={disabled || loading} onclick={refresh}>{loading ? $t('Loading…') : $t('Choose from running apps')}</button>
    {#if choices.length > 0}
      <select class="select" aria-label={$t('Running apps')} bind:value={selected} {disabled}>
        <option value="">{$t('Choose an app…')}</option>
        {#each choices as app (app.id)}<option value={app.id}>{app.name}</option>{/each}
      </select>
      <button class="button small primary" disabled={disabled || !selected} onclick={addSelected}>{$t('Block')}</button>
    {/if}
  </div>
  <form class="button-row" onsubmit={addManual}>
    <input class="input" bind:value={manual} {disabled} maxlength="255" spellcheck="false" aria-label={$t('App to block')}
      placeholder={windows ? 'chrome.exe' : 'com.google.Chrome'} />
    <button class="button small" type="submit" disabled={disabled || !manual.trim()}>{$t('Add')}</button>
  </form>
  {#if message}<p class="muted" role="status">{$t(message)}</p>{/if}
  <p class="muted">{$t('This stops casual use, not a determined person with full control: a renamed or copied program is not recognised. Use view-only access when that matters.')}</p>
</section>

<style>
  .blocked-list { list-style: none; margin: 0; padding: 0; }
  .blocked-list li { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 8px 0; border-bottom: 1px solid var(--border); }
  .blocked-apps h3 { margin: 0 0 4px; font-size: 15px; }
</style>
