<script lang="ts">
  import BrandMark from '../components/BrandMark.svelte';
  import { normalizeServerUrl } from '../lib/api';
  import { SANSER_VERSION } from '../lib/types';
  import { CONFIGURED_SERVER_URL, SERVER_ENDPOINT_LOCKED, preferences } from '../stores/preferences';
  import { session } from '../stores/session';

  let mode = $state<'login' | 'register'>('login');
  let email = $state('');
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
  const endpointLabel = $derived.by(() => {
    if (SERVER_ENDPOINT_LOCKED) return 'Ready';
    try {
      return new URL(serverUrl).host || 'Server setup required';
    } catch {
      return 'Server setup required';
    }
  });

  $effect(() => {
    serverUrl = $preferences.serverUrl;
  });

  function selectMode(next: 'login' | 'register'): void {
    mode = next;
    formError = null;
  }

  async function persistServerUrl(): Promise<string> {
    const normalized = normalizeServerUrl(SERVER_ENDPOINT_LOCKED ? CONFIGURED_SERVER_URL : serverUrl);
    serverUrl = normalized;
    await preferences.save({ ...$preferences, serverUrl: normalized });
    return normalized;
  }

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!canSubmit) return;
    formError = null;
    try {
      const normalized = await persistServerUrl();
      if (mode === 'login') await session.login(normalized, email, password);
      else await session.register(normalized, email, password, displayName);
    } catch (error) {
      formError = error instanceof Error ? error.message : 'Unable to continue';
    }
  }
</script>

<main class="welcome">
  <section class="welcome-story" aria-label="Sanser introduction">
    <div class="welcome-brand">
      <BrandMark size={44} label="Sanser" />
      <div><span>Sanser</span><small>Remote, refined</small></div>
    </div>

    <div class="story-copy">
      <div class="eyebrow"><i></i>Built for the moment latency matters</div>
      <h1>Your desktop.<br /><span>Within reach.</span></h1>
      <p>High-fidelity remote control with deliberate routing, responsive input and a native media path designed to stay out of your way.</p>

      <div class="story-signals" aria-label="Sanser capabilities">
        <span><strong>Auto</strong> route selection</span>
        <span><strong>H.264 + HEVC</strong> video</span>
        <span><strong>Native direct</strong> authenticated transport</span>
      </div>
    </div>

    <div class="signal-preview" aria-hidden="true">
      <div class="signal-window">
        <div class="signal-window-bar"><i></i><i></i><i></i><span>Live workspace</span></div>
        <div class="signal-canvas">
          <div class="signal-sidebar"></div>
          <div class="signal-content"><i></i><i></i><i></i></div>
          <div class="signal-cursor"></div>
        </div>
      </div>
      <div class="signal-latency"><i></i><span>Route ready</span><strong>Direct</strong></div>
    </div>

    <footer class="welcome-version"><span>Sanser {SANSER_VERSION}</span><span>Protocol v2</span><span>Private by design</span></footer>
  </section>

  <section class="welcome-panel" aria-label="Account access">
    <form class="auth-card" onsubmit={submit}>
      <div class="endpoint-status" class:missing={!serverUrl.trim()}>
        <span class="endpoint-dot"></span>
        <div><small>{SERVER_ENDPOINT_LOCKED ? 'Sanser service' : 'Development API endpoint'}</small><strong>{endpointLabel}</strong></div>
        <span class="endpoint-store">Secure API</span>
      </div>

      <header>
        <div class="auth-tabs" role="tablist" aria-label="Account action">
          <button type="button" role="tab" aria-selected={mode === 'login'} class:active={mode === 'login'} onclick={() => selectMode('login')}>Sign in</button>
          <button type="button" role="tab" aria-selected={mode === 'register'} class:active={mode === 'register'} onclick={() => selectMode('register')}>Create account</button>
        </div>
        <h2>{mode === 'login' ? 'Welcome back' : 'A fresh start'}</h2>
        <p>{mode === 'login' ? 'Your computers are one secure sign-in away.' : 'Create an account on your private Sanser deployment.'}</p>
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
        <input id="password" class="input" type="password" bind:value={password} required minlength="12" maxlength="256" autocomplete={mode === 'login' ? 'current-password' : 'new-password'} placeholder="At least 12 characters" />
        {#if mode === 'register'}<small>Use a unique passphrase with at least 12 characters.</small>{/if}
      </div>

      {#if !SERVER_ENDPOINT_LOCKED}
        <details class="advanced-auth" open={!serverUrl.trim()}>
          <summary><span>Developer endpoint</span><small>{serverUrl.trim() ? 'Configured' : 'Required'}</small></summary>
          <div class="field">
            <label for="server-url">Sanser server URL</label>
            <input id="server-url" class="input" type="url" bind:value={serverUrl} required spellcheck="false" autocomplete="url" placeholder="https://api.example.com" />
            <small>This field is available only in an unconfigured developer build.</small>
          </div>
        </details>
      {/if}

      {#if formError ?? $session.error}<div class="notice error" role="alert">{formError ?? $session.error}</div>{/if}

      <button class="button primary auth-submit" type="submit" disabled={$session.busy || !canSubmit}>
        <span>{$session.busy ? 'Connecting securely…' : mode === 'login' ? 'Continue to Sanser' : 'Create secure account'}</span>
        {#if !$session.busy}<span aria-hidden="true">→</span>{/if}
      </button>
      <div class="auth-footer">
        <span>Tokens stay in OS secure storage</span>
        <button class="text-button" type="button" disabled title="Password recovery API is not available in this build">Recovery · Planned</button>
      </div>
    </form>
  </section>
</main>
