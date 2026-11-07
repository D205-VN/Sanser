import { get, writable } from 'svelte/store';
import { setViewerAudio } from '../lib/platform';

// Local session preference: muting this viewer never changes the host or peers.
export const viewerAudio = writable<Record<string, boolean>>({});
export function prepareViewerAudio(id: string, muted: boolean): boolean {
  const current = get(viewerAudio);
  const existing = current[id];
  if (existing !== undefined) return existing;
  viewerAudio.set(Object.fromEntries([...Object.entries(current).slice(-7), [id, muted]]));
  return muted;
}
export async function muteViewerAudio(id: string, muted: boolean): Promise<void> {
  await setViewerAudio(id, muted);
  viewerAudio.update((current) => ({ ...current, [id]: muted }));
}

// A receiver started without audio cannot be unmuted over the live control path.
export function forgetViewerAudio(id: string): void {
  viewerAudio.update((current) => Object.fromEntries(Object.entries(current).filter(([key]) => key !== id)));
}
