<script lang="ts">
  import { normalizeServerUrl } from '../lib/api';
  import { SANSER_VERSION } from '../lib/types';
  import { preferences } from '../stores/preferences';
  import { session } from '../stores/session';

  let mode = $state<'login' | 'register'>('login');
  let email = $state('');
  let password = $state('');
  let displayName = $state('');
  let serverUrl = $state('http://127.0.0.1:5174');
  let localError = $state<string | null>(null);

  $effect(() => {
    serverUrl = $preferences.serverUrl;
  });

  async function persistServerUrl(): Promise<string> {
    const normalized = normalizeServerUrl(serverUrl);
    serverUrl = normalized;
    await preferences.save({ ...$preferences, serverUrl: normalized });
    return normalized;
  }

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    localError = null;
    try {
      const normalized = await persistServerUrl();
      if (mode === 'login') await session.login(normalized, email, password);
      else await session.register(normalized, email, password, displayName);
    } catch (error) {
      localError = error instanceof Error ? error.message : 'Unable to continue';
    }
  }

  async function useLocalMode(): Promise<void> {
    localError = null;
    try {
      const normalized = await persistServerUrl();
      session.useLocal(normalized);
    } catch (error) {
      localError = error instanceof Error ? error.message : 'Unable to start local mode';
    }
  }
</script>

<main class="welcome">
  <section class="welcome-story" aria-label="Sanser introduction">
    <div class="welcome-brand"><img src="/sanser-mark.svg" alt="" /><span>Sanser</span></div>
    <div class="story-copy">
      <div class="eyebrow">Remote desktop, rebuilt</div>
      <h1>Fast when it matters.<br />Quiet when it doesn’t.</h1>
      <p>Sanser connects your computers over LAN or Internet with route-aware Auto, Direct and Relay modes—without an external VPN dependency.</p>
      <div class="story-signals">
        <span>H.264 / HEVC / Auto</span><span>Protocol v2</span><span>SNV2</span>
      </div>
    </div>
    <div class="welcome-version">Sanser {SANSER_VERSION}</div>
  </section>

  <section class="welcome-panel">
    <form class="auth-card" onsubmit={submit}>
      <header>
        <div class="auth-tabs" role="tablist" aria-label="Account action">
          <button type="button" class:active={mode === 'login'} onclick={() => (mode = 'login')}>Sign in</button>
          <button type="button" class:active={mode === 'register'} onclick={() => (mode = 'register')}>Create account</button>
        </div>
        <h2>{mode === 'login' ? 'Welcome back' : 'Create your Sanser account'}</h2>
        <p>{mode === 'login' ? 'Continue to your computers.' : 'Sync devices securely through your server.'}</p>
      </header>

      {#if mode === 'register'}
        <div class="field">
          <label for="display-name">Display name</label>
          <input id="display-name" class="input" bind:value={displayName} maxlength="80" autocomplete="name" />
        </div>
      {/if}
      <div class="field">
        <label for="email">Email</label>
        <input id="email" class="input" type="email" bind:value={email} required maxlength="254" autocomplete="email" />
      </div>
      <div class="field">
        <label for="password">Password</label>
        <input id="password" class="input" type="password" bind:value={password} required minlength="12" maxlength="256" autocomplete={mode === 'login' ? 'current-password' : 'new-password'} />
        {#if mode === 'register'}<small>Use at least 12 characters.</small>{/if}
      </div>

      <details class="advanced-auth">
        <summary>Advanced server</summary>
        <div class="field">
          <label for="server-url">Server URL</label>
          <input id="server-url" class="input" type="url" bind:value={serverUrl} required spellcheck="false" />
          <small>HTTPS is required outside localhost.</small>
        </div>
      </details>

      {#if localError ?? $session.error}<div class="notice error" role="alert">{localError ?? $session.error}</div>{/if}

      <button class="button primary auth-submit" type="submit" disabled={$session.busy}>
        {$session.busy ? 'Connecting…' : mode === 'login' ? 'Sign in' : 'Create account'}
      </button>
      <button class="button auth-submit" type="button" disabled={$session.busy} onclick={useLocalMode}>Continue in local mode</button>
      <p class="auth-note">Local mode keeps preferences on this computer. Discovery becomes available when the local server sidecar is installed.</p>
      <button class="text-button" type="button" disabled title="Password recovery API is not available in this build">Forgot password · Planned</button>
    </form>
  </section>
</main>
