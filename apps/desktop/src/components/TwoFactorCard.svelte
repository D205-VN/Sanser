<script lang="ts">
  import { t } from '../lib/i18n';
  import { encodeQr, qrPath } from '../lib/qr';
  import type { SecondFactor, TwoFactorEnrollment, TwoFactorStatus } from '../lib/types';
  import { session } from '../stores/session';

  type Step = 'idle' | 'password' | 'scan' | 'codes' | 'disable' | 'regenerate';
  let status = $state<TwoFactorStatus | null>(null);
  let step = $state<Step>('idle');
  let enrollment = $state<TwoFactorEnrollment | null>(null);
  let recoveryCodes = $state<string[]>([]);
  let password = $state('');
  let code = $state('');
  let useRecoveryCode = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let loadError = $state<string | null>(null);
  const qr = $derived(enrollment ? encodeQr(enrollment.otpauthUri) : null);

  async function load(): Promise<void> {
    const client = session.client();
    if (!client) return;
    try { status = await client.twoFactorStatus(); loadError = null; }
    catch (caught) { loadError = caught instanceof Error ? caught.message : $t('Unable to load two-step verification'); }
  }
  $effect(() => { void load(); });

  function reset(next: Step = 'idle'): void {
    step = next;
    password = '';
    code = '';
    useRecoveryCode = false;
    error = null;
    if (next === 'idle') enrollment = null;
  }

  async function run(action: () => Promise<void>): Promise<void> {
    busy = true;
    error = null;
    try { await action(); }
    catch (caught) { error = caught instanceof Error ? caught.message : $t('Something went wrong. Please try again.'); }
    finally { busy = false; }
  }

  const factor = (): SecondFactor => (useRecoveryCode ? { recoveryCode: code.trim() } : { code: code.replace(/\s/g, '') });

  function submitPassword(event: SubmitEvent): void {
    event.preventDefault();
    void run(async () => {
      const client = session.client();
      if (!client) return;
      enrollment = await client.enrollTwoFactor(password);
      password = '';
      step = 'scan';
    });
  }

  function submitCode(event: SubmitEvent): void {
    event.preventDefault();
    void run(async () => {
      const client = session.client();
      if (!client) return;
      recoveryCodes = await client.confirmTwoFactor(code.replace(/\s/g, ''));
      enrollment = null;
      code = '';
      step = 'codes';
      await load();
    });
  }

  function submitReauth(event: SubmitEvent): void {
    event.preventDefault();
    void run(async () => {
      const client = session.client();
      if (!client) return;
      if (step === 'disable') {
        await client.disableTwoFactor(password, factor());
        reset();
      } else {
        recoveryCodes = await client.regenerateRecoveryCodes(password, factor());
        reset('codes');
      }
      await load();
    });
  }

  function finishCodes(): void {
    recoveryCodes = [];
    reset();
  }
</script>

