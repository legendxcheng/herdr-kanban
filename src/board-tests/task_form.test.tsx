// @vitest-environment jsdom
import { board, renderBoard } from "./fixtures";
import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";

describe("任务优先级", () => {
  it("保存中锁定表单且只提交一次，失败保留完整草稿并可重试", async () => {
    const base = vi.mocked(invoke).getMockImplementation()!;
    let rejectSave!: (error: Error) => void;
    vi.mocked(invoke).mockImplementation((command, args, options) => command === "create_task"
      ? new Promise((_resolve, reject) => { rejectSave = reject; }) : base(command, args, options));
    await renderBoard();
    fireEvent.click(screen.getByRole("button", { name: /新建任务/ }));
    const dialog = screen.getByRole("dialog", { name: "新建任务" });
    fireEvent.paste(within(dialog).getByRole("textbox", { name: "提示词" }), {
      clipboardData: { getData: (type: string) => type === "text/plain" ? "失败也不能丢失正文" : "", files: [] },
    });
    fireEvent.change(within(dialog).getByRole("combobox"), { target: { value: "high" } });
    const save = within(dialog).getByRole("button", { name: "添加任务" });
    fireEvent.click(save); fireEvent.submit(dialog);
    fireEvent.keyDown(save, { key: "Escape" });
    expect(screen.getByRole("dialog")).toBe(dialog);
    expect(within(dialog).getByRole("textbox", { name: "提示词" }).getAttribute("contenteditable")).toBe("false");
    expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "create_task")).toHaveLength(1);
    rejectSave(new Error("保存失败"));
    await screen.findByRole("alert");
    expect(document.activeElement).toBe(within(dialog).getByRole("textbox", { name: "提示词" }));
    expect(within(dialog).getByRole("textbox", { name: "提示词" }).textContent).toBe("失败也不能丢失正文");
    expect((within(dialog).getByRole("combobox") as HTMLSelectElement).value).toBe("high");
    vi.mocked(invoke).mockImplementation(base);
    fireEvent.click(save);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(await screen.findByText("自动标题")).toBeTruthy();
  });

  it("新建保存提示词和优先级，编辑可读回自动标题", async () => {
    await renderBoard();
    fireEvent.click(screen.getByRole("button", { name: /新建任务/ }));
    const dialog = screen.getByRole("dialog", { name: "新建任务" });
    const prompt = within(dialog).getByRole("textbox", { name: "提示词" });
    fireEvent.paste(prompt, { clipboardData: { getData: (type: string) => type === "text/plain" ? "请检查新建表单" : "", files: [] } });
    fireEvent.change(within(dialog).getByRole("combobox", { name: "优先级" }), { target: { value: "urgent" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "添加任务" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    const card = (await screen.findByText("自动标题")).closest(".task-card")! as HTMLElement;
    fireEvent.click(within(card).getByRole("button", { name: "编辑" }));
    const edit = screen.getByRole("complementary", { name: "编辑任务" });
    expect((within(edit).getByRole("textbox", { name: "标题" }) as HTMLInputElement).value).toBe("自动标题");
    expect(within(edit).getByRole("textbox", { name: "提示词" }).textContent).toBe("请检查新建表单");
    expect((within(edit).getByRole("combobox", { name: "优先级" }) as HTMLSelectElement).value).toBe("urgent");
  });

  it("快捷键只打开顶部新建表单，取消不创建，创建失败保留输入", async () => {
    const base = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation((command, args, options) => command === "create_task"
      ? Promise.reject(new Error("数据库写入失败")) : base(command, args, options));
    await renderBoard();
    fireEvent.keyDown(document, { key: "n" });
    const dialog = screen.getByRole("dialog", { name: "新建任务" });
    fireEvent.paste(within(dialog).getByRole("textbox", { name: "提示词" }), { clipboardData: { files: [], getData: (type: string) => type === "text/plain" ? "保留草稿" : "" } });
    fireEvent.change(within(dialog).getByRole("combobox", { name: "优先级" }), { target: { value: "high" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "添加任务" }));
    await screen.findByRole("alert");
    expect(within(dialog).getByRole("textbox", { name: "提示词" }).textContent).toBe("保留草稿");
    expect((within(dialog).getByRole("combobox") as HTMLSelectElement).value).toBe("high");
    fireEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(board.extraTasks).toHaveLength(0);
  });

  it("新建时保存紧急优先级，编辑可修改，下一次新建恢复一般", async () => {
    await renderBoard();
    expect(screen.queryByRole("textbox", { name: "标题" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /新建任务/ }));
    fireEvent.paste(screen.getByRole("textbox", { name: "提示词" }), { clipboardData: { files: [], getData: (type: string) => type === "text/plain" ? "紧急故障" : "" } });
    fireEvent.change(screen.getByRole("combobox", { name: "优先级" }), { target: { value: "urgent" } });
    fireEvent.click(screen.getByRole("button", { name: "添加任务" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    const card = (await screen.findByText("自动标题")).closest(".task-card")! as HTMLElement;
    fireEvent.click(within(card).getByRole("button", { name: "编辑" }));
    const editor = screen.getByRole("complementary", { name: "编辑任务" });
    const priority = within(editor).getByRole("combobox", { name: "优先级" });
    expect((priority as HTMLSelectElement).value).toBe("urgent");
    fireEvent.change(priority, { target: { value: "high" } });
    fireEvent.click(within(editor).getByRole("button", { name: "保存" }));
    await waitFor(() => expect(within(editor).getByRole("status").textContent).toBe("已保存"));
    expect(board.extraTasks[0].priority).toBe("high");
    fireEvent.click(screen.getByRole("button", { name: /新建任务/ }));
    expect((within(screen.getByRole("dialog", { name: "新建任务" })).getByRole("combobox", { name: "优先级" }) as HTMLSelectElement).value).toBe("normal");
  });
});
