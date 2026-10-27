// @vitest-environment jsdom
import { project, renderBoard } from "./fixtures";
import { act, fireEvent, render, screen, within, waitFor } from "@testing-library/react";
import { KanbanBoard, TaskBoard } from "../KanbanBoard";
import { invoke } from "@tauri-apps/api/core";
import { expect, it, vi } from "vitest";

it("清理空闲会话必须二次确认，取消不关闭，重复确认仅提交一次", async () => {
  const base=vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation(async (cmd,args,options) => {
    if(cmd==="preview_idle_cleanup") return { id:"preview-1", count:2 };
    if(cmd==="execute_idle_cleanup") return { id:"preview-1",closed:1,skipped:1,failed:0,items:[] };
    return base(cmd,args,options);
  });
  await renderBoard();
  fireEvent.click(screen.getByRole("button",{name:"关闭空闲会话"}));
  let dialog=await screen.findByRole("dialog",{name:"关闭空闲会话"});
  await within(dialog).findByText(/待关闭 2 个会话/);
  expect(document.activeElement).toBe(within(dialog).getByRole("button",{name:"取消"}));
  expect(within(dialog).getByText(/InProgress/)).toBeTruthy();
  fireEvent.click(within(dialog).getByRole("button",{name:"取消"}));
  expect(vi.mocked(invoke).mock.calls.some(([cmd])=>cmd==="execute_idle_cleanup")).toBe(false);
  fireEvent.click(screen.getByRole("button",{name:"关闭空闲会话"}));
  dialog=await screen.findByRole("dialog",{name:"关闭空闲会话"});
  const confirm=await within(dialog).findByRole("button",{name:"确认关闭"});
  await waitFor(()=>expect(confirm.matches(":disabled")).toBe(false));
  fireEvent.click(confirm); fireEvent.click(confirm);
  expect(await within(dialog).findByText("关闭 1，跳过 1，失败 0")).toBeTruthy();
  expect(vi.mocked(invoke).mock.calls.filter(([cmd])=>cmd==="execute_idle_cleanup")).toEqual([["execute_idle_cleanup",{projectId:project.id,previewId:"preview-1"}]]);
});

it("没有候选时不能确认，Escape 关闭弹窗", async () => {
  const base=vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation(async(cmd,args,options)=>cmd==="preview_idle_cleanup"?{id:"empty",count:0,unresolved:[]}:base(cmd,args,options));
  await renderBoard(); fireEvent.click(screen.getByRole("button",{name:"关闭空闲会话"}));
  const dialog=await screen.findByRole("dialog");
  await within(dialog).findByText("没有可清理的空闲会话。");
  expect(within(dialog).getByRole("button",{name:"确认关闭"}).matches(":disabled")).toBe(true);
  fireEvent.keyDown(dialog,{key:"Escape"});
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(vi.mocked(invoke).mock.calls.some(([cmd])=>cmd==="execute_idle_cleanup")).toBe(false);
});

it("All Tasks 不显示跨 Workspace 清理入口", async()=>{
  render(<TaskBoard projects={[project]} />);
  expect(screen.queryByRole("button",{name:"关闭空闲会话"})).toBeNull();
});

it("切换 Workspace 后迟到的预览不会显示或被执行", async()=>{
  const base=vi.mocked(invoke).getMockImplementation()!;
  let resolve!:(value:unknown)=>void;
  vi.mocked(invoke).mockImplementation((cmd,args,options)=>cmd==="preview_idle_cleanup"?new Promise(done=>{resolve=done;}):base(cmd,args,options));
  const view=render(<KanbanBoard project={project} />);
  fireEvent.click(await screen.findByRole("button",{name:"关闭空闲会话"}));
  await screen.findByRole("dialog");
  view.rerender(<KanbanBoard project={{...project,id:"other",name:"Other"}} />);
  await act(async()=>resolve({id:"old",count:3,unresolved:[]}));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(vi.mocked(invoke).mock.calls.some(([cmd])=>cmd==="execute_idle_cleanup")).toBe(false);
});

it("未知关闭提供只读核实，并可从后续预览找到上次结果", async()=>{
  const base=vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation(async(cmd,args,options)=>{
    if(cmd==="preview_idle_cleanup") return {id:"new",count:0,unresolved:["old"]};
    if(cmd==="idle_cleanup_result") return {id:"old",closed:0,skipped:0,failed:1,items:[{pane_id:"pane",state:"unknown",error:"cleanup.unknown"}]};
    return base(cmd,args,options);
  });
  await renderBoard(); fireEvent.click(screen.getByRole("button",{name:"关闭空闲会话"}));
  const dialog=await screen.findByRole("dialog");
  fireEvent.click(await within(dialog).findByRole("button",{name:"核实上次未确定的清理结果"}));
  await within(dialog).findByText("关闭 0，跳过 0，失败 1");
  fireEvent.click(within(dialog).getByRole("button",{name:"核实关闭结果"}));
  await waitFor(()=>expect(vi.mocked(invoke).mock.calls.filter(([cmd])=>cmd==="idle_cleanup_result")).toHaveLength(2));
  expect(invoke).toHaveBeenCalledWith("idle_cleanup_result",{projectId:project.id,previewId:"old"});
  expect(vi.mocked(invoke).mock.calls.some(([cmd])=>cmd==="execute_idle_cleanup")).toBe(false);
});
