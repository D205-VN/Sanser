import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';
vi.mock('../stores/preferences', async () => {
  const { writable } = await import('svelte/store');
  return { CONFIGURED_SERVER_URL: '', preferences: writable({ serverUrl: '', customServer: false }) };
});
import Welcome from './Welcome.svelte';
afterEach(cleanup);

it('allows setting a server when the build has no configured default', () => {
  render(Welcome);
  expect(screen.getByLabelText('Sanser server URL')).toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Sign in' })).toBeDisabled();
});
