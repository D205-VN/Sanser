<script lang="ts">
  import { t } from '../lib/i18n';
  import { createProblemReport, issueUrl, openGithubIssue, openProblemReportFolder } from '../lib/recording';
  import { SANSER_VERSION, type RuntimeStatus } from '../lib/types';
  import { diagnostics } from '../stores/diagnostics';

  let { runtime }: { runtime: RuntimeStatus } = $props();
  let busy = $state(false);
  let savedPath = $state<string | null>(null);
  let error = $state<string | null>(null);

  async function report(): Promise<void> {
    busy = true; error = null;
    try {
      const result = await createProblemReport(diagnostics.exportJson(), { platform: runtime.platform, capabilities: runtime.capabilities });
      savedPath = result.path;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : $t('Unable to create the problem report');
    } finally { busy = false; }
  }

  async function openIssue(): Promise<void> {
    error = null;
    try { await openGithubIssue(issueUrl(SANSER_VERSION, runtime.platform)); }
    catch (cause) { error = cause instanceof Error ? cause.message : $t('Unable to open the browser'); }
  }
</script>

<div class="stack report-problem">
  <div><h3>{$t('Report a problem')}</h3><p class="muted">{$t('Creates a zip with diagnostics, the latest connection reports, the app and OS version and a capability summary. Passwords, tokens, e-mail addresses, IP addresses and names are removed. Nothing is sent automatically.')}</p></div>
  <div class="button-row">
    <button class="button primary" disabled={busy} onclick={report}>{busy ? $t('Creating report…') : $t('Report a problem')}</button>
    <button class="button" onclick={openIssue}>{$t('Open GitHub issue')}</button>
    {#if savedPath}<button class="button ghost" onclick={() => openProblemReportFolder()}>{$t('Open reports folder')}</button>{/if}
  </div>
  {#if savedPath}<div class="notice" role="status">{$t('Report saved: {path}. Review it, then attach it to the GitHub issue if you want to share it.', { path: savedPath })}</div>{/if}
  {#if error}<div class="notice error" role="alert">{$t(error)}</div>{/if}
</div>
