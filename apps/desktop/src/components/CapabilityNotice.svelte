<script lang="ts">
  import type { Capability } from '../lib/types';
  import StatusPill from './StatusPill.svelte';

  let { title, capability }: { title: string; capability: Capability } = $props();
</script>

<div class="capability-line">
  <div><strong>{title}</strong><span>{capability.reason ?? 'Ready'}</span></div>
  <StatusPill state={capability.state} label={capability.state === 'available' ? 'Available' : capability.state === 'planned' ? 'Planned' : 'Unavailable'} />
</div>
