<script lang="ts">
  import { get } from 'svelte/store';
  import type { RuntimeStatus, SharingMember } from '../lib/types';
  import { host } from '../stores/host';
  import { presence } from '../stores/presence';
  import { sharing } from '../stores/sharing';
  let { runtime }: { runtime: RuntimeStatus } = $props();
  let selected = $state('');
  let actionError = $state<string | null>(null);
  const group = $derived($sharing.ownGroup);
  const active = $derived(group?.members.filter((member) => member.state === 'invited' || member.state === 'accepted') ?? []);
  const ownComputer = $derived($sharing.devices.find((device) => device.id === $host.deviceId)?.computerId);
  const candidates = $derived($sharing.devices.filter((device) =>
    device.deviceRole === 'client' && device.id !== $presence.deviceId && device.id !== $host.deviceId &&
    (!ownComputer || device.computerId !== ownComputer) && device.crossPlatform === true &&
    device.capabilities.nativeTransport && !active.some((member) => member.viewerDeviceId === device.id)
  ));
  function name(id: string): string { return $sharing.devices.find((device) => device.id === id)?.name ?? `Computer ${id.slice(0, 8)}`; }
  function viewerState(member: SharingMember): string {
    const error = $sharing.peerErrors[member.id];
    if (error) return error;
    if (member.state === 'invited') return 'Invitation sent · waiting for approval';
    return member.startedAt !== undefined && !$sharing.paused ? 'Watching · view only' : 'Connecting · waiting for the first frame';
  }
  async function run(operation: () => Promise<void>): Promise<void> {
    actionError = null;
    try { await operation(); }
    catch (error) { actionError = error instanceof Error ? error.message : 'Unable to update sharing'; }
  }
  async function invite(): Promise<void> { if (!get(host).online) { actionError = 'Go online before inviting another computer'; return; } if (selected) await run(async () => { await sharing.invite(selected); selected = ''; }); }
</script>

<article class="card card-body stack sharing-panel">
  <div class="sharing-heading">
    <div><h2 class="card-title">Share with multiple computers</h2><p class="card-subtitle">Up to 3 computers can watch. Viewers cannot control your mouse or keyboard.</p></div>
    {#if group}<button class="button small danger" disabled={$sharing.busy} onclick={() => run(() => sharing.stop())}>Stop sharing</button>{/if}
  </div>
  {#if actionError}<div class="notice error" role="alert">{actionError}</div>{/if}
  {#if !$sharing.supported}
    <div class="notice">{$sharing.error ?? 'Checking sharing support…'} <button class="button small" onclick={() => run(() => sharing.initialize(runtime))}>Check again</button></div>
  {:else if !group}
    <div class="button-row"><button class="button primary" disabled={!$host.online || $sharing.busy || $host.engineRunning || $host.launchingSessionId !== null} onclick={() => run(() => sharing.create())}>Start sharing</button><span class="muted">{$host.online ? 'Only computers in your account can be invited.' : 'Go online first.'}</span></div>
  {:else}
    {#if $sharing.paused}<div class="notice">This sharing group is paused on this computer. Stop sharing and start a new group to resume.</div>{/if}
    <p class="sharing-count" role="status">{$sharing.paused ? 0 : active.filter((member) => member.startedAt !== undefined && member.state === 'accepted').length} watching · {active.length}/{group.maxViewers} places reserved</p>
    {#each active as member (member.id)}
      <div class="viewer-row"><div><strong>{name(member.viewerDeviceId)}</strong><p>{viewerState(member)}</p></div><button class="button small danger" disabled={$sharing.busy} onclick={() => run(() => sharing.remove(member))}>Remove</button></div>
    {/each}
    {#if active.length < group.maxViewers}
      <div class="invite-row">
        <label for="sharing-viewer">Invite a computer</label>
        <select id="sharing-viewer" bind:value={selected} disabled={$sharing.busy || candidates.length === 0}>
          <option value="">Choose a computer…</option>
          {#each candidates as device (device.id)}<option value={device.id}>{device.name} · {device.platform}{device.online ? '' : ' · Offline'}</option>{/each}
        </select>
        <button class="button primary" disabled={!selected || $sharing.busy} onclick={invite}>Invite</button>
      </div>
      {#if candidates.length === 0}<p class="muted">Sign in to Sanser on another computer to make it available here.</p>{/if}
    {/if}
    {#if $sharing.error}<div class="notice error" role="alert">{$sharing.error}</div>{/if}
  {/if}
</article>

<style>
  .sharing-panel { margin: 0 0 24px; }
  .sharing-heading, .viewer-row { display: flex; align-items: center; justify-content: space-between; gap: 16px; }
  .viewer-row { padding: 14px 0; border-bottom: 1px solid var(--border); }
  .viewer-row p { margin: 5px 0 0; color: var(--text-muted); font-size: 13px; }
  .sharing-count { color: var(--accent); margin: 0; }
  .invite-row { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }
  .invite-row select { flex: 1; min-width: 200px; }
  @media (max-width: 700px) { .sharing-heading { align-items: flex-start; flex-direction: column; } }
</style>
