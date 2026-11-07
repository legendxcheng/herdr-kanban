// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { DispatchRecovery } from "./DispatchRecovery";
import { resolveDispatch } from "./api";
import type { Task } from "./types";

vi.mock("./api", () => ({ resolveDispatch: vi.fn().mockResolvedValue(undefined) }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });
const task = {
  id: "task", binding: { state: "unknown", operation: {
    id: "operation", prompt: "original prompt", created_at: "2026-11-07T00:00:00Z", target: { pane_id: "w1:p1" },
  } },
} as Task;

it("allows an explicit sent decision with no note and without sending another prompt", async () => {
  const saved = vi.fn();
  render(<DispatchRecovery task={task} onClose={() => {}} onSaved={saved} />);
  const sent = screen.getByRole("button", { name: "确认已发送" });
  expect((sent as HTMLButtonElement).disabled).toBe(false);
  fireEvent.click(sent);
  await waitFor(() => expect(saved).toHaveBeenCalledOnce());
  expect(resolveDispatch).toHaveBeenCalledWith("task", "operation", true, "");
});

it("keeps a distinct not-sent decision and passes through an optional note", async () => {
  render(<DispatchRecovery task={task} onClose={() => {}} onSaved={() => {}} />);
  fireEvent.change(screen.getByLabelText("备注（可选）"), { target: { value: "Composer inspected" } });
  fireEvent.click(screen.getByRole("button", { name: "确认未发送" }));
  await waitFor(() => expect(resolveDispatch).toHaveBeenCalledWith("task", "operation", false, "Composer inspected"));
});
