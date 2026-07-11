<script module lang="ts">
  let nextMarkId = 0;
</script>

<script lang="ts">
  let {
    size = 40,
    label = null,
    class: className = ''
  }: { size?: number; label?: string | null; class?: string } = $props();

  const gradientId = `sanser-mark-gradient-${nextMarkId++}`;
  const glowId = `sanser-mark-glow-${nextMarkId++}`;
</script>

<svg
  class={`brand-mark ${className}`}
  width={size}
  height={size}
  viewBox="0 0 64 64"
  fill="none"
  xmlns="http://www.w3.org/2000/svg"
  role={label ? 'img' : undefined}
  aria-hidden={label ? undefined : 'true'}
  aria-label={label ?? undefined}
>
  {#if label}<title>{label}</title>{/if}
  <defs>
    <linearGradient id={gradientId} x1="13" y1="8" x2="53" y2="57" gradientUnits="userSpaceOnUse">
      <stop stop-color="#C6C0FF" />
      <stop offset="0.48" stop-color="#8E9BFF" />
      <stop offset="1" stop-color="#61E2C0" />
    </linearGradient>
    <filter id={glowId} x="-35%" y="-35%" width="170%" height="170%">
      <feGaussianBlur stdDeviation="2.2" />
    </filter>
  </defs>

  <rect x="2" y="2" width="60" height="60" rx="17" fill="#0B0F18" />
  <rect x="2.75" y="2.75" width="58.5" height="58.5" rx="16.25" stroke="white" stroke-opacity="0.12" stroke-width="1.5" />
  <path
    d="M43.4 20.2C40.9 17.7 37.3 16.2 32.9 16.2H25.6C20.3 16.2 16 20.2 16 25.2C16 30.1 20.1 33.9 25.2 33.9H37.4C40 33.9 42 35.6 42 38C42 40.5 39.9 42.3 37.2 42.3H29.1C25.4 42.3 22.5 41.2 20.3 39"
    stroke={`url(#${gradientId})`}
    stroke-width="7"
    stroke-linecap="round"
    stroke-linejoin="round"
    opacity="0.24"
    filter={`url(#${glowId})`}
  />
  <path
    d="M43.4 20.2C40.9 17.7 37.3 16.2 32.9 16.2H25.6C20.3 16.2 16 20.2 16 25.2C16 30.1 20.1 33.9 25.2 33.9H37.4C40 33.9 42 35.6 42 38C42 40.5 39.9 42.3 37.2 42.3H29.1C25.4 42.3 22.5 41.2 20.3 39"
    stroke={`url(#${gradientId})`}
    stroke-width="5.25"
    stroke-linecap="round"
    stroke-linejoin="round"
  />
  <circle cx="46.9" cy="17.5" r="2.8" fill="#69E3C2" />
  <circle cx="46.9" cy="17.5" r="5.6" fill="#69E3C2" fill-opacity="0.13" />
</svg>

<style>
  .brand-mark {
    display: block;
    flex: 0 0 auto;
    overflow: visible;
    filter: drop-shadow(0 12px 24px rgba(75, 64, 194, 0.24));
  }
</style>
