const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs');
// Run against a local Vite server; IPC is simulated and no real task is sent.
const dir = require('node:path').resolve(__dirname, '../.scratch/skill-popover');
fs.mkdirSync(dir, {recursive:true});
const baseline = process.argv.includes('--baseline');
(async () => {
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage({viewport:{width:1280,height:800}});
    const errors=[]; page.on('pageerror', e=>errors.push(e.message));
    await page.addInitScript(() => {
      const project={id:'p',name:'Skill 提示验收',root_path:'E:/fixture',herdr_workspace:'fixture',created_at:'',herdr_session:'default',workspace_id:'fixture',sync_status:'ready',directory_source:'herdr',herdr_directory:'E:/fixture'};
      const description='Create distinctive, production-grade frontend interfaces with high design quality. Use this skill when the user asks to build web components, pages, or applications. Generates creative, polished code that avoids generic AI aesthetics.';
      window.probe={calls:[],tasks:[],skills:[
        {name:'frontend-design',description},
        {name:'a-very-long-skill-name-'.repeat(8),description:'中文说明：检查长名称和描述能否完整阅读。'},
        {name:'no-description',description:''},
        {name:'huge-description',description:'很长的完整说明，用于检查滚动后仍能读到末尾。'.repeat(90)+'END-OF-DESCRIPTION'},
        ...Array.from({length:8},(_,i)=>({name:'skill-'+i,description:'候选说明 '+i})),
      ].map(s=>({...s,skill_md_path:'E:/skills/'+s.name+'/SKILL.md'}))};
      window.__TAURI_INTERNALS__={invoke:async(command,args={})=>{
        probe.calls.push({command,args});
        switch(command){
          case 'list_projects':return [project];
          case 'sync_projects':return {projects:[project],workspaces:[],operations:[]};
          case 'workspace_operations':case 'herdr_list_agents':return [];
          case 'list_tasks':return probe.tasks;
          case 'search_skills':return probe.skills;
          case 'search_files':return [{rel_path:'src/'+('long-directory/').repeat(10)+'App.tsx',abs_path:'E:/fixture/App.tsx'}];
          case 'create_task':{const t={id:'task',project_id:'p',name:args.name,prompt_json:args.promptJson,priority:args.priority,status:'todo',created_at:'',updated_at:''};probe.tasks.push(t);return t;}
          case 'update_task':Object.assign(probe.tasks[0],{name:args.name,prompt_json:args.promptJson,priority:args.priority});return;
          default:throw Error('Unexpected IPC '+command);
        }
      }};
    });
    await page.goto(process.env.KANBAN_VERIFY_URL || 'http://127.0.0.1:1441');
    await page.getByRole('button',{name:/新建任务/}).click();
    const prompt=page.getByRole('textbox',{name:'提示词',exact:true});
    await prompt.fill('$ask');
    await page.locator('.mention-item').first().waitFor();
    await page.screenshot({path:dir+(baseline?'/before.png':'/after-1280.png')});
    const metrics=await page.locator('.mention-item').first().evaluate(e=>{
      const name=e.querySelector('.mention-label').getBoundingClientRect();
      const desc=e.querySelector('.mention-desc').getBoundingClientRect();
      return {nameWidth:name.width,nameBottom:name.bottom,descriptionTop:desc.top};
    });
    console.log(JSON.stringify({baseline,metrics}));
    assert.ok(metrics.nameWidth>250, 'Skill name must use the available text column, not a squeezed letter-wide strip');
    assert.ok(metrics.descriptionTop>=metrics.nameBottom,'Description must appear below the name');
    if(baseline)return;
    const geometry=[];
    for(const [width,height] of [[900,600],[1280,800],[1920,1080],[390,844]]){
      await page.setViewportSize({width,height});
      await prompt.fill('$ask');
      await page.waitForTimeout(100);
      const r=await page.locator('.mention-popover').evaluate(e=>{
        const b=e.getBoundingClientRect();
        return {x:b.x,y:b.y,width:b.width,height:b.height,scrollWidth:e.scrollWidth,clientWidth:e.clientWidth};
      });
      assert.ok(r.x>=7 && r.x+r.width<=width-7 && r.y>=7 && r.y+r.height<=height-7,JSON.stringify(r));
      assert.ok(r.scrollWidth<=r.clientWidth,'No horizontal overflow');
      geometry.push({viewportWidth:width,viewportHeight:height,...r});
      await page.screenshot({path:dir+'/after-'+width+'.png'});
    }
    await page.setViewportSize({width:900,height:600});
    await prompt.fill('$ask');
    await page.waitForTimeout(100);
    const formScroll=await page.locator('.task-form-body').evaluate(e=>e.scrollTop);
    async function visibleHeading(){
      const r=await page.locator('.mention-popover').evaluate(e=>{
        const b=e.getBoundingClientRect(), n=e.querySelector('.active .mention-label').getBoundingClientRect();
        return {top:n.top,bottom:n.bottom,popTop:b.top,popBottom:b.bottom,scroll:e.scrollTop};
      });
      assert.ok(r.top>=r.popTop && r.top<r.popBottom-10,'Selected heading must be visible '+JSON.stringify(r));
      assert.equal(await page.locator('.task-form-body').evaluate(e=>e.scrollTop),formScroll,'Keyboard must not scroll the form');
    }
    // A stationary pointer over the list must not override keyboard selection
    // when scrolling moves a different row beneath it.
    await page.locator('.mention-item').first().hover();
    await prompt.press('ArrowUp');
    await page.waitForTimeout(200);
    await visibleHeading();
    assert.equal(await page.locator('.mention-item.active .mention-label').textContent(),'skill-7');
    await prompt.press('Enter');
    assert.equal(await prompt.locator('.mention-chip').textContent(),'$skill-7');
    await prompt.fill('$keys');
    await page.locator('.mention-item').first().waitFor();
    await prompt.press('ArrowUp');
    await prompt.press('ArrowDown');await visibleHeading();
    for(let i=0;i<3;i++)await prompt.press('ArrowDown');
    await visibleHeading();
    assert.equal(await page.locator('.mention-item.active .mention-label').textContent(),'huge-description');
    const huge=page.locator('.mention-item').nth(3);
    assert.ok((await huge.locator('.mention-desc').textContent()).endsWith('END-OF-DESCRIPTION'));
    assert.equal(await page.locator('.mention-item').nth(2).locator('.mention-desc').count(),0);
    const descriptionMetrics=await huge.locator('.mention-desc').evaluate(e=>({client:e.clientHeight,scroll:e.scrollHeight,clamp:getComputedStyle(e).webkitLineClamp}));
    assert.ok(descriptionMetrics.scroll<=descriptionMetrics.client+1,'Full description must not be clipped inside its text box');
    assert.equal(descriptionMetrics.clamp,'none');
    // Scroll the preceding row into view, then genuinely move the pointer onto
    // it. Highlight must change without changing the user's scroll position.
    await page.locator('.mention-popover').evaluate(e=>{
      e.scrollTop+=e.children[2].getBoundingClientRect().top-e.getBoundingClientRect().top-e.clientTop;
    });
    const hoverScroll=await page.locator('.mention-popover').evaluate(e=>e.scrollTop);
    await page.locator('.mention-item').nth(2).hover();
    assert.equal(await page.locator('.mention-item.active .mention-label').textContent(),'no-description');
    assert.equal(await page.locator('.mention-popover').evaluate(e=>e.scrollTop),hoverScroll);
    await page.mouse.move(0,0);
    await prompt.press('ArrowDown');
    await visibleHeading();
    // Read within the tall description without snapping back to its heading.
    await page.locator('.mention-popover').evaluate(e=>e.scrollTop+=160);
    const scroll=await page.locator('.mention-popover').evaluate(e=>e.scrollTop);
    await huge.locator('.mention-desc').hover({position:{x:40,y:220}});
    assert.equal(await page.locator('.mention-popover').evaluate(e=>e.scrollTop),scroll);
    const endVisible=await huge.locator('.mention-desc').evaluate(e=>{
      const box=e.closest('.mention-popover');
      box.scrollTop+=e.getBoundingClientRect().bottom-(box.getBoundingClientRect().top+box.clientTop+box.clientHeight);
      const range=document.createRange();
      range.setStart(e.firstChild,e.textContent.length-'END-OF-DESCRIPTION'.length);range.setEnd(e.firstChild,e.textContent.length);
      const r=range.getBoundingClientRect(), b=box.getBoundingClientRect();
      return r.top>=b.top && r.bottom<=b.bottom;
    });
    assert.ok(endVisible,'Description tail must be reachable by list scrolling');
    await prompt.press('ArrowDown');await visibleHeading();
    await prompt.press('Enter');
    assert.equal(await prompt.locator('.mention-chip').textContent(),'$skill-0');
    await prompt.fill('$again');await page.locator('.mention-item').first().waitFor();
    await prompt.press('Tab');assert.equal(await prompt.locator('.mention-chip').textContent(),'$frontend-design');
    await prompt.fill('$again');await page.locator('.mention-item').first().waitFor();
    await prompt.press('Escape');assert.equal(await page.locator('.mention-popover').count(),0);
    assert.equal(await page.getByRole('dialog').count(),1);
    await prompt.fill('$mouse');await page.locator('.mention-item').first().click();
    assert.equal(await prompt.locator('.mention-chip').textContent(),'$frontend-design');
    await prompt.fill('@App');await page.locator('.mention-item').first().waitFor();
    assert.equal(await page.locator('.mention-desc').count(),0);
    assert.ok(await page.locator('.mention-popover').evaluate(e=>e.scrollWidth<=e.clientWidth));
    await prompt.press('Enter');assert.equal(await prompt.locator('.mention-chip').getAttribute('ref'),'E:/fixture/App.tsx');
    await page.getByRole('textbox',{name:'标题',exact:true}).fill('Skill 验收任务');
    await page.getByRole('button',{name:'添加任务',exact:true}).click();
    await page.getByRole('button',{name:'编辑',exact:true}).click();
    await prompt.fill('$edit');await page.locator('.mention-item').first().waitFor();
    await prompt.press('Tab');
    await page.getByRole('button',{name:'保存',exact:true}).click();
    const saved=await page.evaluate(()=>JSON.parse(probe.tasks[0].prompt_json));
    assert.deepEqual(saved.content[0].content[0].attrs,{kind:'skill',label:'frontend-design',ref:'E:/skills/frontend-design/SKILL.md'});
    assert.deepEqual(errors,[]);
    fs.writeFileSync(dir+'/results.json',JSON.stringify({passed:true,coverage:'Chromium + real React/Tiptap; mocked Tauri IPC, no native/Herdr acceptance',metrics,geometry,errors},null,2));
    console.log('PASS: layout, full text, viewport bounds, keyboard scrolling/wraparound, Enter/Tab/Escape, mouse, file references, create/edit persistence');
  }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1});
