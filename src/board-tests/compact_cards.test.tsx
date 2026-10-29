// @vitest-environment jsdom
import { board, renderBoard } from "./fixtures";
import { fireEvent, render, screen, within } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { TaskCard } from "../TaskCard";

function renderCard() {
  const onSelect = vi.fn(), onViewPrompt = vi.fn(), onFlowRecover = vi.fn();
  render(<TaskCard task={board.storedTask} agentStatus="unknown" onChanged={vi.fn()} onRemoved={vi.fn()}
    onEditTask={vi.fn()} onDispatchTask={vi.fn()} onComplete={vi.fn()} onStart={vi.fn()}
    onSelect={onSelect} onViewPrompt={onViewPrompt} onFlowRecover={onFlowRecover} />);
  return { onSelect, onViewPrompt, onFlowRecover };
}

it.each(["pending", "unknown", "failed"])("%s 卡片的提示词入口打开只读表单，不修改或派发任务", async state => {
  board.storedTask.flow = { kind: "start", state, session_id: null, operation_id: "flow", error: "", pane_id: null };
  await renderBoard("in_progress");
  fireEvent.click(screen.getByRole("button", { name: "查看提示词" }));
  const dialog = await screen.findByRole("dialog", { name: "查看提示词" });
  expect(within(dialog).getByRole("textbox", { name: "提示词" }).getAttribute("contenteditable")).toBe("false");
  expect(within(dialog).getByText("请修复拖放派发")).toBeTruthy();
  expect(within(dialog).queryByRole("button", { name: "保存" })).toBeNull();
  expect(vi.mocked(invoke).mock.calls.some(([command]) => ["update_task", "start_task", "dispatch_to_herdr", "codex_model_catalog"].includes(command))).toBe(false);
});

it("未决卡片折叠恢复操作但保留可用提示词入口，展开和恢复不触发选卡", () => {
  board.storedTask.status = "in_progress";
  board.storedTask.flow = { kind: "start", state: "unknown", session_id: null, operation_id: "flow", error: "timeout", pane_id: null };
  const callbacks = renderCard();
  expect(screen.queryByRole("button", { name: "核实启动结果" })).toBeNull();
  expect(screen.getByRole("button", { name: "标记完成" }).matches(":disabled")).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: "查看提示词" }));
  expect(callbacks.onViewPrompt).toHaveBeenCalledOnce();
  fireEvent.click(screen.getByRole("button", { name: "需核实" }));
  fireEvent.click(screen.getByRole("button", { name: "核实启动结果" }));
  expect(callbacks.onFlowRecover).toHaveBeenCalledOnce();
  expect(callbacks.onSelect).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "需核实" }));
  expect(screen.queryByRole("button", { name: "核实启动结果" })).toBeNull();
});

it("模型详情可展开并收起，操作不切换当前终端", () => {
  board.storedTask.status = "in_progress";
  board.storedTask.codex_settings = { model: "test-model", effort: "high" };
  const { onSelect } = renderCard();
  const details = screen.getByRole("button", { name: "任务详情" });
  expect(details.getAttribute("aria-expanded")).toBe("false");
  fireEvent.click(details);
  expect(details.getAttribute("aria-expanded")).toBe("true");
  expect(screen.getByText("test-model · high").closest("[hidden]")).toBeNull();
  fireEvent.click(details);
  expect(screen.getByText("test-model · high").closest("[hidden]")).not.toBeNull();
  expect(onSelect).not.toHaveBeenCalled();
});
