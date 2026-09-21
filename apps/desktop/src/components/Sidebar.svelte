<script lang="ts">
  import type { Page } from '../lib/types';
  import BrandMark from './BrandMark.svelte';
  import Icon from './Icon.svelte';
  import { connection } from '../stores/connection';
  import { host as hostStore } from '../stores/host';

  let { active, navigate }: { active: Page; navigate: (page: Page) => void } = $props();

  const primary: { page: Page; label: string }[] = [
    { page: 'computers', label: 'Computers' },
    { page: 'host', label: 'Host' },
    { page: 'session', label: 'Active session' }
  ];
  const secondary: { page: Page; label: string }[] = [
    { page: 'settings', label: 'Settings' },
    { page: 'about', label: 'About' }
  ];
</script>

<aside class="sidebar">
  <div class="brand">
    <BrandMark size={38} label="Sanser" />
    <div><strong>Sanser</strong></div>

  </div>

  <nav aria-label="Main navigation">
    <div class="nav-group">
      <div class="nav-label">Workspace</div>
      {#each primary as item}
        <button class:active={active === item.page} class="nav-button" title={item.label} aria-label={item.label} aria-current={active === item.page ? 'page' : undefined} onclick={() => navigate(item.page)}>
          <span class="nav-icon"><Icon name={item.page} /></span><span class="nav-text">{item.label}</span>
          {#if item.page === 'session' && $connection.session}<span class="nav-dot" aria-label="Session in progress"></span>{/if}
          {#if item.page === 'host' && $hostStore.sessions.some((item) => item.status === 'pending')}<span class="nav-dot" aria-label="Pending request"></span>{/if}
        </button>
      {/each}
    </div>
    <div class="nav-group">
      <div class="nav-label">System</div>
      {#each secondary as item}
        <button class:active={active === item.page} class="nav-button" title={item.label} aria-label={item.label} aria-current={active === item.page ? 'page' : undefined} onclick={() => navigate(item.page)}>
          <span class="nav-icon"><Icon name={item.page} /></span><span class="nav-text">{item.label}</span>
        </button>
      {/each}
    </div>
  </nav>

</aside>
