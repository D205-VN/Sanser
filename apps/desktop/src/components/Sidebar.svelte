<script lang="ts">
  import { t, tk } from '../lib/i18n';
  import type { Page } from '../lib/types';
  import BrandMark from './BrandMark.svelte';
  import Icon from './Icon.svelte';
  import { connection } from '../stores/connection';
  import { host as hostStore } from '../stores/host';

  let { active, navigate }: { active: Page; navigate: (page: Page) => void } = $props();

  const primary: { page: Page; label: string }[] = [
    { page: 'computers', label: tk('Computers') },
    { page: 'host', label: tk('Host') },
    { page: 'session', label: tk('Active session') }
  ];
  const secondary: { page: Page; label: string }[] = [
    { page: 'settings', label: tk('Settings') },
    { page: 'about', label: tk('About') }
  ];
</script>

<aside class="sidebar">
  <div class="brand">
    <BrandMark size={38} label="Sanser" />
    <div><strong>{$t('Sanser')}</strong><span>{$t('Remote desktop')}</span></div>

  </div>

  <nav aria-label={$t('Main navigation')}>
    <div class="nav-group">
      <div class="nav-label">{$t('Workspace')}</div>
      {#each primary as item}
        <button class:active={active === item.page} class="nav-button" title={$t(item.label)} aria-label={$t(item.label)} aria-current={active === item.page ? 'page' : undefined} onclick={() => navigate(item.page)}>
          <span class="nav-icon"><Icon name={item.page} /></span><span class="nav-text">{$t(item.label)}</span>
          {#if item.page === 'session' && $connection.session}<span class="nav-dot" aria-label={$t('Session in progress')}></span>{/if}
          {#if item.page === 'host' && $hostStore.sessions.some((item) => item.status === 'pending')}<span class="nav-dot" aria-label={$t('Pending request')}></span>{/if}
        </button>
      {/each}
    </div>
    <div class="nav-group">
      <div class="nav-label">{$t('System')}</div>
      {#each secondary as item}
        <button class:active={active === item.page} class="nav-button" title={$t(item.label)} aria-label={$t(item.label)} aria-current={active === item.page ? 'page' : undefined} onclick={() => navigate(item.page)}>
          <span class="nav-icon"><Icon name={item.page} /></span><span class="nav-text">{$t(item.label)}</span>
        </button>
      {/each}
    </div>
  </nav>

  <button class="sidebar-computer" class:online={$hostStore.online} onclick={() => navigate('host')} title={$t('Manage screen sharing on this computer')}>
    <span class="sidebar-computer-icon"><Icon name="monitor" /></span>
    <span><strong>{$t('This computer')}</strong><small>{$hostStore.online ? $t('Available to share') : $t('Screen sharing off')}</small></span>
    <i aria-hidden="true"></i>
  </button>
</aside>
