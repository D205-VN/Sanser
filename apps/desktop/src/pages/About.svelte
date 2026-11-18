<script lang="ts">
  import { t } from '../lib/i18n';
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
    <h1>{$t('Sanser')}</h1>
    <p>{$t('Access your Mac and Windows computers remotely.')}</p>
    <p>{$t('Version {version}', { version: SANSER_VERSION })}</p>
    <div class="about-updates"><button class="button primary" disabled={checking} onclick={onCheckForUpdates}>{checking ? $t('Checking…') : updateAvailable ? $t('Update Sanser') : $t('Check for updates')}</button></div>
  </div>
</section>

<style>
  .about-updates { margin-top: 24px; }
</style>