<article class="card card-body settings-card stack">
  <div><h2 class="card-title">{$t('Two-step verification')}</h2><p class="card-subtitle">{$t('Ask for a code from an authenticator app when you sign in.')}</p></div>
  {#if loadError}<div class="notice error" role="alert">{$t(loadError)}</div>{/if}
  {#if error}<div class="notice error" role="alert">{$t(error)}</div>{/if}

  {#if step === 'codes'}
    <div class="notice warning" role="status">{$t('Save these recovery codes somewhere safe. Each one signs you in once if you lose your authenticator. They will not be shown again.')}</div>
    <ol class="recovery-codes" data-testid="recovery-codes">{#each recoveryCodes as recovery (recovery)}<li><code>{recovery}</code></li>{/each}</ol>
    <span class="button-row"><button class="button" onclick={() => { void navigator.clipboard.writeText(recoveryCodes.join('\n')); }}>{$t('Copy codes')}</button><button class="button primary" onclick={finishCodes}>{$t('I saved these codes')}</button></span>
  {:else if step === 'password'}
    <form class="stack" onsubmit={submitPassword}>
      <p>{$t('Other computers signed in to this account will be signed out and must sign in again with a code.')}</p>
      <div class="field"><label for="tfa-password">{$t('Current password')}</label><input id="tfa-password" class="input" type="password" bind:value={password} required maxlength="256" autocomplete="current-password" /></div>
      <span class="button-row"><button class="button primary" type="submit" disabled={busy || !password}>{busy ? $t('Please wait…') : $t('Continue')}</button><button class="button" type="button" disabled={busy} onclick={() => reset()}>{$t('Cancel')}</button></span>
    </form>
  {:else if step === 'scan' && enrollment && qr}
    <form class="stack" onsubmit={submitCode}>
      <p>{$t('Scan this QR code with your authenticator app, then enter the 6-digit code it shows.')}</p>
      <svg class="tfa-qr" viewBox={`0 0 ${qr.size + 8} ${qr.size + 8}`} role="img" aria-label={$t('QR code for your authenticator app')} shape-rendering="crispEdges"><rect width="100%" height="100%" fill="#fff" /><path d={qrPath(qr)} fill="#000" /></svg>
      <div class="field"><label for="tfa-secret">{$t('Or enter this key manually')}</label><code id="tfa-secret" class="tfa-secret">{enrollment.secret.replace(/(.{4})/g, '$1 ').trim()}</code></div>
      <div class="field"><label for="tfa-code">{$t('Verification code')}</label><input id="tfa-code" class="input" bind:value={code} required inputmode="numeric" autocomplete="one-time-code" maxlength="7" placeholder="123456" /></div>
      <span class="button-row"><button class="button primary" type="submit" disabled={busy || !/^\d{6}$/.test(code.replace(/\s/g, ''))}>{busy ? $t('Please wait…') : $t('Turn on')}</button><button class="button" type="button" disabled={busy} onclick={() => reset()}>{$t('Cancel')}</button></span>
    </form>
  {:else if step === 'disable' || step === 'regenerate'}
    <form class="stack" onsubmit={submitReauth}>
      <p>{step === 'disable' ? $t('Turning this off means your password alone can sign in to your account.') : $t('Your current recovery codes will stop working.')}</p>
      <div class="field"><label for="tfa-reauth-password">{$t('Current password')}</label><input id="tfa-reauth-password" class="input" type="password" bind:value={password} required maxlength="256" autocomplete="current-password" /></div>
      <div class="field"><label for="tfa-reauth-code">{useRecoveryCode ? $t('Recovery code') : $t('Verification code')}</label><input id="tfa-reauth-code" class="input" bind:value={code} required autocomplete="one-time-code" maxlength="11" /></div>
      <button class="text-button" type="button" onclick={() => { useRecoveryCode = !useRecoveryCode; code = ''; }}>{useRecoveryCode ? $t('Use an authenticator code') : $t('Use a recovery code')}</button>
      <span class="button-row"><button class:danger={step === 'disable'} class:primary={step !== 'disable'} class="button" type="submit" disabled={busy || !password || !code.trim()}>{busy ? $t('Please wait…') : step === 'disable' ? $t('Turn off') : $t('Create new codes')}</button><button class="button" type="button" disabled={busy} onclick={() => reset()}>{$t('Cancel')}</button></span>
    </form>
  {:else if status?.enabled}
    <div class="notice" role="status">{$t('On. {count} recovery codes left.', { count: status.recoveryCodesRemaining })}</div>
    <span class="button-row"><button class="button" onclick={() => reset('regenerate')}>{$t('New recovery codes')}</button><button class="button danger" onclick={() => reset('disable')}>{$t('Turn off')}</button></span>
  {:else if status}
    <div class="notice">{$t('Off. Your password alone can sign in to your account.')}</div>
    <span class="button-row"><button class="button primary" onclick={() => reset('password')}>{$t('Turn on two-step verification')}</button></span>
  {/if}
</article>

<style>
  .tfa-qr { width: 200px; height: 200px; border-radius: 8px; }
  .tfa-secret { display: block; font-size: 15px; letter-spacing: 0.06em; user-select: all; word-break: break-all; }
  .recovery-codes { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 6px 24px; margin: 0; padding-left: 24px; }
  .recovery-codes code { font-size: 15px; letter-spacing: 0.06em; }
</style>
