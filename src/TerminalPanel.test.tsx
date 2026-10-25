// @vitest-environment jsdom
import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { TerminalPanel } from './TerminalPanel';
import type { Task } from './types';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), Channel: class { onmessage = () => {}; } }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
const task = { id: 'a', name: '任务 A', binding: { state: 'valid', target: { pane_id: 'p1' } } } as Task;
beforeEach(() => {
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({ left: 600, top: 48, right: 1000, bottom: 700, width: 400, height: 652 } as DOMRect);
  vi.mocked(invoke).mockReset().mockImplementation(async (_command, args) => (args as { connectionId: string })?.connectionId);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

it('reports the native rectangle and hides for dialogs without releasing the Agent', async () => {
  render(<TerminalPanel task={task} />);
  await screen.findByText('Herdr 已打开');
  expect(invoke).toHaveBeenCalledWith('terminal_open', expect.objectContaining({ bounds: expect.objectContaining({ x: 600, y: 48, width: 400, visible: true }) }));
  const dialog = document.createElement('div'); dialog.setAttribute('role', 'dialog');
  act(() => { document.body.append(dialog); });
  await waitFor(() => expect(invoke).toHaveBeenLastCalledWith('terminal_bounds', expect.objectContaining({ bounds: expect.objectContaining({ visible: false }) })));
  act(() => { dialog.remove(); });
  await waitFor(() => expect(invoke).toHaveBeenLastCalledWith('terminal_bounds', expect.objectContaining({ bounds: expect.objectContaining({ visible: true }) })));
  expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === 'terminal_close')).toHaveLength(0);
  expect(document.querySelector('.xterm')).toBeNull();
});

it('ignores old close notifications and late readiness after selecting another task', async () => {
  const channels: { onmessage: (packet: unknown) => void }[] = [];
  let ready!: () => void;
  vi.mocked(invoke).mockImplementation(async (name, args) => {
    if (name === 'terminal_open') {
      channels.push((args as { output: typeof channels[number] }).output);
      if (channels.length === 1) await new Promise<void>(resolve => { ready = resolve; });
    }
  });
  const view = render(<TerminalPanel task={task} />);
  await screen.findByText('连接中…');
  const oldId = (vi.mocked(invoke).mock.calls[0][1] as { connectionId: string }).connectionId;
  view.rerender(<TerminalPanel task={{ ...task, id: 'b', name: '任务 B' }} />);
  await screen.findByText('Herdr 已打开');
  await act(async () => { channels[0].onmessage({ kind: 'closed', message: 'old failure' }); ready(); });
  expect(screen.getByText('Herdr 已打开')).toBeTruthy();
  expect(screen.queryByRole('alert')).toBeNull();
  expect(invoke).toHaveBeenCalledWith('terminal_close', { connectionId: oldId });
});

it('keeps the native viewer present when WebView reports occlusion', async () => {
  render(<TerminalPanel task={task} />);
  await screen.findByText('Herdr 已打开');
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('terminal_bounds', expect.anything()));
  const hidden = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden');
  await act(async () => { document.dispatchEvent(new Event('visibilitychange')); });
  const updates = vi.mocked(invoke).mock.calls.filter(([name]) => name === 'terminal_bounds');
  expect(updates.every(([, args]) => (args as { bounds: { visible: boolean } }).bounds.visible)).toBe(true);
  hidden.mockRestore();
});

it('serializes bounds updates and sends the latest layout after an in-flight update', async () => {
  let finish!: () => void; let updates = 0;
  vi.mocked(invoke).mockImplementation(async name => {
    if (name === 'terminal_bounds' && ++updates === 1) await new Promise<void>(resolve => { finish = resolve; });
  });
  render(<TerminalPanel task={task} />);
  await waitFor(() => expect(updates).toBe(1));
  const dialog = document.createElement('div'); dialog.className = 'dialog-overlay';
  act(() => { document.body.append(dialog); });
  await act(async () => { finish(); });
  await waitFor(() => expect(invoke).toHaveBeenLastCalledWith('terminal_bounds', expect.objectContaining({ bounds: expect.objectContaining({ visible: false }) })));
  act(() => { dialog.remove(); });
});
