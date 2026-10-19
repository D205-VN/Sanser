import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
const mocks = vi.hoisted(() => ({ status: vi.fn(), configure: vi.fn() }));
vi.mock('../lib/platform', () => ({ windowsUacStatus: mocks.status, configureWindowsUac: mocks.configure }));
import WindowsUac from './WindowsUac.svelte';
beforeEach(() => {
  vi.resetAllMocks();
  mocks.status.mockResolvedValue({ available: true, enabled: false, ready: false });
  mocks.configure.mockResolvedValue({ available: true, enabled: true, ready: true });
});
afterEach(cleanup);
it('requires an explicit action before requesting administrator approval', async () => {
  render(WindowsUac);
  const button = await screen.findByRole('button', { name: 'Enable UAC support' });
  await waitFor(() => expect(button).toBeEnabled());
  expect(mocks.configure).not.toHaveBeenCalled();
  await fireEvent.click(button);
  await waitFor(() => expect(mocks.configure).toHaveBeenCalledWith(true));
  expect(await screen.findByText(/next connection will use it automatically/)).toBeInTheDocument();
});
it('does not replace the privileged helper while a host session is running', async () => {
  render(WindowsUac, { sessionRunning: true });
  const button = await screen.findByRole('button', { name: 'Enable UAC support' });
  expect(button).toBeDisabled();
  expect(mocks.configure).not.toHaveBeenCalled();
});
it('keeps UAC disabled when the administrator cancels setup', async () => {
  mocks.configure.mockRejectedValue(new Error('Administrator approval cancelled'));
  render(WindowsUac);
  const button = await screen.findByRole('button', { name: 'Enable UAC support' });
  await waitFor(() => expect(button).toBeEnabled());
  await fireEvent.click(button);
  expect(await screen.findByRole('alert')).toHaveTextContent('Administrator approval cancelled');
  expect(screen.queryByRole('button', { name: 'Disable UAC support' })).not.toBeInTheDocument();
});
it('allows removing enabled UAC support', async () => {
  mocks.status.mockResolvedValue({ available: true, enabled: true, ready: true });
  mocks.configure.mockResolvedValue({ available: true, enabled: false, ready: false });
  render(WindowsUac);
  await fireEvent.click(await screen.findByRole('button', { name: 'Disable UAC support' }));
  await waitFor(() => expect(mocks.configure).toHaveBeenCalledWith(false));
});
