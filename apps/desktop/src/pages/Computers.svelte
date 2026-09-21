<script lang="ts">
  import { nativeCompatibilityReason } from '../lib/nativeCompatibility';
  import { SvelteSet } from 'svelte/reactivity';
  import { onMount } from 'svelte';
  import Icon from '../components/Icon.svelte';
  import StatusPill from '../components/StatusPill.svelte';
  import type { Device, Page, RuntimeStatus } from '../lib/types';
  import { connection } from '../stores/connection';
  import { diagnostics } from '../stores/diagnostics';
  import { preferences } from '../stores/preferences';
  import { host as hostStore } from '../stores/host';
  import { presence } from '../stores/presence';
  import { session } from '../stores/session';

  let { runtime, navigate }: { runtime: RuntimeStatus; navigate: (page: Page) => void } = $props();
  let devices = $state<Device[]>([]);
  let query = $state('');
  let filter = $state<'all' | 'online' | 'offline' | 'pinned'>('all');
  let actionId = $state<string | null>(null);
  let lastUpdated = $state<Date | null>(null);
  let mounted = false;
  let revision = 0;
  let loading = $state(false);
  let error = $state<string | null>(null);
  let editingId = $state<string | null>(null);
  let editingName = $state('');
  let connectingId = $state<string | null>(null);
  let loadInFlight = false;
  let layout = $state<'grid' | 'list'>('grid');
  function platformLabel(platform: string): string {
    const value = platform.toLowerCase();
    return value.includes('mac') ? 'macOS' : value.includes('windows') ? 'Windows' : platform;
  }
  function lastSeenLabel(value: string | null): string {
    if (!value) return 'Not reported';
    const date = new Date(value);
    if (!Number.isFinite(date.getTime())) return 'Not reported';
    return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
  }
  const remoteDevices = $derived(devices.filter((device) => device.id !== $presence.deviceId && device.id !== $hostStore.deviceId));
  const onlineCount = $derived(remoteDevices.filter((device) => device.online).length);
  const pinnedCount = $derived(remoteDevices.filter((device) => device.pinned).length);
  const visibleDevices = $derived(
    remoteDevices
      .filter((device) => filter === 'all' || (filter === 'pinned' ? device.pinned : filter === 'online' ? device.online : !device.online))
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
    if (showLoading) error = null;
    const currentRevision = revision;
    try {
      const items: Device[] = [];
      const cursors = new SvelteSet<string>();
      let cursor: string | undefined;
      do {
        const response = await client.devices(cursor);
        items.push(...response.items);
        cursor = response.nextCursor ?? undefined;
        if (cursor && cursors.has(cursor)) throw new Error('Unable to load more computers. Please refresh.');
        if (cursor) cursors.add(cursor);
      } while (cursor);
      if (!mounted || currentRevision !== revision) return;
      devices = [...new Map(items.map((device) => [device.id, device])).values()];
      lastUpdated = new Date();
      error = null;
    } catch (caught) {
      if (!mounted || currentRevision !== revision) return;
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
      if (!mounted || client !== session.client()) {
        await client.disconnectSession(created.id).catch(() => undefined);
        return;
      }
      connection.begin(created);
      navigate('session');
    } catch (caught) {
      error = caught instanceof Error ? caught.message : 'Unable to request a connection';
    } finally {
      connectingId = null;
    }
  }

  function connectionBlockReason(device: Device): string | null {
    if ($connection.preparing) return 'Wait for the current connection attempt to finish cleaning up';
    if ($connection.session && ['pending', 'accepted', 'connecting', 'connected'].includes($connection.session.status)) return 'Finish your current session before connecting to another computer';
    const compatibilityError = nativeCompatibilityReason(runtime, device, $preferences.stream.codec);
    if (compatibilityError) return compatibilityError;
    if (!device.online) return 'This computer is offline';
    if (device.streaming) return 'This computer is already streaming';
    if (runtime.capabilities.clientEngine.state !== 'available') return runtime.capabilities.clientEngine.reason ?? 'Install the desktop app to connect';
    if (!$presence.online || !$presence.deviceId) return $presence.error ?? 'This computer is not registered with the server yet';

    const nativeCompatible =
      runtime.capabilities.nativeDirect.state === 'available' &&
      device.capabilities.nativeTransport &&
      device.route !== null &&
      $presence.routeAddress !== null;
    const relayCompatible =
      runtime.capabilities.nativeDirect.state === 'available' &&
      device.capabilities.nativeTransport;
    if ($preferences.networkMode === 'relay' && !relayCompatible) return 'Update Sanser on both computers to use relay';
    if ($preferences.networkMode === 'direct' && !nativeCompatible) return 'A direct connection is unavailable. Try Auto in Settings.';
    if (!nativeCompatible && !relayCompatible) return 'These computers cannot connect with the current settings';
    return null;
  }

  async function togglePin(device: Device): Promise<void> {
    const client = session.client();
    if (!client || actionId) return;
    actionId = device.id;
    revision += 1;
    error = null;
    try {
      const updated = await client.pinDevice(device.id, !device.pinned);
      devices = devices.map((item) => (item.id === device.id ? updated : item));
    } catch (caught) {
      error = caught instanceof Error ? caught.message : 'Unable to update pin';
    } finally { actionId = null; revision += 1; }
  }

  async function toggleTrust(device: Device): Promise<void> {
    const trusted = $preferences.trustedDeviceIds.includes(device.id);
    if (
      !trusted &&
      !window.confirm(
        `Trust “${device.name}” for unattended access? Requests from this device may be accepted automatically by this host.`
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
    if (!client || actionId) return;
    if (!name || name.length > 80) { error = 'Use a computer name between 1 and 80 characters.'; return; }
    actionId = device.id;
    revision += 1;
    error = null;
    try {
      const updated = await client.renameDevice(device.id, name);
      devices = devices.map((item) => (item.id === device.id ? updated : item));
      editingId = null;
    } catch (caught) {
      error = caught instanceof Error ? caught.message : 'Unable to rename computer';
    } finally { actionId = null; revision += 1; }
  }

  async function remove(device: Device): Promise<void> {
    const client = session.client();
    if (!client || !window.confirm(`Remove “${device.name}” from your account?`)) return;
    if (actionId) return;
    actionId = device.id;
    revision += 1;
    error = null;
    try {
      await client.removeDevice(device.id);
      devices = devices.filter((item) => item.id !== device.id);
    } catch (caught) {
      error = caught instanceof Error ? caught.message : 'Unable to remove computer';
    } finally { actionId = null; revision += 1; }
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

  onMount(() => {
    mounted = true;
    void loadDevices();
    const timer = window.setInterval(() => {
      if (document.visibilityState === 'visible') void loadDevices(false);
    }, 5_000);
    return () => { mounted = false; window.clearInterval(timer); };
  });
</script>

<section class="page computers-page">
  <header class="page-header">
    <div><h1>Your computers<span class="heading-count" aria-hidden="true">{remoteDevices.length}</span></h1><p>Choose a computer to connect.</p></div>
    <div class="button-row"><button class="button" onclick={() => navigate('host')}>Share this computer</button><button class="button" onclick={() => void loadDevices()} disabled={loading}><Icon name="refresh" />{loading ? 'Refreshing…' : 'Refresh'}</button></div>
  </header>

  {#if $connection.session && ['pending', 'accepted', 'connecting', 'connected'].includes($connection.session.status)}
    <div class="session-banner"><span><i></i>{$connection.engineRunning ? 'Your remote window is open' : 'A connection is in progress'}</span><button class="button small" onclick={() => navigate('session')}>Return to session →</button></div>
  {/if}

  {#if $presence.error && !$presence.online && runtime.capabilities.clientEngine.state === 'available'}
    <div class="notice warning registration-notice" role="status"><span><strong>This computer is not ready to connect.</strong> {$presence.error}</span><button class="button small" disabled={$presence.busy} onclick={() => presence.start(runtime)}>Retry registration</button></div>
  {/if}
  <div class="computer-toolbar">
    <label class="search-box"><Icon name="search" /><input type="search" bind:value={query} placeholder="Search computers" aria-label="Search computers" /></label>
    <div class="filter-tabs" role="group" aria-label="Computer status">
      {#each ['all', 'online', 'offline', 'pinned'] as option}
        <button class:active={filter === option} aria-pressed={filter === option} onclick={() => (filter = option as typeof filter)}>{option === 'all' ? 'All computers' : option}<span class="filter-count">{option === 'all' ? remoteDevices.length : option === 'online' ? onlineCount : option === 'pinned' ? pinnedCount : remoteDevices.length - onlineCount}</span></button>
      {/each}
    </div>
    <div class="layout-switch" role="group" aria-label="Computer layout"><button class:active={layout === 'grid'} aria-pressed={layout === 'grid'} aria-label="Card view" onclick={() => layout = 'grid'}><Icon name="grid" /></button><button class:active={layout === 'list'} aria-pressed={layout === 'list'} aria-label="List view" onclick={() => layout = 'list'}><Icon name="list" /></button></div>
  </div>

  {#if error}<div class="notice error" role="alert">{error}</div>{/if}
  {#if loading && devices.length === 0}
    <div class="grid two computer-grid" aria-label="Loading computers"><div class="skeleton"></div><div class="skeleton"></div></div>
  {:else if visibleDevices.length === 0}
    <div class="card empty">
      <div><div class="empty-mark"><Icon name="computers" /></div><h2>{remoteDevices.length === 0 ? 'Add your first remote computer' : 'No matching computers'}</h2><p>{remoteDevices.length === 0 ? 'Install Sanser on a Windows PC or Mac and bring it online with this account.' : 'Try another search or status filter.'}</p>{#if remoteDevices.length === 0}<ol class="setup-checklist"><li><span>1</span>Install Sanser on another computer</li><li><span>2</span>Sign in with this account and enable Host</li><li><span>3</span>Refresh this list and request a connection</li></ol>{/if}{#if remoteDevices.length > 0}<button class="button" onclick={() => { query = ''; filter = 'all'; }}>Clear filters</button>{:else}<button class="button primary" onclick={() => navigate(runtime.capabilities.hostEngine.state === 'available' ? 'host' : 'about')}>Set up your workspace →</button>{/if}</div>
    </div>
  {:else}
    <div class="grid two computer-grid" class:computer-list={layout === 'list'}>
      {#each visibleDevices as device (device.id)}
        {@const blockReason = connectionBlockReason(device)}
        <article class="card computer-card" class:device-online={device.online}>
          <div class="computer-top">
            <div class="device-glyph" class:windows-device={platformLabel(device.platform) === 'Windows'}><Icon name={platformLabel(device.platform) === 'macOS' ? 'laptop' : platformLabel(device.platform) === 'Windows' ? 'windows' : 'monitor'} /></div>
            <div class="computer-title">
              {#if editingId === device.id}
                <form onsubmit={(event) => { event.preventDefault(); void saveRename(device); }} class="rename-form">
                  <input class="input" bind:value={editingName} maxlength="80" aria-label="Computer name" />
                  <button class="button small" type="submit" disabled={actionId !== null || !editingName.trim()}>Save</button>
                  <button class="button small ghost" type="button" onclick={() => (editingId = null)}>Cancel</button>
                </form>
              {:else}
                <div><h2>{device.pinned ? '★ ' : ''}{device.name}</h2><p>{platformLabel(device.platform)} · {device.deviceRole === 'client' ? 'Sharing is off' : 'Remote computer'}</p></div>
              {/if}
            </div>
            <StatusPill state={device.online ? 'online' : 'offline'} label={device.streaming ? 'Streaming' : device.online ? 'Online' : 'Offline'} />
          </div>
          {#if device.lastSeenAt}<p class="device-last-seen" title={device.lastSeenAt}>Last seen {lastSeenLabel(device.lastSeenAt)}</p>{/if}
          <div class="computer-actions">
            <button class="button primary" disabled={blockReason !== null || connectingId !== null} title={blockReason ?? 'Request a secure session'} onclick={() => connect(device)}>{connectingId === device.id ? 'Requesting…' : 'Connect'}<Icon name="arrow" /></button>
            <button class="button small ghost" disabled={actionId !== null} aria-pressed={device.pinned} onclick={() => togglePin(device)}><Icon name="pin" />{device.pinned ? 'Pinned' : 'Pin'}</button>
            <details class="device-menu">
              <summary class="button small ghost" aria-label={`Manage ${device.name}`}><Icon name="more" /></summary>
              <div class="device-menu-items">
                <button disabled={actionId !== null} onclick={() => beginRename(device)}>Rename computer</button>
                <button onclick={() => inspect(device)}>View diagnostics</button>
                {#if runtime.capabilities.hostEngine.state === 'available' && (device.deviceRole === 'client' || (!device.deviceRole && device.platform.toLowerCase().includes('mac')) || $preferences.trustedDeviceIds.includes(device.id))}<button disabled={actionId !== null} onclick={() => toggleTrust(device)}>{$preferences.trustedDeviceIds.includes(device.id) ? 'Revoke trust' : 'Trust device'}</button>{/if}
                <button class="danger-text" disabled={actionId !== null || device.streaming} onclick={() => remove(device)}>Remove computer</button>
              </div>
            </details>
          </div>
          {#if blockReason}<div class="connect-reason"><Icon name="about" /><span>{blockReason}</span></div>{/if}

        </article>
      {/each}
    </div>
  {/if}
  <footer class="workspace-footer"><span role="status">{lastUpdated ? `Updated ${lastUpdated.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}` : 'Waiting for server'}</span></footer>
</section>
