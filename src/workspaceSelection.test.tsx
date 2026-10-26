// @vitest-environment jsdom
import { project, board, bindTask } from "./board-tests/fixtures";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { WorkspaceAddDialog } from "./WorkspaceAddDialog";
import { MachineDialog } from "./MachineDialog";
import { WorkspaceRecovery } from "./WorkspaceRecovery";
import App from "./App";
import { ProjectSidebar } from "./ProjectSidebar";
import type { Machine } from "./types";

const remote: Machine = { id: "remote", name: "Dev", kind: "ssh", session: "development", profile_id: "profile", ssh_target: "devpc", disabled: false, helper_name: "verified" };
const choices = (machine = "local") => ({ machine_id: machine, session: machine === "local" ? "default" : "development", workspaces: [
  { id:"a", name:"Alpha", directory:"C:/Alpha", project_id:"pa", shown:false, blocked:false },
  { id:"b", name:"Beta", directory:"C:/Beta", project_id:"pb", shown:false, blocked:false },
  { id:"c", name:"Current", directory:"C:/Current", project_id:"pc", shown:true, blocked:false },
] });

it("searches and atomically adds selected existing workspaces without creating Herdr resources", async () => {
  const saved = vi.fn(), closed = vi.fn();
  vi.mocked(invoke).mockImplementation(async command => command === "workspace_choices" ? choices() : undefined);
  render(<WorkspaceAddDialog machines={[remote]} onSaved={saved} onClose={closed} />);
  fireEvent.click(await screen.findByRole("checkbox",{name:/Alpha/}));
  fireEvent.change(screen.getByLabelText("搜索 Workspace"),{target:{value:"Beta"}});
  expect(screen.queryByRole("checkbox",{name:/Alpha/})).toBeNull();
  fireEvent.click(screen.getByRole("checkbox",{name:/Beta/}));
  fireEvent.click(screen.getByRole("button",{name:"添加所选（2）"}));
  await waitFor(()=>expect(saved).toHaveBeenCalledOnce());
  expect(invoke).toHaveBeenCalledWith("add_existing_workspaces",{machineId:"local",session:"default",workspaceIds:["a","b"]});
  expect(vi.mocked(invoke).mock.calls.some(([cmd])=>cmd==="create_project")).toBe(false);
  expect(closed).toHaveBeenCalledOnce();
});

it("keeps the selected draft after an atomic add failure and cancellation writes nothing further", async () => {
  const saved = vi.fn(), closed = vi.fn();
  vi.mocked(invoke).mockImplementation(async command => { if(command === "workspace_choices") return choices(); throw new Error("workspace.missing"); });
  render(<WorkspaceAddDialog machines={[]} onSaved={saved} onClose={closed} />);
  fireEvent.click(await screen.findByRole("checkbox",{name:/Alpha/}));
  fireEvent.click(screen.getByRole("button",{name:"添加所选（1）"}));
  await screen.findByRole("alert");
  expect((screen.getByRole("checkbox",{name:/Alpha/}) as HTMLInputElement).checked).toBe(true);
  expect(saved).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button",{name:"取消"})); expect(closed).toHaveBeenCalledOnce();
  expect(vi.mocked(invoke).mock.calls.filter(([cmd])=>cmd==="add_existing_workspaces")).toHaveLength(1);
});

it("rejects stale local candidates after switching to a remote computer", async () => {
  let finish!: (value:unknown)=>void;
  vi.mocked(invoke).mockImplementation((command,args) => command === "workspace_choices" && (args as {machineId:string}).machineId === "local" ? new Promise(resolve=>{finish=resolve;}) : Promise.resolve({...choices("remote"),workspaces:[{...choices().workspaces[1],name:"Remote Beta"}]}));
  render(<WorkspaceAddDialog machines={[remote]} onSaved={()=>{}} onClose={()=>{}} />);
  fireEvent.change(screen.getByLabelText("执行电脑"),{target:{value:"remote"}});
  await screen.findByRole("checkbox",{name:/Remote Beta/});
  await act(async()=>finish(choices()));
  expect(screen.queryByRole("checkbox",{name:/Alpha/})).toBeNull();
  fireEvent.click(screen.getByRole("checkbox",{name:/Remote Beta/}));
  fireEvent.click(screen.getByRole("button",{name:"添加所选（1）"}));
  await waitFor(()=>expect(invoke).toHaveBeenCalledWith("add_existing_workspaces",{machineId:"remote",session:"development",workspaceIds:["b"]}));
});

