<script lang="ts">
  import { onDestroy } from 'svelte';
  import StatusPill from '../components/StatusPill.svelte';
  import Toggle from '../components/Toggle.svelte';
  import { normalizeServerUrl } from '../lib/api';
  import type { LoginSession, NetworkMode, Preferences, QualityProfile, RuntimeStatus, SettingsSection, VideoCodec } from '../lib/types';
  import { exportDiagnostics } from '../lib/platform';
  import { diagnostics } from '../stores/diagnostics';
  import { SERVER_ENDPOINT_LOCKED, preferences } from '../stores/preferences';
  import { session } from '../stores/session';
  import Diagnostics from './Diagnostics.svelte';

  let { runtime, onSignOut, initialSection = 'stream' }: { runtime: RuntimeStatus; onSignOut: () => Promise<void>; initialSection?: SettingsSection } = $props();
  let active = $state<SettingsSection>('stream');
  let showDetails = $state(false);
  let serverUrl = $state('');
  let saveError = $state(false);
  let saveTimer: number | undefined;
  let saving = $state(false);
  let saveMessage = $state<string | null>(null);
  let loginSessions = $state<LoginSession[]>([]);
  let accountBusy = $state(false);
  let securityBusy = $state(false);
  let currentPassword = $state('');
  let newPassword = $state('');
  let confirmPassword = $state('');

  const sections: { id: SettingsSection; label: string }[] = [
    { id: 'stream', label: 'Quality' }, { id: 'video', label: 'Video' },
    { id: 'audio', label: 'Audio' }, { id: 'input', label: 'Input' },
    { id: 'network', label: 'Network' }, { id: 'host', label: 'Host' }, { id: 'security', label: 'Security' },
    { id: 'diagnostics', label: 'Diagnostics' }, { id: 'account', label: 'Account' }
  ];

  $effect(() => {
    active = initialSection;
    showDetails = initialSection === 'diagnostics';
  });

  $effect(() => {
    serverUrl = $preferences.serverUrl;
  });

  async function saveRoot(update: Partial<Preferences>): Promise<void> {
    window.clearTimeout(saveTimer);
    saveError = false;
    saving = true;
    try {
      await preferences.save({ ...$preferences, ...update });
      saveMessage = 'Changes saved';
      saveTimer = window.setTimeout(() => (saveMessage = null), 3_000);
    } catch (error) {
      saveError = true;
      saveMessage = error instanceof Error ? error.message : 'Unable to save changes. Please try again.';
    } finally { saving = false; }
  }

  async function saveServer(): Promise<void> {
    if (SERVER_ENDPOINT_LOCKED) return;
    try {
      const normalized = normalizeServerUrl(serverUrl);
      serverUrl = normalized;
      await saveRoot({ serverUrl: normalized });
      if (!saveError) {
        await onSignOut();
        await session.initialize(normalized);
      }
    } catch (error) {
      saveError = true;
      saveMessage = error instanceof Error ? error.message : 'Invalid server URL';
    }
  }

  async function saveStream(update: Partial<typeof $preferences.stream>): Promise<void> {
    await saveRoot({ stream: { ...$preferences.stream, ...update, profile: update.profile ?? 'custom' } });
  }

  async function saveHost(update: Partial<typeof $preferences.host>): Promise<void> {
    await saveRoot({ host: { ...$preferences.host, ...update } });
  }

  async function saveInput(update: Partial<typeof $preferences.input>): Promise<void> {
    await saveRoot({ input: { ...$preferences.input, ...update } });
  }

  async function loadLoginSessions(): Promise<void> {
    const client = session.client();
    if (!client) return;
    accountBusy = true;
    saveMessage = null;
    try {
      loginSessions = await client.loginSessions();
    } catch (error) {
      saveError = true;
      saveMessage = error instanceof Error ? error.message : 'Unable to load login sessions';
    } finally {
      accountBusy = false;
    }
  }

  async function revokeLoginSession(id: string): Promise<void> {
    const client = session.client();
    if (!client) return;
    accountBusy = true;
    try {
      await client.revokeLoginSession(id);
      loginSessions = loginSessions.filter((item) => item.id !== id);
    } catch (error) {
      saveError = true;
      saveMessage = error instanceof Error ? error.message : 'Unable to revoke session';
    } finally {
      accountBusy = false;
    }
  }

  async function changePassword(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    saveMessage = null;
    if (newPassword.length < 12) {
      saveMessage = 'The new password must contain at least 12 characters';
      return;
    }
    if (newPassword !== confirmPassword) {
      saveMessage = 'The new passwords do not match';
      return;
    }
    const client = session.client();
    if (!client) return;
    securityBusy = true;
    try {
      await client.changePassword(currentPassword, newPassword);
      currentPassword = '';
      newPassword = '';
      confirmPassword = '';
      await onSignOut();
    } catch (error) {
      saveError = true;
      saveMessage = error instanceof Error ? error.message : 'Unable to change password';
    } finally {
      securityBusy = false;
    }
  }

  async function exportEvents(): Promise<void> {
    try {
      const result = await exportDiagnostics(diagnostics.exportJson());
      saveError = false;
      saveMessage = `Exported: ${result.path}`;
    } catch (error) {
      saveError = true;
      saveMessage = error instanceof Error ? error.message : 'Unable to export diagnostics';
    }
  }

  async function setDiagnosticsEnabled(value: boolean): Promise<void> {
    await saveRoot({ diagnosticsEnabled: value });
    diagnostics.setEnabled($preferences.diagnosticsEnabled);
  }

  function chooseSection(section: SettingsSection): void {
    active = section;
    saveMessage = null;
    saveError = false;
    if (section === 'account') void loadLoginSessions();
  }
  onDestroy(() => window.clearTimeout(saveTimer));
