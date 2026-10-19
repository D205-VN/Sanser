import { get, writable } from 'svelte/store';
import { groupVoiceStatus, setGroupVoice, type GroupVoiceScope, type GroupVoiceStatus } from '../lib/platform';

export interface GroupVoiceState extends GroupVoiceStatus { busy: boolean; available: boolean; error: string | null }
const statusReadError = 'Unable to verify voice status. The microphone may still be active; use Mute mic or Leave voice to stop it.';
const initial = (): GroupVoiceState => ({ joined: false, microphone: false, deafened: false, capture: false, code: 'idle', busy: false, available: false, error: null });
// In-memory consent belongs to this native group/session only. Never restore a microphone preference.
export function createGroupVoice() {
  const state = writable<GroupVoiceState>(initial());
  let scope: GroupVoiceScope | null = null;
  let generation = 0;
  let polling = false;
  let sending = false;
  let commandTail: Promise<void> = Promise.resolve();
  function send(target: GroupVoiceScope, next: typeof intent): Promise<void> {
    const operation = commandTail.then(() => setGroupVoice(target, next));
    commandTail = operation.catch(() => undefined);
    return operation;
  }
  let intent = { joined: false, microphone: false, deafened: false };
  async function configure(next: typeof intent): Promise<void> {
    if (!scope || sending || get(state).code === 'unsupported') return;
    const target = scope; const epoch = generation;
    sending = true;
    state.update((value) => ({ ...value, busy: true, error: null }));
    try {
      await send(target, next);
      if (epoch !== generation) return;
      intent = { ...next };
      state.update((value) => ({ ...value, ...next, capture: next.microphone && value.capture, code: next.microphone ? 'starting' : next.joined ? 'listening' : 'idle' }));
      await refresh();
    } catch (error) {
      if (epoch === generation) state.update((value) => ({ ...value, error: error instanceof Error ? error.message : 'Unable to change group voice' }));
    } finally {
      sending = false;
      if (epoch === generation) state.update((value) => ({ ...value, busy: false }));
    }
  }
  async function refresh(): Promise<void> {
    if (!scope || polling) return;
    polling = true; const target = scope; const epoch = generation;
    try {
      const observed = await groupVoiceStatus(target);
      if (epoch === generation) {
        // Native state is authoritative about permission and device errors.
        const failed = ['permission-denied', 'no-device', 'unavailable', 'stopped', 'unsupported'].includes(observed.code);
        state.update((value) => ({ ...value, ...observed, microphone: observed.microphone && !failed, capture: observed.capture && !failed, available: observed.code !== 'unsupported', error: value.error === statusReadError ? null : value.error }));
        if (failed) intent.microphone = false;
      }
    } catch {
      // Losing the status pipe does not stop capture. Keep the last confirmed
      // consent/capture state so Mute and Leave remain available.
      if (epoch === generation) state.update((value) => ({ ...value, available: false, error: value.joined ? statusReadError : null }));
    } finally { polling = false; }
  }
  function attach(next: GroupVoiceScope): void {
    generation++; scope = next; intent = { joined: false, microphone: false, deafened: false }; state.set(initial());
  }
  async function detach(): Promise<void> {
    const previous = scope;
    generation++; scope = null; intent = { joined: false, microphone: false, deafened: false }; state.set(initial());
    if (previous) await send(previous, { joined: false, microphone: false, deafened: false }).catch(() => undefined);
  }
  return { subscribe: state.subscribe, attach, detach, refresh,
    join: () => configure({ joined: true, microphone: false, deafened: false }),
    leave: () => configure({ joined: false, microphone: false, deafened: false }),
    toggleMicrophone: () => intent.joined ? configure({ joined: true, microphone: !intent.microphone, deafened: false }) : Promise.resolve(),
    toggleDeafen: () => intent.joined ? configure({ joined: true, microphone: false, deafened: !get(state).deafened }) : Promise.resolve()
  };
}
