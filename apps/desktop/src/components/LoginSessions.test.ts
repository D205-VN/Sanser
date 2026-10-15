import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';
import LoginSessions from './LoginSessions.svelte';
import type { LoginSession } from '../lib/types';

afterEach(cleanup);
const session = (id: string, extra: Partial<LoginSession> = {}): LoginSession => ({
  id, deviceName: 'Studio PC', platform: 'windows-x86_64', current: false,
  createdAt: '2026-10-14T07:00:00Z', lastSeenAt: '2026-10-15T08:00:00Z', expiresAt: '2026-10-22T08:00:00Z', ...extra
});

it('keeps different computers with identical names and shows the current sign-in first', () => {
  const { container } = render(LoginSessions, { sessions: [session('a'), session('current', { current: true }), session('b')], busy: false, onRevoke: vi.fn() });
  expect(container.querySelector('.login-session-list strong')).toHaveTextContent('This sign-in');
  expect(screen.getAllByRole('button', { name: 'Sign out' })).toHaveLength(2);
  expect(screen.getAllByText('Studio PC')).toHaveLength(3);
  expect(container.textContent).not.toContain('2026-10-15T08:00:00Z');
});

it('folds anonymous historical sign-ins without merging them or revoking the current session', async () => {
  const revoke = vi.fn().mockResolvedValue(undefined);
  const old = { deviceName: 'Sanser Desktop', platform: 'unknown' };
  const { container } = render(LoginSessions, { sessions: [session('current', { ...old, current: true }), session('old-a', old), session('old-b', old)], busy: false, onRevoke: revoke });
  const details = container.querySelector('details');
  expect(details).not.toHaveAttribute('open');
  expect(details?.querySelector('summary')).toHaveTextContent('(2)');
  expect(revoke).not.toHaveBeenCalled();
  const button = screen.getAllByRole('button', { name: 'Sign out', hidden: true })[0];
  if (!button) throw new Error('Missing historical sign-out action');
  await fireEvent.click(button);
  expect(revoke).toHaveBeenCalledWith('old-a');
});