</script>

<section class="page settings-page">
  <header class="page-header"><div><h1>Settings</h1><p>Changes save automatically.</p></div>{#if saving}<span class="save-message" role="status">Saving…</span>{/if}</header>
  {#if saveMessage}<div class="notice" class:error={saveError} role="status">{saveMessage}</div>{/if}
  <div class="settings-layout">
    <nav class="settings-nav" aria-label="Settings sections">
      {#each sections.filter((section) => section.id !== 'audio' || runtime.capabilities.clientAudio === true || runtime.capabilities.hostAudio === true) as section}<button class:active={active === section.id} aria-current={active === section.id ? 'page' : undefined} onclick={() => chooseSection(section.id)}>{section.label}</button>{/each}
    </nav>
    <div class="settings-content">
      {#if active === 'stream'}
        <article class="card card-body settings-card stack">
          <div><h2 class="card-title">Quality</h2><p class="card-subtitle">Choose the default quality for new connections.</p></div>
          <div class="profile-grid">
            {#each [
              ['auto', 'Auto', '1080p at 60 FPS · 20 Mb/s.'],
              ['competitive', 'Competitive', '720p at 120 FPS · 12 Mb/s.'],
              ['balanced', 'Balanced', '1080p at 60 FPS · 20 Mb/s.'],
              ['quality', 'Quality', '1440p at 60 FPS · 40 Mb/s.'],
              ['custom', 'Custom', 'Use Video settings. The host controls custom capture quality.']
            ] as profile}
              <button class="profile-option" class:active={$preferences.stream.profile === profile[0]} onclick={() => saveStream({ profile: profile[0] as QualityProfile })}><strong>{profile[1]}</strong><span>{profile[2]}</span></button>
            {/each}
          </div>
        </article>
      {:else if active === 'video'}
        <article class="card card-body settings-card stack">
          <div><h2 class="card-title">Video</h2><p class="card-subtitle">Editing a video option switches to Custom. Presets use the values listed in Stream.</p></div>
          <div class="grid two">
            <div class="field"><label for="codec">Codec</label><select id="codec" class="select" value={$preferences.stream.codec} onchange={(event) => saveStream({ codec: (event.currentTarget as HTMLSelectElement).value as VideoCodec })}><option value="auto">Auto</option><option value="h264">H.264 · compatibility</option><option value="hevc">HEVC · smaller bitrate</option></select></div>
            <div class="field"><label for="resolution">Resolution</label><select id="resolution" class="select" value={$preferences.stream.resolution} onchange={(event) => saveStream({ resolution: (event.currentTarget as HTMLSelectElement).value as typeof $preferences.stream.resolution })}><option value="auto">Auto</option><option value="720p">1280 × 720</option><option value="1080p">1920 × 1080</option><option value="1440p">2560 × 1440</option><option value="2160p">3840 × 2160</option></select></div>
            <div class="field"><label for="fps">Frame rate</label><select id="fps" class="select" value={$preferences.stream.fps} onchange={(event) => saveStream({ fps: Number((event.currentTarget as HTMLSelectElement).value) as typeof $preferences.stream.fps })}><option value="30">30 FPS</option><option value="60">60 FPS</option><option value="90">90 FPS</option><option value="120">120 FPS</option></select></div>
            <div class="field"><label for="bitrate">Bitrate · {$preferences.stream.bitrateMbps} Mb/s</label><input id="bitrate" type="range" min="1" max="100" step="1" value={$preferences.stream.bitrateMbps} onchange={(event) => saveStream({ bitrateMbps: Number((event.currentTarget as HTMLInputElement).value) })} /></div>
          </div>
          <div class="notice">Choose Auto unless you need a specific video format. HEVC requires support on both computers.</div>
        </article>
      {:else if active === 'audio'}
        <article class="card card-body settings-card stack">
          <div><h2 class="card-title">Audio</h2><p class="card-subtitle">Share sound during a connection.</p></div>
          <Toggle checked={$preferences.host.audioEnabled && (runtime.capabilities.clientAudio === true || runtime.capabilities.hostAudio === true)} disabled={runtime.capabilities.clientAudio !== true && runtime.capabilities.hostAudio !== true} label="Session audio" description="Share sound from the remote computer." onchange={(value) => saveHost({ audioEnabled: value })} />
        </article>
      {:else if active === 'input'}
        <article class="card card-body settings-card stack">
          <div><h2 class="card-title">Input</h2><p class="card-subtitle">Choose how the remote mouse behaves.</p></div>
          <p class="card-subtitle">To release control, use the shortcut shown in the remote window.</p>
          <div class="field"><label for="mouse-mode">Mouse capture</label><select id="mouse-mode" class="select" value={$preferences.input.mouseMode} onchange={(event) => saveInput({ mouseMode: (event.currentTarget as HTMLSelectElement).value as typeof $preferences.input.mouseMode })}><option value="auto">Auto</option><option value="absolute">Absolute · desktop</option><option value="relative">Relative · games</option></select></div>
        </article>
      {:else if active === 'network'}
        <article class="card card-body settings-card stack">
          <div><h2 class="card-title">Network</h2><p class="card-subtitle">Choose how your computers connect.</p></div>
          <div class="network-options">
            {#each [['auto', 'Auto', 'Try direct first, then use relay if needed.'], ['direct', 'Direct', 'Connect directly. No relay fallback.'], ['relay', 'Relay', 'Connect through the Sanser relay service.']] as mode}
              <button class="network-option" class:active={$preferences.networkMode === mode[0]} onclick={() => saveRoot({ networkMode: mode[0] as NetworkMode })}><strong>{mode[1]}</strong><span>{mode[2]}</span></button>
            {/each}
          </div>
          {#if !SERVER_ENDPOINT_LOCKED}
            <details class="sharing-help"><summary>Server settings</summary><div class="field"><label for="general-server">Server URL</label><div class="inline-field"><input id="general-server" class="input" type="url" bind:value={serverUrl} spellcheck="false" /><button class="button" onclick={saveServer}>Save</button></div><small>Changing the server signs you out.</small></div></details>
          {/if}
        </article>
      {:else if active === 'host'}
        <article class="card card-body settings-card stack">
          <div><h2 class="card-title">Host</h2><p class="card-subtitle">Choose how this computer shares its screen.</p></div>
          <details class="sharing-help"><summary>Advanced network setup</summary>
          <div class="field">
            <label for="direct-udp-port">Direct UDP port</label>
            <input
              id="direct-udp-port"
              class="input"
              type="number"
              min="1024"
              max="65535"
              step="1"
              value={$preferences.host.directUdpPort}
              onchange={(event) => saveHost({ directUdpPort: Number((event.currentTarget as HTMLInputElement).value) })}
            />
            <small>Keep this port fixed so UPnP or a manual router rule remains valid.</small>
          </div>
          <div class="notice">
            Manual setup: reserve the host computer's LAN address, forward external UDP
            <strong>{$preferences.host.directUdpPort}</strong> to the same internal UDP port, and allow Sanser through the system firewall.
          </div>
          </details>
          <Toggle checked={$preferences.host.autoOnline} disabled={runtime.capabilities.hostEngine.state !== 'available'} label="Bring host online automatically" description="Allow connection requests when you open Sanser." onchange={(value) => saveHost({ autoOnline: value })} />
          <Toggle checked={$preferences.host.autoAcceptOwnDevices} disabled={runtime.capabilities.hostEngine.state !== 'available'} label="Auto accept trusted devices" description={`Accept only requester identities trusted from Computers on this host (${$preferences.trustedDeviceIds.length}).`} onchange={(value) => saveHost({ autoAcceptOwnDevices: value })} />
          {#if runtime.capabilities.hostAudio === true}<Toggle checked={$preferences.host.audioEnabled} label="System audio" description="Share sound from this computer." onchange={(value) => saveHost({ audioEnabled: value })} />{/if}
          <Toggle checked={$preferences.host.inputEnabled} disabled={runtime.capabilities.hostEngine.state !== 'available'} label="Remote input" description="Allow remote keyboard and mouse control." onchange={(value) => saveHost({ inputEnabled: value })} />
        </article>
      {:else if active === 'security'}
        <article class="card card-body settings-card stack"><form class="password-form" onsubmit={changePassword}><h3>Change password</h3><p>All signed-in sessions are revoked after this change.</p><div class="field"><label for="current-password">Current password</label><input id="current-password" class="input" type="password" bind:value={currentPassword} required minlength="12" maxlength="256" autocomplete="current-password" /></div><div class="grid two"><div class="field"><label for="new-password">New password</label><input id="new-password" class="input" type="password" bind:value={newPassword} required minlength="12" maxlength="256" autocomplete="new-password" /></div><div class="field"><label for="confirm-password">Confirm new password</label><input id="confirm-password" class="input" type="password" bind:value={confirmPassword} required minlength="12" maxlength="256" autocomplete="new-password" /></div></div><button class="button" type="submit" disabled={securityBusy}>{securityBusy ? 'Updating…' : 'Update password'}</button></form></article>
      {:else if active === 'diagnostics'}
        <article class="card card-body stack">
          <div><h2 class="card-title">Diagnostics</h2><p class="card-subtitle">Use these tools when a connection is not working.</p></div>
          <Toggle checked={$preferences.diagnosticsEnabled} label="Collect basic diagnostics" description="Keep a local record of connections and errors." onchange={setDiagnosticsEnabled} />
          <div class="button-row">
            <button class="button" onclick={exportEvents}>Export diagnostics</button>
            <button class="button danger" onclick={() => diagnostics.clear()}>Clear events</button>
          </div>
          <button class="button ghost" aria-expanded={showDetails} aria-controls="connection-details" onclick={() => (showDetails = !showDetails)}>{showDetails ? 'Hide connection details' : 'Show connection details'}</button>
        </article>
        {#if showDetails}<div id="connection-details"><Diagnostics {runtime} /></div>{/if}
      {:else if active === 'account'}
        <article class="card card-body settings-card stack"><div><h2 class="card-title">Account</h2><p class="card-subtitle">{$session.account?.email}</p></div><div class="button-row"><button class="button" onclick={loadLoginSessions} disabled={accountBusy}>Refresh sessions</button><button class="button danger" onclick={onSignOut}>Sign out</button></div>{#if loginSessions.length > 0}<div class="login-session-list">{#each loginSessions as item (item.id)}<div><div><strong>{item.deviceName}</strong><span>{item.platform} · last seen {item.lastSeenAt}</span></div><StatusPill state={item.current ? 'online' : 'neutral'} label={item.current ? 'Current' : 'Active'} />{#if !item.current}<button class="button small danger" disabled={accountBusy} onclick={() => revokeLoginSession(item.id)}>Revoke</button>{/if}</div>{/each}</div>{:else}<div class="notice">No login-session list has been loaded yet. Select “Refresh sessions” to retrieve it from the server.</div>{/if}</article>
      {/if}
    </div>
  </div>
</section>
