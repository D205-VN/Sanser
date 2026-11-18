<script lang="ts">
  import { locale, setLocale, t, type Locale } from '../lib/i18n';

  const options: { value: Locale; short: string; name: string }[] = [
    { value: 'vi', short: 'VI', name: 'Tiếng Việt' },
    { value: 'en', short: 'EN', name: 'English' }
  ];
  let failed = $state(false);

  async function choose(value: Locale): Promise<void> {
    failed = false;
    try { await setLocale(value); } catch { failed = true; }
  }
</script>

<div class="language-toggle" role="group" aria-label={$t('Language')} title={failed ? $t('Unable to save changes. Please try again.') : $t('Language')}>
  {#each options as option (option.value)}
    <button type="button" class:active={$locale === option.value} aria-pressed={$locale === option.value} aria-label={option.name} lang={option.value} onclick={() => choose(option.value)}>{option.short}</button>
  {/each}
</div>

<style>
  .language-toggle { display: inline-flex; flex: 0 0 auto; padding: 2px; border: 1px solid var(--border); border-radius: 8px; background: var(--surface); }
  button { min-width: 32px; height: 26px; padding: 0 7px; border: 0; border-radius: 6px; color: var(--muted); background: transparent; font: inherit; font-size: 12px; font-weight: 600; letter-spacing: .03em; cursor: pointer; }
  button:hover { color: var(--text); }
  button.active { color: var(--text); background: var(--surface-raised); box-shadow: 0 0 0 1px var(--border-strong); }
</style>
