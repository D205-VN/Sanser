<script lang="ts">
  import { onMount } from 'svelte';
  import { sessionFileStatus, type FileTransfer } from '../lib/platform';
  import type { Page } from '../lib/types';
  let { hostSession, clientSession, navigate }: { hostSession: string | null; clientSession: string | null; navigate: (page: Page) => void } = $props();
  let invitations = $state<{ role: 'host' | 'client'; session: string; transfer: FileTransfer }[]>([]);
  let alive = false;
  let pending = false;
  async function refresh() {
    if (pending) return;
    pending = true;
    const candidates = [{ role: 'host' as const, session: hostSession }, { role: 'client' as const, session: clientSession }];
    try {
      const results = await Promise.all(candidates.map(async ({ role, session }) => {
        if (!session) return null;
        try {
          const status = await sessionFileStatus(role, session);
          return status.available && status.incoming?.state === 'offered' ? { role, session, transfer: status.incoming } : null;
        } catch { return null; }
      }));
      if (alive) invitations = results.filter((item): item is NonNullable<typeof item> => item !== null).filter((item) => item.session === (item.role === 'host' ? hostSession : clientSession));
    } finally {pending = false;}
  }
  onMount(() => {alive = true; void refresh(); const timer = setInterval(() => void refresh(), 1000); return () => {alive = false; clearInterval(timer);};});
  $effect(() => {void hostSession; void clientSession; if (alive) void refresh();});
</script>

{#each invitations as invitation (`${invitation.role}:${invitation.session}:${invitation.transfer.id}`)}
  {#if invitation.session === (invitation.role === 'host' ? hostSession : clientSession)}
    <aside class="notice file-invitation" role="status">
      <div><strong>Incoming file: {invitation.transfer.name}</strong><p>Review the file transfer in {invitation.role === 'host' ? 'Host' : 'Active session'} to accept or decline. Nothing is saved before you accept.</p></div>
      <button class="button small primary" onclick={() => navigate(invitation.role === 'host' ? 'host' : 'session')}>Review file</button>
    </aside>
  {/if}
{/each}

<style>
  .file-invitation {display: flex; align-items: center; justify-content: space-between; gap: 16px; margin-bottom: 16px;}
  strong {overflow-wrap: anywhere;}
  p {margin: 6px 0 0;}
  button {flex-shrink: 0;}
  @media (max-width: 680px) {.file-invitation {align-items: stretch; flex-direction: column;}}
</style>
