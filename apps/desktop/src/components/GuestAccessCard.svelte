<script lang="ts">
  import { onDestroy } from 'svelte';
  import { t } from '../lib/i18n';
  import Icon from './Icon.svelte';
  import type { CreatedGuestGrant, GuestGrant } from '../lib/types';
  import { host as hostStore } from '../stores/host';
  import { session } from '../stores/session';

  let created = $state<CreatedGuestGrant | null>(null);
  let active = $state<GuestGrant[]>([]);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let now = $state(Math.floor(Date.now() / 1_000));
  const timer = window.setInterval(() => { now = Math.floor(Date.now() / 1_000); }, 1_000);
  onDestroy(() => window.clearInterval(timer));

  const waiting = $derived(active.find((grant) => grant.state === 'waiting' && grant.expiresAt > now) ?? null);
  const shown = $derived(created && created.expiresAt > now && waiting?.id === created.id ? created : null);
  const remaining = $derived(shown ? Math.max(0, shown.expiresAt - now) : 0);

  async function load(): Promise<void> {
    const client = session.client();
    const deviceId = $hostStore.deviceId;
    if (!client || !deviceId || !$hostStore.online) { active = []; return; }
    try { active = await client.guestGrants(deviceId); }
    catch (caught) { error = caught instanceof Error ? caught.message : $t('Unable to load support codes'); }
  }

  $effect(() => {
    if ($hostStore.online && $hostStore.deviceId) void load();
    else { active = []; created = null; }
  });

  async function createCode(): Promise<void> {
    const client = session.client();
    const deviceId = $hostStore.deviceId;
    if (!client || !deviceId) return;
    busy = true;
    error = null;
    try {
      created = await client.createGuestGrant(deviceId);
      await load();
    } catch (caught) {
      error = caught instanceof Error ? caught.message : $t('Unable to create a support code');
    } finally { busy = false; }
  }

  async function revoke(id: string): Promise<void> {
    const client = session.client();
    if (!client) return;
    busy = true;
    error = null;
    try {
      await client.revokeGuestGrant(id);
      if (created?.id === id) created = null;
      await load();
    } catch (caught) {
      error = caught instanceof Error ? caught.message : $t('Unable to revoke the support code');
    } finally { busy = false; }
  }
</script>

<article class="card card-body stack">
  <div class="panel-heading"><span class="panel-icon"><Icon name="link" /></span><div><h2 class="card-title">{$t('Support session')}</h2><p class="card-subtitle">{$t('Let someone without a Sanser account see this screen. You still accept every request.')}</p></div></div>
  {#if error}<div class="notice error" role="alert">{$t(error)}</div>{/if}
  {#if !$hostStore.online}
    <div class="notice">{$t('Go online to create a support code.')}</div>
  {:else if shown}
    <div class="guest-code" aria-live="polite">
      <div><span>{$t('Access code')}</span><strong data-testid="guest-access-code">{shown.accessCode}</strong></div>
      <div><span>{$t('Password')}</span><strong data-testid="guest-password">{shown.password}</strong></div>
    </div>
    <p class="card-subtitle">{$t('Works once and expires in {minutes}:{seconds}. Share it only with the person you are helping.', { minutes: Math.floor(remaining / 60), seconds: String(remaining % 60).padStart(2, '0') })}</p>
    <span class="button-row"><button class="button danger" disabled={busy} onclick={() => revoke(shown.id)}>{$t('Revoke code')}</button></span>
  {:else if waiting}
    <div class="notice">{$t('A support code is active until {time}. It is shown only once; revoke it to create a new one.', { time: new Date(waiting.expiresAt * 1_000).toLocaleTimeString() })}</div>
    <span class="button-row"><button class="button danger" disabled={busy} onclick={() => revoke(waiting.id)}>{$t('Revoke code')}</button></span>
  {:else}
    {#each active.filter((grant) => grant.state === 'redeemed') as grant (grant.id)}
      <div class="notice">{$t('{name} is using a support code.', { name: grant.guestName ?? $t('A guest') })} <button class="button small danger" disabled={busy} onclick={() => revoke(grant.id)}>{$t('End guest access')}</button></div>
    {/each}
    <span class="button-row"><button class="button primary" disabled={busy} onclick={createCode}>{busy ? $t('Working…') : $t('Create support code')}</button></span>
  {/if}
</article>

<style>
  .guest-code { display: flex; gap: 24px; flex-wrap: wrap; }
  .guest-code div { display: flex; flex-direction: column; gap: 4px; }
  .guest-code strong { font-size: 24px; letter-spacing: 0.08em; font-variant-numeric: tabular-nums; user-select: all; }
</style>
