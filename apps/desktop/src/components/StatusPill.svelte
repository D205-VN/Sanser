<script lang="ts">
  import type { CapabilityState } from '../lib/types';
  let { state, label }: { state: CapabilityState | 'online' | 'offline' | 'warning' | 'neutral'; label: string } = $props();
</script>

<span class="status {state}" title={label}>{label}</span>
