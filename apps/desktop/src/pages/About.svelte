<script lang="ts">
  import StatusPill from '../components/StatusPill.svelte';
  import BrandMark from '../components/BrandMark.svelte';
  import { NATIVE_PROTOCOL, PROTOCOL_VERSION, SANSER_VERSION, type RuntimeStatus } from '../lib/types';

  let { runtime }: { runtime: RuntimeStatus } = $props();
</script>

<section class="page about-page">
  <div class="about-hero">
    <BrandMark size={92} label="Sanser" />
    <div class="eyebrow">Remote, refined</div>
    <h1>Sanser {SANSER_VERSION}</h1>
    <p>A lightweight Tauri desktop shell for low-latency remote desktop and game streaming.</p>
    <div class="about-identifiers"><span>Protocol v{PROTOCOL_VERSION}</span><span>{NATIVE_PROTOCOL}</span><span>com.sanser.desktop</span></div>
  </div>
  <div class="grid three about-cards">
    <article class="card card-body"><h2 class="card-title">Network</h2><p class="card-subtitle">Auto, Direct and Relay. No Tailscale dependency. The current native path uses authenticated suitable direct routes.</p></article>
    <article class="card card-body"><h2 class="card-title">Video</h2><p class="card-subtitle">H.264, HEVC and Auto with a compatibility-first encoder fallback.</p></article>
    <article class="card card-body"><h2 class="card-title">Runtime</h2><p class="card-subtitle">{runtime.platform}</p><div class="about-runtime"><StatusPill state={runtime.capabilities.desktopShell.state} label={runtime.capabilities.desktopShell.state} /></div></article>
  </div>
  <div class="notice">Sanser is honest about capability state: unavailable or planned functions remain disabled until their native implementation is installed and verified.</div>
</section>
