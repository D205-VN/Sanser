import { render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { ConnectionSession, RuntimeStatus } from '../lib/types';
import { connection } from '../stores/connection';
import { session } from '../stores/session';
import GuestSession from './GuestSession.svelte';

vi.mock('../lib/platform', async (original) => ({ ...(await original<typeof import('../lib/platform')>()), stopEngine: vi.fn(() => Promise.resolve()) }));
// The remote-desktop view itself is covered by ActiveSession.test.ts.
vi.mock('./ActiveSession.svelte', () => ({ default: () => undefined }));
vi.mock('../stores/presence', () => ({ presence: { stop: vi.fn(() => Promise.resolve()), subscribe: (run: (value: unknown) => void) => { run({}); return () => undefined; } } }));

const runtime = { capabilities: { desktopShell: { state: 'unavailable' } } } as unknown as RuntimeStatus;

beforeEach(() => { vi.useFakeTimers(); });
afterEach(() => { vi.useRealTimers(); connection.clear(); vi.restoreAllMocks(); });

it('returns to the sign-in screen by itself when the host ends the support session', async () => {
  const endGuest = vi.spyOn(session, 'endGuest');
  connection.begin({ id: 'guest-session', status: 'disconnected' } as ConnectionSession);
  render(GuestSession, { runtime });
  expect(screen.getByText('The support session has ended.')).toBeInTheDocument();
  expect(screen.getByText('Returning to the sign-in screen…')).toBeInTheDocument();
  expect(endGuest).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(3_000);
  expect(endGuest).toHaveBeenCalledTimes(1);
});

it('does not leave while the support session is still waiting or running', async () => {
  const endGuest = vi.spyOn(session, 'endGuest');
  connection.begin({ id: 'guest-session', status: 'pending' } as ConnectionSession);
  render(GuestSession, { runtime });
  await vi.advanceTimersByTimeAsync(10_000);
  expect(endGuest).not.toHaveBeenCalled();
});
