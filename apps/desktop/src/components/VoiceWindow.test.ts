import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import VoiceWindow from './VoiceWindow.svelte';

const fake = vi.hoisted(() => {
  const stop = vi.fn();
  class Room {
    static last: Room;
    connect = vi.fn(() => Promise.resolve());
    disconnect = vi.fn(() => Promise.resolve());
    stopMicrophones = vi.fn();
    setDeafened = vi.fn();
    startAudio = vi.fn(() => Promise.resolve());
    createMicrophone = vi.fn(() => Promise.resolve({ publish: () => Promise.resolve(), unpublish: () => Promise.resolve(), stop }));
    signal: (event: string) => void = () => undefined;
    onSignal(callback: (event: string) => void): void { this.signal = callback; }
    constructor() { Room.last = this; }
  }
  return { Room, stop, close: vi.fn(() => Promise.resolve()), invoke: vi.fn() };
});
vi.mock('../../../voice-lab/src/livekit', () => ({ LiveKitVoiceRoom: fake.Room }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: fake.invoke }));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ close: fake.close }) }));

beforeEach(() => {
  vi.clearAllMocks();
  fake.invoke.mockResolvedValue({ url: 'wss://voice.example/', token: 'abc.def.ghi', kind: 'host' });
});
afterEach(() => { vi.restoreAllMocks(); });

it('consumes one private grant, joins listen-only, and stops capture before closing', async () => {
  const view = render(VoiceWindow);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Unmute mic' })).toBeEnabled());
  expect(fake.invoke).toHaveBeenNthCalledWith(1, 'take_voice_window_grant');
  await waitFor(() => expect(fake.invoke).toHaveBeenNthCalledWith(2, 'voice_window_ready'));
  expect(fake.Room.last.createMicrophone).not.toHaveBeenCalled();
  await fireEvent.click(screen.getByRole('button', { name: 'Unmute mic' }));
  await waitFor(() => expect(screen.getByRole('button', { name: 'Mute mic' })).toBeEnabled());
  await fireEvent.click(screen.getByRole('button', { name: 'Deafen voice' }));
  expect(fake.stop).toHaveBeenCalled();
  await fireEvent.click(screen.getByRole('button', { name: 'Hear voice' }));
  expect(fake.Room.last.createMicrophone).toHaveBeenCalledTimes(1);
  await fireEvent.click(screen.getByRole('button', { name: 'Leave voice' }));
  await waitFor(() => expect(fake.close).toHaveBeenCalledTimes(1));
  expect(fake.Room.last.stopMicrophones).toHaveBeenCalled();
  view.unmount();
});

it('does not connect or open a microphone when a grant arrives after teardown', async () => {
  let resolve!: (value: unknown) => void;
  fake.invoke.mockReturnValue(new Promise((done) => { resolve = done; }));
  const view = render(VoiceWindow);
  const grant = { url: 'wss://voice.example/', token: 'abc.def.ghi', kind: 'client' };
  view.unmount(); resolve(grant);
  await waitFor(() => expect(grant.token).toBe(''));
  expect(fake.invoke).not.toHaveBeenCalledWith('voice_window_ready');
  expect(fake.close).not.toHaveBeenCalled();
});

it('never confirms readiness before connecting, or after a late connection following teardown', async () => {
  let resolve!: () => void;
  const connecting = vi.spyOn(fake.Room.prototype, 'onSignal');
  connecting.mockImplementation(function(this: InstanceType<typeof fake.Room>, callback: (event: string) => void) {
    this.signal = callback;
    this.connect.mockImplementation(() => new Promise<void>((done) => { resolve = done; }));
  });
  const view = render(VoiceWindow);
  await waitFor(() => expect(fake.Room.last.connect).toHaveBeenCalled());
  expect(fake.invoke).not.toHaveBeenCalledWith('voice_window_ready');
  view.unmount(); resolve();
  await waitFor(() => expect(fake.Room.last.disconnect).toHaveBeenCalled());
  expect(fake.invoke).not.toHaveBeenCalledWith('voice_window_ready');
});

it('stops its room when the native readiness acknowledgement fails', async () => {
  fake.invoke.mockImplementation((command: string) => command === 'voice_window_ready'
    ? Promise.reject(new Error('closed'))
    : Promise.resolve({ url: 'wss://voice.example/', token: 'abc.def.ghi', kind: 'host' }));
  const view = render(VoiceWindow);
  await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent('Unable to open voice.'));
  expect(fake.Room.last.stopMicrophones).toHaveBeenCalled();
  expect(fake.Room.last.disconnect).toHaveBeenCalled();
  expect(fake.Room.last.createMicrophone).not.toHaveBeenCalled();
  view.unmount();
});

it('network reconnection clears microphone consent and never restores it', async () => {
  const view = render(VoiceWindow);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Unmute mic' })).toBeEnabled());
  await fireEvent.click(screen.getByRole('button', { name: 'Unmute mic' }));
  await waitFor(() => expect(screen.getByRole('button', { name: 'Mute mic' })).toBeEnabled());
  fake.Room.last.signal('reconnecting');
  await waitFor(() => expect(screen.getByRole('button', { name: 'Unmute mic' })).toBeDisabled());
  expect(fake.stop).toHaveBeenCalled();
  expect(fake.Room.last.createMicrophone).toHaveBeenCalledTimes(1);
  view.unmount();
});

it('the combined host Stop action stops mic before asking Sanser to stop sharing', async () => {
  const view = render(VoiceWindow);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Unmute mic' })).toBeEnabled());
  await fireEvent.click(screen.getByRole('button', { name: 'Stop sharing' }));
  await waitFor(() => expect(fake.invoke).toHaveBeenCalledWith('stop_voice_sharing'));
  expect(fake.Room.last.stopMicrophones).toHaveBeenCalled();
  expect(fake.Room.last.createMicrophone).not.toHaveBeenCalled();
  view.unmount();
});
