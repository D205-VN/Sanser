import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
const mocks = vi.hoisted(() => ({ displays: vi.fn(), select: vi.fn() }));
vi.mock('../lib/platform', () => ({ sessionDisplays: mocks.displays, selectSessionDisplay: mocks.select }));
import SessionDisplays from './SessionDisplays.svelte';
const two = { available: true, selected: 0, displays: [
  { index: 0, width: 2560, height: 1440, primary: true },
  { index: 1, width: 1080, height: 1920, primary: false }
] };
beforeEach(() => {
  vi.resetAllMocks();
  mocks.displays.mockResolvedValue(two);
  mocks.select.mockResolvedValue(undefined);
});
afterEach(cleanup);
it('lists the host displays and requests a switch for this session only', async () => {
  render(SessionDisplays, { sessionId: 'session-a' });
  const second = await screen.findByRole('button', { name: 'Display 2 · 1080×1920' });
  expect(screen.getByRole('button', { name: 'Display 1 · 2560×1440 · Main' })).toHaveAttribute('aria-pressed', 'true');
  await fireEvent.click(second);
  expect(mocks.select).toHaveBeenCalledExactlyOnceWith('session-a', 1);
  expect(screen.getByRole('status')).toHaveTextContent('Switching…');
  await fireEvent.click(second);
  expect(mocks.select).toHaveBeenCalledTimes(1);
});
it('does not offer a picker for a single display or an older host', async () => {
  mocks.displays.mockResolvedValue({ available: true, selected: 0, displays: [two.displays[0]] });
  render(SessionDisplays, { sessionId: 'session-a' });
  await waitFor(() => expect(mocks.displays).toHaveBeenCalled());
  expect(screen.queryByRole('group', { name: 'Host displays' })).not.toBeInTheDocument();
  cleanup();
  mocks.displays.mockResolvedValue({ available: false, selected: 0, displays: [] });
  render(SessionDisplays, { sessionId: 'session-a' });
  await waitFor(() => expect(mocks.displays).toHaveBeenCalledTimes(2));
  expect(screen.queryByRole('group', { name: 'Host displays' })).not.toBeInTheDocument();
});
it('labels legacy connections as single-display without polling the engine', () => {
  render(SessionDisplays, { sessionId: 'session-a', legacy: true });
  expect(screen.getByText(/only the host's main display/)).toBeInTheDocument();
  expect(mocks.displays).not.toHaveBeenCalled();
});
it('shows a rejected switch and lets the viewer try again', async () => {
  mocks.select.mockRejectedValueOnce(new Error('The host does not offer this display'));
  render(SessionDisplays, { sessionId: 'session-a' });
  await fireEvent.click(await screen.findByRole('button', { name: 'Display 2 · 1080×1920' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('The host does not offer this display');
  expect(screen.getByRole('button', { name: 'Display 2 · 1080×1920' })).toBeEnabled();
});
