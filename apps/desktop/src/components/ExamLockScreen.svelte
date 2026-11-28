<script lang="ts">
  import { t } from '../lib/i18n';
  import BrandMark from './BrandMark.svelte';
  import { examSeat } from '../stores/exam';

  // Replaces the whole app while an exam runs on this computer, so the
  // student cannot open another Sanser page (or a remote computer) meanwhile.
  const policy = $derived($examSeat.policy);
  const reconnecting = $derived(policy !== null && policy.roomId === '');
  const apps = $derived(policy ? [...new Set(policy.allowedApps.map((app) => app.name))] : []);
  let unlockCode = $state('');
  let unlocking = $state(false);
  let unlockError = $state<string | null>(null);

  async function emergencyUnlock(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (unlocking || !unlockCode.trim()) return;
    unlocking = true;
    unlockError = null;
    try {
      if (!(await examSeat.unlock(unlockCode))) unlockError = 'That code is not right. Ask your supervisor again.';
      else unlockCode = '';
    } finally { unlocking = false; }
  }

  const synced = $derived($examSeat.lastSyncAt ? new Date($examSeat.lastSyncAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' }) : null);
</script>

<main class="exam-lock" class:paused={policy?.memberState === 'paused' && !reconnecting}>
  <BrandMark size={56} label="Sanser" />
  {#if $examSeat.finished}
    <h1>{$examSeat.finished === 'removed' ? $t('You were removed from the exam') : $examSeat.finished === 'unlocked' ? $t('This computer was unlocked with the emergency code') : $t('The exam has ended')}</h1>
    <p>{$t('This computer is unlocked. You can use it normally again.')}</p>
    <button class="button primary" onclick={() => examSeat.dismiss()}>{$t('Back to Sanser')}</button>
  {:else if reconnecting}
    <h1>{$t('Reconnecting to your exam…')}</h1>
    <p>{$t('Sanser restarted during an exam. This computer stays locked until the supervisor can be reached.')}</p>
  {:else if policy?.memberState === 'paused'}
    <h1>{$t('Your supervisor paused this computer')}</h1>
    <p>{$t('Wait here. The supervisor will let you continue.')}</p>
  {:else if policy}
    <h1>{policy.title}</h1>
    <p>{$t('Supervised by {name}', { name: policy.supervisorName })}</p>
    <div class="card card-body rules">
      <div><span>{$t('Allowed apps')}</span><strong>{apps.length ? apps.join(', ') : $t('Sanser only')}</strong></div>
      <div><span>{$t('AI')}</span><strong>{policy.blockAi ? $t('Blocked') : $t('Not blocked')}</strong></div>
    </div>
    <p class="hint">{apps.length ? $t('Switch to an allowed app to work. Any other app is hidden and reported.') : $t('Wait for your supervisor to choose the exam app.')}</p>
  {/if}
  {#if !$examSeat.finished}
    <details class="emergency">
      <summary>{$t('Emergency unlock')}</summary>
      <form onsubmit={emergencyUnlock}>
        <p>{$t('Only if this computer is stuck: ask your supervisor for the emergency code. Using it leaves the exam and the supervisor sees it.')}</p>
        <div class="inline-field">
          <input class="input" aria-label={$t('Emergency code')} autocomplete="off" spellcheck="false" maxlength="12" placeholder="XXXX-XXXX" bind:value={unlockCode} />
          <button class="button" type="submit" disabled={unlocking || !unlockCode.trim()}>{unlocking ? $t('Checking…') : $t('Unlock')}</button>
        </div>
        {#if unlockError}<p class="unlock-error" role="alert">{$t(unlockError)}</p>{/if}
      </form>
    </details>
    <p class="sync" role="status">{$examSeat.error ? $t('Not reaching the supervisor: {reason}', { reason: $t($examSeat.error) }) : synced ? $t('Reported to the supervisor at {time}', { time: synced }) : ''}</p>
  {/if}
</main>

<style>
  .exam-lock { min-height: 100vh; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 14px; padding: 32px; text-align: center; background: var(--bg, #0f1117); }
  .exam-lock.paused { background: #3a1d12; }
  h1 { margin: 6px 0 0; font-size: 26px; }
  p { margin: 0; color: var(--muted); max-width: 520px; }
  .rules { display: grid; grid-template-columns: 1fr 1fr; gap: 18px; min-width: 360px; text-align: left; }
  .rules div { display: flex; flex-direction: column; gap: 4px; }
  .rules span { color: var(--muted); font-size: 12px; }
  .hint { color: inherit; }
  .sync { font-size: 12px; }
  .emergency { margin-top: 12px; max-width: 420px; font-size: 13px; color: var(--muted); }
  .emergency summary { cursor: pointer; }
  .emergency form { display: grid; gap: 10px; margin-top: 10px; }
  .emergency input { text-transform: uppercase; letter-spacing: 0.12em; }
  .unlock-error { color: var(--danger, #f87171); }
</style>
