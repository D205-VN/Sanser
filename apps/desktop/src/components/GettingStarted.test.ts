import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import GettingStarted from './GettingStarted.svelte';

const base = { hasOtherDevice: false, hostReachable: false, connected: false, hostAlwaysReady: false };
beforeEach(() => localStorage.clear());
afterEach(cleanup);

it('ticks steps from live state and links to the right page', async () => {
  const navigate = vi.fn();
  render(GettingStarted, { ...base, hasOtherDevice: true, navigate });
  expect(screen.getByText('1 of 3 done')).toBeInTheDocument();
  await fireEvent.click(screen.getByRole('button', { name: 'Open Host' }));
  expect(navigate).toHaveBeenCalledWith('host');
  await fireEvent.click(screen.getByRole('button', { name: 'Open Settings' }));
  expect(navigate).toHaveBeenCalledWith('settings');
});

it('remembers the first connection and disappears once the basics are done', async () => {
  const view = render(GettingStarted, { ...base, hasOtherDevice: true, hostReachable: true, navigate: vi.fn() });
  expect(screen.getByText('2 of 3 done')).toBeInTheDocument();
  await view.rerender({ ...base, hasOtherDevice: true, hostReachable: true, connected: true, navigate: vi.fn() });
  expect(screen.queryByRole('heading', { name: 'Get started' })).not.toBeInTheDocument();
  cleanup();
  // The session has ended, but the first connection still counts.
  render(GettingStarted, { ...base, hasOtherDevice: true, hostReachable: true, navigate: vi.fn() });
  expect(screen.queryByRole('heading', { name: 'Get started' })).not.toBeInTheDocument();
});

it('stays hidden after the user hides it', async () => {
  render(GettingStarted, { ...base, navigate: vi.fn() });
  await fireEvent.click(screen.getByRole('button', { name: 'Hide guide' }));
  expect(screen.queryByRole('heading', { name: 'Get started' })).not.toBeInTheDocument();
  cleanup();
  render(GettingStarted, { ...base, navigate: vi.fn() });
  expect(screen.queryByRole('heading', { name: 'Get started' })).not.toBeInTheDocument();
});
