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

it("未知结果先核实，显式重试才再次请求关闭", async () => {
 const item={id:"i",machine:"Local",workspace:"Hidden",target:{pane_id:"p",status:"working",title:"Pending Agent"},state:"unknown",error:"lost reply"};
 const history={id:"old",state:"done",scopes:[],items:[item]};
 vi.mocked(invoke).mockImplementation(async cmd=>{
  if(cmd==="preview_session_cleanup")return {id:"new",state:"preview",scopes:[],items:[]};
  if(cmd==="session_cleanup_history")return [history];
  if(cmd==="session_resource_list")return [];
  if(cmd==="session_cleanup_result")return history;
  if(cmd==="retry_session_cleanup")return {...history,items:[{...item,state:"closed",error:""}]};
  return [];
 });
 render(<SessionCleanupButton/>);fireEvent.click(screen.getByRole("button",{name:"清理未绑定会话"}));
 const dialog=await screen.findByRole("dialog");fireEvent.click(await within(dialog).findByText("最近清理记录"));
 const old=await within(dialog).findByRole("button",{name:/核实第 1 次记录/});await waitFor(()=>expect(old.matches(":disabled")).toBe(false));fireEvent.click(old);
 const retry=await within(dialog).findByRole("button",{name:"重新核实并重试仍存在的目标"});
 expect(vi.mocked(invoke).mock.calls.filter(([cmd])=>cmd==="retry_session_cleanup")).toHaveLength(0);
 await waitFor(()=>expect(retry.matches(":disabled")).toBe(false));fireEvent.click(retry);await within(dialog).findByText("已释放");
 expect(vi.mocked(invoke).mock.calls.filter(([cmd])=>cmd==="retry_session_cleanup")).toHaveLength(1);
});
it("资源记录可单独预览并保留再次确认边界",async()=>{
 const resource={id:5,task_id:"deleted",target:{machine_id:"local",workspace_id:"hidden",pane_id:"old",title:"Deleted Agent"},reason:"deleted",state:"failed",error:"offline"};
 vi.mocked(invoke).mockImplementation(async cmd=>{
  if(cmd==="session_resource_list")return [resource];
  if(cmd==="session_cleanup_history")return [];
  if(cmd==="preview_session_cleanup")return {id:"global",state:"preview",items:[],scopes:[]};
  if(cmd==="preview_session_resource")return {id:"resource",state:"preview",scopes:[],items:[{...resource,id:"item",machine:"Local",workspace:"hidden",state:"ready",error:""}]};
  return [];
 });
 render(<SessionCleanupButton/>);fireEvent.click(screen.getByRole("button",{name:"清理未绑定会话"}));
 const dialog=await screen.findByRole("dialog");fireEvent.click(await within(dialog).findByText(/尚未确认释放的资源记录（1）/));
 const inspect=await within(dialog).findByRole("button",{name:"查看并处理"});await waitFor(()=>expect(inspect.matches(":disabled")).toBe(false));fireEvent.click(inspect);
 await within(dialog).findByText("待关闭");expect(vi.mocked(invoke).mock.calls).toContainEqual(["preview_session_resource",{resourceId:5}]);
 expect(vi.mocked(invoke).mock.calls.some(([cmd])=>cmd==="execute_session_cleanup")).toBe(false);
});
