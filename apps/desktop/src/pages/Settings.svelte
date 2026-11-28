<script lang="ts">
  import { t, tk } from '../lib/i18n';
  import { onDestroy } from 'svelte';
  import LoginSessions from '../components/LoginSessions.svelte';
  import LanguageToggle from '../components/LanguageToggle.svelte';
  import Toggle from '../components/Toggle.svelte';
  import HostPrivacySettings from '../components/HostPrivacySettings.svelte';
  import BlockedApps from '../components/BlockedApps.svelte';
  import ReportProblem from '../components/ReportProblem.svelte';
  import StreamAdvisor from '../components/StreamAdvisor.svelte';
  import { normalizeServerUrl } from '../lib/api';
  import type { LoginSession, NetworkMode, Preferences, QualityProfile, RuntimeStatus, SettingsSection, VideoCodec } from '../lib/types';
  import { exportDiagnostics, loginItemStatus, setLoginItem, type LoginItemStatus } from '../lib/platform';
  import { syncWakeOnLan } from '../lib/wakeOnLan';
  import { host as hostStore } from '../stores/host';
  import { diagnostics } from '../stores/diagnostics';
  import { CONFIGURED_SERVER_URL, preferences } from '../stores/preferences';
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
  let loginItem = $state<LoginItemStatus>({ available: false, enabled: false });
  let loginItemError = $state<string | null>(null);
  void loginItemStatus().then((status) => { loginItem = status; }).catch(() => undefined);

  async function changeLoginItem(enabled: boolean): Promise<void> {
    loginItemError = null;
    try { loginItem = await setLoginItem(enabled); }
    catch (error) { loginItemError = error instanceof Error ? error.message : String(error); }
  }
  let accountBusy = $state(false);
  let securityBusy = $state(false);
  let currentPassword = $state('');
  let newPassword = $state('');
  let confirmPassword = $state('');
  let wakeBusy = $state(false);
  let wakeMessage = $state<{ text: string; error: boolean } | null>(null);

  const sections: { id: SettingsSection; label: string }[] = [
    { id: 'stream', label: tk('Quality') }, { id: 'video', label: tk('Video') },
    { id: 'audio', label: tk('Audio') }, { id: 'input', label: tk('Input') },
    { id: 'network', label: tk('Network') }, { id: 'host', label: tk('Host') }, { id: 'security', label: tk('Security') },
    { id: 'diagnostics', label: tk('Diagnostics') }, { id: 'account', label: tk('Account') }
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
      saveMessage = $t('Changes saved');
      saveTimer = window.setTimeout(() => (saveMessage = null), 3_000);
    } catch (error) {
      saveError = true;
      saveMessage = error instanceof Error ? error.message : $t('Unable to save changes. Please try again.');
    } finally { saving = false; }
  }

  async function saveServer(): Promise<void> {
    try {
      const normalized = normalizeServerUrl(serverUrl);
      serverUrl = normalized;
      await saveRoot({ serverUrl: normalized, customServer: normalized !== CONFIGURED_SERVER_URL });
      if (!saveError) {
        await onSignOut();
        await session.initialize(normalized);
      }
    } catch (error) {
      saveError = true;
      saveMessage = error instanceof Error ? error.message : $t('Invalid server URL');
    }
  }

  async function saveStream(update: Partial<typeof $preferences.stream>): Promise<void> {
    await saveRoot({ stream: { ...$preferences.stream, ...update, profile: update.profile ?? 'custom' } });
  }

  async function saveHost(update: Partial<typeof $preferences.host>): Promise<void> {
    await saveRoot({ host: { ...$preferences.host, ...update } });
  }

  async function setWakeOnLan(enabled: boolean): Promise<void> {
    wakeBusy = true;
    wakeMessage = null;
    try {
      await saveHost({ wakeOnLan: enabled });
      const client = session.client();
      const hostId = $hostStore.deviceId;
      if (!client || !$hostStore.online || !hostId) {
        wakeMessage = { text: enabled ? tk('Wake-on-LAN details will be shared the next time Host is online.') : tk('Saved Wake-on-LAN details will be removed the next time Host is online.'), error: false };
        return;
      }
      const result = await syncWakeOnLan(client, hostId, enabled, !enabled);
      wakeMessage = result === 'no-adapters'
        ? { text: tk('No wired or Wi-Fi adapter with a local network address was found.'), error: true }
        : { text: enabled ? tk('Wake-on-LAN details shared with your account.') : tk('Wake-on-LAN details removed from your account.'), error: false };
    } catch (error) {
      wakeMessage = { text: error instanceof Error ? error.message : tk('Unable to update Wake-on-LAN'), error: true };
    } finally {
      wakeBusy = false;
    }
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
      saveMessage = error instanceof Error ? error.message : $t('Unable to load login sessions');
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
      saveMessage = error instanceof Error ? error.message : $t('Unable to revoke session');
    } finally {
      accountBusy = false;
    }
  }

  async function changePassword(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    saveMessage = null;
    if (newPassword.length < 12) {
      saveMessage = $t('The new password must contain at least 12 characters');
      return;
    }
    if (newPassword !== confirmPassword) {
      saveMessage = $t('The new passwords do not match');
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
      saveMessage = error instanceof Error ? error.message : $t('Unable to change password');
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
      saveMessage = error instanceof Error ? error.message : $t('Unable to export diagnostics');
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
  <header class="page-header"><div><h1>{$t('Settings')}</h1><p>{$t('Changes save automatically.')}</p></div>{#if saving}<span class="save-message" role="status">{$t('Saving…')}</span>{/if}</header>
  {#if saveMessage}<div class="notice" class:error={saveError} role="status">{$t(saveMessage)}</div>{/if}
  <div class="settings-layout">
    <nav class="settings-nav" aria-label={$t('Settings sections')}>
      {#each sections.filter((section) => section.id !== 'audio' || runtime.capabilities.clientAudio === true || runtime.capabilities.hostAudio === true) as section}<button class:active={active === section.id} aria-current={active === section.id ? 'page' : undefined} onclick={() => chooseSection(section.id)}>{$t(section.label)}</button>{/each}
    </nav>
    <div class="settings-content">
      {#if active === 'stream'}
        <article class="card card-body settings-card stack">
          <div><h2 class="card-title">{$t('Quality')}</h2><p class="card-subtitle">{$t('Choose the default quality for new connections.')}</p></div>
          <div class="profile-grid">
            {#each [
              ['auto', $t('Auto'), $t('Up to 1920 × 1200 at 60 FPS · 25 Mb/s.')],
              ['competitive', $t('Ultra Low Latency'), $t('Auto resolution at 60 FPS · 25 Mb/s. Latest frame, adaptive recovery.')],
              ['balanced', $t('Balanced'), $t('1080p at 60 FPS · 20 Mb/s.')],
              ['quality', $t('Quality'), $t('1440p at 60 FPS · 40 Mb/s.')],
              ['custom', $t('Custom'), $t('Use Video settings. The host controls custom capture quality.')]
            ] as profile}
              <button class="profile-option" class:active={$preferences.stream.profile === profile[0]} onclick={() => saveStream({ profile: profile[0] as QualityProfile })}><strong>{profile[1]}</strong><span>{profile[2]}</span></button>
            {/each}
          </div>
        </article>
        <StreamAdvisor {runtime} />
      {:else if active === 'video'}
        <article class="card card-body settings-card stack">
          <div><h2 class="card-title">{$t('Video')}</h2><p class="card-subtitle">{$t('Editing a video option switches to Custom. Presets use the values listed in Stream.')}</p></div>
          <div class="grid two">
            <div class="field"><label for="codec">{$t('Codec')}</label><select id="codec" class="select" value={$preferences.stream.codec} onchange={(event) => saveStream({ codec: (event.currentTarget as HTMLSelectElement).value as VideoCodec })}><option value="auto">{$t('Auto')}</option><option value="h264">{$t('H.264 · compatibility')}</option><option value="hevc">{$t('HEVC · smaller bitrate')}</option></select></div>
            <div class="field"><label for="resolution">{$t('Resolution')}</label><select id="resolution" class="select" value={$preferences.stream.resolution} onchange={(event) => saveStream({ resolution: (event.currentTarget as HTMLSelectElement).value as typeof $preferences.stream.resolution })}><option value="auto">{$t('Auto')}</option><option value="720p">1280 × 720</option><option value="1080p">1920 × 1080</option><option value="1440p">2560 × 1440</option><option value="2160p">3840 × 2160</option></select></div>
            <div class="field"><label for="fps">{$t('Frame rate')}</label><select id="fps" class="select" value={$preferences.stream.fps} onchange={(event) => saveStream({ fps: Number((event.currentTarget as HTMLSelectElement).value) as typeof $preferences.stream.fps })}><option value="30">{$t('30 FPS')}</option><option value="60">{$t('60 FPS')}</option><option value="90">{$t('90 FPS')}</option><option value="120">{$t('120 FPS')}</option></select></div>
            <div class="field"><label for="bitrate">{$t('Bitrate')} · {$preferences.stream.bitrateMbps} Mb/s</label><input id="bitrate" type="range" min="1" max="100" step="1" value={$preferences.stream.bitrateMbps} onchange={(event) => saveStream({ bitrateMbps: Number((event.currentTarget as HTMLInputElement).value) })} /></div>
          </div>
          <div class="notice">{$t('Choose Auto unless you need a specific video format. HEVC requires support on both computers.')}</div>
        </article>
      {:else if active === 'audio'}
        <article class="card card-body settings-card stack">
          <div><h2 class="card-title">{$t('Audio')}</h2><p class="card-subtitle">{$t('Share sound during a connection.')}</p></div>
          <Toggle checked={$preferences.host.audioEnabled && (runtime.capabilities.clientAudio === true || runtime.capabilities.hostAudio === true)} disabled={runtime.capabilities.clientAudio !== true && runtime.capabilities.hostAudio !== true} label={$t('Session audio')} description={$t('Share sound from the remote computer.')} onchange={(value) => saveHost({ audioEnabled: value })} />
        </article>
      {:else if active === 'input'}
        <article class="card card-body settings-card stack">
          <div><h2 class="card-title">{$t('Input')}</h2><p class="card-subtitle">{$t('Choose how the remote mouse behaves.')}</p></div>
          <p class="card-subtitle">{$t('To release control, use the shortcut shown in the remote window.')}</p>
          <div class="field"><label for="mouse-mode">{$t('Mouse capture')}</label><select id="mouse-mode" class="select" value={$preferences.input.mouseMode} onchange={(event) => saveInput({ mouseMode: (event.currentTarget as HTMLSelectElement).value as typeof $preferences.input.mouseMode })}><option value="auto">{$t('Auto')}</option><option value="absolute">{$t('Absolute · desktop')}</option><option value="relative">{$t('Relative · games')}</option></select></div>
        </article>
      {:else if active === 'network'}
        <article class="card card-body settings-card stack">
          <div><h2 class="card-title">{$t('Network')}</h2><p class="card-subtitle">{$t('Choose how your computers connect.')}</p></div>
          <div class="network-options">
            {#each [['auto', $t('Auto'), $t('Try direct first, then use relay if needed.')], ['direct', $t('Direct'), $t('Connect directly. No relay fallback.')], ['relay', $t('Relay'), $t('Connect through the Sanser relay service.')]] as mode}
              <button class="network-option" class:active={$preferences.networkMode === mode[0]} onclick={() => saveRoot({ networkMode: mode[0] as NetworkMode })}><strong>{mode[1]}</strong><span>{mode[2]}</span></button>
            {/each}
          </div>
          <details class="sharing-help"><summary>{$t('Server settings')}</summary><div class="field"><label for="general-server">{$t('Server URL')}</label><div class="inline-field"><input id="general-server" class="input" type="url" bind:value={serverUrl} spellcheck="false" /><button class="button" disabled={saving} onclick={saveServer}>{$t('Save')}</button></div><small>{$t('Use the same server on both computers. Changing the server signs you out.')}</small>{#if CONFIGURED_SERVER_URL}<button class="text-button" disabled={saving} onclick={() => { serverUrl = CONFIGURED_SERVER_URL; void saveServer(); }}>{$t('Use default server')}</button>{/if}</div></details>
        </article>
      {:else if active === 'host'}
        <article class="card card-body settings-card stack">
          <div><h2 class="card-title">{$t('Host')}</h2><p class="card-subtitle">{$t('Choose how this computer shares its screen.')}</p></div>
          {#if runtime.platform.startsWith('windows')}
            <p class="card-subtitle">{$t('Closing the window keeps Sanser running in the system tray. Use the tray icon to reopen Sanser or choose Quit Sanser to stop it.')}</p>
          {/if}
          <details class="sharing-help"><summary>{$t('Advanced network setup')}</summary>
          <div class="field">
            <label for="direct-udp-port">{$t('Direct UDP port')}</label>
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
            <small>{$t('Keep this port fixed so UPnP or a manual router rule remains valid.')}</small>
          </div>
          <div class="notice">
            {$t('Manual setup: reserve the host computer\'s LAN address, forward external UDP')}
            <strong>{$preferences.host.directUdpPort}</strong> {$t('to the same internal UDP port, and allow Sanser through the system firewall.')}
          </div>
          </details>
          {#if loginItem.available}
            <Toggle checked={loginItem.enabled} label={$t('Start Sanser when you sign in')} description={$t('Open Sanser in the background after this computer starts. Combine with automatic host to keep this computer reachable.')} onchange={(value) => { void changeLoginItem(value); }} />
            {#if loginItemError}<p class="card-subtitle danger-text" role="alert">{$t(loginItemError)}</p>{/if}
          {/if}
          <Toggle checked={$preferences.host.autoOnline} disabled={runtime.capabilities.hostEngine.state !== 'available'} label={$t('Bring host online automatically')} description={$t('Allow connection requests when you open Sanser.')} onchange={(value) => saveHost({ autoOnline: value })} />
          <Toggle checked={$preferences.host.autoAcceptOwnDevices} disabled={runtime.capabilities.hostEngine.state !== 'available'} label={$t('Auto accept trusted devices')} description={$preferences.trustedDeviceIds.length > 0 ? $t('{count} trusted computer(s) can connect without asking.', { count: $preferences.trustedDeviceIds.length }) : $t('No trusted computers yet. Choose Accept & trust on a request in Host to remember that computer.')} onchange={(value) => saveHost({ autoAcceptOwnDevices: value })} />
          {#if runtime.capabilities.hostAudio === true}<Toggle checked={$preferences.host.audioEnabled} label={$t('System audio')} description={$t('Share sound from this computer.')} onchange={(value) => saveHost({ audioEnabled: value })} />{/if}
          <Toggle checked={$preferences.host.inputEnabled} disabled={runtime.capabilities.hostEngine.state !== 'available'} label={$t('Remote input')} description={$t('Allow remote keyboard and mouse control.')} onchange={(value) => saveHost({ inputEnabled: value })} />
          <Toggle checked={$preferences.host.clipboardEnabled} disabled={runtime.capabilities.clipboard.state !== 'available' || !$preferences.host.inputEnabled} label={$t('Text clipboard')} description={$t('Allow manual text clipboard requests in control sessions. Reconnect after changing this setting.')} onchange={(value) => saveHost({ clipboardEnabled: value })} />
          <HostPrivacySettings {runtime} />
          <BlockedApps {runtime} />
          <Toggle checked={$preferences.host.wakeOnLan} disabled={runtime.capabilities.hostEngine.state !== 'available' || wakeBusy} label={$t('Allow Wake-on-LAN')} description={$t('Share this computer\'s network adapter (MAC) addresses with your account so another Sanser computer on the same local network can wake it from sleep. Requires Wake-on-LAN to be enabled in BIOS/UEFI and the operating system.')} onchange={(value) => { void setWakeOnLan(value); }} />
          {#if wakeMessage}<p class="card-subtitle" class:danger-text={wakeMessage.error} role="status">{$t(wakeMessage.text)}</p>{/if}
        </article>
      {:else if active === 'security'}
        <article class="card card-body settings-card stack"><form class="password-form" onsubmit={changePassword}><h3>{$t('Change password')}</h3><p>{$t('All signed-in sessions are revoked after this change.')}</p><div class="field"><label for="current-password">{$t('Current password')}</label><input id="current-password" class="input" type="password" bind:value={currentPassword} required minlength="12" maxlength="256" autocomplete="current-password" /></div><div class="grid two"><div class="field"><label for="new-password">{$t('New password')}</label><input id="new-password" class="input" type="password" bind:value={newPassword} required minlength="12" maxlength="256" autocomplete="new-password" /></div><div class="field"><label for="confirm-password">{$t('Confirm new password')}</label><input id="confirm-password" class="input" type="password" bind:value={confirmPassword} required minlength="12" maxlength="256" autocomplete="new-password" /></div></div><button class="button" type="submit" disabled={securityBusy}>{securityBusy ? $t('Updating…') : $t('Update password')}</button></form></article>
      {:else if active === 'diagnostics'}
        <article class="card card-body stack">
          <div><h2 class="card-title">{$t('Diagnostics')}</h2><p class="card-subtitle">{$t('Use these tools when a connection is not working.')}</p></div>
          <Toggle checked={$preferences.diagnosticsEnabled} label={$t('Collect basic diagnostics')} description={$t('Keep a local record of connections and errors.')} onchange={setDiagnosticsEnabled} />
          <div class="button-row">
            <button class="button" onclick={exportEvents}>{$t('Export diagnostics')}</button>
            <button class="button danger" onclick={() => diagnostics.clear()}>{$t('Clear events')}</button>
          </div>
          <ReportProblem {runtime} />
          <button class="button ghost" aria-expanded={showDetails} aria-controls="connection-details" onclick={() => (showDetails = !showDetails)}>{showDetails ? $t('Hide connection details') : $t('Show connection details')}</button>
        </article>
        {#if showDetails}<div id="connection-details"><Diagnostics {runtime} /></div>{/if}
      {:else if active === 'account'}
        <article class="card card-body settings-card language-card" style="display: flex; align-items: center; justify-content: space-between; gap: 16px; flex-wrap: wrap;"><div><h2 class="card-title">{$t('Language')}</h2><p class="card-subtitle">{$t('Choose the language for Sanser on this computer.')}</p></div><LanguageToggle /></article>
        <article class="card card-body settings-card stack"><div><h2 class="card-title">{$t('Account')}</h2><p class="card-subtitle">{$session.account?.email}</p></div><div class="button-row"><button class="button" onclick={loadLoginSessions} disabled={accountBusy}>{$t('Refresh sessions')}</button><button class="button danger" onclick={onSignOut}>{$t('Sign out')}</button></div>{#if loginSessions.length > 0}<LoginSessions sessions={loginSessions} busy={accountBusy} onRevoke={revokeLoginSession} />{:else}<div class="notice">{$t('No login-session list has been loaded yet. Select “Refresh sessions” to retrieve it from the server.')}</div>{/if}</article>
      {/if}
    </div>
  </div>
</section>
