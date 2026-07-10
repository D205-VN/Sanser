<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '../components/Icon.svelte';
  import StatusPill from '../components/StatusPill.svelte';
  import type { Device, Page, QualityProfile, RuntimeStatus } from '../lib/types';
  import { connection } from '../stores/connection';
  import { diagnostics } from '../stores/diagnostics';
  import { preferences } from '../stores/preferences';
  import { session } from '../stores/session';

  let { runtime, navigate }: { runtime: RuntimeStatus; navigate: (page: Page) => void } = $props();
  let devices = $state<Device[]>([]);
  let query = $state('');
  let filter = $state<'all' | 'online' | 'offline'>('all');
  let loading = $state(false);
  let error = $state<string | null>(null);
  let editingId = $state<string | null>(null);
  let editingName = $state('');

  const visibleDevices = $derived(
    devices
      .filter((device) => filter === 'all' || (filter === 'online' ? device.online : !device.online))
      .filter((device) => `${device.name} ${device.platform} ${device.gpu ?? ''}`.toLowerCase().includes(query.trim().toLowerCase()))
      .sort((left, right) => Number(right.pinned) - Number(left.pinned) || Number(right.online) - Number(left.online) || left.name.localeCompare(right.name))
  );

  async function loadDevices(): Promise<void> {
    const client = session.client();
    if (!client) {
      devices = [];
      return;
    }
    loading = true;
    error = null;
    try {
      devices = (await client.devices()).items;
    } catch (caught) {
      error = caught instanceof Error ? caught.message : 'Unable to load computers';
    } finally {
      loading = false;
    }
  }

  async function connect(device: Device): Promise<void> {
    const client = session.client();
    if (!client || !device.online) return;
    error = null;
    try {
      const created = await client.createSession(device.id, $preferences.networkMode, $preferences.stream.profile);
      connection.begin(created);
      navigate('session');
    } catch (caught) {
      error = caught instanceof Error ? caught.message : 'Unable to request a connection';
    }
  }

  async function togglePin(device: Device): Promise<void> {
    const client = session.client();
    if (!client) return;
    try {
      const updated = await client.pinDevice(device.id, !device.pinned);
      devices = devices.map((item) => (item.id === device.id ? updated : item));
    } catch (caught) {
      error = caught instanceof Error ? caught.message : 'Unable to update pin';
    }
  }

  function beginRename(device: Device): void {
    editingId = device.id;
    editingName = device.name;
  }

  async function saveRename(device: Device): Promise<void> {
    const client = session.client();
    const name = editingName.trim();
    if (!client || name.length < 1 || name.length > 80) return;
    try {
      const updated = await client.renameDevice(device.id, name);
      devices = devices.map((item) => (item.id === device.id ? updated : item));
      editingId = null;
    } catch (caught) {
      error = caught instanceof Error ? caught.message : 'Unable to rename computer';
    }
  }

  async function remove(device: Device): Promise<void> {
    const client = session.client();
    if (!client || !window.confirm(`Remove “${device.name}” from your account?`)) return;
    try {
      await client.removeDevice(device.id);
      devices = devices.filter((item) => item.id !== device.id);
    } catch (caught) {
      error = caught instanceof Error ? caught.message : 'Unable to remove computer';
    }
  }

  function inspect(device: Device): void {
    diagnostics.add({
      level: 'info',
      category: 'network',
      message: `Opened diagnostics for ${device.name}`,
      details: { latencyMs: device.latencyMs, route: device.route, online: device.online }
    });
    navigate('diagnostics');
  }

  async function setProfile(profile: QualityProfile): Promise<void> {
    await preferences.save({ ...$preferences, stream: { ...$preferences.stream, profile } });
  }

  onMount(() => {
    void loadDevices();
  });
</script>

