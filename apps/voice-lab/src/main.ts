/// <reference types="vite/client" />
import './style.css';
import { VoiceSession, type VoiceState } from './session';
import { LiveKitVoiceRoom } from './livekit';
import { VoiceMeasurements } from './stats';
import { LabAccount, LabAccountError, type SharingChoice } from './account';

function element<T extends HTMLElement>(id: string): T {
  const node = document.getElementById(id);if (!node) throw new Error('Missing UI element');return node as T;
}
const form = element<HTMLFormElement>('connect-form'), endpoint = element<HTMLInputElement>('endpoint'), token = element<HTMLInputElement>('token');
const mic = element<HTMLButtonElement>('mic'), leave = element<HTMLButtonElement>('leave'), deafen = element<HTMLButtonElement>('deafen');
const resume = element<HTMLButtonElement>('resume'), download = element<HTMLButtonElement>('export');
let activeRoom: LiveKitVoiceRoom | undefined, measurements = new VoiceMeasurements(), measuring = false;
const account = new LabAccount();
let choices: SharingChoice[] = [], admissionGeneration = 0, admissionPending = false, authBusy = false;
let watchedChoice: SharingChoice | undefined, checkingMembership = false;
let watchedRevision: string | undefined;
const session = new VoiceSession(() => {
  measurements = new VoiceMeasurements();download.disabled = true;
  return activeRoom = new LiveKitVoiceRoom(element('audio-sink'));
});
const errors: Record<NonNullable<VoiceState['error']>, string> = {
  'invalid-config': 'Dùng wss:// cho server và token tham gia hợp lệ. ws:// chỉ dùng với localhost.',
  'connection-failed': 'Chưa vào được phòng. Kiểm tra server, token còn hạn và quyền tham gia phòng.',
  'connection-lost': 'Kết nối bị gián đoạn. Mic đã dừng; bấm tham gia lại khi mạng ổn định.',
  'microphone-denied': 'Trình duyệt chưa được cấp quyền mic. Cho phép trong cài đặt rồi bấm Mở mic lại.',
  'microphone-failed': 'Không mở hoặc duy trì được mic. Kiểm tra thiết bị và thử Mở mic lại.',
  'room-capacity': 'Bản thử hỗ trợ tối đa 4 người trong một phòng.'
};
function renderAccount(): void {
  const busy = session.state.joined || session.state.phase === 'joining' || admissionPending;
  element('account-form').hidden = account.signedIn;
  element('sharing-form').hidden = !account.signedIn;
  for (const id of ['sanser-server', 'email', 'password']) element<HTMLInputElement>(id).disabled = busy || authBusy;
  element<HTMLButtonElement>('login').disabled = busy || authBusy;
  element<HTMLButtonElement>('account-join').disabled = busy || authBusy || choices.length === 0;
  element<HTMLButtonElement>('refresh-sharing').disabled = busy || authBusy;
  element<HTMLSelectElement>('sharing-choice').disabled = busy || authBusy;
  endpoint.disabled = token.disabled = busy || authBusy;
  element<HTMLButtonElement>('join').disabled = busy || authBusy;
  leave.disabled = !(busy || authBusy);
}
function accountError(error: unknown): void {
  if (error instanceof LabAccountError && error.code === 'cancelled') return;
  element('account-status').textContent = error instanceof LabAccountError && error.code === 'config'
    ? 'Chỉ dùng HTTPS hoặc HTTP trên localhost, không có thông tin đăng nhập trong URL.'
    : error instanceof LabAccountError && error.code === 'login'
      ? 'Chưa đăng nhập được hoặc phiên đã hết hạn. Đăng nhập lại để tiếp tục.'
      : error instanceof LabAccountError && error.code === 'membership'
        ? 'Vai trò này không còn được phép vào phòng. Mic đã dừng; kiểm tra phiên chia sẻ.'
        : 'Chưa truy cập được Voice Lab. Kiểm tra server, cấu hình thử nghiệm và CORS.';
}
function cancelAdmission(): void {
  ++admissionGeneration;watchedChoice = undefined;watchedRevision = undefined;admissionPending = authBusy = false;account.cancelRequests();
}
async function loadChoices(generation: number): Promise<void> {
  const next = await account.choices();if (generation !== admissionGeneration) return;
  choices = next;
  const select = element<HTMLSelectElement>('sharing-choice');select.replaceChildren();
  choices.forEach((choice, index) => { const option = document.createElement('option');option.value = String(index);option.textContent = choice.label;select.append(option); });
  element('account-status').textContent = choices.length ? 'Đã tải phiên chia sẻ. Tham gia sẽ chỉ nghe; mic vẫn tắt.' : 'Chưa có phiên chia sẻ. Mở Sanser, bắt đầu chia sẻ và chấp nhận lời mời trước.';
}
const unsubscribe = session.subscribe((state) => {
  if (!state.joined && state.phase !== 'joining') activeRoom = undefined;
  if (!state.joined) for (const id of ['jitter', 'rtt', 'buffer']) {
    element(id).textContent = '—';element(`${id}-range`).textContent = 'Chưa có dữ liệu mới';
  }
  const connecting = state.phase === 'joining';
  endpoint.disabled = token.disabled = state.joined || connecting;
  element<HTMLButtonElement>('join').disabled = state.joined || connecting;
  leave.disabled = !(state.joined || connecting);
  mic.disabled = !state.joined || state.deafened;
  deafen.disabled = !state.joined;
  mic.setAttribute('aria-pressed', String(state.microphone));deafen.setAttribute('aria-pressed', String(state.deafened));
  element('mic-label').textContent = state.micPending ? 'Hủy mở mic' : state.microphone ? 'Tắt mic' : 'Mở mic';
  element('deafen-label').textContent = state.deafened ? 'Bật nghe' : 'Tắt nghe';
  element('room-state').textContent = connecting ? 'Đang tham gia · mic đang tắt' : state.microphone ? 'Đã kết nối · mic đang mở' : state.joined ? state.deafened ? 'Đã tắt nghe · mic đang tắt' : 'Đang nghe · mic đang tắt' : 'Chưa tham gia · mic đang tắt';
  element('mic-caption').textContent = state.micPending ? 'Đang chờ quyền hoặc thiết bị mic…' : state.microphone ? 'Mic của bạn đang được phát vào phòng.' : 'Bạn quyết định khi nào mở mic.';
  element('members').textContent = `${state.members} / 4`;
  element('voice-orb').classList.toggle('live', state.microphone);mic.classList.toggle('on', state.microphone);
  element('connection-dot').classList.toggle('live', state.joined);
  resume.hidden = !state.playbackBlocked || !state.joined;
  const error = element('error');error.hidden = !state.error;error.textContent = state.error ? errors[state.error] : '';
  if ((state.phase === 'idle' || state.phase === 'error') && watchedChoice) cancelAdmission();
  renderAccount();
});
const uiCleanup: (() => void)[] = [];
function listen(node: HTMLElement, type: string, handler: EventListener): void {
  node.addEventListener(type, handler);uiCleanup.push(() => node.removeEventListener(type, handler));
}
listen(form, 'submit', (event) => {
  if (session.state.joined || session.state.phase === 'joining' || admissionPending || authBusy) { event.preventDefault();return; }
  cancelAdmission();
  event.preventDefault();const secret = token.value;token.value = '';
  void session.join(endpoint.value, secret);
});
listen(element('account-form'), 'submit', (event) => {
  event.preventDefault();if (authBusy || admissionPending || session.state.joined || session.state.phase === 'joining') return;
  cancelAdmission();const generation = admissionGeneration;authBusy = true;choices = [];
  const password = element<HTMLInputElement>('password'), secret = password.value;password.value = '';renderAccount();
  void account.login(element<HTMLInputElement>('sanser-server').value, element<HTMLInputElement>('email').value, secret)
    .then(() => loadChoices(generation)).catch((error: unknown) => { if (generation === admissionGeneration) accountError(error); })
    .finally(() => { if (generation === admissionGeneration) { authBusy = false;renderAccount(); } });
});
listen(element('refresh-sharing'), 'click', () => {
  if (authBusy || admissionPending || session.state.joined || session.state.phase === 'joining') return;
  const generation = admissionGeneration;authBusy = true;renderAccount();
  void loadChoices(generation).catch((error: unknown) => { if (generation === admissionGeneration) accountError(error); })
    .finally(() => { if (generation === admissionGeneration) { authBusy = false;renderAccount(); } });
});
listen(element('sharing-form'), 'submit', (event) => {
  event.preventDefault();if (authBusy || admissionPending || session.state.joined || session.state.phase === 'joining') return;
  const choice = choices[Number(element<HTMLSelectElement>('sharing-choice').value)];if (!choice) return;
  const generation = ++admissionGeneration;admissionPending = true;renderAccount();
  void account.grant(choice).then(async (grant) => {
    if (generation !== admissionGeneration) return;
    await session.join(grant.url, grant.token);
    if (generation === admissionGeneration && session.state.joined) { watchedChoice = choice;watchedRevision = grant.roomRevision; }
  }).catch((error: unknown) => { if (generation === admissionGeneration) { watchedChoice = undefined;session.leave();accountError(error); } })
    .finally(() => { if (generation === admissionGeneration) { admissionPending = false;renderAccount(); } });
});
listen(element('logout'), 'click', () => {
  cancelAdmission();session.leave();account.logout();choices = [];
  element('account-status').textContent = 'Đã đăng xuất khỏi bản thử.';renderAccount();
});
listen(mic, 'click', () => void session.setMicrophone(!(session.state.microphone || session.state.micPending)));
listen(deafen, 'click', () => session.setDeafened(!session.state.deafened));
listen(leave, 'click', () => { cancelAdmission();session.leave();renderAccount(); });listen(resume, 'click', () => void session.resumeAudio());
// This is cooperative trial cleanup, NOT SFU-enforced token revocation. A grant
// error/timeout ends voice rather than restoring mic or silently switching rooms.
const membershipTimer = window.setInterval(() => {
  if (!watchedChoice || !session.state.joined || checkingMembership) return;
  const choice = watchedChoice, generation = admissionGeneration;checkingMembership = true;
  void account.grant(choice).then((grant) => {
    if (generation !== admissionGeneration) return;
    if (grant.roomRevision !== watchedRevision) {
      cancelAdmission();session.leave();
      element('account-status').textContent = 'Quyền tham gia phòng đã thay đổi. Mic đã dừng; bấm tham gia lại để vào phòng mới.';
      renderAccount();
    }
  }).catch((error: unknown) => {
    if (generation === admissionGeneration) { cancelAdmission();session.leave();accountError(error);renderAccount(); }
  }).finally(() => { checkingMembership = false; });
}, 10000);
const timer = window.setInterval(() => {
  if (measuring || !session.state.joined || !activeRoom) return;
  const room = activeRoom, target = measurements;measuring = true;
  void room.readStats().then((sources) => {
    if (activeRoom !== room || measurements !== target || !session.state.joined) return;
    measurements.observe(sources, performance.now(), session.state.microphone);
    const data = measurements.export() as { metrics: Record<string, { p50Ms: number; p95Ms: number } | null>; samples: { metrics: Record<string, number> }[] };
    for (const [key, id] of [['receiverJitterMaxMs', 'jitter'], ['senderRemoteRttMaxMs', 'rtt'], ['jitterBufferResidenceAvgMs', 'buffer']]) {
      if (!key || !id) continue;
      const metric = data.metrics[key], latest = data.samples.at(-1)?.metrics[key];
      element(id).textContent = latest === undefined ? '—' : `${latest.toFixed(1)} ms`;
      element(`${id}-range`).textContent = metric ? `P50 ${metric.p50Ms.toFixed(1)} · P95 ${metric.p95Ms.toFixed(1)} ms` : 'Chưa có dữ liệu';
    }
    download.disabled = data.samples.length === 0;
  }).catch(() => undefined).finally(() => { measuring = false; });
}, 1000);
listen(download, 'click', () => {
  const url = URL.createObjectURL(new Blob([JSON.stringify({ ...measurements.export(), ended: !session.state.joined }, null, 2)], { type: 'application/json' }));
  const link = document.createElement('a');link.href = url;link.download = 'sanser-livekit-voice-observations.json';link.click();
  window.setTimeout(() => URL.revokeObjectURL(url), 1000);
});
const stop = () => { token.value = '';element<HTMLInputElement>('password').value = '';cancelAdmission();session.leave();renderAccount(); };
const endPage = () => { stop();account.logout();choices = [];element('account-status').textContent = 'Đã rời bản thử. Đăng nhập lại để tiếp tục.';renderAccount(); };
window.addEventListener('pagehide', endPage);window.addEventListener('beforeunload', endPage);
const visibility = () => { if (document.visibilityState === 'hidden') stop(); };
document.addEventListener('visibilitychange', visibility);
export function destroy(): void {
  clearInterval(timer);clearInterval(membershipTimer);endPage();window.removeEventListener('pagehide', endPage);window.removeEventListener('beforeunload', endPage);
  document.removeEventListener('visibilitychange', visibility);
  for (const cleanup of uiCleanup) cleanup();unsubscribe();
}
import.meta.hot?.dispose(destroy);
