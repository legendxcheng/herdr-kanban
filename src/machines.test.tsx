// @vitest-environment jsdom
import "./board-tests/fixtures";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { MachineDialog } from "./MachineDialog";

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
