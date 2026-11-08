<script lang="ts">
  import BrandMark from '../components/BrandMark.svelte';
  import { SANSER_VERSION } from '../lib/types';
  let { checking = false, updateAvailable = false, onCheckForUpdates }: {
    checking?: boolean;
    updateAvailable?: boolean;
    onCheckForUpdates: () => void;
  } = $props();
</script>

<section class="page about-page">
  <div class="about-hero">
    <BrandMark size={76} label="Sanser" />
    <h1>Sanser</h1>
    <p>Access your Mac and Windows computers remotely.</p>
    <p>Version {SANSER_VERSION}</p>
    <div class="about-updates"><button class="button primary" disabled={checking} onclick={onCheckForUpdates}>{checking ? 'Checking…' : updateAvailable ? 'Update Sanser' : 'Check for updates'}</button></div>
  </div>
</section>

<style>
  .about-updates { margin-top: 24px; }
</style>
