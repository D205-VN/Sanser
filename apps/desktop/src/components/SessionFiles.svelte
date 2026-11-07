<script lang="ts">
  import { onMount } from 'svelte';
  import { sessionFileStatus, sessionFileAction, type FileTransferAction, type FileTransferStatus } from '../lib/platform';
  import type { EngineKind } from '../lib/types';
  let { sessionId, kind = 'client' }: { sessionId: string; kind?: EngineKind } = $props();
  let status = $state<FileTransferStatus>({ available: false, incoming: null, outgoing: null });
  let selected = $state<File | null>(null);
  let busy = $state(false);
  let error = $state('');
  let alive = false;
  let identity = '';
  let revision = 0;
  let uploadId = '';
  let pumping = false;
  let owner: { id: string; role: EngineKind; session: string } | null = null;
  const active = (state?: string) => !!state && !['completed', 'cancelled'].includes(state);
  const size = (bytes: number) => bytes < 1024 ? `${bytes} B` : bytes < 1048576 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1048576).toFixed(1)} MB`;
  function abandon(id: string, oldKind: EngineKind, oldSession: string) {
    if (id) void sessionFileAction(oldKind, oldSession, { operation: 'cancel', id }).catch(() => {});
  }
  $effect(() => {
    const next = `${kind}:${sessionId}`;
    if (identity !== next) {
      if (owner) abandon(owner.id, owner.role, owner.session);
      owner = null;
      identity = next; revision++; selected = null; uploadId = ''; busy = false; error = '';
      status = { available: false, incoming: null, outgoing: null };
    }
  });
  async function refresh() {
    const id = sessionId, role = kind, version = revision;
    try {
      const next = await sessionFileStatus(role, id);
      if (alive && id === sessionId && role === kind && version === revision) status = next;
    } catch {
      if (alive && id === sessionId && role === kind && version === revision) status = { ...status, available: false };
    }
  }
  onMount(() => {
    alive = true; void refresh(); const timer = setInterval(() => void refresh(), 500);
    return () => { alive = false; revision++; clearInterval(timer); if (owner) abandon(owner.id, owner.role, owner.session); };
  });
  async function action(command: FileTransferAction) {
    if (busy) return;
    if (command.operation === 'cancel' && command.id === uploadId) {uploadId = ''; owner = null;}
    const id = sessionId, role = kind, version = ++revision;
    busy = true; error = '';
    try {
      const next = await sessionFileAction(role, id, command);
      if (alive && id === sessionId && role === kind && version === revision) {
        status = next;
        if (command.operation === 'offer') {
          uploadId = next.outgoing?.id ?? '';
          owner = uploadId ? { id: uploadId, role, session: id } : null;
        }
      } else if (command.operation === 'offer' && next.outgoing) {
        abandon(next.outgoing.id, role, id);
      }
    } catch (failure) {
      if (alive && id === sessionId && role === kind && version === revision) error = failure instanceof Error ? failure.message : 'Unable to complete this file operation.';
    } finally { if (alive && id === sessionId && role === kind && version === revision) busy = false; }
  }
  // Read at most 4 KiB at a time. Wait for the receiver's disk-write ACK before
  // reading/sending another chunk. Never load a whole file into WebView memory.
  $effect(() => {
    const outgoing = status.outgoing;
    if (alive && !pumping && !busy && selected && outgoing?.id === uploadId && outgoing.state === 'sending') {
      void pump(selected, outgoing.id, sessionId, kind);
    }
  });
  async function pump(file: File, transferId: string, session: string, role: EngineKind) {
    pumping = true;
    const current = () => alive && session === sessionId && role === kind && uploadId === transferId;
    try {
      let next = await sessionFileStatus(role, session);
      let submitted = -1;
      while (current() && next.available && next.outgoing?.id === transferId && next.outgoing.state === 'sending') {
        const offset = next.outgoing.bytes;
        if (offset !== submitted) {
          submitted = offset;
          const bytes = offset < file.size ? Array.from(new Uint8Array(await file.slice(offset, offset + 4096).arrayBuffer())) : null;
          if (!current()) break;
          next = await sessionFileAction(role, session, bytes ? { operation: 'chunk', id: transferId, offset, bytes } : { operation: 'finish', id: transferId });
        }
        if (current()) status = next;
        if (next.outgoing?.state !== 'sending') break;
        await new Promise<void>((resolve) => setTimeout(resolve, 120));
        if (!current()) break;
        next = await sessionFileStatus(role, session);
      }
      if (current()) status = next;
    } catch (failure) {
      if (current()) {error = failure instanceof Error ? failure.message : 'Upload stopped.';uploadId = '';owner = null;abandon(transferId, role, session);}
    } finally {pumping = false;}
  }
  function choose(event: Event) {
    const file = (event.currentTarget as HTMLInputElement).files?.[0] ?? null;
    selected = file; error = file && file.size > 25 * 1048576 ? 'Choose a file no larger than 25 MB.' : '';
  }
</script>

<details class="card text-tools session-files" open={status.incoming?.state === 'offered'}>
  <summary><span>File transfer</span><span class="text-tools-status">{status.available ? 'Available' : 'Waiting for file support'}</span></summary>
  <div class="card-body stack">
    <p class="muted">SNV2 control sessions with updated desktop engines support files up to 25 MB. The other person must accept. Received files are saved in Downloads/Sanser with a unique name after SHA-256 verification.</p>
    <label class="field" for={`session-file-${kind}`}>Choose a file
      <input id={`session-file-${kind}`} type="file" disabled={!status.available || busy || active(status.outgoing?.state)} onchange={choose} />
    </label>
    <div class="button-row"><button class="button small primary" disabled={!status.available || busy || !selected || selected.size > 25 * 1048576 || active(status.outgoing?.state)} onclick={() => selected && action({ operation: 'offer', name: selected.name, size: selected.size })}>Offer file</button></div>
    {#each [{ label: 'Incoming', transfer: status.incoming }, { label: 'Outgoing', transfer: status.outgoing }] as row}
      {#if row.transfer}
        <div class="file-transfer-item stack">
          <div><strong>{row.label}: {row.transfer.name}</strong><p class="muted">{row.transfer.state} · {size(row.transfer.bytes)} / {size(row.transfer.size)}</p></div>
          {#if row.transfer.state !== 'offered'}<progress max={row.transfer.size || 1} value={row.transfer.state === 'completed' ? row.transfer.size || 1 : row.transfer.bytes} aria-label={`${row.label} file progress`}></progress>{/if}
          {#if row.transfer.savedPath}<p class="file-save-path">Saved: {row.transfer.savedPath}</p>{/if}
          {#if row.transfer.error}<p role="status">{row.transfer.error}</p>{/if}
          <div class="button-row">
            {#if row.label === 'Incoming' && row.transfer.state === 'offered'}<button class="button small primary" disabled={!status.available || busy} onclick={() => row.transfer && action({ operation: 'accept', id: row.transfer.id })}>Accept and save</button>{/if}
            {#if active(row.transfer.state)}<button class="button small danger" disabled={busy} onclick={() => row.transfer && action({ operation: 'cancel', id: row.transfer.id })}>{row.label === 'Incoming' && row.transfer.state === 'offered' ? 'Decline' : 'Cancel transfer'}</button>{/if}
          </div>
        </div>
      {/if}
    {/each}
    <p class="muted">Files use a small background bandwidth budget. Leaving this panel cancels its upload. Files are never opened automatically.</p>
    {#if error}<p class="notice error" role="alert">{error}</p>{/if}
  </div>
</details>
