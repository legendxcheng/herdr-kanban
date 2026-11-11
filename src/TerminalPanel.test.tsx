// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { TerminalPanel } from './TerminalPanel';
import type { Project, Task } from './types';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), Channel: class { onmessage = () => {}; } }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
const task = { id: 'a', name: '任务 A', binding: { state: 'valid', target: { pane_id: 'p1' } } } as Task;
beforeEach(() => {
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({ left: 600, top: 48, right: 1000, bottom: 700, width: 400, height: 652 } as DOMRect);
  vi.mocked(invoke).mockReset().mockImplementation(async (_command, args) => (args as { connectionId: string })?.connectionId);
});
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

it('reconnects the selected Workspace shell after a reset without creating or dispatching a task', async () => {
  vi.useFakeTimers();
  const channels: { onmessage: (packet: unknown) => void }[] = [];
  vi.mocked(invoke).mockImplementation(async (name, args) => {
    if (name === 'workspace_terminal_open') channels.push((args as { output: typeof channels[number] }).output);
  });
  const workspace = { id: 'remote-project', machine_id: 'remote', name: 'Remote' } as Project;
  const view = render(<TerminalPanel task={null} workspace={workspace} />);
  await act(async () => {});
  await act(async () => { channels[0].onmessage({ kind: 'closed', message: 'terminal.disconnected' }); });
  await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
  expect(channels).toHaveLength(2);
  expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === 'workspace_terminal_open').map(([, args]) => (args as { reconnect: boolean }).reconnect)).toEqual([false, true]);
  expect(screen.getByText('Herdr 已打开')).toBeTruthy();
  await act(async () => { channels[1].onmessage({ kind: 'closed', message: 'terminal.disconnected' }); });
  view.rerender(<TerminalPanel task={null} />);
  await act(async () => { await vi.advanceTimersByTimeAsync(300000); });
  expect(channels).toHaveLength(2);
  expect(vi.mocked(invoke).mock.calls.every(([name]) => ['workspace_terminal_open', 'terminal_close', 'terminal_bounds'].includes(name))).toBe(true);
  expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === 'workspace_terminal_open').every(([, args]) => (args as { projectId: string }).projectId === workspace.id)).toBe(true);
});

it('automatically reconnects a disconnected task at most six times and cancels old selection retries', async () => {
  vi.useFakeTimers();
  vi.mocked(invoke).mockImplementation(async name => { if (name === 'terminal_open') throw 'terminal.disconnected'; });
  const view = render(<TerminalPanel task={task} />);
  await act(async () => { await Promise.resolve(); });
  for (const delay of [2000, 5000, 10000, 30000, 60000]) {
    await act(async () => { await vi.advanceTimersByTimeAsync(delay); });
  }
  expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === 'terminal_open')).toHaveLength(6);
  await act(async () => { await vi.advanceTimersByTimeAsync(300000); });
  expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === 'terminal_open')).toHaveLength(6);
  view.rerender(<TerminalPanel task={{ ...task, id: 'b' }} />);
  await act(async () => { await Promise.resolve(); });
  view.rerender(<TerminalPanel task={null} />);
  await act(async () => { await vi.advanceTimersByTimeAsync(300000); });
  expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === 'terminal_open')).toHaveLength(7);
});

it('opens the original unknown task in the mode granted by the backend without confirming dispatch', async () => {
  vi.mocked(invoke).mockImplementation(async (name, args) => {
    if (name === 'terminal_open') (args as { output: { onmessage: (packet: unknown) => void } }).output.onmessage({ kind: 'mode', message: 'read_only' });
  });
  const unknown = { ...task, flow: { kind: 'start', state: 'unknown', operation_id: 'start' }, binding: { ...task.binding, state: 'unknown' } } as Task;
  const view = render(<TerminalPanel task={unknown} />);
  await screen.findByText('只读查看：派发结果待确认');
  expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === 'terminal_open')).toHaveLength(1);
  expect(vi.mocked(invoke).mock.calls.some(([name]) => name === 'resolve_dispatch' || name === 'start_task')).toBe(false);
  vi.mocked(invoke).mockImplementation(async () => undefined);
  view.rerender(<TerminalPanel task={task} />);
  await screen.findByText('Herdr 已打开');
});

it.each(['flow.session_unverified', 'flow.session_mismatch'])('retries only temporarily unidentified sessions: %s', async error => {
  vi.useFakeTimers();
  let opens = 0;
  vi.mocked(invoke).mockImplementation(async name => {
    if (name === 'terminal_open' && ++opens === 1) throw error;
  });
  render(<TerminalPanel task={task} />);
  await act(async () => {});
  expect(screen.getByRole('alert').textContent).toContain(error === 'flow.session_unverified' ? '暂未识别' : '不一致');
  await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
  expect(opens).toBe(error === 'flow.session_unverified' ? 2 : 1);
  if (error === 'flow.session_unverified') expect(screen.getByText('Herdr 已打开')).toBeTruthy();
  expect(vi.mocked(invoke).mock.calls.every(([name]) => ['terminal_open', 'terminal_close', 'terminal_bounds'].includes(name))).toBe(true);
});

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

it('offers verified Workspace recovery after identity changes without invoking task repair', async () => {
  let opens = 0;
  vi.mocked(invoke).mockImplementation(async name => {
    if (name === 'workspace_terminal_open' && ++opens === 1) throw 'binding.identity_changed';
  });
  const workspace = { id: 'project', name: 'game-h5', machine_id: 'remote', herdr_session: 'default', workspace_id: 'w9' } as Project;
  render(<TerminalPanel task={null} workspace={workspace} machineName="xc" />);
  const alert = await screen.findByRole('alert');
  expect(alert.textContent).toContain('无法安全恢复原来的普通终端');
  expect(alert.textContent).not.toContain('修复绑定');
  fireEvent.click(screen.getByRole('button', { name: '核实并重连' }));
  await screen.findByText('Herdr 已打开');
  expect(opens).toBe(2);
  expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === 'terminal_open' || name === 'repair_binding')).toHaveLength(0);
  expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === 'workspace_terminal_open').every(([, args]) => (args as { projectId: string }).projectId === workspace.id)).toBe(true);
});
