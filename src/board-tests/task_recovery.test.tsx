// @vitest-environment jsdom
import { board, bindTask, project, renderBoard } from "./fixtures";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { TaskSummaryBoard } from "../TaskSummaryBoard";

function unknownDispatch() {
  bindTask();
  board.storedTask.binding!.state = "unknown";
  board.storedTask.binding!.operation = { id: "manual-op", prompt: "original", created_at: "2026-11-07", target: board.storedTask.binding!.target! };
  const original = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation(async (name, args, options) => {
    if (name === "board_tasks") return [board.storedTask];
    if (name === "task_recovery_status") return [{ task_id: board.storedTask.id, operation_id: "manual-op", state: "needs_attention", attempt: 6, error: "dispatch.verify_required" }];
    return original(name, args, options);
  });
}

it("opens an unknown manual dispatch from Todo without changing its business status", async () => {
  unknownDispatch();
  await renderBoard();
  fireEvent.click(screen.getByText(board.storedTask.name, { selector: ".task-name" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("terminal_open", expect.objectContaining({ taskId: board.storedTask.id })));
  expect(screen.queryByRole("complementary", { name: "编辑任务" })).toBeNull();
  expect(board.storedTask.status).toBe("todo");
});

it("opens the same unconfirmed original terminal from All Tasks", async () => {
  unknownDispatch();
  render(<TaskSummaryBoard projects={[project]} />);
  fireEvent.click(await screen.findByText(board.storedTask.name, { selector: ".task-name" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("terminal_open", expect.objectContaining({ taskId: board.storedTask.id })));
  expect(board.storedTask.status).toBe("todo");
});

it("restarts an exhausted dispatch-only recovery using its current operation", async () => {
  unknownDispatch();
  await renderBoard();
  fireEvent.click(await screen.findByRole("button", { name: "重新自动核验" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("retry_task_recovery", { taskId: board.storedTask.id, operationId: "manual-op" }));
  expect(vi.mocked(invoke).mock.calls.some(([name]) => name === "start_task" || name === "resolve_dispatch" || name === "dispatch_to_herdr")).toBe(false);
});
