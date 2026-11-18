<script lang="ts">
  import { t } from '../lib/i18n';
  import type { Capability } from '../lib/types';
  import StatusPill from './StatusPill.svelte';

  let { title, capability }: { title: string; capability: Capability } = $props();
</script>

<div class="capability-line">
  <div><strong>{title}</strong><span>{$t(capability.reason ?? '') || $t('Ready')}</span></div>
  <StatusPill state={capability.state} label={capability.state === 'available' ? $t('Available') : capability.state === 'planned' ? $t('Planned') : $t('Unavailable')} />
</div>
