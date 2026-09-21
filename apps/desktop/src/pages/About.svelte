<script lang="ts">
  import BrandMark from '../components/BrandMark.svelte';
  import { SANSER_VERSION } from '../lib/types';
</script>

<section class="page about-page">
  <div class="about-hero">
    <BrandMark size={76} label="Sanser" />
    <h1>Sanser</h1>
    <p>Access your Mac and Windows computers remotely.</p>
    <p>Version {SANSER_VERSION}</p>
  </div>
</section>
