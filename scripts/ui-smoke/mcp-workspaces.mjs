import assert from 'node:assert/strict';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { once } from 'node:events';
import { chromium } from 'playwright';
import { workbenchState, goalDetail } from '../../src/mcp_tests/workbench_upgrade_fixture.mjs';
import { baseState, project } from '../../src/mcp_tests/work_result_app_fixture.mjs';

// Shipped Apps in a private, synthetic MCP Host. No credentials or real projects.
const baseline = process.argv.includes('--baseline');
const output = new URL(`../../artifacts/mcp-workspaces/${baseline ? 'baseline/' : ''}`, import.meta.url);
await mkdir(output, { recursive: true });
const server = createServer((request, response) => {
  response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
  response.end('<!doctype html><style>body{margin:0}iframe{display:block;width:100%;height:720px;border:0}</style>');
});
server.listen(0, '127.0.0.1');
await once(server, 'listening');
const browser = await chromium.launch({ headless: true,
  ...(process.env.WEBCODEX_SMOKE_BROWSER ? { executablePath: process.env.WEBCODEX_SMOKE_BROWSER } : {}) });
const report = { fixture: true, nativeHost: false, browser: browser.version(), cases: [] };

async function mount(kind, width, mode = 'fullscreen', empty = false) {
  const html = await readFile(new URL(`../../src/mcp_${kind}_app.html`, import.meta.url), 'utf8');
  // Deliberately disagree with the Host theme, so system preference cannot hide a bug.
  const page = await browser.newPage({ viewport: { width, height: 720 }, colorScheme: 'light', reducedMotion: 'reduce' });
  page.setDefaultTimeout(5000);
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(`http://127.0.0.1:${server.address().port}/`);
  await page.evaluate(({ html, state, kind, mode, empty, workbenchState, goalDetail }) => {
    const frame = document.createElement('iframe');
    frame.setAttribute('sandbox', 'allow-scripts allow-same-origin');
    window.fixture = { sizes: [], calls: [], errors: [], closed: false };
    const notify = (method, params) => frame.contentWindow.postMessage({ jsonrpc: '2.0', method, params }, '*');
    const result = output => ({ structuredContent: { success: true, output } });
    const reply = (id, value) => frame.contentWindow.postMessage({ jsonrpc: '2.0', id, result: value }, '*');
    const runner = { runners: [{ client_id: 'special', connected: true }], truncated: false };
    const projects = { items: [{ type: 'resource_link', uri: 'webcodex-resource://project/YWdlbnQ6c3BlY2lhbA/ZGVtbw',
      name: state.project, title: 'WebCodex', _meta: { kind: 'project', project: state.project,
        client_id: 'special', path: '/projects/webcodex' } }], next_offset: null };
    const fixtureWorkbench = structuredClone(workbenchState);
    const fixtureTask = fixtureWorkbench.tasks.tasks[0];
    fixtureTask.project = state.project;
    fixtureTask.workbench_actions = {
      cancel: { project: state.project, run_id: 'wc_agent_run_abcdefghijklmnop' },
      observe: { project: state.project, run_id: 'wc_agent_run_abcdefghijklmnop' },
      reconcile: { task_id: fixtureTask.task_id, attempt_id: 'wc_agent_task_attempt_abcdefghijklmnop' },
    };
    fixtureWorkbench.todos = [{ session_id: 'wc_sess_abcdefghijklmnop', message_id: 'wc_msg_abcdefghijklmnop', message: 'Review the requested permission' }];
    const fixtureGoal = structuredClone(goalDetail);
    fixtureGoal.plan.progress_summary = 'Navigation and review are underway.';
    fixtureGoal.plan.steps = [{ id: 'step_navigation', title: 'Make navigation clear', status: 'in_progress', updated_at_unix_ms: 1 }];
    fixtureGoal.correlations = [{ kind: 'agent_task', reference_id: fixtureTask.task_id, created_at_unix_ms: 1 }];
    const goalResource = { type: 'resource_link', uri: 'webcodex-resource://goal/Z29hbA', name: fixtureGoal.summary.title, title: fixtureGoal.summary.title, _meta: { kind: 'goal', goal_id: fixtureGoal.summary.goal_id } };
    fixture.completedTodo = false;
    fixture.workSubmitAttempts = 0;
    window.changeContext = context => notify('ui/notifications/host-context-changed', context);
    window.closeApp = () => frame.contentWindow.postMessage({ jsonrpc: '2.0', id: 9000, method: 'ui/resource-teardown', params: {} }, '*');
    addEventListener('message', event => {
      if (event.source !== frame.contentWindow || event.data?.jsonrpc !== '2.0') return;
      const message = event.data;
      if (message.id === 9000 && message.result) fixture.closed = true;
      else if (message.method === 'ui/initialize') reply(message.id, {
        protocolVersion: '2026-01-26', hostCapabilities: {}, hostContext: { theme: 'dark', displayMode: mode },
      });
      else if (message.method === 'ui/notifications/initialized') {
        notify('ui/notifications/tool-input', { arguments: kind === 'workbench' || empty ? {} : { project: state.project } });
        const value = result(kind === 'workbench' ? { runners: runner, projects } : { work_result: empty ? null : state });
        if (empty) value._meta = { 'webcodex/workResultThread': { empty: true, session_id: null } };
        notify('ui/notifications/tool-result', value);
      } else if (message.method === 'ui/notifications/size-changed') {
        fixture.sizes.push(message.params);
        if (mode === 'inline') frame.style.height = `${message.params.height}px`;
      } else if (message.method === 'tools/call') {
        fixture.calls.push(message.params);
        const name = message.params.name;
        if (name === 'list_runners') reply(message.id, result(runner));
        else if (name === 'search_webcodex_resources') reply(message.id, result(message.params.arguments.kind === 'project' ? projects : message.params.arguments.kind === 'goal' ? { items: [goalResource], next_offset: null } : { items: [], next_offset: null }));
        else if (name === 'list_sessions') reply(message.id, result({ sessions: [], next_offset: null }));
        else if (name === 'read_workbench_state') {
          const send = () => reply(message.id, result({ ...fixtureWorkbench, todos: fixture.completedTodo ? [] : fixtureWorkbench.todos }));
          // A real Host can deliver the first state after the initial size event.
          // Exercise this ordering deterministically instead of relying on timing.
          if (mode === 'inline' && !fixture.delayedState) { fixture.delayedState = true; setTimeout(send, 150); }
          else send();
        }
        else if (name === 'read_agent_task') reply(message.id, result({ task: { summary: fixtureTask, instruction: 'Inspect the selected workspace safely.' } }));
        else if (name === 'cancel_coding_agent') reply(message.id, result({ cancel_requested: true }));
        else if (name === 'observe_coding_agent') reply(message.id, result({ run_id: message.params.arguments.run_id, events: [{ sequence: 1, kind: 'text', text: 'Observed bounded run output' }] }));
        else if (name === 'reconcile_agent_task_coding_run') reply(message.id, result({ task_id: message.params.arguments.task_id }));
        else if (name === 'get_session_assignment') reply(message.id, result({ session_id: message.params.arguments.session_id, message_id: message.params.arguments.message_id, assignment_fence: 'wsa2_abcdefghijklmnopqrstuv', todo: { message_id: message.params.arguments.message_id } }));
        else if (name === 'complete_session_message') { fixture.completedTodo = true; reply(message.id, result({ completed: true })); }
        else if (name === 'submit_workbench_work') {
          fixture.workSubmitAttempts += 1;
          if (fixture.workSubmitAttempts === 1) reply(message.id, { structuredContent: { success: false, error: 'Runner start response was lost', output: { workbench_submission: { task_id: 'wc_agent_task_ABCDEFGHIJKLMNOP', phase: 'dispatch', retry: 'same_key_and_unchanged_input_only' } } } });
          else reply(message.id, result({ task_id: 'wc_agent_task_ABCDEFGHIJKLMNOP' }));
        }
        else if (name === 'read_webcodex_resource') reply(message.id, result({uri:message.params.arguments.uri,kind:message.params.arguments.uri.includes('/goal/')?'goal':'file',data:message.params.arguments.uri.includes('/goal/')?{goal:{summary:fixtureGoal.summary,objective:fixtureGoal.objective},summary_only:true}:{text:'# Workspace notes\n<script>fixtureMarkup()</script>\nSafe text preview.'}}));
        else if (name === 'get_goal') reply(message.id, result({ goal: fixtureGoal }));
        else if (name === 'list_project_files') reply(message.id, result({project:state.project,path:message.params.arguments.path||'',entries:[{path:'src',kind:'dir'},{path:'README.md',kind:'file'}],next_offset:null}));
        else if (name === 'get_work_result_state') reply(message.id, result({ work_result: state }));
        else { fixture.errors.push(name); reply(message.id, { structuredContent: { success: false, error: 'Unexpected fixture call' } }); }
      }
    });
    frame.srcdoc = html; document.body.append(frame);
  }, { html, state: baseState, kind, mode, empty, workbenchState, goalDetail });
  const frame = page.frames().find(frame => frame.parentFrame());
  await frame.locator('#status').filter({ hasNotText: /Connecting/ }).waitFor();
  return { page, frame, errors };
}

