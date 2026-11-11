// @vitest-environment jsdom
import "./board-tests/fixtures";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { MachineDialog } from "./MachineDialog";
import { useState } from "react";
import type { Machine } from "./types";
import { setLocale } from "./i18n";

const remoteMachine: Machine = {
  id: "remote", name: "devpc", kind: "ssh", profile_id: "profile", ssh_target: "devpc",
  session: "default", platform: "windows", disabled: false, helper_name: "helper",
};

it.each([false, true])("rejects blank nicknames and cancels without writing for disabled=%s", async disabled => {
  const close = vi.fn();
  vi.mocked(invoke).mockImplementation(async command => command === "saved_machines" ? [] : undefined);
  render(<MachineDialog machines={[{ ...remoteMachine, disabled }]} onChanged={vi.fn()} onClose={close} />);
  fireEvent.click(screen.getByRole("button", { name: "修改昵称" }));
  fireEvent.change(screen.getByLabelText("电脑昵称"), { target: { value: "   " } });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  expect(screen.getByRole("alert").textContent).toContain("请输入电脑昵称");
  fireEvent.change(screen.getByLabelText("电脑昵称"), { target: { value: "不保存的昵称" } });
  fireEvent.click(screen.getByRole("button", { name: "取消" }));
  expect(screen.queryByLabelText("电脑昵称")).toBeNull();
  expect(screen.getByText("Remote · devpc")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "修改昵称" }));
  expect((screen.getByLabelText("电脑昵称") as HTMLInputElement).value).toBe("devpc");
  fireEvent.keyDown(screen.getByLabelText("电脑昵称"), { key: "Escape" });
  expect(screen.queryByLabelText("电脑昵称")).toBeNull();
  expect(close).not.toHaveBeenCalled();
  expect(vi.mocked(invoke).mock.calls.some(([name]) => name === "rename_machine")).toBe(false);
});

it("edits the selected same-name computer in English, including when it is disabled", async () => {
  setLocale("en-US");
  vi.mocked(invoke).mockImplementation(async command => command === "saved_machines" ? [] : undefined);
  try {
    render(<MachineDialog machines={[remoteMachine, { ...remoteMachine, id: "other", disabled: true, ssh_target: "otherpc" }]}
      onChanged={vi.fn()} onClose={vi.fn()} />);
    fireEvent.click(screen.getAllByRole("button", { name: "Edit nickname" })[1]);
    fireEvent.change(screen.getByLabelText("Computer nickname"), { target: { value: "Home PC" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByLabelText("Computer nickname")).toBeNull());
    expect(invoke).toHaveBeenCalledWith("rename_machine", { id: "other", name: "Home PC" });
    expect(screen.getByText("otherpc · default")).toBeTruthy();
  } finally { act(() => setLocale("zh-CN")); }
});

it("edits a Chinese nickname through the application form and reads back the saved name", async () => {
  let saved = { ...remoteMachine };
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === "saved_machines") return [];
    if (command === "list_machines") return [{ ...saved }];
    if (command === "rename_machine") {
      const input = args as { id: string; name: string };
      expect(input.id).toBe("remote");
      saved = { ...saved, name: input.name };
    }
  });
  function ManagedMachines() {
    const [machines, setMachines] = useState([remoteMachine]);
    return <MachineDialog machines={machines} onChanged={() => {
      void invoke<Machine[]>("list_machines").then(setMachines);
    }} onClose={() => {}} />;
  }
  render(<ManagedMachines />);
  fireEvent.click(screen.getByRole("button", { name: "修改昵称" }));
  const input = screen.getByLabelText("电脑昵称") as HTMLInputElement;
  expect(input.value).toBe("devpc");
  expect(document.activeElement).toBe(input);
  fireEvent.change(input, { target: { value: "  公司台式机（开发）  " } });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  expect(await screen.findByText("Remote · 公司台式机（开发）")).toBeTruthy();
  expect(saved.name).toBe("公司台式机（开发）");
  expect(screen.getByText("devpc · default")).toBeTruthy();
  expect(document.activeElement).toBe(screen.getByRole("button", { name: "修改昵称" }));
});