it("creates on the selected remote computer using a remote path and no local picker", async () => {
  const saved = vi.fn();
  vi.mocked(invoke).mockImplementation(async (command,args) => command === "workspace_choices" ? choices((args as {machineId:string}).machineId) : {});
  render(<WorkspaceAddDialog machines={[remote]} onSaved={saved} onClose={()=>{}} />);
  fireEvent.change(screen.getByLabelText("执行电脑"),{target:{value:"remote"}});
  fireEvent.click(screen.getByRole("button",{name:"新建工作空间"}));
  expect((screen.getByLabelText("执行电脑") as HTMLSelectElement).value).toBe("remote");
  expect(screen.queryByRole("button",{name:"浏览"})).toBeNull();
  fireEvent.change(screen.getByLabelText("工作空间名称"),{target:{value:"Created"}});
  fireEvent.change(screen.getByLabelText("工作目录"),{target:{value:"D:/OnlyRemote"}});
  fireEvent.click(screen.getByRole("button",{name:"创建工作空间"}));
  await waitFor(()=>expect(saved).toHaveBeenCalledOnce());
  expect(invoke).toHaveBeenCalledWith("create_project",{name:"Created",rootPath:"D:/OnlyRemote",operationId:expect.any(String),machineId:"remote"});
});

it("only opens remote candidates after connection verification, without selecting all", async () => {
  let connected!: (value:unknown)=>void;
  vi.mocked(invoke).mockImplementation((command,args) => {
    if(command === "workspace_choices") return Promise.resolve(choices((args as {machineId:string}).machineId));
    if(command === "saved_machines") return Promise.resolve([{id:"profile",label:"Dev",target:"devpc",session:"development",enabled:true}]);
    if(command === "connect_machine") return new Promise(resolve=>{connected=resolve;});
    return Promise.resolve();
  });
  render(<WorkspaceAddDialog machines={[]} onSaved={()=>{}} onClose={()=>{}} />);
  fireEvent.click(screen.getByRole("button",{name:"添加远端电脑"}));
  await screen.findByRole("option",{name:/Dev/});
  fireEvent.click(screen.getByRole("button",{name:"连接"}));
  expect(vi.mocked(invoke).mock.calls.filter(([cmd,args])=>cmd==="workspace_choices" && (args as {machineId:string}).machineId==="remote")).toHaveLength(0);
  await act(async()=>connected(remote));
  await screen.findByRole("checkbox",{name:/Alpha/});
  expect((screen.getByLabelText("执行电脑") as HTMLSelectElement).value).toBe("remote");
  expect((screen.getByRole("button",{name:"添加所选（0）"}) as HTMLButtonElement).disabled).toBe(true);
});

it("keeps the sidebar add flow open while refreshing a newly connected computer", async () => {
  vi.mocked(invoke).mockImplementation(async (command,args) => {
    if(command === "workspace_choices") return choices((args as {machineId:string}).machineId);
    if(command === "saved_machines") return [{id:"profile",label:"Dev",target:"devpc",session:"development",enabled:true}];
    if(command === "connect_machine") return remote;
  });
  const changed=vi.fn();
  render(<ProjectSidebar machines={[]} projects={[]} selectedId={null} onSelect={()=>{}} onChanged={changed} />);
  fireEvent.click(screen.getAllByRole("button",{name:"添加 Workspace"})[0]);
  fireEvent.click(screen.getByRole("button",{name:"添加远端电脑"}));
  await screen.findByRole("option",{name:/Dev/});
  fireEvent.click(screen.getByRole("button",{name:"连接"}));
  await screen.findByRole("checkbox",{name:/Alpha/});
  expect(screen.getByRole("dialog",{name:"添加 Workspace"})).toBeTruthy();
  expect(changed).toHaveBeenCalled();
  expect((screen.getByLabelText("执行电脑") as HTMLSelectElement).value).toBe("remote");
});

