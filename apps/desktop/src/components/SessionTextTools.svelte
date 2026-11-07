<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from './Icon.svelte';
  import { sessionUserCapabilities, sessionUserAction, type SessionUserCapabilities } from '../lib/platform';
  import { validUserText } from '../lib/userText';
  let { sessionId }: { sessionId: string } = $props();
  let capabilities = $state<SessionUserCapabilities>({ unicodeInput: false, clipboard: false, localClipboard: false });
  let value = $state('');
  let busy = $state(false);
  let composing = $state(false);
  let notice = $state('');
  let error = $state('');
  let alive = false;
  let identity = '';
  let revision = 0;
  $effect(() => {
    if (identity !== sessionId) {
      identity = sessionId; revision++; capabilities = { unicodeInput: false, clipboard: false, localClipboard: false };
      value = ''; busy = false; composing = false; notice = ''; error = '';
    }
  });
  async function refresh() {
    const id = sessionId;
    try { const next = await sessionUserCapabilities(id); if (alive && id === sessionId) capabilities = next; }
    catch { if (alive && id === sessionId) capabilities = { unicodeInput: false, clipboard: false, localClipboard: false }; }
  }
  onMount(() => {
    alive = true; void refresh(); const timer = setInterval(() => void refresh(), 1000);
    return () => { alive = false; revision++; clearInterval(timer); };
  });
  async function act(operation: 'text' | 'clipboard-read' | 'clipboard-write' | 'copy-local' | 'paste-local') {
    if (busy || composing) return;
    if (operation === 'text' && (!capabilities.unicodeInput || !validUserText(value))) return;
    if ((operation === 'clipboard-read' || operation === 'clipboard-write') && !capabilities.clipboard) return;
    if (operation === 'clipboard-write' && (!value || !validUserText(value, true))) return;
    if ((operation === 'copy-local' || operation === 'paste-local') && !capabilities.localClipboard) return;
    const id = sessionId, attempt = ++revision, snapshot = value;
    busy = true; notice = ''; error = '';
    const current = () => alive && id === sessionId && attempt === revision;
    try {
      if (operation === 'copy-local') {
        await sessionUserAction(id,'clipboard-local-write',snapshot);
        if (current()) notice = 'Copied on this computer.';
      } else if (operation === 'paste-local') {
        const text = await sessionUserAction(id,'clipboard-local-read');
        if (!validUserText(text, true)) throw new Error('Clipboard text must be at most 8 KB.');
        if (current()) value = text;
      } else {
        const text = await sessionUserAction(id, operation, operation === 'clipboard-read' ? '' : snapshot);
        if (current()) {
          if (operation === 'clipboard-read') { value = text; notice = 'Received. Review the text, then copy it locally if needed.'; }
          else notice = operation === 'text' ? 'Text queued for the selected field on the host.' : 'Host clipboard updated.';
        }
      }
    } catch (failure) { if (current()) error = failure instanceof Error ? failure.message : 'Unable to complete this text operation.'; }
    finally { if (current()) busy = false; }
  }
</script>

<details class="card text-tools">
  <summary><Icon name="clipboard" /><span>Text and clipboard</span><span class="text-tools-status">{busy ? 'Working…' : composing ? 'Composing text…' : capabilities.unicodeInput || capabilities.clipboard ? 'Available' : 'Waiting for host support'}</span></summary>
  <div class="card-body stack">
    <label class="field" for="session-text">Text to send or receive
      <textarea id="session-text" class="input" rows="4" bind:value autocomplete="off" spellcheck="false" disabled={busy}
        oncompositionstart={() => { composing = true; }}
        oncompositionend={(event) => { value = event.currentTarget.value; composing = false; }}
        placeholder="Type here, or read text from the host clipboard"></textarea>
    </label>
    <div class="button-row">
      <button class="button small primary" disabled={busy || composing || !capabilities.unicodeInput || !validUserText(value)} onclick={() => act('text')}><Icon name="arrow" />Send text</button>
      <button class="button small" disabled={busy || composing || !capabilities.clipboard} onclick={() => act('clipboard-read')}>Read host clipboard</button>
      <button class="button small" disabled={busy || composing || !value || !capabilities.clipboard || !validUserText(value, true)} onclick={() => act('clipboard-write')}>Write host clipboard</button>
      <button class="button small" disabled={busy || composing || !capabilities.localClipboard} onclick={() => act('paste-local')}>Paste locally</button>
      <button class="button small" disabled={busy || composing || !value || !capabilities.localClipboard} onclick={() => act('copy-local')}>Copy locally</button>
    </div>
    <p class="muted">Finish choosing characters in your local input method before sending. Send text: up to 256 UTF-16 units (emoji may use more than one). Clipboard: up to 8 KB. Clipboard requests require the host's Text clipboard setting.</p>
    {#if notice}<p role="status">{notice}</p>{/if}
    {#if error}<p class="notice error" role="alert">{error}</p>{/if}
  </div>
</details>
