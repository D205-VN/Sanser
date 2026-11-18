<script lang="ts">
  import { t } from '../lib/i18n';
  import { onMount } from 'svelte';
  import Icon from './Icon.svelte';
  import { selectSessionDisplay, sessionDisplays, type SessionDisplays } from '../lib/platform';
  let { sessionId, legacy = false }: { sessionId: string; legacy?: boolean } = $props();
  const empty: SessionDisplays = { available: false, selected: 0, displays: [] };
  let view = $state<SessionDisplays>(empty);
  let pending = $state<number | null>(null);
  let pendingSince = 0;
  let error = $state('');
  let alive = false;
  let identity = '';
  $effect(() => {
    if (identity !== sessionId) { identity = sessionId; view = empty; pending = null; error = ''; }
  });
  async function refresh() {
    if (legacy) return;
    const id = sessionId;
    try {
      const next = await sessionDisplays(id);
      if (!alive || id !== sessionId) return;
      view = next;
      if (pending !== null && next.selected === pending) pending = null;
      else if (pending !== null && Date.now() - pendingSince > 6_000) { pending = null; error = $t('The host did not switch displays. Try again, or keep the current display.'); }
    } catch { if (alive && id === sessionId) view = empty; }
  }
  onMount(() => {
    alive = true; void refresh(); const timer = setInterval(() => void refresh(), 1000);
    return () => { alive = false; clearInterval(timer); };
  });
  async function choose(index: number) {
    if (pending !== null || index === view.selected || !view.displays.some((item) => item.index === index)) return;
    const id = sessionId;
    error = ''; pending = index; pendingSince = Date.now();
    try { await selectSessionDisplay(id, index); }
    catch (failure) { if (alive && id === sessionId) { pending = null; error = failure instanceof Error ? failure.message : $t('Unable to switch displays'); } }
  }
</script>

{#if legacy}
  <div class="notice">{$t('This connection shows only the host\'s main display. Switching displays needs a newer connection between both computers.')}</div>
{:else if view.available && view.displays.length > 1}
  <div class="card display-picker" role="group" aria-label={$t('Host displays')}>
    <div class="card-body stack">
      <div class="display-picker-header"><Icon name="monitor" /><strong>{$t('Host displays')}</strong>{#if pending !== null}<span class="muted" role="status">{$t('Switching…')}</span>{/if}</div>
      <div class="button-row">
        {#each view.displays as item (item.index)}
          <button class="button small" class:primary={item.index === view.selected} aria-pressed={item.index === view.selected}
            disabled={pending !== null} onclick={() => choose(item.index)}
            title={item.primary ? $t('Main display') : undefined}>
            {$t('Display {number}', { number: item.index + 1 })} · {item.width}×{item.height}{item.primary ? ` · ${$t('Main')}` : ''}
          </button>
        {/each}
      </div>
      {#if error}<p class="notice error" role="alert">{$t(error)}</p>{/if}
    </div>
  </div>
{/if}

<style>
  .display-picker-header { display: flex; align-items: center; gap: 0.5rem; }
</style>
