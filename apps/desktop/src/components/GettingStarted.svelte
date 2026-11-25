<script lang="ts">
  import { t } from '../lib/i18n';
  import type { Page } from '../lib/types';

  // Each step ticks itself from live state, so the guide needs no setup of its own.
  let {
    hasOtherDevice,
    hostReachable,
    connected,
    hostAlwaysReady,
    navigate
  }: {
    hasOtherDevice: boolean;
    hostReachable: boolean;
    connected: boolean;
    hostAlwaysReady: boolean;
    navigate: (page: Page) => void;
  } = $props();

  const DISMISSED = 'sanser.getting-started.dismissed.v1';
  const CONNECTED = 'sanser.getting-started.connected.v1';

  function read(key: string): boolean {
    try { return localStorage.getItem(key) === '1'; } catch { return false; }
  }
  function remember(key: string): void {
    try { localStorage.setItem(key, '1'); } catch { /* storage blocked: the guide just shows again */ }
  }

  let dismissed = $state(read(DISMISSED));
  let connectedBefore = $state(read(CONNECTED));
  $effect(() => {
    if (connected && !connectedBefore) { connectedBefore = true; remember(CONNECTED); }
  });

  const steps = $derived([
    { done: hasOtherDevice, title: $t('Install Sanser on a second device'), detail: $t('Windows PC, Mac or phone. Sign in there with this same account.'), action: null },
    { done: hostReachable, title: $t('Turn on Host on the computer you want to reach'), detail: $t('Open Sanser on that computer, choose Host and go online.'), action: { label: $t('Open Host'), page: 'host' as Page } },
    { done: connectedBefore, title: $t('Make your first connection'), detail: $t('Pick an online computer below and choose Connect.'), action: null },
    { done: hostAlwaysReady, title: $t('Optional: keep this computer ready'), detail: $t('In Settings → Host, start Sanser when you sign in and bring Host online automatically.'), action: { label: $t('Open Settings'), page: 'settings' as Page } }
  ]);
  const required = $derived(steps.slice(0, 3));
  const doneCount = $derived(required.filter((step) => step.done).length);
  const visible = $derived(!dismissed && doneCount < required.length);

  function dismiss(): void {
    dismissed = true;
    remember(DISMISSED);
  }
</script>

{#if visible}
  <section class="card getting-started" aria-labelledby="getting-started-title">
    <header>
      <div>
        <h2 id="getting-started-title">{$t('Get started')}</h2>
        <p>{$t('{done} of {total} done', { done: doneCount, total: required.length })}</p>
      </div>
      <button class="button small ghost" onclick={dismiss}>{$t('Hide guide')}</button>
    </header>
    <div class="progress" aria-hidden="true"><span style={`width: ${(doneCount / required.length) * 100}%`}></span></div>
    <ol>
      {#each steps as step, index (index)}
        <li class:done={step.done}>
          <span class="mark" aria-hidden="true">{step.done ? '✓' : index + 1}</span>
          <div>
            <strong>{step.title}</strong>
            <small>{step.detail}</small>
          </div>
          {#if step.action && !step.done}<button class="button small" onclick={() => navigate(step.action.page)}>{step.action.label}</button>{/if}
          <span class="sr-only">{step.done ? $t('Done') : $t('Not done yet')}</span>
        </li>
      {/each}
    </ol>
  </section>
{/if}

<style>
  .getting-started { display: flex; flex-direction: column; gap: 14px; padding: 18px 20px; margin-bottom: 16px; }
  header { display: flex; justify-content: space-between; align-items: flex-start; gap: 12px; }
  h2 { margin: 0; font-size: 16px; }
  header p { margin: 2px 0 0; color: var(--muted); font-size: 13px; }
  .progress { height: 4px; border-radius: 999px; background: var(--border); overflow: hidden; }
  .progress span { display: block; height: 100%; background: var(--accent); transition: width 0.3s; }
  ol { list-style: none; margin: 0; padding: 0; display: grid; gap: 10px; }
  li { display: flex; align-items: center; gap: 12px; }
  li > div { display: flex; flex-direction: column; min-width: 0; flex: 1; }
  li strong { font-size: 13px; font-weight: 600; }
  li small { color: var(--muted); font-size: 12px; }
  li.done strong { color: var(--muted); text-decoration: line-through; }
  .mark { display: grid; place-items: center; width: 24px; height: 24px; flex-shrink: 0; border: 1px solid var(--border-strong); border-radius: 8px; color: var(--accent); font-size: 12px; }
  li.done .mark { background: var(--accent); border-color: var(--accent); color: #fff; }
  .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; }
</style>
