// @vitest-environment jsdom
import { board, project, agent } from "./board-tests/fixtures";
import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { BindingDialog } from "./BindingDialog";

it.each(["pending", "unknown"])("does not offer saving a binding during an unresolved %s operation", async state => {
  board.storedTask.flow = { session_id: null, operation_id: "start", kind: "start", state, error: "", pane_id: null };
  render(<BindingDialog task={board.storedTask} project={project} onClose={() => {}} onSaved={() => {}} />);
  await screen.findByRole("option", { name: /测试 Agent/ });
  fireEvent.change(screen.getByLabelText("独立 Agent"), { target: { value: agent.pane_id } });
  const save = screen.getByRole("button", { name: "保存绑定" }) as HTMLButtonElement;
  expect(save.disabled).toBe(true);
  fireEvent.click(save);
  expect(vi.mocked(invoke).mock.calls.some(([name]) => name === "repair_binding")).toBe(false);
  expect(screen.getByText("任务有未决操作，请先核实卡片上的启动或派发结果")).toBeTruthy();
});
