<script lang="ts">
  import type { Page, RuntimeStatus } from '../lib/types';
  import BrandMark from './BrandMark.svelte';
  import Icon from './Icon.svelte';
  import StatusPill from './StatusPill.svelte';

  let { active, runtime, navigate }: { active: Page; runtime: RuntimeStatus; navigate: (page: Page) => void } = $props();

  const primary: { page: Page; label: string }[] = [
    { page: 'computers', label: 'Computers' },
    { page: 'host', label: 'Host' },
    { page: 'session', label: 'Active session' }
  ];
  const secondary: { page: Page; label: string }[] = [
    { page: 'settings', label: 'Settings' },
    { page: 'diagnostics', label: 'Diagnostics' },
    { page: 'about', label: 'About' }
  ];
</script>

<aside class="sidebar">
  <div class="brand">
    <BrandMark size={38} label="Sanser" />
    <div><strong>Sanser</strong><span>Remote, refined</span></div>
    <em>2.0</em>
  </div>

  <nav aria-label="Main navigation">
    <div class="nav-group">
      <div class="nav-label">Workspace</div>
      {#each primary as item}
        <button class:active={active === item.page} class="nav-button" aria-current={active === item.page ? 'page' : undefined} onclick={() => navigate(item.page)}>
          <span class="nav-icon"><Icon name={item.page} /></span>{item.label}
        </button>
      {/each}
    </div>
    <div class="nav-group">
      <div class="nav-label">System</div>
      {#each secondary as item}
        <button class:active={active === item.page} class="nav-button" aria-current={active === item.page ? 'page' : undefined} onclick={() => navigate(item.page)}>
          <span class="nav-icon"><Icon name={item.page} /></span>{item.label}
        </button>
      {/each}
    </div>
  </nav>

  <div class="sidebar-footer">
    <span class="footer-label">Runtime</span>
    <div class="runtime-line"><StatusPill state={runtime.capabilities.desktopShell.state} label={runtime.platform} /></div>
    <div class="runtime-line">Protocol v{runtime.protocolVersion} · SNV2</div>
  </div>
</aside>
