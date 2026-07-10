<script lang="ts">
  let { name }: { name: 'computers' | 'host' | 'session' | 'settings' | 'diagnostics' | 'about' | 'search' | 'monitor' } = $props();
</script>

<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  {#if name === 'computers'}
    <rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8 20h8M12 16v4"/>
  {:else if name === 'host'}
    <rect x="4" y="3" width="16" height="18" rx="2"/><path d="M8 7h8M8 11h8M8 15h5"/>
  {:else if name === 'session'}
    <path d="M5 3h14a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H9l-5 4v-4a2 2 0 0 1-1-2V5a2 2 0 0 1 2-2Z"/><path d="m10 8 5 3-5 3V8Z"/>
  {:else if name === 'settings'}
    <circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.9l.1.1-2.8 2.8-.1-.1a1.7 1.7 0 0 0-1.9-.3 1.7 1.7 0 0 0-1 1.6v.2h-4V21a1.7 1.7 0 0 0-1-1.6 1.7 1.7 0 0 0-1.9.3l-.1.1L4.2 17l.1-.1a1.7 1.7 0 0 0 .3-1.9A1.7 1.7 0 0 0 3 14H2.8v-4H3a1.7 1.7 0 0 0 1.6-1 1.7 1.7 0 0 0-.3-1.9L4.2 7 7 4.2l.1.1A1.7 1.7 0 0 0 9 4.6 1.7 1.7 0 0 0 10 3V2.8h4V3a1.7 1.7 0 0 0 1 1.6 1.7 1.7 0 0 0 1.9-.3l.1-.1L19.8 7l-.1.1a1.7 1.7 0 0 0-.3 1.9 1.7 1.7 0 0 0 1.6 1h.2v4H21a1.7 1.7 0 0 0-1.6 1Z"/>
  {:else if name === 'diagnostics'}
    <path d="M3 12h4l2-6 4 12 2-6h6"/>
  {:else if name === 'about'}
    <circle cx="12" cy="12" r="9"/><path d="M12 11v5M12 8h.01"/>
  {:else if name === 'search'}
    <circle cx="11" cy="11" r="7"/><path d="m20 20-4-4"/>
  {:else}
    <rect x="3" y="4" width="18" height="13" rx="2"/><path d="M8 21h8M12 17v4"/>
  {/if}
</svg>
