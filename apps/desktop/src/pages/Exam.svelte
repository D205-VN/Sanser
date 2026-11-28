<script lang="ts">
  import { t, tk } from '../lib/i18n';
  import ExamJoin from '../components/ExamJoin.svelte';
  import ExamSupervisor from '../components/ExamSupervisor.svelte';

  const MODE_KEY = 'sanser.exam.mode.v1';
  function readMode(): 'supervise' | 'take' {
    try { return localStorage.getItem(MODE_KEY) === 'take' ? 'take' : 'supervise'; } catch { return 'supervise'; }
  }
  let mode = $state<'supervise' | 'take'>(readMode());
  function choose(next: 'supervise' | 'take'): void {
    mode = next;
    try { localStorage.setItem(MODE_KEY, next); } catch { /* not remembered */ }
  }
  const modes = [
    { id: 'supervise' as const, label: tk('Supervise') },
    { id: 'take' as const, label: tk('Take an exam') }
  ];
</script>

<section class="page exam-page">
  <header class="page-header">
    <div><h1>{$t('Exam')}</h1><p>{$t('Lock student computers to the allowed apps and watch them from here.')}</p></div>
  </header>
  <div class="filter-tabs" role="group" aria-label={$t('Exam role')}>
    {#each modes as option (option.id)}
      <button class:active={mode === option.id} aria-pressed={mode === option.id} onclick={() => choose(option.id)}>{$t(option.label)}</button>
    {/each}
  </div>
  {#if mode === 'supervise'}<ExamSupervisor />{:else}<ExamJoin />{/if}
</section>
