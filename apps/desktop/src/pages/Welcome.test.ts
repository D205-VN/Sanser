import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';
const auth = vi.hoisted(() => ({ login: vi.fn(), register: vi.fn(), initialize: vi.fn(), logout: vi.fn().mockResolvedValue(undefined) }));
vi.mock('../stores/session', async () => {
  const { writable } = await import('svelte/store');
  return { session: { ...writable({ busy: false, error: null }), ...auth } };
});
import Welcome from './Welcome.svelte';
afterEach(() => { cleanup(); vi.clearAllMocks(); });

it.each([false, true])('passes the remember-sign-in choice (%s) with the original login form', async (remember) => {
  const { container } = render(Welcome);
  const checkbox = screen.getByRole('checkbox', { name: 'Keep me signed in' });
  expect(checkbox).not.toBeChecked();
  if (remember) await fireEvent.click(checkbox);
  await fireEvent.input(screen.getByLabelText('Email'), { target: { value: 'test@example.test' } });
  await fireEvent.input(screen.getByLabelText('Password'), { target: { value: 'Test-password123' } });
  const endpoint = screen.queryByLabelText('Sanser server URL');
  if (endpoint) await fireEvent.input(endpoint, { target: { value: 'https://sanser.example' } });
  const form = container.querySelector('form');
  if (!form) throw new Error('Missing form');
  await fireEvent.submit(form);
  await waitFor(() => expect(auth.login).toHaveBeenCalledWith(expect.any(String), 'test@example.test', 'Test-password123', remember));
});

it('allows showing a password without submitting the form', async () => {
  render(Welcome);
  const input = screen.getByLabelText('Password');
  await fireEvent.input(input, { target: { value: 'a-private-passphrase' } });
  await fireEvent.click(screen.getByRole('button', { name: 'Show password' }));
  expect(input).toHaveAttribute('type', 'text');
  expect(input).toHaveValue('a-private-passphrase');
  await fireEvent.click(screen.getByRole('button', { name: 'Hide password' }));
  expect(input).toHaveAttribute('type', 'password');
  expect(auth.login).not.toHaveBeenCalled();
});

it('rejects mismatched registration passwords before calling the server', async () => {
  const { container } = render(Welcome);
  await fireEvent.click(screen.getByRole('tab', { name: 'Create account' }));
  await fireEvent.input(screen.getByLabelText('Email'), { target: { value: 'test@example.test' } });
  await fireEvent.input(screen.getByLabelText('Password'), { target: { value: 'a-private-passphrase' } });
  await fireEvent.input(screen.getByLabelText('Confirm password'), { target: { value: 'different-passphrase' } });
  const endpoint = screen.queryByLabelText('Sanser server URL');
  if (endpoint) await fireEvent.input(endpoint, { target: { value: 'https://sanser.example' } });
  const form = container.querySelector('form');
  if (!form) throw new Error('Missing form');
  await fireEvent.submit(form);
  expect(screen.getByRole('alert')).toHaveTextContent('Passwords do not match');
  expect(auth.register).not.toHaveBeenCalled();
});
