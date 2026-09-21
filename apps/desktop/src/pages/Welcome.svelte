<script lang="ts">
  import BrandMark from '../components/BrandMark.svelte';
  import { normalizeServerUrl } from '../lib/api';
  import { CONFIGURED_SERVER_URL, SERVER_ENDPOINT_LOCKED, preferences } from '../stores/preferences';
  import { session } from '../stores/session';

  let mode = $state<'login' | 'register'>('login');
  let email = $state('');
  let showPassword = $state(false);
  let rememberSignIn = $state(false);
  let confirmPassword = $state('');
  let password = $state('');
  let displayName = $state('');
  let serverUrl = $state('');
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
    const normalized = normalizeServerUrl(SERVER_ENDPOINT_LOCKED ? CONFIGURED_SERVER_URL : serverUrl);
    serverUrl = normalized;
    if (normalized !== $preferences.serverUrl) await session.logout();
    await preferences.save({ ...$preferences, serverUrl: normalized });
    return normalized;
  }

  async function retrySavedSignIn(): Promise<void> {
    formError = null;
    try { await session.initialize(await persistServerUrl()); }
    catch (error) { formError = error instanceof Error ? error.message : 'Unable to restore sign-in'; }
  }

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!canSubmit || $session.busy) return;
    if (mode === 'register' && password !== confirmPassword) { formError = 'Passwords do not match.'; return; }
    formError = null;
    try {
      const normalized = await persistServerUrl();
      if (mode === 'login') await session.login(normalized, email, password, rememberSignIn);
      else await session.register(normalized, email, password, displayName, rememberSignIn);
    } catch (error) {
      formError = error instanceof Error ? error.message : 'Unable to continue';
    }
  }
</script>

<main class="welcome welcome-simple">
  <section class="welcome-panel" aria-label="Account access">
    <form class="auth-card" onsubmit={submit}>
      <div class="auth-brand"><BrandMark size={40} label="Sanser" /><strong>Sanser</strong></div>

      <header>
        <div class="auth-tabs" role="tablist" aria-label="Account action">
          <button type="button" role="tab" aria-selected={mode === 'login'} class:active={mode === 'login'} onclick={() => selectMode('login')}>Sign in</button>
          <button type="button" role="tab" aria-selected={mode === 'register'} class:active={mode === 'register'} onclick={() => selectMode('register')}>Create account</button>
        </div>
        <h2>{mode === 'login' ? 'Sign in' : 'Create account'}</h2>
        <p>{mode === 'login' ? 'Use the same account on your computers.' : 'One account to connect your computers.'}</p>
      </header>

      {#if mode === 'register'}
        <div class="field">
          <label for="display-name">Display name <span>Optional</span></label>
          <input id="display-name" class="input" bind:value={displayName} maxlength="80" autocomplete="name" placeholder="How should we address you?" />
        </div>
      {/if}
      <div class="field">
        <label for="email">Email</label>
        <input id="email" class="input" type="email" bind:value={email} required maxlength="254" autocomplete="email" inputmode="email" placeholder="you@example.com" />
      </div>
      <div class="field">
        <label for="password">Password</label>
        <div class="password-input"><input id="password" class="input" type={showPassword ? 'text' : 'password'} bind:value={password} required minlength="12" maxlength="256" autocomplete={mode === 'login' ? 'current-password' : 'new-password'} placeholder={mode === 'login' ? 'Enter your password' : 'At least 12 characters'} /><button type="button" class="password-visibility" aria-label={showPassword ? 'Hide password' : 'Show password'} aria-pressed={showPassword} onclick={() => (showPassword = !showPassword)}>{showPassword ? 'Hide' : 'Show'}</button></div>
        {#if mode === 'register'}<small>Use a unique passphrase with at least 12 characters.</small>{/if}
      </div>

      {#if mode === 'register'}
        <div class="field"><label for="confirm-signup-password">Confirm password</label><input id="confirm-signup-password" class="input" type={showPassword ? 'text' : 'password'} bind:value={confirmPassword} required minlength="12" maxlength="256" autocomplete="new-password" placeholder="Enter your password again" /></div>
      {/if}
      {#if !SERVER_ENDPOINT_LOCKED}
        <details class="advanced-auth" open={!serverUrl.trim()}>
          <summary><span>Server settings</span><small>{serverUrl.trim() ? 'Configured' : 'Required'}</small></summary>
          <div class="field">
            <label for="server-url">Sanser server URL</label>
            <input id="server-url" class="input" type="url" bind:value={serverUrl} required spellcheck="false" autocomplete="url" placeholder="https://api.example.com" />
            <small>Enter the address provided by your administrator.</small>
          </div>
        </details>
      {/if}

      {#if formError ?? $session.error}<div class="notice error" role="alert">{formError ?? $session.error}</div>{/if}

      <label class="remember-sign-in"><input type="checkbox" bind:checked={rememberSignIn} disabled={$session.busy} /><span>Keep me signed in</span></label>
      {#if rememberSignIn}<small>Sign in again after 7 days without use.</small>{/if}
      <button class="button primary auth-submit" type="submit" disabled={$session.busy || !canSubmit}>
        <span>{$session.busy ? 'Please wait…' : mode === 'login' ? 'Sign in' : 'Create account'}</span>
        {#if !$session.busy}<span aria-hidden="true">→</span>{/if}
      </button>
      {#if $session.error}<div class="auth-footer"><button class="text-button" type="button" disabled={$session.busy} onclick={retrySavedSignIn}>Retry saved sign-in</button></div>{/if}
    </form>
  </section>
</main>
