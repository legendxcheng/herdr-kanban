// Real React/Tiptap in Edge; native WezTerm IPC is simulated. No prompt is sent.
// PLAYWRIGHT_MODULE may point to an existing Playwright installation.
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const out = path.resolve(__dirname, '../.scratch/language-verification');
fs.mkdirSync(out, { recursive: true });
(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  try {
    const page = await browser.newPage({ locale: 'fr-FR', viewport: { width: 1280, height: 800 } });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript(() => {
      const project = { id: 'p', name: 'Language acceptance', root_path: 'G:/fixture', herdr_workspace: 'fixture', created_at: '', herdr_session: 'default', workspace_id: 'w1', sync_status: 'ready', directory_source: 'herdr', herdr_directory: 'G:/fixture' };
      const target = { session: 'default', workspace_id: 'w1', pane_id: 'pane1', terminal_id: 'term1', agent: 'codex', process_id: '100', title: 'Agent', status: 'idle' };
      window.probe = { calls: [], projects: [project], tasks: [{ id: 't', project_id: 'p', name: 'Keep this task', status: 'in_progress', priority: 'high', archived_at: null, cleanup_required: false, prompt_json: '{"type":"doc","content":[{"type":"paragraph"}]}', binding: { state: 'valid', target, operation: null } }] };
      let callbackId = 0;
      window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
      window.__TAURI_INTERNALS__ = {
        transformCallback() { return ++callbackId; }, unregisterCallback() {},
        invoke: async (command, args = {}) => {
          probe.calls.push({ command, args });
          switch (command) {
            case 'plugin:event|listen': return ++callbackId;
            case 'plugin:event|unlisten': return;
            case 'list_projects': return probe.projects;
            case 'sync_projects': return { projects: probe.projects, workspaces: [], operations: [], session: 'default' };
            case 'project_git_statuses': return [{ project_id: 'p', root_path: project.root_path, state: 'branch', head: 'main' }];
            case 'workspace_operations': return [];
            case 'list_tasks': return probe.tasks;
            case 'herdr_list_agents': return [{ pane_id: 'pane1', name: 'codex', title: 'Agent', status: 'idle', target, verified_task_id: 't' }];
            case 'terminal_open':
              return args.connectionId;
            case 'terminal_bounds': case 'terminal_close': return;
            case 'task_overview': return [{ project_id: 'p', todo: 0, in_progress: 1, done: 0, today_done: 0 }];
            case 'create_task': throw 'image.storage: fixture diagnostic';
            default: throw new Error('Unexpected IPC: ' + command);
          }
        },
      };
    });
    await page.goto(process.env.KANBAN_VERIFY_URL || 'http://127.0.0.1:1449');
    await page.getByText('Herdr open', { exact: true }).waitFor();
    assert.equal(await page.locator('html').getAttribute('lang'), 'en-US', 'Unsupported system language uses English');
    const terminalCalls = () => page.evaluate(() => probe.calls.filter(call => ['terminal_open', 'terminal_close'].includes(call.command)).map(call => ({ command: call.command, id: call.args.connectionId })));
    const initial = await terminalCalls();
    const geometry = [];
    for (const locale of ['zh-CN', 'en-US']) {
      await page.locator('.language-select select').selectOption(locale);
      for (const [width, height] of [[900, 600], [1280, 800], [1920, 1080]]) {
        await page.setViewportSize({ width, height });
        await page.screenshot({ path: path.join(out, `board-${locale}-${width}.png`) });
        const measured = await page.evaluate(() => {
          const selector = document.querySelector('.language-select select').getBoundingClientRect();
          const sidebar = document.querySelector('.sidebar');
          return { width: innerWidth, height: innerHeight, selector: { x: selector.x, y: selector.y, right: selector.right, bottom: selector.bottom }, overflow: sidebar.scrollWidth - sidebar.clientWidth, pageOverflow: document.documentElement.scrollWidth - innerWidth };
        });
        assert.ok(measured.selector.bottom <= height && measured.selector.right <= width);
        assert.ok(measured.overflow <= 1 && measured.pageOverflow <= 1, JSON.stringify(measured));
        geometry.push({ locale, ...measured });
      }
    }
    assert.deepEqual(await terminalCalls(), initial, 'Switching language must not reconnect terminal');
    await page.waitForFunction(() => probe.calls.some(call => call.command === 'terminal_bounds'));
    await page.getByRole('button', { name: /New task/ }).click();
    const editor = page.getByRole('textbox', { name: 'Prompt', exact: true });
    await editor.fill('中文 draft remains intact');
    await page.getByRole('combobox', { name: 'Priority' }).selectOption('urgent');
    await page.getByRole('button', { name: 'Add task', exact: true }).click();
    await page.getByRole('alert').waitFor();
    // Modal focus remains trapped; exercise the locale store directly while open.
    await page.evaluate(async () => (await import('/src/i18n.ts')).setLocale('zh-CN'));
    assert.equal(await page.getByRole('textbox', { name: '提示词', exact: true }).textContent(), '中文 draft remains intact');
    assert.equal(await page.getByRole('combobox', { name: '优先级' }).inputValue(), 'urgent');
    assert.match(await page.getByRole('alert').textContent(), /图片读写失败.*fixture diagnostic/);
    for (const locale of ['zh-CN', 'en-US']) {
      await page.evaluate(async locale => (await import('/src/i18n.ts')).setLocale(locale), locale);
      await page.setViewportSize({ width: 900, height: 600 });
      await page.screenshot({ path: path.join(out, `form-${locale}-900.png`) });
    }
    await page.getByRole('button', { name: 'Cancel', exact: true }).click();
    await page.getByRole('button', { name: 'Task overview', exact: true }).click();
    await page.getByRole('table').waitFor();
    await page.screenshot({ path: path.join(out, 'overview-en-US-900.png') });
    await page.getByRole('button', { name: 'Close', exact: true }).click();
    await page.reload();
    await page.getByText('Herdr open', { exact: true }).waitFor();
    assert.equal(await page.locator('html').getAttribute('lang'), 'en-US', 'Saved preference survives reload');
    assert.equal(await page.locator('.language-select select').inputValue(), 'en-US');
    assert.deepEqual(errors, []);
    fs.writeFileSync(path.join(out, 'result.json'), JSON.stringify({ passed: true, geometry, errors, evidence: 'Edge + real React/Tiptap; simulated native-terminal IPC' }, null, 2));
    console.log('Language browser verification passed: ' + out);
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
