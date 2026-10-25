// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { TaskFormDialog } from "./TaskFormDialog";
import type { Project } from "./types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const project = { id: "p", root_path: "E:/fixture" } as Project;
const image = { id: "ddbc5177-fab2-4aa6-a6e1-d46fc29fdaed", name: "截图.png", mime: "image/png", bytes: 80, width: 2, height: 2 };
beforeEach(() => {
  vi.mocked(invoke).mockReset().mockImplementation(async (command) => {
    switch (command) {
      case "open_image_draft": return "draft-1";
      case "import_prompt_image": return image;
      case "preview_prompt_image": return "data:image/png;base64,AA==";
      case "close_image_draft": case "create_task": return;
      default: throw new Error(`Unexpected IPC: ${command}`);
    }
  });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

it("pastes a screenshot into the prompt and saves its durable reference with the task", async () => {
  render(<TaskFormDialog project={project} onClose={() => {}} onSaved={() => {}} />);
  fireEvent.change(screen.getByRole("textbox", { name: "标题" }), { target: { value: "带截图的任务" } });
  const prompt = screen.getByRole("textbox", { name: "提示词" });
  fireEvent.focus(prompt);
  const file = new File([new Uint8Array([137, 80, 78, 71])], "截图.png", { type: "image/png" });
  fireEvent.paste(prompt, { clipboardData: { files: [file], items: [{ kind: "file", type: "image/png", getAsFile: () => file }], getData: () => "" } });
  expect(await screen.findByRole("img", { name: "截图.png" })).toBeTruthy();
  await waitFor(() => expect((screen.getByRole("button", { name: "添加任务" }) as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(screen.getByRole("button", { name: "添加任务" }));
  await waitFor(() => expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "create_task")).toBe(true));
  const [, args] = vi.mocked(invoke).mock.calls.find(([command]) => command === "create_task")!;
  expect(args).toMatchObject({ draftId: "draft-1", name: "带截图的任务" });
  expect(JSON.parse((args as { promptJson: string }).promptJson).content).toContainEqual({ type: "promptImage", attrs: { imageId: image.id, name: "截图.png", bytes: 80 } });
});

it("rejects an over-limit multi-image paste without discarding existing input", async () => {
  render(<TaskFormDialog project={project} onClose={() => {}} onSaved={() => {}} />);
  fireEvent.change(screen.getByRole("textbox", { name: "标题" }), { target: { value: "保留任务" } });
  const prompt=screen.getByRole("textbox",{name:"提示词"});
  const files=Array.from({length:21},(_,index)=>new File(["PNG"],`${index}.png`,{type:"image/png"}));
  fireEvent.paste(prompt,{clipboardData:{files,items:[],getData:()=>""}});
  expect((await screen.findByRole("alert")).textContent).toContain("20");
  expect(vi.mocked(invoke).mock.calls.some(([command])=>command==="import_prompt_image")).toBe(false);
  expect((screen.getByRole("textbox",{name:"标题"}) as HTMLInputElement).value).toBe("保留任务");
});

it("can delete a pasted image and undo the deletion without losing its reference", async () => {
  render(<TaskFormDialog project={project} onClose={() => {}} onSaved={() => {}} />);
  const prompt=screen.getByRole("textbox",{name:"提示词"});
  const file=new File(["PNG"],"截图.png",{type:"image/png"});
  fireEvent.paste(prompt,{clipboardData:{files:[file],items:[],getData:()=>""}});
  await screen.findByRole("img",{name:"截图.png"});
  fireEvent.click(screen.getByRole("button",{name:"删除图片: 截图.png"}));
  expect(screen.queryByRole("img")).toBeNull();
  fireEvent.keyDown(prompt,{key:"z",code:"KeyZ",ctrlKey:true});
  expect(await screen.findByRole("img",{name:"截图.png"})).toBeTruthy();
});

it("keeps a slow two-image paste before text entered while import is pending", async () => {
  const base=vi.mocked(invoke).getMockImplementation()!;
  let finish!: (value:unknown)=>void;
  vi.mocked(invoke).mockImplementation((command,args,options)=>command==='import_prompt_image'
    ? (args as {name:string}).name==='first.png' ? new Promise(resolve=>{finish=resolve;}) : Promise.resolve({...image,id:'ddbc5177-fab2-4aa6-a6e1-d46fc29fdaee',name:'second.png'})
    : base(command,args,options));
  render(<TaskFormDialog project={project} onClose={()=>{}} onSaved={()=>{}}/>);
  fireEvent.change(screen.getByRole('textbox',{name:'标题'}),{target:{value:'保持图文顺序'}});
  const prompt=screen.getByRole('textbox',{name:'提示词'});
  const files=['first.png','second.png'].map(name=>new File(['PNG'],name,{type:'image/png'}));
  fireEvent.paste(prompt,{clipboardData:{files,items:[],getData:()=>''}});
  await waitFor(()=>expect(finish).toBeTypeOf('function'));
  fireEvent.paste(prompt,{clipboardData:{files:[],getData:(type:string)=>type==='text/plain'?'typed later':''}});
  finish({...image,name:'first.png'});
  await screen.findByRole('img',{name:'second.png'});
  fireEvent.click(screen.getByRole('button',{name:'添加任务'}));
  await waitFor(()=>expect(vi.mocked(invoke).mock.calls.some(([command])=>command==='create_task')).toBe(true));
  const [,args]=vi.mocked(invoke).mock.calls.find(([command])=>command==='create_task')!;
  const content=JSON.parse((args as {promptJson:string}).promptJson).content;
  const order=content.map((node:any)=>node.type==='promptImage'?node.attrs.name:(node.content??[]).map((child:any)=>child.text??'').join('')).filter(Boolean);
  expect(order).toEqual(['first.png','second.png','typed later']);
});

it("opens a thumbnail in a larger preview and Escape returns to the task without closing it", async () => {
  render(<TaskFormDialog project={project} onClose={vi.fn()} onSaved={()=>{}}/>);
  const prompt=screen.getByRole('textbox',{name:'提示词'});
  fireEvent.paste(prompt,{clipboardData:{files:[new File(['PNG'],'截图.png',{type:'image/png'})],items:[],getData:()=>''}});
  await screen.findByRole('img',{name:'截图.png'});
  fireEvent.click(screen.getByRole('button',{name:'放大查看图片: 截图.png'}));
  const preview=screen.getByRole('dialog',{name:'图片预览: 截图.png'});
  expect(preview.querySelector('img')).toBeTruthy();
  fireEvent.keyDown(preview,{key:'Escape'});
  expect(screen.queryByRole('dialog',{name:'图片预览: 截图.png'})).toBeNull();
  expect(screen.getByRole('dialog',{name:'新建任务'})).toBeTruthy();
});

it("chooses one image when the clipboard declares alternative image representations", async () => {
  render(<TaskFormDialog project={project} onClose={()=>{}} onSaved={()=>{}}/>);
  const files=[new File(['PNG'],'image.png',{type:'image/png'}),new File(['JPEG'],'image.jpg',{type:'image/jpeg'})];
  fireEvent.paste(screen.getByRole('textbox',{name:'提示词'}),{clipboardData:{files,types:['Files'],items:files.map(file=>({kind:'file',type:file.type,getAsFile:()=>file})),getData:()=>''}});
  await screen.findByRole('img',{name:'截图.png'});
  await waitFor(()=>expect(vi.mocked(invoke).mock.calls.filter(([command])=>command==='import_prompt_image')).toHaveLength(1));
  expect(vi.mocked(invoke).mock.calls.find(([command])=>command==='import_prompt_image')?.[1]).toMatchObject({mime:'image/png'});
});
