<script lang="ts">
  import { locale, t } from '../lib/i18n';
  import { onMount, untrack } from 'svelte';
  import Icon from './Icon.svelte';
  import { createCollab, type ChatMessage } from '../stores/collab';
  import { closeAnnotationOverlay, collabPost, collabSnapshot, openAnnotationOverlay, PEN_COLORS, setAnnotationPen, setGroupAnnotations, utf8Bytes, CHAT_BYTES, validChat, type CollabKind } from '../lib/collab';
  import { showChatAlert } from '../lib/connectionAlerts';
  import { isTauriRuntime } from '../lib/platform';
  import { diagnostics } from '../stores/diagnostics';

  let { kind, sessionId, selfName = '', names = {} }: { kind: CollabKind; sessionId?: string; selfName?: string; names?: Record<string, string> } = $props();

  let alive = false;
  const collab = createCollab({
    snapshot: collabSnapshot,
    post: collabPost,
    incoming: (message) => {
      if (typeof document === 'undefined' || document.hasFocus()) return;
      void showChatAlert(display(message), message.text, $t('Sanser · New message'), () => alive).catch(() => undefined);
    }
  });
  let open = $state<boolean>(false);
  let draft = $state('');
  let composing = $state(false);
  let color = $state(0);
  let showStrokes = $state<boolean>(true);
  let overlayOpen = $state(false);
  let annotationError = $state<string | null>(null);
  let legacySince = $state<number | null>(null);
  let now = $state(Date.now());
  let list = $state<HTMLElement | null>(null);

  $effect(() => {
    const scope = { kind, sessionId };
    untrack(() => { collab.attach(scope); void collab.refresh(); });
  });
  onMount(() => {
    alive = true;
    const timer = window.setInterval(() => { now = Date.now(); void collab.refresh(); }, 250);
    const focus = () => collab.setReading(open && document.hasFocus());
    window.addEventListener('focus', focus); window.addEventListener('blur', focus);
    return () => {
      alive = false; window.clearInterval(timer);
      window.removeEventListener('focus', focus); window.removeEventListener('blur', focus);
      collab.detach();
      if (overlayOpen) void closeAnnotationOverlay().catch(() => undefined);
    };
  });
  $effect(() => { collab.setReading(open && (typeof document === 'undefined' || document.hasFocus())); });

  // Older peers: wait a few seconds for capability negotiation before warning.
  const waitingForPeers = $derived($collab.live && $collab.available && $collab.peers === 0 && $collab.legacyPeers > 0);
  $effect(() => { if (waitingForPeers) { if (legacySince === null) legacySince = Date.now(); } else legacySince = null; });
  const legacy = $derived(legacySince !== null && now - legacySince > 4000);
  const ready = $derived($collab.live && $collab.available && $collab.peers > 0);
  const bytes = $derived(utf8Bytes(draft));

  // The capture-excluded overlay follows the session while annotations are on.
  $effect(() => {
    const wanted = ready && $collab.annotations && showStrokes && isTauriRuntime();
    untrack(() => {
      if (wanted === overlayOpen) return;
      overlayOpen = wanted;
      const action = wanted ? openAnnotationOverlay({ kind, sessionId }, $locale) : closeAnnotationOverlay();
      void action.then(() => { annotationError = null; }).catch((error: unknown) => {
        if (wanted) overlayOpen = false;
        annotationError = message(error, 'Unable to show annotations');
        diagnostics.add({ level: 'warn', category: 'session', message: `Annotations: ${annotationError}` });
      });
    });
  });
  $effect(() => {
    const count = $collab.messages.length;
    const element = list;
    if (count && element) untrack(() => queueMicrotask(() => { element.scrollTop = element.scrollHeight; }));
  });

  function message(error: unknown, fallback: string): string {
    return error instanceof Error ? error.message : typeof error === 'string' ? error : fallback;
  }
  function display(item: ChatMessage): string {
    if (item.own) return $t('You');
    if (kind === 'host') return names[item.from] || item.name || $t('Viewer');
    if (item.from === 'host' || item.from === '-') return item.name ? $t('{name} (host)', { name: item.name }) : $t('Host');
    return item.name || $t('Viewer');
  }
  function time(at: number): string {
    return new Date(at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  }
  async function send(): Promise<void> {
    if (composing || !validChat(draft) || !ready) return;
    const text = draft;
    if (await collab.send(text, selfName)) draft = draft === text ? '' : draft;
  }
  function keydown(event: KeyboardEvent): void {
    if (event.key === 'Enter' && !event.shiftKey && !composing && !event.isComposing) { event.preventDefault(); void send(); }
  }
  async function annotate(operation: () => Promise<unknown>, fallback: string): Promise<void> {
    annotationError = null;
    try { await operation(); } catch (error) { annotationError = message(error, fallback); }
  }
</script>

<details class="card session-chat" bind:open>
  <summary>
    <Icon name="chat" /><span>{$t('Chat and annotations')}</span>
    {#if $collab.unread > 0}<span class="unread" aria-label={$t('{count} unread messages', { count: $collab.unread })}>{$collab.unread > 99 ? '99+' : $collab.unread}</span>{/if}
    <span class="chat-status">{ready ? (kind === 'host' ? $t('{count} connected', { count: $collab.peers }) : $t('Available')) : legacy ? $t('Update needed') : $t('Waiting for the other computer')}</span>
  </summary>
  <div class="card-body stack">
    {#if $collab.live && !$collab.available}
      <p class="notice" role="status">{$t('Update Sanser on this computer to use chat and annotations. The session continues normally.')}</p>
    {:else if legacy}
      <p class="notice" role="status">{$t('The other computer needs to update Sanser to use chat and annotations. The session continues normally.')}</p>
    {:else}
      {#if ready && $collab.legacyPeers > 0 && kind === 'host'}<p class="muted">{$t('Some viewers use an older Sanser and will not see chat or annotations until they update.')}</p>{/if}
      <div class="messages" role="log" aria-live="polite" aria-label={$t('Chat messages')} bind:this={list}>
        {#each $collab.messages as item (item.seq)}
          <div class="message" class:own={item.own}><span class="meta"><strong>{display(item)}</strong> · {time(item.atMs)}</span><p>{item.text}</p></div>
        {:else}
          <p class="muted empty">{ready ? $t('No messages yet. Messages disappear when the session ends.') : $t('Chat starts when the other computer connects.')}</p>
        {/each}
      </div>
      <label class="field" for={`chat-${kind}`}>{$t('Message')}
        <textarea id={`chat-${kind}`} class="input" rows="2" bind:value={draft} autocomplete="off" disabled={!ready || $collab.sending}
          oncompositionstart={() => { composing = true; }} oncompositionend={(event) => { draft = event.currentTarget.value; composing = false; }}
          onkeydown={keydown} placeholder={$t('Type a message · Enter to send, Shift+Enter for a new line')}></textarea>
      </label>
      <div class="button-row">
        <button class="button small primary" disabled={!ready || composing || $collab.sending || !validChat(draft)} onclick={send}><Icon name="arrow" />{$t('Send')}</button>
        <span class="muted" class:over={bytes > CHAT_BYTES}>{$t('{used}/{max} bytes · plain text only', { used: bytes, max: CHAT_BYTES })}</span>
      </div>
      {#if $collab.error}<p class="notice error" role="alert">{$t($collab.error)}</p>{/if}

      <section class="annotations" aria-label={$t('Annotations')}>
        <div class="annotation-heading"><Icon name="pen" /><div><strong>{$t('Annotations')}</strong><p class="muted">{$t('Draw on the shared screen to point at something. Strokes fade after a few seconds and are never recorded in the video.')}</p></div></div>
        {#if kind === 'host'}
          <label class="check"><input type="checkbox" checked={$collab.annotations} disabled={!ready} onchange={(event) => annotate(() => setGroupAnnotations(event.currentTarget.checked), 'Unable to change annotations')} />{$t('Allow everyone to annotate')}</label>
        {:else if !$collab.annotations && ready}
          <p class="muted" role="status">{$t('The host turned annotations off.')}</p>
        {/if}
        {#if $collab.annotations}
          <label class="check"><input type="checkbox" bind:checked={showStrokes} disabled={!ready} />{$t('Show annotations on screen')}</label>
          <div class="pen-colors" role="radiogroup" aria-label={$t('Pen colour')}>
            {#each PEN_COLORS as swatch (swatch.id)}
              <button type="button" role="radio" class="swatch" aria-checked={color === swatch.id} aria-label={$t(swatch.label)} title={$t(swatch.label)} style={`--swatch:${swatch.value}`} onclick={() => { color = swatch.id; }}></button>
            {/each}
          </div>
          <div class="button-row">
            <button class="button small" disabled={!ready || !overlayOpen} onclick={() => annotate(() => setAnnotationPen(true, color, selfName), 'Unable to start the pen')}><Icon name="pen" />{$t('Draw on screen')}</button>
            <button class="button small" disabled={!ready} onclick={() => annotate(() => collabPost({ kind, sessionId }, { type: 'clear', name: selfName }), 'Unable to clear annotations')}>{$t('Clear annotations')}</button>
          </div>
          <p class="muted">{$t('While drawing, press Esc or Done to return to normal use.')}</p>
        {/if}
        {#if annotationError}<p class="notice error" role="alert">{$t(annotationError)}</p>{/if}
      </section>
    {/if}
  </div>
</details>

<style>
  .session-chat summary { display: flex; align-items: center; gap: 10px; padding: 16px 18px; cursor: pointer; list-style: none; }
  .session-chat summary :global(svg) { width: 18px; height: 18px; color: var(--accent); }
  .chat-status { margin-left: auto; color: var(--muted); font-size: 13px; }
  .unread { min-width: 20px; padding: 1px 6px; border-radius: 999px; background: var(--accent); color: var(--sn-background, #fff); font-size: 12px; font-weight: 600; text-align: center; }
  .messages { max-height: 240px; min-height: 64px; overflow-y: auto; display: flex; flex-direction: column; gap: 8px; padding: 10px; border: 1px solid var(--border); border-radius: 10px; background: var(--surface); }
  .message { max-width: 85%; align-self: flex-start; }
  .message.own { align-self: flex-end; text-align: right; }
  .message p { margin: 2px 0 0; padding: 7px 10px; border-radius: 10px; background: var(--sn-background); white-space: pre-wrap; overflow-wrap: anywhere; font-size: 14px; text-align: left; }
  .message.own p { background: var(--accent-soft, var(--sn-background)); }
  .meta { color: var(--muted); font-size: 12px; }
  .empty { margin: auto 0; }
  .over { color: var(--red, #d33); }
  .annotations { border-top: 1px solid var(--border); padding-top: 14px; display: grid; gap: 10px; }
  .annotation-heading { display: flex; gap: 10px; align-items: flex-start; }
  .annotation-heading :global(svg) { width: 18px; height: 18px; color: var(--accent); flex-shrink: 0; margin-top: 2px; }
  .annotation-heading p { margin: 4px 0 0; }
  .check { display: flex; gap: 8px; align-items: center; font-size: 14px; }
  .pen-colors { display: flex; gap: 8px; }
  .swatch { width: 26px; height: 26px; border-radius: 50%; border: 2px solid var(--border); background: var(--swatch); cursor: pointer; padding: 0; }
  .swatch[aria-checked='true'] { border-color: var(--text, #000); box-shadow: 0 0 0 2px var(--surface); }
</style>
