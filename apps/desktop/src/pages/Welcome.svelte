<script lang="ts">
  import { t } from '../lib/i18n';
  import BrandMark from '../components/BrandMark.svelte';
  import LanguageToggle from '../components/LanguageToggle.svelte';
  import { normalizeServerUrl } from '../lib/api';
  import { CONFIGURED_SERVER_URL, preferences } from '../stores/preferences';
  import { session } from '../stores/session';

  let mode = $state<'login' | 'register'>('login');
  let email = $state('');
  let showPassword = $state(false);
  let rememberSignIn = $state(false);
  let confirmPassword = $state('');
  let password = $state('');
  let displayName = $state('');
  let serverUrl = $state('');
  let showServerSettings = $state(false);
  const needsServerSettings = $derived(!serverUrl.trim() || serverUrl !== CONFIGURED_SERVER_URL);
  let formError = $state<string | null>(null);

  const canSubmit = $derived(
    serverUrl.trim().length > 0 &&
      email.trim().length > 3 &&
      password.length >= 12 &&
      (mode === 'login' || displayName.trim().length <= 80)
  );
  $effect(() => {
    serverUrl = $preferences.serverUrl;
  });

  function selectMode(next: 'login' | 'register'): void {
    if ($session.busy) return;
    mode = next;
    password = '';
    confirmPassword = '';
    showPassword = false;
    formError = null;
  }

  async function persistServerUrl(): Promise<string> {
    const normalized = normalizeServerUrl(serverUrl);
    serverUrl = normalized;
    if (normalized !== $preferences.serverUrl) await session.logout();
    await preferences.save({ ...$preferences, serverUrl: normalized, customServer: normalized !== CONFIGURED_SERVER_URL });
    return normalized;
  }

  async function retrySavedSignIn(): Promise<void> {
    formError = null;
    try { await session.initialize(await persistServerUrl()); }
    catch (error) { formError = error instanceof Error ? error.message : $t('Unable to restore sign-in'); }
  }

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!canSubmit || $session.busy) return;
    if (mode === 'register' && password !== confirmPassword) { formError = $t('Passwords do not match.'); return; }
    formError = null;
    try {
      const normalized = await persistServerUrl();
      if (mode === 'login') await session.login(normalized, email, password, rememberSignIn);
      else await session.register(normalized, email, password, displayName, rememberSignIn);
    } catch (error) {
      formError = error instanceof Error ? error.message : $t('Unable to continue');
    }
  }
</script>

<main class="welcome welcome-simple">
  <div class="welcome-language"><LanguageToggle /></div>
  <section class="welcome-panel" aria-label={$t('Account access')}>
    <form class="auth-card" onsubmit={submit}>
      <div class="auth-brand"><BrandMark size={40} label="Sanser" /><strong>{$t('Sanser')}</strong></div>

      <header>
        <div class="auth-tabs" role="tablist" aria-label={$t('Account action')}>
          <button type="button" role="tab" aria-selected={mode === 'login'} class:active={mode === 'login'} onclick={() => selectMode('login')}>{$t('Sign in')}</button>
          <button type="button" role="tab" aria-selected={mode === 'register'} class:active={mode === 'register'} onclick={() => selectMode('register')}>{$t('Create account')}</button>
        </div>
        <h2>{mode === 'login' ? $t('Sign in') : $t('Create account')}</h2>
        <p>{mode === 'login' ? $t('Use the same account on your computers.') : $t('One account to connect your computers.')}</p>
      </header>

      {#if mode === 'register'}
        <div class="field">
          <label for="display-name">{$t('Display name')} <span>{$t('Optional')}</span></label>
          <input id="display-name" class="input" bind:value={displayName} maxlength="80" autocomplete="name" placeholder={$t('How should we address you?')} />
        </div>
      {/if}
      <div class="field">
        <label for="email">{$t('Email')}</label>
        <input id="email" class="input" type="email" bind:value={email} required maxlength="254" autocomplete="email" inputmode="email" placeholder={$t('you@example.com')} />
      </div>
      <div class="field">
        <label for="password">{$t('Password')}</label>
        <div class="password-input"><input id="password" class="input" type={showPassword ? 'text' : 'password'} bind:value={password} required minlength="12" maxlength="256" autocomplete={mode === 'login' ? 'current-password' : 'new-password'} placeholder={mode === 'login' ? $t('Enter your password') : $t('At least 12 characters')} /><button type="button" class="password-visibility" aria-label={showPassword ? $t('Hide password') : $t('Show password')} aria-pressed={showPassword} onclick={() => (showPassword = !showPassword)}>{showPassword ? $t('Hide') : $t('Show')}</button></div>
        {#if mode === 'register'}<small>{$t('Use a unique passphrase with at least 12 characters.')}</small>{/if}
      </div>

      {#if mode === 'register'}
        <div class="field"><label for="confirm-signup-password">{$t('Confirm password')}</label><input id="confirm-signup-password" class="input" type={showPassword ? 'text' : 'password'} bind:value={confirmPassword} required minlength="12" maxlength="256" autocomplete="new-password" placeholder={$t('Enter your password again')} /></div>
      {/if}
      {#if showServerSettings || needsServerSettings}
        <details class="advanced-auth" open>
          <summary><span>{serverUrl.trim() ? $t('Custom server') : $t('Set up a server')}</span></summary>
          <div class="field">
            <label for="server-url">{$t('Sanser server URL')}</label>
            <input id="server-url" class="input" type="url" bind:value={serverUrl} required spellcheck="false" autocomplete="url" placeholder={$t('https://api.example.com')} />
            <small>{$t('This server handles sign-in and finding your computers. Use the same server on both computers.')}</small>
            {#if CONFIGURED_SERVER_URL}<button type="button" class="text-button" onclick={() => { serverUrl = CONFIGURED_SERVER_URL; showServerSettings = false; }}>{$t('Use default server')}</button>{/if}
          </div>
        </details>
      {/if}

      {#if formError ?? $session.error}<div class="notice error" role="alert">{$t(formError ?? $session.error ?? '')}</div>{/if}

      <label class="remember-sign-in"><input type="checkbox" bind:checked={rememberSignIn} disabled={$session.busy} /><span>{$t('Keep me signed in')}</span></label>
      {#if rememberSignIn}<small>{$t('Sign in again after 7 days without use.')}</small>{/if}
      <button class="button primary auth-submit" type="submit" disabled={$session.busy || !canSubmit}>
        <span>{$session.busy ? $t('Please wait…') : mode === 'login' ? $t('Sign in') : $t('Create account')}</span>
        {#if !$session.busy}<span aria-hidden="true">→</span>{/if}
      </button>
      {#if !showServerSettings && !needsServerSettings}<div class="auth-footer"><button class="text-button" type="button" onclick={() => { showServerSettings = true; }}>{$t('Use a different server')}</button></div>{/if}
      {#if $session.error}<div class="auth-footer"><button class="text-button" type="button" disabled={$session.busy} onclick={retrySavedSignIn}>{$t('Retry saved sign-in')}</button></div>{/if}
    </form>
  </section>
</main>

<style>
  .welcome-language { position: absolute; top: 16px; right: 16px; z-index: 2; }
</style>
