<script lang="ts">
  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import { t } from '../lib/i18n';
  import { host } from '../stores/host';
  import { sharing } from '../stores/sharing';
  import { preferences } from '../stores/preferences';
  import { session } from '../stores/session';
  import type { ConnectionSession, SharingMember } from '../lib/types';
  import { pollHostRecordings, revokeHostRecording, type HostRecordingNotice } from '../stores/recording';

  let notices = $state<HostRecordingNotice[]>([]);
  let inFlight = false;

  function sessionIds(): string[] {
    const direct: string[] = (get(host).sessions as ConnectionSession[]).filter((item) => item.status === 'accepted').map((item) => item.id);
    const members: SharingMember[] = $sharing.ownGroup?.members ?? [];
    const shared: string[] = members
      .filter((member) => member.state === 'accepted' && member.sessionId)
      .map((member) => member.sessionId as string);
    return [...new Set([...direct, ...shared])];
  }

  async function poll(): Promise<void> {
    const client = session.client();
    const state = get(host);
    if (inFlight || !client || !state.online || !state.deviceId) { if (!state.online) notices = []; return; }
    const ids = sessionIds();
    if (!ids.length) { notices = []; return; }
    inFlight = true;
    try { notices = await pollHostRecordings(client, state.deviceId, ids, $preferences.host.allowRecording); }
    finally { inFlight = false; }
  }

  async function revoke(sessionId: string): Promise<void> {
    const client = session.client();
    const hostDeviceId = get(host).deviceId;
    if (!client || !hostDeviceId) return;
    await revokeHostRecording(client, hostDeviceId, sessionId).catch(() => undefined);
    notices = notices.filter((notice) => notice.sessionId !== sessionId);
  }

  onMount(() => {
    void poll();
    const timer = window.setInterval(() => { void poll(); }, 2_000);
    return () => window.clearInterval(timer);
  });
</script>

{#each notices as notice (notice.sessionId)}
  <div class="notice warning host-recording" role="status">
    <span class="host-recording-dot" aria-hidden="true"></span>
    <span>{$t('{name} is recording your screen on their computer.', { name: notice.viewerName })}</span>
    <button class="button small" onclick={() => revoke(notice.sessionId)}>{$t('Stop their recording')}</button>
  </div>
{/each}

<style>
  .host-recording { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
  .host-recording-dot { width: 10px; height: 10px; border-radius: 50%; background: var(--danger, #d92d20); }
</style>
