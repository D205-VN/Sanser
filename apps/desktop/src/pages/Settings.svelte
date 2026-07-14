<script lang="ts">
  import CapabilityNotice from '../components/CapabilityNotice.svelte';
  import BrandMark from '../components/BrandMark.svelte';
  import StatusPill from '../components/StatusPill.svelte';
  import Toggle from '../components/Toggle.svelte';
  import { normalizeServerUrl } from '../lib/api';
  import type { LoginSession, NetworkMode, Preferences, QualityProfile, RuntimeStatus, SettingsSection, VideoCodec } from '../lib/types';
  import { NATIVE_PROTOCOL, PROTOCOL_VERSION, SANSER_VERSION } from '../lib/types';
  import { exportDiagnostics } from '../lib/platform';
  import { diagnostics } from '../stores/diagnostics';
  import { SERVER_ENDPOINT_LOCKED, preferences } from '../stores/preferences';
  import { session } from '../stores/session';

  let { runtime }: { runtime: RuntimeStatus } = $props();
  let active = $state<SettingsSection>('general');
  let serverUrl = $state('');
  let saveMessage = $state<string | null>(null);
  let loginSessions = $state<LoginSession[]>([]);
  let accountBusy = $state(false);
  let securityBusy = $state(false);
  let currentPassword = $state('');
  let newPassword = $state('');
  let confirmPassword = $state('');

  const sections: { id: SettingsSection; label: string }[] = [
    { id: 'general', label: 'General' }, { id: 'stream', label: 'Stream' }, { id: 'video', label: 'Video' },
    { id: 'audio', label: 'Audio' }, { id: 'input', label: 'Input' }, { id: 'gamepad', label: 'Gamepad' },
    { id: 'network', label: 'Network' }, { id: 'host', label: 'Host' }, { id: 'security', label: 'Security' },
    { id: 'diagnostics', label: 'Diagnostics' }, { id: 'account', label: 'Account' }, { id: 'about', label: 'About' }
  ];

  $effect(() => {
    serverUrl = $preferences.serverUrl;
  });

  async function saveRoot(update: Partial<Preferences>): Promise<void> {
    await preferences.save({ ...$preferences, ...update });
    saveMessage = 'Saved';
    window.setTimeout(() => (saveMessage = null), 1_500);
  }

  async function saveServer(): Promise<void> {
    if (SERVER_ENDPOINT_LOCKED) return;
    try {
      const normalized = normalizeServerUrl(serverUrl);
      serverUrl = normalized;
      await saveRoot({ serverUrl: normalized });
    } catch (error) {
      saveMessage = error instanceof Error ? error.message : 'Invalid server URL';
    }
  }

  async function saveStream(update: Partial<typeof $preferences.stream>): Promise<void> {
    await saveRoot({ stream: { ...$preferences.stream, ...update } });
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
      await session.logout();
    } catch (error) {
      saveMessage = error instanceof Error ? error.message : 'Unable to change password';
    } finally {
      securityBusy = false;
    }
  }

  async function exportEvents(): Promise<void> {
    const result = await exportDiagnostics(diagnostics.exportJson());
    saveMessage = `Exported: ${result.path}`;
  }

  async function setDiagnosticsEnabled(value: boolean): Promise<void> {
    diagnostics.setEnabled(value);
    await saveRoot({ diagnosticsEnabled: value });
  }

  function chooseSection(section: SettingsSection): void {
    active = section;
    saveMessage = null;
    if (section === 'account') void loadLoginSessions();
  }
</script>

