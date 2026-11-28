<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { t, tk } from '../lib/i18n';
  import { EXAM_APP_PRESETS, type ExamApp, type ExamMember, type ExamRoom, type ExamRoomDetail, type ExamViolation } from '../lib/exam';
  import { listRunningApps } from '../lib/platform';
  import { session } from '../stores/session';
  import { validBlockedAppId } from '../stores/preferences';
  import Toggle from './Toggle.svelte';

  // Supervisor side of exam mode. Polls the room every few seconds and the
  // screenshots separately, fetching only pictures newer than the last one.
  const ROOM_POLL_MS = 3_000;
  const SCREEN_POLL_MS = 2_000;
  const WATCH_REPEAT_MS = 60_000;

  let rooms = $state<ExamRoom[]>([]);
  let room = $state<ExamRoomDetail | null>(null);
  let screens = $state<Record<string, { src: string; capturedAt: number }>>({});
  let focused = $state<string | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);

  let title = $state('');
  let hours = $state(2);
  let blockAi = $state(true);
  let allowed = $state<ExamApp[]>([]);
  let running = $state<ExamApp[]>([]);
  let manual = $state('');

  let roomTimer: number | null = null;
  let screenTimer: number | null = null;
  let watchTimer: number | null = null;
  let screensSince = 0;

  const focusedMember = $derived(room?.members.find((member) => member.id === focused) ?? null);

  function fail(cause: unknown, fallback: string): void {
    error = cause instanceof Error ? cause.message : fallback;
  }

  async function loadRooms(): Promise<void> {
    const client = session.client();
    if (!client) return;
    try {
      rooms = await client.examRooms();
      const open = rooms.find((item) => item.state === 'open');
      if (open && !room) await openRoom(open.id);
    } catch (cause) { fail(cause, 'Unable to load exams'); }
  }

  async function openRoom(id: string): Promise<void> {
    const client = session.client();
    if (!client) return;
    room = await client.examRoom(id);
    screens = {};
    screensSince = 0;
    startPolling();
  }

  function stopPolling(): void {
    for (const timer of [roomTimer, screenTimer, watchTimer]) if (timer !== null) window.clearInterval(timer);
    roomTimer = screenTimer = watchTimer = null;
  }

  function startPolling(): void {
    stopPolling();
    void pollScreens();
    roomTimer = window.setInterval(() => void pollRoom(), ROOM_POLL_MS);
    screenTimer = window.setInterval(() => void pollScreens(), SCREEN_POLL_MS);
    watchTimer = window.setInterval(() => { if (focused) void act(focused, 'watch', true); }, WATCH_REPEAT_MS);
  }

  async function pollRoom(): Promise<void> {
    const client = session.client();
    if (!client || !room) return;
    try {
      room = await client.examRoom(room.id);
      error = null;
      if (room.state !== 'open') stopPolling();
    } catch (cause) { fail(cause, 'Unable to refresh the exam'); }
  }

  async function pollScreens(): Promise<void> {
    const client = session.client();
    if (!client || !room || room.state !== 'open') return;
    try {
      const fresh = await client.examScreens(room.id, screensSince);
      if (fresh.length === 0) return;
      const next = { ...screens };
      for (const screen of fresh) {
        next[screen.memberId] = { src: `data:image/jpeg;base64,${screen.jpegBase64}`, capturedAt: screen.capturedAt };
        screensSince = Math.max(screensSince, screen.capturedAt);
      }
      screens = next;
    } catch { /* the next poll retries */ }
  }

  async function create(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const client = session.client();
    if (!client || !title.trim()) return;
    busy = true; error = null;
    try {
      room = await client.createExamRoom(title.trim(), allowed, blockAi, hours);
      screens = {};
      screensSince = 0;
      startPolling();
      void loadRooms();
    } catch (cause) { fail(cause, 'Unable to start the exam'); }
    finally { busy = false; }
  }

  // The desktop web view has no window.confirm (it answers "no" silently),
  // so a risky button asks for a second click within a few seconds instead.
  let armed = $state<string | null>(null);
  let armedTimer: number | null = null;
  function confirmed(key: string): boolean {
    if (armedTimer !== null) window.clearTimeout(armedTimer);
    if (armed === key) { armed = null; armedTimer = null; return true; }
    armed = key;
    armedTimer = window.setTimeout(() => { armed = null; armedTimer = null; }, 5_000);
    return false;
  }

  async function act(memberId: string, action: 'pause' | 'resume' | 'remove' | 'watch', quiet = false): Promise<void> {
    const client = session.client();
    if (!client || !room) return;
    if (action === 'remove' && !confirmed(`remove:${memberId}`)) return;
    try {
      room = await client.examMemberAction(room.id, memberId, action);
      if (action === 'remove' && focused === memberId) focused = null;
    } catch (cause) { if (!quiet) fail(cause, 'Unable to update the student'); }
  }

  function focus(member: ExamMember): void {
    focused = member.id;
    void act(member.id, 'watch', true);
  }

  async function end(): Promise<void> {
    const client = session.client();
    if (!client || !room || !confirmed('end')) return;
    busy = true;
    try {
      room = await client.endExamRoom(room.id);
      stopPolling();
      focused = null;
      void loadRooms();
    } catch (cause) { fail(cause, 'Unable to end the exam'); }
    finally { busy = false; }
  }

  async function saveApps(next: ExamApp[], nextBlockAi = room?.blockAi ?? blockAi): Promise<void> {
    allowed = next;
    blockAi = nextBlockAi;
    const client = session.client();
    if (!client || !room || room.state !== 'open') return;
    try { room = await client.updateExamRoom(room.id, { allowedApps: next, blockAi: nextBlockAi }); }
    catch (cause) { fail(cause, 'Unable to change the allowed apps'); }
  }

  function togglePreset(apps: ExamApp[]): void {
    const current = room?.state === 'open' ? room.allowedApps : allowed;
    const chosen = apps.every((app) => current.some((item) => item.id.toLowerCase() === app.id.toLowerCase()));
    const ids = new Set(apps.map((app) => app.id.toLowerCase()));
    void saveApps(chosen ? current.filter((item) => !ids.has(item.id.toLowerCase())) : [...current, ...apps.filter((app) => !current.some((item) => item.id.toLowerCase() === app.id.toLowerCase()))]);
  }

  function addApp(app: ExamApp): void {
    const current = room?.state === 'open' ? room.allowedApps : allowed;
    if (!validBlockedAppId(app.id) || current.some((item) => item.id.toLowerCase() === app.id.trim().toLowerCase())) return;
    void saveApps([...current, { id: app.id.trim(), name: app.name.trim() || app.id.trim() }]);
  }

  function removeApp(id: string): void {
    const current = room?.state === 'open' ? room.allowedApps : allowed;
    void saveApps(current.filter((item) => item.id !== id));
  }

  async function loadRunning(): Promise<void> {
    try { running = await listRunningApps(); } catch { running = []; }
  }

  function addManual(): void {
    if (manual.trim()) addApp({ id: manual.trim(), name: manual.trim() });
    manual = '';
  }

  const kinds: Record<string, string> = {
    'left-app': tk('Switched to another app'),
    'ai-app': tk('AI app closed'),
    'ai-site': tk('AI website opened'),
    'guard-failed': tk('Exam guard stopped'),
    'quit-attempt': tk('Tried to quit Sanser'),
    disconnected: tk('Lost connection'),
    'emergency-unlock': tk('Used the emergency code and left the exam')
  };
  let showUnlock = $state(false);
  function describe(violation: ExamViolation): string {
    if (violation.kind === 'disconnected') return $t('Lost connection for {seconds} s', { seconds: violation.detail });
    return `${$t(kinds[violation.kind] ?? violation.kind)}${violation.detail ? ` · ${violation.detail}` : ''}`;
  }
  const time = (unix: number) => new Date(unix * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
  function statusLabel(member: ExamMember): string {
    if (!member.online) return $t('Disconnected');
    return member.state === 'paused' ? $t('Paused') : $t('Taking the exam');
  }

  onMount(() => { void loadRooms(); void loadRunning(); });
  onDestroy(stopPolling);
</script>

{#if error}<div class="notice error" role="alert">{$t(error)}</div>{/if}

{#if room && room.state === 'open'}
  <article class="card card-body exam-room">
    <div class="room-head">
      <div>
        <h2 class="card-title">{room.title}</h2>
        <p class="card-subtitle">{$t('Students enter this code in Sanser → Exam → Take an exam.')}</p>
      </div>
      <div class="code" aria-label={$t('Exam code')}>{room.code.slice(0, 3)} {room.code.slice(3)}</div>
      <button class="button danger" disabled={busy} onclick={end}>{armed === 'end' ? $t('Click again to end for everyone') : $t('End exam')}</button>
    </div>
    <div class="room-rules">
      <span>{$t('Allowed apps')}: {room.allowedApps.length ? [...new Set(room.allowedApps.map((app) => app.name))].join(', ') : $t('Sanser only')}</span>
      <span>{room.blockAi ? $t('AI apps and websites blocked') : $t('AI not blocked')}</span>
      <span>{$t('Ends by itself at {time}', { time: time(room.expiresAt) })}</span>
    </div>
    {#if room.unlockCode}
      <div class="room-unlock">
        <span>{$t('Emergency code for a stuck computer')}:</span>
        {#if showUnlock}<strong class="unlock-code">{room.unlockCode.slice(0, 4)}-{room.unlockCode.slice(4)}</strong>{:else}<strong aria-hidden="true">••••-••••</strong>{/if}
        <button class="button small ghost" onclick={() => (showUnlock = !showUnlock)}>{showUnlock ? $t('Hide') : $t('Show')}</button>
        <small>{$t('Read it only to the student who is stuck; it unlocks that computer even without internet.')}</small>
      </div>
    {/if}
    <details class="room-settings">
      <summary>{$t('Change allowed apps')}</summary>
      {@render appPicker(room.allowedApps, room.blockAi)}
    </details>
  </article>

  {#if room.members.length === 0}
    <div class="card empty"><div><h2>{$t('Waiting for students')}</h2><p>{$t('Each student opens Sanser, chooses Exam → Take an exam and types the code above.')}</p></div></div>
  {:else}
    <div class="exam-grid">
      {#each room.members as member (member.id)}
        <button class="card student" class:offline={!member.online} class:paused={member.state === 'paused'} class:selected={focused === member.id} onclick={() => focus(member)}>
          {#if screens[member.id]}<img src={screens[member.id]?.src} alt={$t('Screen of {name}', { name: member.displayName })} />{:else}<div class="no-screen">{$t('No picture yet')}</div>{/if}
          <span class="student-line"><strong>{member.displayName}</strong>{#if member.violationCount > 0}<span class="badge" title={$t('Violations')}>{member.violationCount}</span>{/if}</span>
          <small>{statusLabel(member)}{member.foregroundApp ? ` · ${member.foregroundApp}` : ''}</small>
        </button>
      {/each}
    </div>
  {/if}

  {#if focusedMember}
    <article class="card card-body focus" aria-label={$t('Student detail')}>
      <div class="room-head">
        <div><h2 class="card-title">{focusedMember.displayName}</h2><p class="card-subtitle">{statusLabel(focusedMember)} · {$t('Pictures every 2 seconds while this is open')}</p></div>
        <div class="button-row">
          {#if focusedMember.state === 'paused'}<button class="button primary" onclick={() => act(focusedMember.id, 'resume')}>{$t('Let them continue')}</button>
          {:else}<button class="button" onclick={() => act(focusedMember.id, 'pause')}>{$t('Pause this computer')}</button>{/if}
          <button class="button danger" onclick={() => act(focusedMember.id, 'remove')}>{armed === `remove:${focusedMember.id}` ? $t('Click again to remove') : $t('Remove from exam')}</button>
          <button class="button ghost" onclick={() => (focused = null)}>{$t('Close')}</button>
        </div>
      </div>
      {#if screens[focusedMember.id]}<img class="large" src={screens[focusedMember.id]?.src} alt={$t('Screen of {name}', { name: focusedMember.displayName })} /><small>{$t('Captured at {time}', { time: time(screens[focusedMember.id]?.capturedAt ?? 0) })}</small>{/if}
    </article>
  {/if}

  <article class="card card-body">
    <h2 class="card-title">{$t('Violations')}</h2>
    {#if room.violations.length === 0}<p class="card-subtitle">{$t('Nothing so far.')}</p>{:else}
      <ul class="violations">
        {#each room.violations as violation, index (index)}
          <li><time>{time(violation.createdAt)}</time><strong>{violation.displayName}</strong><span>{describe(violation)}</span></li>
        {/each}
      </ul>
    {/if}
  </article>
{:else}
  {#if room && room.state === 'ended'}<div class="notice" role="status">{$t('The exam "{title}" has ended. Every student computer is unlocked.', { title: room.title })}</div>{/if}
  <form class="card card-body stack" onsubmit={create}>
    <div><h2 class="card-title">{$t('Start an exam')}</h2><p class="card-subtitle">{$t('Student computers keep only the allowed apps in front, AI apps are closed, and you see each screen here.')}</p></div>
    <div class="field"><label for="exam-title">{$t('Exam name')}</label><input id="exam-title" class="input" maxlength="80" required bind:value={title} placeholder={$t('Math test, class 10A')} /></div>
    <div class="field"><label for="exam-hours">{$t('Longest duration')}</label>
      <select id="exam-hours" class="select" bind:value={hours}>{#each [1, 2, 3, 4, 6, 8] as value (value)}<option value={value}>{$t('{count} hours', { count: value })}</option>{/each}</select>
    </div>
    {@render appPicker(allowed, blockAi)}
    <div class="button-row"><button class="button primary" disabled={busy || !title.trim()}>{busy ? $t('Starting…') : $t('Start exam')}</button></div>
  </form>
{/if}

{#snippet appPicker(current: ExamApp[], aiBlocked: boolean)}
  <div class="stack app-picker">
    <div><strong>{$t('Allowed apps')}</strong><p class="card-subtitle">{$t('Students can use only these apps. With none, they see only Sanser.')}</p></div>
    <div class="presets">
      {#each EXAM_APP_PRESETS as preset (preset.name)}
        <label class="preset"><input type="checkbox" checked={preset.apps.every((app) => current.some((item) => item.id.toLowerCase() === app.id.toLowerCase()))} onchange={() => togglePreset(preset.apps)} />{preset.name}</label>
      {/each}
    </div>
    {#if current.some((app) => !EXAM_APP_PRESETS.some((preset) => preset.apps.some((item) => item.id.toLowerCase() === app.id.toLowerCase())))}
      <ul class="custom-apps">
        {#each current.filter((app) => !EXAM_APP_PRESETS.some((preset) => preset.apps.some((item) => item.id.toLowerCase() === app.id.toLowerCase()))) as app (app.id)}
          <li><span>{app.name}<small>{app.id}</small></span><button type="button" class="button small ghost" onclick={() => removeApp(app.id)}>{$t('Remove')}</button></li>
        {/each}
      </ul>
    {/if}
    <div class="add-row">
      <select class="select" aria-label={$t('Add a running app')} onchange={(event) => { const app = running.find((item) => item.id === (event.currentTarget as HTMLSelectElement).value); if (app) addApp(app); (event.currentTarget as HTMLSelectElement).value = ''; }}>
        <option value="">{$t('Add an app open on this computer…')}</option>
        {#each running as app (app.id)}<option value={app.id}>{app.name}</option>{/each}
      </select>
    </div>
    <div class="add-row">
      <input class="input" bind:value={manual} placeholder={$t('Or type an app: com.example.App or app.exe')} aria-label={$t('App identifier')} onkeydown={(event) => { if (event.key === 'Enter') { event.preventDefault(); addManual(); } }} />
      <button type="button" class="button" disabled={!manual.trim()} onclick={addManual}>{$t('Add')}</button>
    </div>
    <Toggle checked={aiBlocked} label={$t('Block AI')} description={$t('Close ChatGPT, Claude, Gemini, Copilot and similar apps, and flag AI websites.')} onchange={(value) => void saveApps(current, value)} />
  </div>
{/snippet}

<style>
  .room-unlock { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; margin-top: 10px; font-size: 13px; color: var(--muted); }
  .room-unlock strong { color: var(--text); letter-spacing: 0.12em; font-family: ui-monospace, monospace; }
  .room-unlock small { flex-basis: 100%; }
  .exam-room { display: flex; flex-direction: column; gap: 12px; margin-bottom: 16px; }
  .room-head { display: flex; align-items: center; gap: 16px; justify-content: space-between; flex-wrap: wrap; }
  .code { font-size: 32px; font-weight: 700; letter-spacing: 6px; font-variant-numeric: tabular-nums; color: var(--accent); }
  .room-rules { display: flex; flex-wrap: wrap; gap: 8px 18px; color: var(--muted); font-size: 13px; }
  .room-settings summary { cursor: pointer; color: var(--accent); font-size: 13px; }
  .exam-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 12px; margin-bottom: 16px; }
  .student { display: flex; flex-direction: column; gap: 6px; padding: 10px; text-align: left; cursor: pointer; color: inherit; font: inherit; border: 1px solid var(--border); }
  .student.selected { border-color: var(--accent); }
  .student.paused { border-color: var(--warning, #d89b2b); }
  .student.offline { opacity: 0.6; border-color: var(--danger, #d65353); }
  .student img, .no-screen { width: 100%; aspect-ratio: 16 / 10; object-fit: cover; border-radius: 8px; background: var(--surface-2, rgba(127,127,127,0.12)); }
  .no-screen { display: grid; place-items: center; color: var(--muted); font-size: 12px; }
  .student-line { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .student small { color: var(--muted); font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .badge { min-width: 22px; padding: 1px 7px; border-radius: 999px; background: var(--danger, #d65353); color: #fff; font-size: 12px; text-align: center; }
  .focus { display: flex; flex-direction: column; gap: 10px; margin-bottom: 16px; }
  .focus .large { width: 100%; max-height: 60vh; object-fit: contain; border-radius: 10px; background: #000; }
  .focus small { color: var(--muted); }
  .violations { list-style: none; margin: 0; padding: 0; display: grid; gap: 6px; max-height: 320px; overflow: auto; font-size: 13px; }
  .violations li { display: grid; grid-template-columns: max-content minmax(90px, 160px) 1fr; gap: 12px; }
  .violations time { white-space: nowrap; }
  .violations time { color: var(--muted); font-variant-numeric: tabular-nums; }
  .app-picker { gap: 10px; }
  .presets { display: flex; flex-wrap: wrap; gap: 8px 16px; }
  .preset { display: inline-flex; gap: 6px; align-items: center; font-size: 13px; }
  .custom-apps { list-style: none; margin: 0; padding: 0; display: grid; gap: 6px; }
  .custom-apps li { display: flex; justify-content: space-between; align-items: center; gap: 8px; font-size: 13px; }
  .custom-apps small { display: block; color: var(--muted); }
  .add-row { display: flex; gap: 8px; }
  .add-row .input, .add-row .select { flex: 1; }
</style>