it("observes terminal cancellation without declaring a successful connection", async () => {
  const connected = vi.fn();
  vi.mocked(invoke).mockImplementation(async command => {
    if(command === "saved_machines") return [];
    if(command === "enroll_machine") return {id:"attempt",state:"waiting",error:""};
    if(command === "enrollment_status") return {id:"attempt",state:"cancelled",error:"machine.enrollment_cancelled"};
  });
  render(<MachineDialog machines={[]} onChanged={()=>{}} onClose={()=>{}} onConnected={connected} />);
  fireEvent.change(screen.getByLabelText("SSH 别名"),{target:{value:"devpc"}});
  fireEvent.click(screen.getByRole("button",{name:"打开 Windows 密码接入终端"}));
  expect((await screen.findByRole("alert")).textContent).toContain("授权终端已关闭");
  expect(connected).not.toHaveBeenCalled();
  expect(vi.mocked(invoke).mock.calls.some(([cmd])=>cmd==="connect_machine")).toBe(false);
  expect(document.querySelector('input[type="password"]')).toBeNull();
});

it("hides the current workspace immediately and does not let a stale sync restore it", async () => {
  bindTask(); board.storedTask.status = "in_progress";
  let hidden = false, slow = false, finish!: (value:unknown)=>void;
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((command,args,options)=>{
    if(command === "list_projects") return Promise.resolve(hidden ? [] : [project]);
    if(command === "navigation_state") return Promise.resolve({configured:true,recovery_projects:[]});
    if(command === "sync_projects") return slow && !hidden ? new Promise(resolve=>{finish=resolve;}) : Promise.resolve({projects:hidden?[]:[project],workspaces:[],operations:[],session:"default"});
    if(command === "hide_project") { hidden=true; return Promise.resolve(); }
    return base(command,args,options);
  });
  render(<App />); await screen.findByText("Herdr 已打开");
  slow=true; fireEvent(window,new Event("focus"));
  fireEvent.click(screen.getByRole("button",{name:"从侧栏隐藏 测试项目"}));
  await screen.findByText("当前没有显示的 Workspace，可随时重新添加。");
  await act(async()=>finish({projects:[project],workspaces:[],operations:[],session:"default"}));
  expect(screen.queryByRole("button",{name:/^测试项目/})).toBeNull();
  expect(vi.mocked(invoke).mock.calls.some(([cmd])=>cmd==="terminal_close")).toBe(true);
  expect(vi.mocked(invoke).mock.calls.some(([cmd])=>["delete_project","delete_task","complete_task"].includes(cmd))).toBe(false);
});

it("routes failed remote creation retry and candidate recovery to the original machine", async () => {
  vi.mocked(invoke).mockImplementation(async command=>{if(command==="create_project") throw new Error("herdr.timeout"); return {workspaces:[],operations:[]};});
  render(<WorkspaceRecovery operation={{id:"op",machine_id:"remote",session:"development",name:"Created",cwd:"D:/Remote",workspace_id:null,project_id:"p",state:"failed",error:""}} onChanged={()=>{}} />);
  fireEvent.click(screen.getByRole("button",{name:"重新创建"}));
  await screen.findByRole("alert");
  expect(invoke).toHaveBeenCalledWith("create_project",{name:"Created",rootPath:"D:/Remote",operationId:"op",machineId:"remote"});
  expect(invoke).toHaveBeenCalledWith("sync_projects",{machineId:"remote"});
});
