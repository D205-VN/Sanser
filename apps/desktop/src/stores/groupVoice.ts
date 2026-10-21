import { get, writable } from 'svelte/store';
import { groupVoiceStatus, setGroupVoice, type GroupVoiceScope, type GroupVoiceStatus } from '../lib/platform';

export interface GroupVoiceState extends GroupVoiceStatus { busy: boolean; available: boolean; error: string | null }
const statusReadError = 'Unable to verify voice status. The microphone may still be active; use Mute mic or Leave voice to stop it.';
const initial = (): GroupVoiceState => ({ joined: false, microphone: false, deafened: false, capture: false, code: 'idle', busy: false, available: false, error: null });
type Consent = Pick<GroupVoiceStatus, 'joined' | 'microphone' | 'deafened'>;
// Native consent belongs to the current engine, never saved preferences. The
// floating native controls remain available when a page observer is detached.
export function createGroupVoice() {
  const state = writable<GroupVoiceState>(initial());
  let scope: GroupVoiceScope | null = null;
  let generation = 0, revision = 0;
  let polling = false, sending = false, floating = false;
  let pending: { consent: Consent; deadline: number } | null = null;
  let intent: Consent = { joined: false, microphone: false, deafened: false };
  let commandTail: Promise<void> = Promise.resolve();
  function send(target: GroupVoiceScope, next: Consent): Promise<void> {
    const operation = commandTail.then(() => setGroupVoice(target, next));
    commandTail = operation.catch(() => undefined);
    return operation;
  }
  async function configure(next: Consent): Promise<void> {
    if (!scope || sending || pending || get(state).code === 'unsupported') return;
    const target = scope, epoch = generation;
    ++revision; sending = true;
    state.update((value) => ({ ...value, busy: true, error: null }));
    try {
      await send(target, next);
      if (epoch !== generation) return;
      intent = { ...next }; pending = { consent: next, deadline: Date.now() + 3000 };
      state.update((value) => {
        const revoking = !next.joined || (!next.microphone && value.microphone);
        return { ...value, ...(revoking ? {} : next), capture: revoking ? value.capture : false, code: !next.joined ? 'leaving' : revoking ? 'changing' : next.microphone ? 'starting' : 'joining' };
      });
      sending = false;
      await refresh();
    } catch (error) {
      if (epoch === generation) state.update((value) => ({ ...value, error: error instanceof Error ? error.message : 'Unable to change group voice' }));
    } finally {
      sending = false;
      if (epoch === generation) state.update((value) => ({ ...value, busy: pending !== null }));
    }
  }
  async function refresh(): Promise<void> {
    if (!scope || polling) return;
    polling = true; const target = scope, epoch = generation, ticket = revision;
    try {
      const observed = await groupVoiceStatus(target);
      if (epoch !== generation || ticket !== revision) return;
      floating = observed.floatingControls === true;
      const failed = ['permission-denied', 'no-device', 'unavailable', 'stopped', 'unsupported'].includes(observed.code);
      const matches = pending && (['joined', 'microphone', 'deafened'] as const).every((key) => observed[key] === pending?.consent[key]);
      // Writing stdin is not an acknowledgement. EngineOutput can still return
      // the previous idle record until the native worker processes the command.
      if (pending && !matches && !failed && Date.now() < pending.deadline) {
        state.update((value) => ({ ...value, busy: true, available: true }));
        return;
      }
      const unconfirmed = pending !== null && !matches && !failed;
      pending = null;
      intent = { joined: observed.joined, microphone: observed.microphone && !failed, deafened: observed.deafened };
      state.update((value) => ({ ...value, ...observed, ...intent, capture: observed.capture && !failed, busy: sending, available: observed.code !== 'unsupported', error: unconfirmed ? 'The voice change was not confirmed. Check the floating voice controls or try again.' : value.error === statusReadError ? null : value.error }));
    } catch {
      if (epoch === generation) state.update((value) => ({ ...value, available: false, error: value.joined ? statusReadError : null }));
    } finally { polling = false; }
  }
  function attach(next: GroupVoiceScope): void {
    generation++; ++revision; scope = next; pending = null; floating = false; intent = { joined: false, microphone: false, deafened: false }; state.set(initial());
  }
  function detach(): Promise<void> {
    const previous = scope, keepNativeControls = floating;
    generation++; ++revision; scope = null; pending = null;
    intent = { joined: false, microphone: false, deafened: false }; state.set(initial());
    // Keep a call only if this engine actually has visible native controls.
    // Older sidecars still leave after any in-flight unmute on navigation.
    if (previous && !keepNativeControls) return send(previous, { joined: false, microphone: false, deafened: false }).catch(() => undefined);
    return Promise.resolve();
  }
  return { subscribe: state.subscribe, attach, detach, refresh,
    join: () => configure({ joined: true, microphone: false, deafened: false }),
    leave: () => configure({ joined: false, microphone: false, deafened: false }),
    toggleMicrophone: () => intent.joined ? configure({ joined: true, microphone: !intent.microphone, deafened: false }) : Promise.resolve(),
    toggleDeafen: () => intent.joined ? configure({ joined: true, microphone: false, deafened: !get(state).deafened }) : Promise.resolve()
  };
}
