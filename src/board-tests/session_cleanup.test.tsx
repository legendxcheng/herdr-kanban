// @vitest-environment jsdom
import "./fixtures";
import { render, screen, fireEvent, within, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { expect, it, vi } from "vitest";
import { SessionCleanupButton } from "../SessionCleanupButton";
it("全局清理先预览，取消不关闭，确认后展示逐项结果", async () => {
    const batch = { id: "batch", state: "preview", scopes: [{ machine: "Offline PC", workspace: "", error: "offline" }], items: [{ id: "i", machine: "Local", workspace: "Hidden", target: { pane_id: "w1:p1", status: "working", title: "Agent" }, state: "ready", error: "" }] };
    vi.mocked(invoke).mockImplementation(async (cmd) => {
        if (cmd === "preview_session_cleanup")
            return batch;
        if (cmd === "session_cleanup_history" || cmd === "session_resource_list")
            return [];
        if (cmd === "execute_session_cleanup")
            return { ...batch, state: "done", items: [{ ...batch.items[0], state: "closed" }] };
        return [];
    });
    render(<SessionCleanupButton />);
    fireEvent.click(screen.getByRole("button", { name: "清理未绑定会话" }));
    let dialog = await screen.findByRole("dialog");
    await within(dialog).findByText(/Hidden/);
    expect(within(dialog).getByText(/Offline PC/)).toBeTruthy();
    fireEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === "execute_session_cleanup")).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "清理未绑定会话" }));
    dialog = await screen.findByRole("dialog");
    const confirm = await within(dialog).findByRole("button", { name: "确认关闭这些会话" });
    await waitFor(() => expect(confirm.matches(":disabled")).toBe(false));
    fireEvent.click(confirm);
    fireEvent.click(confirm);
    await within(dialog).findByText("已释放");
    expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "execute_session_cleanup")).toHaveLength(1);
});
