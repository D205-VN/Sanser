<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '../components/Icon.svelte';
  import StatusPill from '../components/StatusPill.svelte';
  import type { Device, Page, QualityProfile, RuntimeStatus } from '../lib/types';
  import { connection } from '../stores/connection';
  import { diagnostics } from '../stores/diagnostics';
  import { preferences } from '../stores/preferences';
  import { presence } from '../stores/presence';
  import { session } from '../stores/session';

  let { runtime, navigate }: { runtime: RuntimeStatus; navigate: (page: Page) => void } = $props();
  let devices = $state<Device[]>([]);
  let query = $state('');
  let filter = $state<'all' | 'online' | 'offline'>('all');
  let loading = $state(false);
  let error = $state<string | null>(null);
  let editingId = $state<string | null>(null);
  let editingName = $state('');
  let connectingId = $state<string | null>(null);
  let loadInFlight = false;

  const visibleDevices = $derived(
    devices
      .filter((device) => device.id !== $presence.deviceId)
      .filter((device) => filter === 'all' || (filter === 'online' ? device.online : !device.online))
      .filter((device) => `${device.name} ${device.platform} ${device.gpu ?? ''}`.toLowerCase().includes(query.trim().toLowerCase()))
      .sort((left, right) => Number(right.pinned) - Number(left.pinned) || Number(right.online) - Number(left.online) || left.name.localeCompare(right.name))
  );

  async function loadDevices(showLoading = true): Promise<void> {
    if (loadInFlight) return;
    const client = session.client();
    if (!client) {
      devices = [];
      return;
    }
    loadInFlight = true;
    if (showLoading) loading = true;
    error = null;
    try {
      devices = (await client.devices()).items;
    } catch (caught) {
      error = caught instanceof Error ? caught.message : 'Unable to load computers';
    } finally {
      if (showLoading) loading = false;
      loadInFlight = false;
    }
  }

  async function connect(device: Device): Promise<void> {
    const client = session.client();
    const requesterDeviceId = $presence.deviceId;
    if (!client || !requesterDeviceId || connectionBlockReason(device)) return;
    error = null;
    connectingId = device.id;
    try {
      const created = await client.createSession(
        device.id,
        requesterDeviceId,
        $preferences.networkMode,
        $preferences.stream.profile,
        $preferences.stream.codec
      );
      connection.begin(created);
      navigate('session');
    } catch (caught) {
      error = caught instanceof Error ? caught.message : 'Unable to request a connection';
    } finally {
      connectingId = null;
    }
  }

  function connectionBlockReason(device: Device): string | null {
    if (!device.online) return 'This computer is offline';
    if (device.streaming) return 'This computer is already streaming';
    if (!$presence.online || !$presence.deviceId) return $presence.error ?? 'This Mac is not registered with the server yet';
    if (runtime.capabilities.clientEngine.state !== 'available') {
      return runtime.capabilities.clientEngine.reason ?? 'The native macOS client is unavailable';
    }

    const nativeCompatible =
      runtime.capabilities.nativeDirect.state === 'available' &&
      device.capabilities.nativeTransport &&
      device.route !== null &&
      $presence.routeAddress !== null;
    const relayCompatible = runtime.capabilities.webRtc.state === 'available' && device.capabilities.webRtc;
    if ($preferences.networkMode === 'relay' && !relayCompatible) return 'Relay requires WebRTC on both computers';
    if (!nativeCompatible && !relayCompatible) return 'No verified transport is available on both computers';
    return null;
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

  async function toggleTrust(device: Device): Promise<void> {
    const trusted = $preferences.trustedDeviceIds.includes(device.id);
    if (
      !trusted &&
      !window.confirm(
        `Trust “${device.name}” for unattended access? Requests from this device may be accepted automatically by this Windows host.`
      )
    ) return;
    try {
      const trustedDeviceIds = trusted
        ? $preferences.trustedDeviceIds.filter((id) => id !== device.id)
        : [...new Set([...$preferences.trustedDeviceIds, device.id])];
      await preferences.save({ ...$preferences, trustedDeviceIds });
    } catch (caught) {
      error = caught instanceof Error ? caught.message : 'Unable to update unattended-access trust';
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
    const timer = window.setInterval(() => {
      if (document.visibilityState === 'visible') void loadDevices(false);
    }, 5_000);
    return () => window.clearInterval(timer);
  });
</script>

<section class="page">
  <header class="page-header">
    <div><h1>Computers</h1><p>Your available Sanser hosts, direct and relay capable.</p></div>
    <div class="button-row">
      <select class="select compact-select" aria-label="Default quality profile" value={$preferences.stream.profile} onchange={(event) => setProfile((event.currentTarget as HTMLSelectElement).value as QualityProfile)}>
        <option value="auto">Auto profile</option><option value="competitive">Competitive</option><option value="balanced">Balanced</option><option value="quality">Quality</option><option value="custom">Custom</option>
      </select>
      <button class="button" onclick={() => void loadDevices()} disabled={loading}>Refresh</button>
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
  {:else if visibleDevices.length === 0}
    <div class="card empty">
      <div><div class="empty-mark"><Icon name="computers" /></div><h2>{devices.length === 0 ? 'No computers yet' : 'No matching computers'}</h2><p>{devices.length === 0 ? 'Install Sanser on a Windows host and bring it online with this account.' : 'Try another search or status filter.'}</p></div>
    </div>
  {:else}
    <div class="grid two computer-grid">
      {#each visibleDevices as device (device.id)}
        {@const blockReason = connectionBlockReason(device)}
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
            <span class:ok={device.capabilities.nativeTransport}>Native direct</span><span class:ok={device.capabilities.webRtc}>WebRTC</span><span class:ok={device.capabilities.audio}>Audio</span><span class:ok={device.capabilities.gamepad}>Gamepad</span>
          </div>
          <div class="computer-actions">
            <button class="button primary" disabled={blockReason !== null || connectingId !== null} title={blockReason ?? 'Request a secure session'} onclick={() => connect(device)}>{connectingId === device.id ? 'Requesting…' : 'Connect'}</button>
            <button class="button small" onclick={() => togglePin(device)}>{device.pinned ? 'Unpin' : 'Pin'}</button>
            <button
              class="button small"
              class:danger={$preferences.trustedDeviceIds.includes(device.id)}
              disabled={runtime.capabilities.hostEngine.state !== 'available'}
              title={runtime.capabilities.hostEngine.state === 'available' ? 'Control unattended-access trust on this Windows host' : 'Trust is configured on the Windows host'}
              onclick={() => toggleTrust(device)}
            >{$preferences.trustedDeviceIds.includes(device.id) ? 'Revoke trust' : 'Trust device'}</button>
            <button class="button small" onclick={() => beginRename(device)}>Rename</button>
            <button class="button small" onclick={() => inspect(device)}>Diagnostics</button>
            <button class="button small danger" onclick={() => remove(device)}>Remove</button>
            <button class="button small" disabled title="Wake-on-LAN backend is planned">Wake · Planned</button>
          </div>
          {#if blockReason}<div class="connect-reason">Connect unavailable: {blockReason}</div>{/if}
          {#if device.route}<div class="device-route">Advanced route: {device.route}</div>{/if}
        </article>
      {/each}
    </div>
  {/if}
</section>