<section class="page settings-page">
  <header class="page-header"><div><h1>Settings</h1><p>Sanser stores non-secret preferences locally and credentials in OS secure storage.</p></div>{#if saveMessage}<span class="save-message">{saveMessage}</span>{/if}</header>
  <div class="settings-layout">
    <nav class="settings-nav" aria-label="Settings sections">
      {#each sections as section}<button class:active={active === section.id} onclick={() => chooseSection(section.id)}>{section.label}</button>{/each}
    </nav>
    <div class="settings-content">
      {#if active === 'general'}
        <article class="card card-body settings-card stack">
          <div><h2 class="card-title">General</h2><p class="card-subtitle">Core application behavior.</p></div>
          {#if SERVER_ENDPOINT_LOCKED}
            <div class="notice"><strong>Sanser service is configured.</strong> The release endpoint is built into the application and cannot be changed locally. Database credentials are never stored in the desktop app.</div>
          {:else}
            <div class="field"><label for="general-server">Development API URL</label><div class="inline-field"><input id="general-server" class="input" type="url" bind:value={serverUrl} spellcheck="false" /><button class="button" onclick={saveServer}>Save</button></div><small>This override is available only in an unconfigured developer build.</small></div>
          {/if}
          <div class="field"><label for="language">Language</label><select id="language" class="select" disabled><option>English · Vietnamese translation planned</option></select></div>
          <Toggle checked={false} disabled label="Start minimized" description="Planned: desktop lifecycle integration is not enabled yet." onchange={() => undefined} />
        </article>
      {:else if active === 'stream'}
        <article class="card card-body settings-card stack">
          <div><h2 class="card-title">Stream</h2><p class="card-subtitle">Profiles are sent with each connection request.</p></div>
          <div class="profile-grid">
            {#each [
              ['auto', 'Auto', 'Uses conservative defaults; live adaptation requires the verified native transport.'],
              ['competitive', 'Competitive', 'Shortest queues and lowest input latency.'],
              ['balanced', 'Balanced', '1080p60 with a moderate bitrate.'],
              ['quality', 'Quality', 'Higher resolution and HEVC when available.'],
              ['custom', 'Custom', 'Use the values configured in Video.']
            ] as profile}
              <button class="profile-option" class:active={$preferences.stream.profile === profile[0]} onclick={() => saveStream({ profile: profile[0] as QualityProfile })}><strong>{profile[1]}</strong><span>{profile[2]}</span></button>
            {/each}
          </div>
          <div class="notice">Realtime policy target: newest input → newest frame → continuous audio. End-to-end adaptive feedback remains capability-gated.</div>
        </article>
      {:else if active === 'video'}
        <article class="card card-body settings-card stack">
          <div><h2 class="card-title">Video</h2><p class="card-subtitle">Validated values are passed to the native encoder/client process.</p></div>
          <div class="grid two">
            <div class="field"><label for="codec">Codec</label><select id="codec" class="select" value={$preferences.stream.codec} onchange={(event) => saveStream({ codec: (event.currentTarget as HTMLSelectElement).value as VideoCodec })}><option value="auto">Auto</option><option value="h264">H.264 · compatibility</option><option value="hevc">HEVC · smaller bitrate</option></select></div>
            <div class="field"><label for="resolution">Resolution</label><select id="resolution" class="select" value={$preferences.stream.resolution} onchange={(event) => saveStream({ resolution: (event.currentTarget as HTMLSelectElement).value as typeof $preferences.stream.resolution })}><option value="auto">Auto</option><option value="720p">1280 × 720</option><option value="1080p">1920 × 1080</option><option value="1440p">2560 × 1440</option><option value="2160p">3840 × 2160</option></select></div>
            <div class="field"><label for="fps">Frame rate</label><select id="fps" class="select" value={$preferences.stream.fps} onchange={(event) => saveStream({ fps: Number((event.currentTarget as HTMLSelectElement).value) as typeof $preferences.stream.fps })}><option value="30">30 FPS</option><option value="60">60 FPS</option><option value="90">90 FPS</option><option value="120">120 FPS</option></select></div>
            <div class="field"><label for="bitrate">Bitrate · {$preferences.stream.bitrateMbps} Mb/s</label><input id="bitrate" type="range" min="1" max="100" step="1" value={$preferences.stream.bitrateMbps} onchange={(event) => saveStream({ bitrateMbps: Number((event.currentTarget as HTMLInputElement).value) })} /></div>
          </div>
          <div class="notice">H.264 is the compatibility fallback. HEVC reduces network usage when both hardware endpoints support it. Auto currently selects H.264 until negotiated keyframe-boundary switching is verified.</div>
        </article>
      {:else if active === 'audio'}
        <article class="card card-body settings-card stack">
          <div><h2 class="card-title">Audio</h2><p class="card-subtitle">Native LAN audio is isolated from video and input queues.</p></div>
          <Toggle checked={$preferences.host.audioEnabled} disabled={runtime.capabilities.clientEngine.state !== 'available' && runtime.capabilities.hostEngine.state !== 'available'} label="Session audio" description="Capture and play audio for new native sessions." onchange={(value) => saveHost({ audioEnabled: value })} />
          <div class="field"><label for="audio-output">Output device</label><select id="audio-output" class="select" disabled><option>System default · enumeration planned</option></select></div>
          <Toggle checked={false} disabled label="Microphone forwarding" description="Planned and permission-gated; no microphone permission is requested now." onchange={() => undefined} />
        </article>
      {:else if active === 'input'}
        <article class="card card-body settings-card stack">
          <div><h2 class="card-title">Input</h2><p class="card-subtitle">Realtime input stays in Rust/native code, outside the frontend data path.</p></div>
          <div class="field"><label for="mouse-mode">Mouse capture</label><select id="mouse-mode" class="select" value={$preferences.input.mouseMode} onchange={(event) => saveInput({ mouseMode: (event.currentTarget as HTMLSelectElement).value as typeof $preferences.input.mouseMode })}><option value="auto">Auto</option><option value="absolute">Absolute · desktop</option><option value="relative">Relative · games</option></select></div>
          <div class="field"><label for="release-shortcut">Release shortcut</label><input id="release-shortcut" class="input" value={$preferences.input.releaseShortcut} maxlength="80" onchange={(event) => saveInput({ releaseShortcut: (event.currentTarget as HTMLInputElement).value })} /></div>
          <div class="field"><label for="polling-rate">Polling rate</label><select id="polling-rate" class="select" disabled><option>{$preferences.input.pollingRate} Hz · native negotiation planned</option></select></div>
          <div class="notice">Keyboard and mouse buttons are reliable and ordered. Mouse movement is latest-state-wins and never grows an unbounded queue.</div>
        </article>
      {:else if active === 'gamepad'}
        <article class="card card-body settings-card stack"><div><h2 class="card-title">Gamepad</h2><p class="card-subtitle">Controller forwarding is not exposed until native state-delta transport is installed.</p></div><CapabilityNotice title="Native gamepad" capability={runtime.capabilities.gamepad} /><Toggle checked={false} disabled label="Forward controller" description={runtime.capabilities.gamepad.reason ?? 'Planned'} onchange={() => undefined} /><div class="field"><label for="gamepad-deadzone">Deadzone</label><input id="gamepad-deadzone" type="range" min="0" max="30" value="8" disabled /></div></article>
      {:else if active === 'network'}
        <article class="card card-body settings-card stack">
          <div><h2 class="card-title">Network</h2><p class="card-subtitle">No Tailscale or external VPN is installed or required.</p></div>
          <div class="network-options">
            {#each [['auto', 'Auto', 'Try direct UDP first, then encrypted WSS relay automatically.'], ['direct', 'Direct', 'Use native UDP only; never relay media.'], ['relay', 'Relay', 'Use the end-to-end encrypted Sanser relay over WSS/443.']] as mode}
              <button class="network-option" class:active={$preferences.networkMode === mode[0]} onclick={() => saveRoot({ networkMode: mode[0] as NetworkMode })}><strong>{mode[1]}</strong><span>{mode[2]}</span></button>
            {/each}
          </div>
          <CapabilityNotice title="Native direct" capability={runtime.capabilities.nativeDirect} />
          <CapabilityNotice title="SNV2" capability={runtime.capabilities.nativeSnv2} />
          <div class="notice">Relay packets are encrypted at the desktop endpoints with the ephemeral session credential. The relay forwarding path handles ciphertext only and does not parse video, audio, keyboard or mouse data.</div>
        </article>
      {:else if active === 'host'}
        <article class="card card-body settings-card stack">
          <div><h2 class="card-title">Host</h2><p class="card-subtitle">Defaults for authenticated Windows host sessions.</p></div>
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
            Manual setup: reserve the Windows PC's LAN address, forward external UDP
            <strong>{$preferences.host.directUdpPort}</strong> to the same internal UDP port, and allow Sanser through Windows Firewall.
            Do not forward TCP and do not expose the PostgreSQL/Neon connection.
          </div>
          <Toggle checked={$preferences.host.autoOnline} disabled={runtime.capabilities.hostEngine.state !== 'available'} label="Bring host online automatically" description="Advertise this Windows host after Sanser starts and restores your signed-in account." onchange={(value) => saveHost({ autoOnline: value })} />
          <Toggle checked={$preferences.host.autoAcceptOwnDevices} disabled={runtime.capabilities.hostEngine.state !== 'available'} label="Auto accept trusted devices" description={`Accept only requester identities trusted from Computers on this Windows host (${$preferences.trustedDeviceIds.length}).`} onchange={(value) => saveHost({ autoAcceptOwnDevices: value })} />
          <Toggle checked={$preferences.host.audioEnabled} disabled={runtime.capabilities.hostEngine.state !== 'available'} label="System audio" description="Enable WASAPI loopback on new sessions." onchange={(value) => saveHost({ audioEnabled: value })} />
          <Toggle checked={$preferences.host.inputEnabled} disabled={runtime.capabilities.hostEngine.state !== 'available'} label="Remote input" description="Open the authenticated control backchannel." onchange={(value) => saveHost({ inputEnabled: value })} />
          <Toggle checked={false} disabled label="Lock after disconnect" description="Planned: OS session lifecycle integration is not available." onchange={() => undefined} />
        </article>
      {:else if active === 'security'}
        <article class="card card-body settings-card stack"><div><h2 class="card-title">Security</h2><p class="card-subtitle">Capabilities remain narrow and native arguments are allowlisted.</p></div><CapabilityNotice title="OS secure storage" capability={runtime.capabilities.secureStorage} /><div class="security-list"><div><strong>Content Security Policy</strong><span>Local scripts only; remote content and frames blocked.</span></div><div><strong>Sidecars</strong><span>Only the native host and client engines for this platform.</span></div><div><strong>Session authentication</strong><span>Session-scoped proof; Relay never falls back to an unauthenticated native route.</span></div></div><form class="password-form" onsubmit={changePassword}><h3>Change password</h3><p>All signed-in sessions are revoked after this change.</p><div class="field"><label for="current-password">Current password</label><input id="current-password" class="input" type="password" bind:value={currentPassword} required minlength="12" maxlength="256" autocomplete="current-password" /></div><div class="grid two"><div class="field"><label for="new-password">New password</label><input id="new-password" class="input" type="password" bind:value={newPassword} required minlength="12" maxlength="256" autocomplete="new-password" /></div><div class="field"><label for="confirm-password">Confirm new password</label><input id="confirm-password" class="input" type="password" bind:value={confirmPassword} required minlength="12" maxlength="256" autocomplete="new-password" /></div></div><button class="button" type="submit" disabled={securityBusy}>{securityBusy ? 'Updating…' : 'Update password'}</button></form></article>
      {:else if active === 'diagnostics'}
        <article class="card card-body settings-card stack"><div><h2 class="card-title">Diagnostics</h2><p class="card-subtitle">A bounded in-memory buffer keeps at most 300 sanitized events.</p></div><Toggle checked={$preferences.diagnosticsEnabled} label="Collect basic diagnostics" description="Record connection lifecycle events without raw packets, input movement or secrets." onchange={setDiagnosticsEnabled} /><div class="button-row"><button class="button" onclick={exportEvents}>Export sanitized JSON</button><button class="button danger" onclick={() => diagnostics.clear()}>Clear events</button></div></article>
      {:else if active === 'account'}
        <article class="card card-body settings-card stack"><div><h2 class="card-title">Account</h2><p class="card-subtitle">{$session.account?.email}</p></div><div class="button-row"><button class="button" onclick={loadLoginSessions} disabled={accountBusy}>Refresh sessions</button><button class="button danger" onclick={() => session.logout()}>Sign out</button></div>{#if loginSessions.length > 0}<div class="login-session-list">{#each loginSessions as item (item.id)}<div><div><strong>{item.deviceName}</strong><span>{item.platform} · last seen {item.lastSeenAt}</span></div><StatusPill state={item.current ? 'online' : 'neutral'} label={item.current ? 'Current' : 'Active'} />{#if !item.current}<button class="button small danger" disabled={accountBusy} onclick={() => revokeLoginSession(item.id)}>Revoke</button>{/if}</div>{/each}</div>{:else}<div class="notice">No login-session list has been loaded yet. Select “Refresh sessions” to retrieve it from the server.</div>{/if}</article>
      {:else}
        <article class="card card-body settings-card about-settings"><BrandMark size={76} label="Sanser" /><div><h2>Sanser {SANSER_VERSION}</h2><p>Protocol v{PROTOCOL_VERSION}<br />{NATIVE_PROTOCOL}</p></div></article>
      {/if}
    </div>
  </div>
</section>
