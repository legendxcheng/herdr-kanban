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
  expect(changed).not.toHaveBeenCalled();
  expect((screen.getByRole("button",{name:"连接"}) as HTMLButtonElement).disabled).toBe(false);
  expect(document.querySelector('input[type="password"]')).toBeNull();
  fireEvent.change(screen.getByLabelText("远端 Herdr 会话"),{target:{value:"development"}});
  fireEvent.click(screen.getByRole("button",{name:"打开 Windows 密码接入终端"}));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("enroll_machine",{target:"devpc",session:"development"}));
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
