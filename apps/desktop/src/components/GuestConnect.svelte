<script lang="ts">
  import { t } from '../lib/i18n';
  import { guestDeviceInfo, normalizeAccessCode } from '../lib/guest';
  import { getLocalRouteAddress } from '../lib/platform';
  import type { RuntimeStatus } from '../lib/types';
  import { connection } from '../stores/connection';
  import { presence } from '../stores/presence';
  import { session } from '../stores/session';

  let { runtime, serverUrl, onCancel }: { runtime: RuntimeStatus; serverUrl: () => Promise<string>; onCancel: () => void } = $props();
  let accessCode = $state('');
  let password = $state('');
  let guestName = $state('');
  let error = $state<string | null>(null);
  const canSubmit = $derived(normalizeAccessCode(accessCode) !== null && password.trim().length >= 6 && guestName.trim().length > 0);

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!canSubmit || $session.busy) return;
    error = null;
    try {
      const url = await serverUrl();
      let routeAddress: string | null = null;
      try { routeAddress = await getLocalRouteAddress(url); } catch { routeAddress = null; }
      const device = guestDeviceInfo(runtime, routeAddress);
      const result = await session.startGuest(url, (client) => client.redeemGuestCode(accessCode, password, guestName.trim(), device));
      password = '';
      presence.startGuest(result.deviceId, routeAddress);
      connection.begin(result.session);
    } catch (caught) {
      error = caught instanceof Error ? caught.message : $t('Unable to connect with this code');
    }
  }
</script>

<form class="auth-card" onsubmit={submit}>
  <header>
    <h2>{$t('Connect with a code')}</h2>
    <p>{$t('Enter the access code and password the other person gave you. They still have to accept before you can see their screen.')}</p>
  </header>
  <div class="field">
    <label for="guest-code">{$t('Access code')}</label>
    <input id="guest-code" class="input" bind:value={accessCode} required inputmode="numeric" autocomplete="off" spellcheck="false" maxlength="13" placeholder="123 456 789" />
  </div>
  <div class="field">
    <label for="guest-password">{$t('Password')}</label>
    <input id="guest-password" class="input" bind:value={password} required autocomplete="off" spellcheck="false" maxlength="16" />
  </div>
  <div class="field">
    <label for="guest-name">{$t('Your name')}</label>
    <input id="guest-name" class="input" bind:value={guestName} required maxlength="60" autocomplete="name" placeholder={$t('Shown to the other person')} />
  </div>
  {#if error}<div class="notice error" role="alert">{$t(error)}</div>{/if}
  <button class="button primary auth-submit" type="submit" disabled={$session.busy || !canSubmit}>{$session.busy ? $t('Please wait…') : $t('Connect')}</button>
  <div class="auth-footer"><button class="text-button" type="button" disabled={$session.busy} onclick={onCancel}>{$t('Back to sign in')}</button></div>
</form>