it("keeps a failed nickname draft for retry and prevents duplicate saves", async () => {
  let attempts = 0;
  let finish!: () => void;
  vi.mocked(invoke).mockImplementation(async command => {
    if (command === "saved_machines") return [];
    if (command === "rename_machine") {
      attempts++;
      if (attempts === 1) throw new Error("disk is read-only");
      await new Promise<void>(resolve => { finish = resolve; });
    }
  });
  const changed = vi.fn();
  render(<MachineDialog machines={[remoteMachine]} onChanged={changed} onClose={() => {}} />);
  fireEvent.click(screen.getByRole("button", { name: "修改昵称" }));
  fireEvent.change(screen.getByLabelText("电脑昵称"), { target: { value: "家里笔记本" } });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  expect((await screen.findByRole("alert")).textContent).toContain("disk is read-only");
  expect((screen.getByLabelText("电脑昵称") as HTMLInputElement).value).toBe("家里笔记本");
  expect(changed).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  const saving = screen.getByRole("button", { name: "保存中…" }) as HTMLButtonElement;
  expect(saving.disabled).toBe(true);
  expect((screen.getByRole("button", { name: "取消" }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.submit(saving.closest("form")!);
  expect(attempts).toBe(2);
  await act(async () => finish());
  expect(screen.queryByLabelText("电脑昵称")).toBeNull();
  expect(changed).toHaveBeenCalledOnce();
});

it("keeps a failed connection actionable and never asks for a password in the webview", async () => {
  const changed=vi.fn();
  vi.mocked(invoke).mockImplementation(async command => {
    if(command === "saved_machines") return [{id:"profile",label:"devpc",target:"devpc",session:"default",enabled:true}];
    if(command === "connect_machine") throw new Error("machine.setup_required");
  });
  render(<MachineDialog machines={[]} onChanged={changed} onClose={() => {}} />);
  await waitFor(() => expect((screen.getByLabelText("Herdr 已保存的电脑") as HTMLSelectElement).value).toBe("profile"));
  fireEvent.click(screen.getByRole("button",{name:"连接"}));
  expect(await screen.findByRole("alert")).toBeTruthy();
  expect(changed).toHaveBeenCalledOnce(); // Refresh the staged, unavailable machine after failure.
  expect((screen.getByRole("button",{name:"连接"}) as HTMLButtonElement).disabled).toBe(false);
  expect(document.querySelector('input[type="password"]')).toBeNull();
  fireEvent.change(screen.getByLabelText("远端 Herdr 会话"),{target:{value:"development"}});
  fireEvent.change(screen.getByLabelText("SSH 别名"),{target:{value:"devpc"}});
  fireEvent.click(screen.getByRole("button",{name:"配置 SSH 密钥"}));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("enroll_machine",{target:"devpc",session:"development",configureKey:true}));
});

it("reuses existing SSH authentication and connects the exact registered machine", async () => {
  const connected=vi.fn(); let registered=false;
  vi.mocked(invoke).mockImplementation(async command => {
    if(command === "saved_machines") return registered ? [{id:"profile",label:"xc",target:"xc",session:"default",enabled:true}] : [];
    if(command === "enroll_machine") return {id:"existing",state:"waiting",error:""};
    if(command === "enrollment_status") { registered=true; return {id:"existing",state:"succeeded",error:""}; }
    if(command === "connect_machine") return {id:"remote",name:"xc"};
  });
  render(<MachineDialog machines={[]} onChanged={()=>{}} onClose={()=>{}} onConnected={connected} />);
  fireEvent.change(screen.getByLabelText("SSH 别名"),{target:{value:"xc"}});
  fireEvent.click(screen.getByRole("button",{name:"使用现有 SSH 配置连接"}));
  await waitFor(() => expect(connected).toHaveBeenCalledWith({id:"remote",name:"xc"}));
  expect(invoke).toHaveBeenCalledWith("enroll_machine",{target:"xc",session:"default",configureKey:false});
  expect(invoke).toHaveBeenCalledWith("connect_machine",{profileId:"profile"});
});

it("does not automatically provision a key or connect after existing SSH fails", async () => {
  vi.mocked(invoke).mockImplementation(async command => {
    if(command === "saved_machines") return [];
    if(command === "enroll_machine") return {id:"existing",state:"waiting",error:""};
    if(command === "enrollment_status") return {id:"existing",state:"failed",error:"machine.ssh_connection_failed"};
  });
  render(<MachineDialog machines={[]} onChanged={()=>{}} onClose={()=>{}} />);
  fireEvent.change(screen.getByLabelText("SSH 别名"),{target:{value:"xc"}});
  fireEvent.click(screen.getByRole("button",{name:"使用现有 SSH 配置连接"}));
  expect(await screen.findByRole("alert")).toBeTruthy();
  expect(vi.mocked(invoke).mock.calls.filter(([cmd])=>cmd === "enroll_machine")).toHaveLength(1);
  expect(vi.mocked(invoke).mock.calls.some(([cmd])=>cmd === "connect_machine")).toBe(false);
  expect((screen.getByRole("button",{name:"配置 SSH 密钥"}) as HTMLButtonElement).disabled).toBe(false);
});

it("waits for verification before reporting connection success", async () => {
  const changed=vi.fn();let complete!:(result:unknown)=>void;
  vi.mocked(invoke).mockImplementation(command => {
    if(command === "saved_machines") return Promise.resolve([{id:"profile",label:"devpc",target:"devpc",session:"default",enabled:true}]);
    if(command === "connect_machine") return new Promise(resolve=>{complete=resolve;});
    return Promise.resolve();
  });
  render(<MachineDialog machines={[]} onChanged={changed} onClose={() => {}} />);
  await waitFor(() => expect((screen.getByLabelText("Herdr 已保存的电脑") as HTMLSelectElement).value).toBe("profile"));
  fireEvent.click(screen.getByRole("button",{name:"连接"}));
  expect(changed).not.toHaveBeenCalled();
  expect((screen.getByRole("button",{name:"关闭"}) as HTMLButtonElement).disabled).toBe(true);
  await act(async()=>complete({id:"machine"}));
  expect(changed).toHaveBeenCalledOnce();
});