<section class="page">
  <header class="page-header">
    <div><h1>Computers</h1><p>Your available Sanser hosts, direct and relay capable.</p></div>
    <div class="button-row">
      <select class="select compact-select" aria-label="Default quality profile" value={$preferences.stream.profile} onchange={(event) => setProfile((event.currentTarget as HTMLSelectElement).value as QualityProfile)}>
        <option value="auto">Auto profile</option><option value="competitive">Competitive</option><option value="balanced">Balanced</option><option value="quality">Quality</option><option value="custom">Custom</option>
      </select>
      <button class="button" onclick={loadDevices} disabled={loading || $session.mode !== 'cloud'}>Refresh</button>
    </div>
  </header>

  <div class="computer-toolbar">
    <label class="search-box"><Icon name="search" /><input type="search" bind:value={query} placeholder="Search computers" aria-label="Search computers" /></label>
    <div class="filter-tabs" role="group" aria-label="Computer status">
      {#each ['all', 'online', 'offline'] as option}
        <button class:active={filter === option} onclick={() => (filter = option as typeof filter)}>{option}</button>
      {/each}
    </div>
  </div>

  {#if error}<div class="notice error" role="alert">{error}</div>{/if}
  {#if loading}
    <div class="grid two computer-grid" aria-label="Loading computers"><div class="skeleton"></div><div class="skeleton"></div></div>
  {:else if $session.mode === 'local'}
    <div class="card empty">
      <div><div class="empty-mark"><Icon name="computers" /></div><h2>Local discovery is not active</h2><p>{runtime.capabilities.localDiscovery.reason ?? 'No LAN hosts were discovered.'}</p><StatusPill state={runtime.capabilities.localDiscovery.state} label={runtime.capabilities.localDiscovery.state} /></div>
    </div>
  {:else if visibleDevices.length === 0}
    <div class="card empty">
      <div><div class="empty-mark"><Icon name="computers" /></div><h2>{devices.length === 0 ? 'No computers yet' : 'No matching computers'}</h2><p>{devices.length === 0 ? 'Install Sanser on a Windows host and bring it online with this account.' : 'Try another search or status filter.'}</p></div>
    </div>
  {:else}
    <div class="grid two computer-grid">
      {#each visibleDevices as device (device.id)}
        <article class="card computer-card">
          <div class="computer-top">
            <div class="device-glyph"><Icon name="monitor" /></div>
            <div class="computer-title">
              {#if editingId === device.id}
                <form onsubmit={(event) => { event.preventDefault(); void saveRename(device); }} class="rename-form">
                  <input class="input" bind:value={editingName} maxlength="80" aria-label="Computer name" />
                  <button class="button small" type="submit">Save</button>
                  <button class="button small ghost" type="button" onclick={() => (editingId = null)}>Cancel</button>
                </form>
              {:else}
                <div><h2>{device.name}</h2><p>{device.platform} · {device.gpu ?? 'GPU not reported'}</p></div>
              {/if}
            </div>
            <StatusPill state={device.online ? 'online' : 'offline'} label={device.streaming ? 'Streaming' : device.online ? 'Online' : 'Offline'} />
          </div>
          <div class="device-stats">
            <div><span>Latency</span><strong>{device.latencyMs === null ? '—' : `${device.latencyMs} ms`}</strong></div>
            <div><span>Quality</span><strong>{device.networkQuality}</strong></div>
            <div><span>Codec</span><strong>{device.capabilities.codecs.map((item) => item === 'h264' ? 'H.264' : item === 'hevc' ? 'HEVC' : 'Auto').join(' · ')}</strong></div>
          </div>
          <div class="capability-tags">
            <span class:ok={device.capabilities.nativeTransport}>SNV2</span><span class:ok={device.capabilities.webRtc}>WebRTC</span><span class:ok={device.capabilities.audio}>Audio</span><span class:ok={device.capabilities.gamepad}>Gamepad</span>
          </div>
          <div class="computer-actions">
            <button class="button primary" disabled={!device.online || device.streaming} onclick={() => connect(device)}>Connect</button>
            <button class="button small" onclick={() => togglePin(device)}>{device.pinned ? 'Unpin' : 'Pin'}</button>
            <button class="button small" onclick={() => beginRename(device)}>Rename</button>
            <button class="button small" onclick={() => inspect(device)}>Diagnostics</button>
            <button class="button small danger" onclick={() => remove(device)}>Remove</button>
            <button class="button small" disabled title="Wake-on-LAN backend is planned">Wake · Planned</button>
          </div>
          {#if device.route}<div class="device-route">Advanced route: {device.route}</div>{/if}
        </article>
      {/each}
    </div>
  {/if}
</section>
