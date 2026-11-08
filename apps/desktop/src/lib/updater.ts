import { invoke } from '@tauri-apps/api/core';
import { check, Update } from '@tauri-apps/plugin-updater';

export async function checkForDesktopUpdates(): Promise<Update | null> {
  try {
    return await check({ timeout: 8_000 });
  } catch {
    // The native fallback creates the same Tauri Update resource, retaining
    // normal version comparison and pinned installer signature verification.
    const metadata = await invoke<ConstructorParameters<typeof Update>[0] | null>('check_update_via_github');
    return metadata ? new Update(metadata) : null;
  }
}
