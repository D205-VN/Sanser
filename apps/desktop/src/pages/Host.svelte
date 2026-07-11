<script lang="ts">
  import CapabilityNotice from '../components/CapabilityNotice.svelte';
  import StatusPill from '../components/StatusPill.svelte';
  import Toggle from '../components/Toggle.svelte';
  import type { RuntimeStatus } from '../lib/types';
  import { host } from '../stores/host';
  import { preferences } from '../stores/preferences';
  import { session } from '../stores/session';

  let { runtime }: { runtime: RuntimeStatus } = $props();
  let actionError = $state<string | null>(null);

  const canHost = $derived($session.mode === 'cloud' && runtime.capabilities.hostEngine.state === 'available');

  async function toggleHost(): Promise<void> {
    actionError = null;
    try {
      if ($host.online) await host.offline();
      else await host.online(runtime);
    } catch (error) {
      actionError = error instanceof Error ? error.message : 'Unable to change host state';
    }
  }

  async function updateHost(key: 'audioEnabled' | 'inputEnabled', value: boolean): Promise<void> {
    await preferences.save({ ...$preferences, host: { ...$preferences.host, [key]: value } });
  }
</script>

<section class="page">
  <header class="page-header">
    <div><h1>Host</h1><p>Advertise this Windows computer and control what a remote session may use.</p></div>
    <StatusPill state={$host.online ? 'online' : 'offline'} label={$host.online ? 'Online' : 'Offline'} />
  </header>

  {#if actionError ?? $host.error}<div class="notice error" role="alert">{actionError ?? $host.error}</div>{/if}
  <div class="host-hero card">
    <div class="host-state-orb" class:online={$host.online}><span></span></div>
    <div><h2>{$host.online ? 'This computer is visible' : 'This computer is private'}</h2><p>{$host.online ? 'Authenticated devices on this account may request a session.' : 'No remote connection request can be accepted.'}</p></div>
    <button class:danger={$host.online} class:primary={!$host.online} class="button" disabled={$host.busy || (!canHost && !$host.online)} title={!canHost && !$host.online ? runtime.capabilities.hostEngine.reason ?? 'The Windows host engine is unavailable' : undefined} onclick={toggleHost}>{$host.busy ? 'Working…' : $host.online ? 'Go offline' : 'Go online'}</button>
  </div>

  <div class="grid two host-grid">
    <article class="card card-body stack">
      <div><h2 class="card-title">Capture</h2><p class="card-subtitle">Capture engines are enumerated by the native Windows sidecar.</p></div>
      <div class="field"><label for="monitor">Monitor</label><select id="monitor" class="select" disabled><option>Primary display · enumeration planned</option></select></div>
      <div class="field"><label for="window">Window</label><select id="window" class="select" disabled><option>Entire display · window capture planned</option></select></div>
      <CapabilityNotice title="Windows capture engine" capability={runtime.capabilities.hostEngine} />
    </article>

    <article class="card card-body stack">
      <div><h2 class="card-title">Session permissions</h2><p class="card-subtitle">These controls are passed to the native engine when an authenticated session starts.</p></div>
      <Toggle checked={$preferences.host.audioEnabled} disabled={runtime.capabilities.hostEngine.state !== 'available'} label="System audio" description="Forward WASAPI loopback audio when the engine supports it." onchange={(value) => updateHost('audioEnabled', value)} />
      <Toggle checked={$preferences.host.inputEnabled} disabled={runtime.capabilities.hostEngine.state !== 'available'} label="Remote input" description="Allow authenticated keyboard and mouse injection." onchange={(value) => updateHost('inputEnabled', value)} />
      <Toggle checked={false} disabled label="Clipboard" description={runtime.capabilities.clipboard.reason ?? 'Clipboard is planned.'} onchange={() => undefined} />
    </article>

    <article class="card card-body stack">
      <div><h2 class="card-title">Approval policy</h2><p class="card-subtitle">Automatic acceptance is disabled until signed host event handling is available.</p></div>
      <Toggle checked={false} disabled label="Auto accept own devices" description="Planned: accept only verified devices on the same account." onchange={() => undefined} />
      <div class="notice">Current policy: every request must be approved. The UI will not silently grant input access.</div>
    </article>

    <article class="card card-body stack">
      <div><h2 class="card-title">Current client</h2><p class="card-subtitle">Live host events are shown here after signaling integration.</p></div>
      <div class="empty compact-empty"><div><h3>No active client</h3><p>Capture does not run while idle, keeping CPU and GPU use low.</p><button class="button danger" disabled title="No active native session">Stop session · Unavailable</button></div></div>
    </article>
  </div>
</section>
