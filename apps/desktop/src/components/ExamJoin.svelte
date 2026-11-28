<script lang="ts">
  import { t } from '../lib/i18n';
  import { examSeat } from '../stores/exam';
  import { presence } from '../stores/presence';
  import { session } from '../stores/session';

  // Student side before the exam: the code and the name the supervisor sees.
  let code = $state('');
  let name = $state($session.account?.displayName ?? '');
  let error = $state<string | null>(null);
  let agreed = $state(false);

  async function join(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    error = null;
    const deviceId = $presence.deviceId;
    if (!deviceId) { error = 'This computer is still connecting to Sanser. Try again in a moment.'; return; }
    try { await examSeat.join(code, name.trim(), deviceId); }
    catch (cause) { error = cause instanceof Error ? cause.message : 'Unable to join the exam'; }
  }
</script>

<form class="card card-body stack" onsubmit={join}>
  <div><h2 class="card-title">{$t('Take an exam')}</h2><p class="card-subtitle">{$t('Enter the code your supervisor gives you.')}</p></div>
  {#if error}<div class="notice error" role="alert">{$t(error)}</div>{/if}
  <div class="field"><label for="exam-code">{$t('Exam code')}</label><input id="exam-code" class="input code" maxlength="7" autocomplete="off" required bind:value={code} placeholder="ABC 234" /></div>
  <div class="field"><label for="exam-name">{$t('Your name')}</label><input id="exam-name" class="input" maxlength="60" required bind:value={name} /></div>
  <div class="notice warning">
    <strong>{$t('During the exam')}</strong>
    <ul>
      <li>{$t('Only the apps your supervisor allows stay in front. Others are hidden at once.')}</li>
      <li>{$t('AI apps such as ChatGPT are closed, and AI websites are reported.')}</li>
      <li>{$t('Your supervisor sees your screen every few seconds and can pause this computer.')}</li>
      <li>{$t('Sanser cannot be closed until the supervisor ends the exam.')}</li>
    </ul>
  </div>
  <label class="agree"><input type="checkbox" bind:checked={agreed} />{$t('I understand and want to start')}</label>
  <div class="button-row"><button class="button primary" disabled={!agreed || $examSeat.busy || code.trim().length < 6 || !name.trim()}>{$examSeat.busy ? $t('Joining…') : $t('Join exam')}</button></div>
</form>

<style>
  .code { font-size: 22px; letter-spacing: 4px; text-transform: uppercase; max-width: 200px; }
  ul { margin: 6px 0 0; padding-left: 18px; display: grid; gap: 4px; }
  .agree { display: inline-flex; gap: 8px; align-items: center; font-size: 14px; }
</style>
