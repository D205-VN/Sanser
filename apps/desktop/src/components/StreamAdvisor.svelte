<script lang="ts">
  import { onMount } from 'svelte';
  import { locale, t } from '../lib/i18n';
  import { measureLive, type LiveMeasurement } from '../lib/advisorProbe';
  import { recentNetworkCheck } from '../lib/networkCheck';
  import { adviceToPreferences, hostProfile, measureRefreshRate, viewerProfile, type AdviceGoal, type StreamAdvice } from '../lib/streamAdvisor';
  import type { Device, RuntimeStatus } from '../lib/types';
  import { isTauriRuntime, machineInfo } from '../lib/platform';
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
  let stage = $state<'measuring' | 'thinking'>('measuring');
  let live = $state<LiveMeasurement | null>(null);
  let error = $state<string | null>(null);
  let applied = $state(false);
  let goal = $state<AdviceGoal>('balanced');
  const goals: [AdviceGoal, string, string][] = [
    ['balanced', 'Balanced', 'Smooth and sharp in equal measure.'],
    ['smooth', 'Smooth and stable', 'High frame rate, fewer stutters.'],
    ['sharp', 'Sharp and stable', 'Highest resolution, steady picture.'],
    ['responsive', 'Fastest response', 'Lowest delay for games and drawing.']
  ];

  function chooseGoal(next: AdviceGoal): void {
    if (goal === next) return;
    goal = next; advice = null; applied = false;
  }

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
    busy = true; stage = 'measuring'; error = null; advice = null; applied = false; live = null;
    try {
      const [refreshHz, info, check, history] = await Promise.all([
        measureRefreshRate(),
        machineInfo().catch(() => ({})),
        isTauriRuntime() ? recentNetworkCheck(client).catch(() => null) : Promise.resolve(null),
        client.connectionHistory({ deviceIds: [host.id], limit: 5, direction: 'outgoing' }).catch(() => null)
      ]);
      // A few seconds of real traffic; an older server or an offline host
      // leaves the estimate from latency instead.
      live = await measureLive(client, host.id).catch(() => null);
      stage = 'thinking';
      const path = history?.items.find((entry) => entry.route !== null)?.route ?? 'unknown';
      const serverMs = live?.serverMs ?? check?.serverMs ?? undefined;
      const hostServerMs = live?.host?.serverMs ?? undefined;
      advice = await client.streamAdvice({
        viewer: viewerProfile(runtime, refreshHz, info),
        host: hostProfile(host, live?.host),
        network: {
          path,
          // The relay runs on the server, so both halves add up to the relay path.
          rttMs: serverMs !== undefined && hostServerMs !== undefined ? serverMs + hostServerMs : host.latencyMs ?? undefined,
          udpWorks: check ? check.udp : undefined,
          serverMs,
          downloadMbps: live?.downloadMbps ?? undefined,
          uploadMbps: live?.host?.uploadMbps ?? undefined
        },
        locale: $locale,
        goal
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
    <div class="profile-grid" role="radiogroup" aria-label={$t('What matters most')}>
      {#each goals as [value, label, hint] (value)}
        <button type="button" class="profile-option goal" class:active={goal === value} role="radio" aria-checked={goal === value} onclick={() => chooseGoal(value)}>
          <strong>{$t(label)}</strong><span>{$t(hint)}</span>
        </button>
      {/each}
    </div>
    <div class="inline-field">
      <select class="select" aria-label={$t('Computer you connect to')} bind:value={hostId}>
        {#each hosts as device (device.id)}<option value={device.id}>{device.name}{device.online ? '' : ` · ${$t('Offline')}`}</option>{/each}
      </select>
      <button class="button primary" disabled={busy || !hostId} onclick={ask}>{busy ? (stage === 'measuring' ? $t('Measuring…') : $t('Thinking…')) : $t('Suggest')}</button>
    </div>
  {/if}
  {#if busy && stage === 'measuring'}<p class="card-subtitle">{$t('Measuring the connection to both computers for a few seconds…')}</p>{/if}
  {#if error}<div class="notice error" role="alert">{$t(error)}</div>{/if}
  {#if advice}
    <div class="advice">
      <div><span>{$t('Resolution')}</span><strong>{resolutionLabel(advice.settings.resolution)}</strong></div>
      <div><span>{$t('Frame rate')}</span><strong>{advice.settings.fps} FPS</strong></div>
      <div><span>{$t('Bitrate')}</span><strong>{advice.settings.bitrateMbps} Mb/s</strong></div>
      <div><span>{$t('Codec')}</span><strong>{codecLabel(advice.settings.codec)}</strong></div>
      <div><span>{$t('Network')}</span><strong>{routeLabel(advice.settings.networkMode)}</strong></div>
    </div>
    {#if live}
      <p class="measured">
        {#if live.hostAnswered && live.host}
          {$t('Measured just now')}:
          {#if live.serverMs != null && live.host.serverMs != null}{$t('round trip ~{ms} ms', { ms: live.serverMs + live.host.serverMs })} · {/if}
          {#if live.downloadMbps != null}{$t('this computer downloads {mbps} Mb/s', { mbps: live.downloadMbps })} · {/if}
          {#if live.host.busy}{$t('the host is in a session, so its speed was not tested')}{:else if live.host.uploadMbps != null}{$t('the host uploads {mbps} Mb/s', { mbps: live.host.uploadMbps })}{/if}
          {#if live.host.displayWidth && live.host.displayHeight} · {$t('host screen {width}×{height}', { width: live.host.displayWidth, height: live.host.displayHeight })}{#if live.host.refreshHz} @ {live.host.refreshHz} Hz{/if}{/if}
          {#if live.host.cpuModel} · {live.host.cpuModel}{/if}
        {:else}
          {$t('The host did not answer the measurement (offline or an older Sanser), so only this computer was measured.')}
          {#if live.downloadMbps != null} {$t('this computer downloads {mbps} Mb/s', { mbps: live.downloadMbps })}{/if}
        {/if}
      </p>
    {/if}
    {#if advice.reasons.length}<ul class="reasons">{#each advice.reasons as reason, index (index)}<li>{reason}</li>{/each}</ul>{/if}
    <div class="button-row">
      <button class="button primary" disabled={applied} onclick={apply}>{applied ? $t('Applied') : $t('Use these settings')}</button>
      <small class="source">{advice.source === 'model' ? $t('Suggested by {model}', { model: advice.model ?? 'AI' }) : $t('Suggested by built-in rules (no AI model is set up on the server)')}</small>
    </div>
  {/if}
</article>

<style>
  .goal { min-height: 64px; }
  .advice { display: grid; grid-template-columns: repeat(auto-fit, minmax(110px, 1fr)); gap: 10px; }
  .advice div { display: flex; flex-direction: column; gap: 2px; }
  .advice span { color: var(--muted); font-size: 12px; }
  .reasons { margin: 0; padding-left: 18px; display: grid; gap: 4px; font-size: 13px; }
  .source { color: var(--muted); align-self: center; }
  .measured { margin: 0; color: var(--muted); font-size: 12px; line-height: 1.5; }
  .inline-field .button { white-space: nowrap; }
</style>
