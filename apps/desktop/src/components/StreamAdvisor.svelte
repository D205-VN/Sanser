<script lang="ts">
  import { onMount } from 'svelte';
  import { locale, t } from '../lib/i18n';
  import { recentNetworkCheck } from '../lib/networkCheck';
  import { adviceToPreferences, hostProfile, measureRefreshRate, viewerProfile, type StreamAdvice } from '../lib/streamAdvisor';
  import type { Device, RuntimeStatus } from '../lib/types';
  import { isTauriRuntime } from '../lib/platform';
  import { preferences } from '../stores/preferences';
  import { session } from '../stores/session';

  // Asks the Sanser server (and the language model behind it, when one is
  // set up) for settings that suit this computer and the chosen host.
  let { runtime }: { runtime: RuntimeStatus } = $props();
  let hosts = $state<Device[]>([]);
  let hostId = $state('');
  let modelAvailable = $state<boolean | null>(null);
  let advice = $state<StreamAdvice | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let applied = $state(false);

  onMount(() => {
    const client = session.client();
    if (!client) return;
    // An older server has no advisor; the card then offers the rules only.
    void Promise.resolve().then(() => client.streamAdvisorStatus()).then((status) => { modelAvailable = status.modelAvailable; }).catch(() => { modelAvailable = null; });
    void Promise.resolve().then(() => client.devices()).then((page) => {
      hosts = page.items.filter((device) => device.deviceRole !== 'client');
      hostId = hosts.find((device) => device.online)?.id ?? hosts[0]?.id ?? '';
    }).catch(() => undefined);
  });

  async function ask(): Promise<void> {
    const client = session.client();
    const host = hosts.find((device) => device.id === hostId);
    if (!client || !host) return;
    busy = true; error = null; advice = null; applied = false;
    try {
      const [refreshHz, check, history] = await Promise.all([
        measureRefreshRate(),
        isTauriRuntime() ? recentNetworkCheck(client).catch(() => null) : Promise.resolve(null),
        client.connectionHistory({ deviceIds: [host.id], limit: 5, direction: 'outgoing' }).catch(() => null)
      ]);
      const path = history?.items.find((entry) => entry.route !== null)?.route ?? 'unknown';
      advice = await client.streamAdvice({
        viewer: viewerProfile(runtime, refreshHz),
        host: hostProfile(host),
        network: {
          path,
          rttMs: host.latencyMs ?? undefined,
          udpWorks: check ? check.udp : undefined,
          serverMs: check?.serverMs ?? undefined
        },
        locale: $locale
      });
    } catch (cause) { error = cause instanceof Error ? cause.message : 'Unable to get a suggestion'; }
    finally { busy = false; }
  }

  async function apply(): Promise<void> {
    if (!advice) return;
    try { await preferences.save(adviceToPreferences($preferences, advice)); applied = true; }
    catch (cause) { error = cause instanceof Error ? cause.message : 'Unable to save changes. Please try again.'; }
  }

  const resolutionLabel = (value: string) => (value === 'auto' ? $t('Auto') : value);
  const codecLabel = (value: string) => (value === 'auto' ? $t('Auto') : value === 'hevc' ? 'HEVC' : 'H.264');
  const routeLabel = (value: string) => (value === 'relay' ? $t('Relay') : value === 'direct' ? $t('Direct') : $t('Auto'));
</script>

<article class="card card-body settings-card stack">
  <div>
    <h2 class="card-title">{$t('Suggest settings')}</h2>
    <p class="card-subtitle">
      {modelAvailable ? $t('An AI model on the Sanser server looks at both computers and the network, then suggests settings.') : $t('Sanser looks at both computers and the network, then suggests settings.')}
    </p>
  </div>
  {#if hosts.length === 0}
    <p class="card-subtitle">{$t('Add a host computer to your account first.')}</p>
  {:else}
    <div class="inline-field">
      <select class="select" aria-label={$t('Computer you connect to')} bind:value={hostId}>
        {#each hosts as device (device.id)}<option value={device.id}>{device.name}{device.online ? '' : ` · ${$t('Offline')}`}</option>{/each}
      </select>
      <button class="button primary" disabled={busy || !hostId} onclick={ask}>{busy ? $t('Thinking…') : $t('Suggest')}</button>
    </div>
  {/if}
  {#if error}<div class="notice error" role="alert">{$t(error)}</div>{/if}
  {#if advice}
    <div class="advice">
      <div><span>{$t('Resolution')}</span><strong>{resolutionLabel(advice.settings.resolution)}</strong></div>
      <div><span>{$t('Frame rate')}</span><strong>{advice.settings.fps} FPS</strong></div>
      <div><span>{$t('Bitrate')}</span><strong>{advice.settings.bitrateMbps} Mb/s</strong></div>
      <div><span>{$t('Codec')}</span><strong>{codecLabel(advice.settings.codec)}</strong></div>
      <div><span>{$t('Network')}</span><strong>{routeLabel(advice.settings.networkMode)}</strong></div>
    </div>
    {#if advice.reasons.length}<ul class="reasons">{#each advice.reasons as reason, index (index)}<li>{reason}</li>{/each}</ul>{/if}
    <div class="button-row">
      <button class="button primary" disabled={applied} onclick={apply}>{applied ? $t('Applied') : $t('Use these settings')}</button>
      <small class="source">{advice.source === 'model' ? $t('Suggested by {model}', { model: advice.model ?? 'AI' }) : $t('Suggested by built-in rules (no AI model is set up on the server)')}</small>
    </div>
  {/if}
</article>

<style>
  .advice { display: grid; grid-template-columns: repeat(auto-fit, minmax(110px, 1fr)); gap: 10px; }
  .advice div { display: flex; flex-direction: column; gap: 2px; }
  .advice span { color: var(--muted); font-size: 12px; }
  .reasons { margin: 0; padding-left: 18px; display: grid; gap: 4px; font-size: 13px; }
  .source { color: var(--muted); align-self: center; }
  .inline-field .button { white-space: nowrap; }
</style>