async function run(name, action) {
  const entry = { name, passed: false }; report.cases.push(entry);
  try { await action(); entry.passed = true; console.log(`PASS ${name}`); }
  catch (error) { entry.error = error.message; console.error(`FAIL ${name}: ${error.message}`); }
}

async function readableControls(frame) {
  const contrast = await frame.locator('#refresh').evaluate(node => {
    const style = getComputedStyle(node), canvas = document.createElement('canvas');
    canvas.width = canvas.height = 1;
    const context = canvas.getContext('2d');
    const luminance = color => {
      context.fillStyle = color; context.fillRect(0, 0, 1, 1);
      const channels = [...context.getImageData(0, 0, 1, 1).data].slice(0, 3).map(value => {
        const normalized = value / 255;
        return normalized <= .04045 ? normalized / 12.92 : ((normalized + .055) / 1.055) ** 2.4;
      });
      return channels[0] * .2126 + channels[1] * .7152 + channels[2] * .0722;
    };
    const values = [luminance(style.color), luminance(style.backgroundColor)].sort((a, b) => a - b);
    return (values[1] + .05) / (values[0] + .05);
  });
  assert.ok(contrast >= 4.5, `Refresh text contrast is ${contrast.toFixed(2)}`);
}

try {
  for (const kind of ['workbench', 'work_result']) for (const width of [320, 600, 1024, 1440]) {
    await run(`${kind}: Host theme, layout, context changes, disposal (${width}px)`, async () => {
      const { page, frame, errors } = await mount(kind, width);
      try {
        await page.screenshot({ path: new URL(`${kind}-${width}.png`, output).pathname });
        assert.equal(await frame.evaluate(() => getComputedStyle(document.documentElement).colorScheme), 'dark');
        await readableControls(frame);
        assert.ok(await frame.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'Page overflows horizontally');
        if (kind === 'workbench') {
          assert.equal(await frame.title(), workbenchState.settings.display_name, 'Document title follows the effective Workbench name');
          assert.equal(await frame.locator('#runnerSelect').inputValue(), '');
          assert.equal(await frame.locator('#projectSelect').inputValue(), '');
          assert.equal((await page.evaluate(() => fixture.calls)).filter(call => call.name === 'get_work_result_state').length, 0);
          await frame.locator('#runnerSelect').selectOption('special');
          await frame.locator('#projectSelect option').nth(1).waitFor({ state: 'attached' });
          await frame.locator('#projectSelect').selectOption(project);
          await frame.locator('#overview .metric').first().waitFor();
          await frame.locator('#tabFiles').click();
          await frame.locator('#openDirectory').click();
          await frame.locator('#rows .row').filter({hasText:'README.md'}).getByRole('button',{name:'Preview',exact:true}).click();
          await frame.locator('#markdownPreview').waitFor();
          assert.match(await frame.locator('#markdownPreview').textContent(), /<script>fixtureMarkup/);
          assert.equal(await frame.locator('#markdownPreview script').count(), 0);
          assert.ok(await frame.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'Preview overflows');
          await page.screenshot({path:new URL(`workbench-preview-${width}.png`,output).pathname});
          await frame.locator('#closePreview').click();
          await frame.locator('#tabFiles').focus();
          await frame.locator('#tabFiles').press('Enter');
          await frame.locator('#tabFiles').press('ArrowRight');
          assert.equal(await frame.locator('#tabWork').getAttribute('aria-selected'), 'true');
          await frame.locator('#goalsNav').click();
          assert.equal(await frame.locator('#tabGoals').getAttribute('aria-selected'), 'true');
          await frame.locator('#newGoal').click();
          await frame.locator('#goalTitle').fill('Workspace goal');
          await frame.locator('#goalObjective').fill('Keep the draft visible');
          assert.ok(await frame.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'Goal editor overflows');
          await frame.locator('#closeGoal').click();
          await frame.locator('#homeNav').click();
          await frame.locator('#newHomeWork').click();
          await frame.locator('#workExecutor').selectOption('1');
          assert.match(await frame.locator('#executionHint').textContent(), /default/);
          assert.ok(await frame.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'Work form overflows');
          await frame.locator('#closeWork').click();
          assert.equal(await frame.locator('#sessionSelect').inputValue(), '');
        } else {
          await frame.locator('#tabResults').focus();
          await frame.locator('#tabResults').press('Enter');
          await frame.locator('#tabResults').press('ArrowRight');
          assert.equal(await frame.locator('#tabCollaboration').getAttribute('aria-selected'), 'true');
          await frame.locator('#tabActivity').click();
          assert.equal(await frame.locator('#taskContext').isVisible(), false, 'Raw identity dominates the main view');
          if (width === 320) {
            const summary = frame.locator('.activity-call > summary').first();
            assert.ok(await summary.evaluate(node => node.querySelector('.call-outcome').getBoundingClientRect().top
              >= node.querySelector('.call-main').getBoundingClientRect().bottom), 'Narrow activity still squeezes two columns');
          }
        }
        await page.evaluate(() => changeContext({ theme: 'light', displayMode: 'inline' }));
        await frame.waitForFunction(() => getComputedStyle(document.documentElement).colorScheme === 'light');
        await readableControls(frame);
        await page.evaluate(() => changeContext({ theme: 'invalid', displayMode: 'invalid' }));
        assert.equal(await frame.evaluate(() => getComputedStyle(document.documentElement).colorScheme), 'light');
        assert.ok(await frame.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
        await page.evaluate(() => closeApp());
        await page.waitForFunction(() => fixture.closed);
        const count = await page.evaluate(() => fixture.sizes.length);
        await page.evaluate(() => changeContext({ theme: 'dark', displayMode: 'fullscreen' }));
        await page.waitForTimeout(100);
        assert.equal(await page.evaluate(() => fixture.sizes.length), count, 'Disposed App still reports size');
        assert.deepEqual(await page.evaluate(() => fixture.errors), []);
        assert.deepEqual(errors, []);
      } finally { await page.close(); }
    });
  }
  for (const kind of ['workbench', 'work_result']) await run(`${kind}: inline height follows content without resize loops`, async () => {
    const { page, frame, errors } = await mount(kind, 390, 'inline');
    try {
      if (kind === 'workbench') await frame.locator('#homeWork .work-card').first().waitFor();
      // Baseline the rendered state, not the earlier loading skeleton. A delayed
      // state response changes content height independently of the disclosure.
      await page.waitForFunction(() => {
        const body = document.querySelector('iframe')?.contentDocument?.body;
        return body && fixture.sizes.at(-1)?.height === Math.ceil(body.getBoundingClientRect().height);
      });
      const before = await page.evaluate(() => fixture.sizes.at(-1).height);
      const disclosure = frame.locator(kind === 'workbench' ? '.discovery > summary' : '#activityHistory > summary');
      await disclosure.click();
      await page.waitForFunction(before => fixture.sizes.at(-1).height > before + 20, before);
      await disclosure.click();
      await page.waitForFunction(before => fixture.sizes.at(-1).height <= before, before);
      const count = await page.evaluate(() => fixture.sizes.length);
      // Hosts can repeat an unchanged display mode after a size update.
      await page.evaluate(() => changeContext({ displayMode: 'inline' }));
      await page.waitForTimeout(200);
      assert.equal(await page.evaluate(() => fixture.sizes.length), count, 'Size notification loop');
      assert.ok(await frame.evaluate(() => document.documentElement.scrollHeight <= innerHeight + 1), 'Inline content is clipped');
      assert.deepEqual(errors, []);
    } finally { await page.close(); }
  });
  await run('workbench: projected run actions and assignment-fenced todo completion', async () => {
    const { page, frame, errors } = await mount('workbench', 900, 'fullscreen');
    try {
      const task = frame.locator('#homeWork .work-card').first();
      await task.getByRole('button', { name: 'Cancel' }).click();
      await task.getByText(/Cancellation requested · awaiting terminal observation/).waitFor();
      assert.equal(await task.locator('.badge').textContent(), 'active', 'Cancellation request must not be presented as a terminal state');
      await task.locator('details > summary').click();
      await task.getByRole('button', { name: 'Observe' }).click();
      await task.getByText('Observed bounded run output').waitFor();
      await task.getByRole('button', { name: 'Reconcile' }).click();
      await frame.locator('#sharedAttention').getByRole('button', { name: 'Review' }).click();
      await frame.locator('#todoAnswer').fill('Reviewed with the requested response');
      await frame.locator('#todoDetail').getByRole('button', { name: 'Complete message' }).click();
      await frame.locator('#todoDetail').waitFor({ state: 'hidden' });
      const calls = await page.evaluate(() => fixture.calls);
      assert.deepEqual(calls.find(call => call.name === 'cancel_coding_agent').arguments, { run_id: 'wc_agent_run_abcdefghijklmnop' });
      assert.deepEqual(calls.find(call => call.name === 'observe_coding_agent').arguments, { run_id: 'wc_agent_run_abcdefghijklmnop', wait_secs: 0 });
      assert.deepEqual(calls.find(call => call.name === 'reconcile_agent_task_coding_run').arguments, { task_id: workbenchState.tasks.tasks[0].task_id, attempt_id: 'wc_agent_task_attempt_abcdefghijklmnop' });
      const completion = calls.find(call => call.name === 'complete_session_message').arguments;
      assert.equal(completion.session_id, 'wc_sess_abcdefghijklmnop');
      assert.equal(completion.message_id, 'wc_msg_abcdefghijklmnop');
      assert.equal(completion.answer, 'Reviewed with the requested response');
      assert.equal(completion.expected_assignment_fence, 'wsa2_abcdefghijklmnopqrstuv');
      assert.match(completion.completion_key, /^[0-9a-f-]{36}$/);
      assert.equal((await page.evaluate(() => fixture.calls.filter(call => call.name === 'read_workbench_state').length)) >= 2, true, 'Reconcile and todo success refresh Workbench state');
      await frame.locator('#goalsNav').click();
      await frame.locator('#rows .row').waitFor();
      await frame.locator('#rows .row').getByRole('button', { name: 'Preview' }).click();
      const goalPreview = frame.locator('#markdownPreview');
      await goalPreview.getByText('Steps', { exact: true }).waitFor();
      const goalText = await goalPreview.textContent();
      assert.match(goalText, /Make navigation clear · in_progress/);
      assert.match(goalText, new RegExp(workbenchState.tasks.tasks[0].title));
      assert.match(goalText, new RegExp(workbenchState.tasks.tasks[0].task_id));
      assert.deepEqual(await page.evaluate(() => fixture.errors), []);
      assert.deepEqual(errors, []);
    } finally { await page.close(); }
  });
  await run('workbench: uncertain submission only replays the unchanged payload and key', async () => {
    const { page, frame, errors } = await mount('workbench', 900, 'fullscreen');
    try {
      await frame.locator('#newHomeWork').click();
      await frame.locator('#workProject').selectOption(project);
      await frame.locator('#workExecutor').selectOption('1');
      await frame.locator('#workTitle').fill('Replay safe task');
      await frame.locator('#workInstruction').fill('Run this exact instruction once');
      await frame.locator('#submitWork').click();
      await frame.locator('#workTaskIdentity').filter({ hasText: 'wc_agent_task_ABCDEFGHIJKLMNOP' }).waitFor();
      const first = await page.evaluate(() => fixture.calls.find(call => call.name === 'submit_workbench_work').arguments);
      await frame.locator('#workInstruction').fill('Changed instruction');
      assert.equal(await frame.locator('#submitWork').isDisabled(), true);
      await frame.locator('#closeWork').click();
      await frame.locator('#newHomeWork').click();
      assert.equal(await frame.locator('#workInstruction').inputValue(), 'Changed instruction');
      assert.equal(await frame.locator('#submitWork').isDisabled(), true);
      await frame.locator('#workInstruction').fill('Run this exact instruction once');
      await frame.locator('#submitWork').click();
      await frame.locator('#workStatus').filter({ hasText: 'Submitted Task wc_agent_task_ABCDEFGHIJKLMNOP' }).waitFor();
      const calls = await page.evaluate(() => fixture.calls.filter(call => call.name === 'submit_workbench_work').map(call => call.arguments));
      assert.equal(calls.length, 2);
      assert.deepEqual(calls[1], first);
      assert.equal(await frame.locator('#workTitle').inputValue(), '');
      assert.equal(await frame.locator('#workInstruction').inputValue(), '');
      assert.deepEqual(await page.evaluate(() => fixture.errors), []);
      assert.deepEqual(errors, []);
    } finally { await page.close(); }
  });
  await run('work_result: first-open empty state has no target or runtime reads', async () => {
    const { page, frame, errors } = await mount('work_result', 390, 'fullscreen', true);
    try {
      await frame.locator('#emptyState').waitFor();
      assert.match(await frame.locator('#emptyState').textContent(), /No work result/);
      assert.equal(await frame.locator('#viewTabs').isVisible(), false);
      assert.equal(await frame.locator('#refresh').isEnabled(), false);
      assert.deepEqual(await page.evaluate(() => fixture.calls), []);
      assert.deepEqual(errors, []);
      await page.screenshot({ path: new URL('work-result-empty.png', output).pathname });
    } finally { await page.close(); }
  });
} finally {
  await writeFile(new URL('report.json', output), JSON.stringify(report, null, 2) + '\n');
  await browser.close(); server.close();
}
if (!baseline && report.cases.some(entry => !entry.passed)) process.exitCode = 1;
