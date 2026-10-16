import test from "node:test";
import assert from "node:assert/strict";
import { app, flush, toolResult } from "./app_test_support.mjs";
import { project, session_id, baseState } from "./work_result_app_fixture.mjs";

const projectUri = "webcodex-resource://project/YWdlbnQ6ZGVtbw/cm9vdA";
const fileUri = "webcodex-resource://file/YWdlbnQ6ZGVtbw/cm9vdA/c3JjL2EucnM";
const secondProject = "agent:other:project";
const runnerId = project.split(":")[1];
const runnerPage = {runners:[{client_id:runnerId,connected:true},{client_id:"other",connected:false}],count:2,truncated:false};
const projectItem = { type:"resource_link", uri:projectUri, name:project, title:"Demo Project", _meta:{kind:"project",project,client_id:runnerId,path:"/repo/demo"} };
const fileItem = { type:"resource_link", uri:fileUri, name:"src/a.rs", description:"Rust source", _meta:{kind:"file",project,path:"src/a.rs"} };
const page = (items = [], next_offset = null) => ({items,offset:0,limit:50,next_offset});
const contentOnly = output => ({ content:[{type:"text",text:JSON.stringify({success:true,output})}] });
const refs = view => view.sent.filter(request=>request.method === "ui/update-model-context");
const readRequests = view => view.calls("read_webcodex_resource");
const search = (view, kind) => view.calls("search_webcodex_resources").filter(request=>request.params.arguments.kind===kind);

test("empty reference selection does not show a blank manual-copy field", async () => {
  const view = app("mcp_workbench_app.html");
  await initialize(view);
  assert.equal(view.nodes.copyLabel.hidden, true);
});
async function initialize(view, modalities = {}, context = undefined, extension = false) {
  await view.reply(view.sent[0], {protocolVersion:"2026-01-26",hostCapabilities:{updateModelContext:modalities,...(extension ? {experimental:{"openai/modelContext":{}}} : {})},...(context===undefined ? {} : {hostContext:{"openai/modelContext":context}})});
}
async function launch(modalities = {}) {
  const view = app("mcp_workbench_app.html");
  view.toolInput({});
  view.toolResult({runners:runnerPage,client_id:runnerId,projects:page([projectItem])});
  await initialize(view,modalities);
  return view;
}
async function selectProject(view, value = project) {
  view.nodes.projectSelect.value=value;
  view.nodes.projectSelect.onchange();
  await flush();
}
async function files(view, items = [fileItem]) {
  await selectProject(view);
  view.nodes.tabFiles.onclick();
  await flush();
  await view.reply(search(view,"file").at(-1),toolResult(page(items)));
}
const rowActions = view => view.nodes.rows.children[0].children[1].children;

for (const order of ["input-result-init","result-init-input","init-input-result"]) test(`empty launcher needs an explicit Project even for one result: ${order}`,async()=>{
  const view=app("mcp_workbench_app.html");
  const input=()=>view.toolInput({});
  const result=()=>view.toolResult({runners:runnerPage,projects:page([])});
  for (const step of order.split("-")) { if(step==="input") input(); else if(step==="result") result(); else await initialize(view); }
  await flush();
  assert.equal(view.nodes.projectSelect.value,"");
  assert.equal(view.nodes.projectSelect.children.length,1);
  assert.equal(view.nodes.runnerSelect.value,"");
  assert.equal(view.nodes.projectSelect.disabled,true);
  assert.equal(view.nodes.selectionState.textContent,"Choose a Runner to begin.");
  assert.equal(view.calls("get_work_result_state").length,0);
  assert.equal(view.calls("list_sessions").length,0);
  assert.equal(readRequests(view).length,0);
  assert.equal(refs(view).length,0);
});

test("empty launch recovers missing structuredContent with bounded JSON text",async()=>{
  const view=app("mcp_workbench_app.html");
  view.toolInput({});await initialize(view);
  await view.reply(view.calls("list_runners")[0],contentOnly(runnerPage));
  view.nodes.runnerSelect.value=runnerId;view.nodes.runnerSelect.onchange();await flush();
  const request=search(view,"project")[0];
  assert.equal(request.params.arguments.client_id,runnerId);
  await view.reply(request,contentOnly(page([projectItem])));
  assert.equal(view.nodes.projectSelect.children[1].textContent,"Demo Project · /repo/demo");
  assert.equal(view.nodes.projectSelect.value,"");
  await selectProject(view);
  await view.reply(view.calls("get_work_result_state")[0],contentOnly({work_result:baseState}));
  assert.equal(view.nodes.overview.children.length,4);
});

test("standard initial text launcher fallback uses supplied project resources",async()=>{
  const view=app("mcp_workbench_app.html");
  view.notification("ui/notifications/tool-result",contentOnly({projects:page([projectItem])}));
  await initialize(view);
  assert.equal(view.nodes.projectSelect.children.length,2);
  assert.equal(search(view,"project").length,0);
});

test("explicit launch scope works in either notification order and reads only overview",async()=>{
  for (const resultFirst of [false,true]) {
    const view=app("mcp_workbench_app.html");
    if(resultFirst)view.toolResult({project,session_id});
    view.toolInput({project,session_id});
    if(!resultFirst)view.toolResult({project,session_id});
    await initialize(view);
    assert.equal(view.nodes.projectSelect.value,project);
    assert.equal(view.nodes.sessionSelect.value,session_id);
    assert.deepEqual({...view.calls("get_work_result_state")[0].params.arguments},{project,session_id});
    assert.equal(readRequests(view).length,0);
    assert.equal(search(view,"file").length,0);
  }
});

test("named launch maps canonical Project IDs to a visible option when discovery includes short refs",async()=>{
  const view=app("mcp_workbench_app.html");
  view.toolInput({project:"~p1"});
  view.toolResult({project,projects:page([{...projectItem,_meta:{...projectItem._meta,project_ref:"~p1"}}])});
  await initialize(view);
  assert.equal(view.nodes.projectSelect.value,project);
  assert.ok(view.nodes.projectSelect.children.some(option=>option.value===view.nodes.projectSelect.value));
  assert.equal(view.calls("get_work_result_state")[0].params.arguments.project,project);
});

test("Session list never chooses a candidate and artifacts require explicit Session",async()=>{
  const view=await launch();await selectProject(view);
  await view.reply(view.calls("list_sessions")[0],toolResult({sessions:[{session_id,title:"A Session"}],next_offset:null}));
  assert.equal(view.nodes.sessionSelect.value,"");
  view.nodes.tabArtifacts.onclick();await flush();
  assert.equal(search(view,"artifact").length,0);
  assert.match(view.nodes.selectionState.textContent,/Choose a Workflow Session/);
  view.nodes.sessionSelect.value=session_id;view.nodes.sessionSelect.onchange();await flush();
  assert.deepEqual({...search(view,"artifact")[0].params.arguments},{kind:"artifact",project,session_id,offset:0,limit:50});
});

test("Project switch immediately clears rows, preview and Session, fencing late replies",async()=>{
  const view=await launch();await files(view);
  rowActions(view)[0].onclick();await flush();
  const oldRead=readRequests(view)[0];
  const oldOverview=view.calls("get_work_result_state")[0];
  await selectProject(view,secondProject);
  assert.equal(view.nodes.rows.children.length,0);
  assert.equal(view.nodes.preview.hidden,true);
  assert.equal(view.nodes.previewContent.textContent,"");
  assert.equal(view.nodes.sessionSelect.value,"");
  await view.reply(oldRead,toolResult({uri:fileUri,kind:"file",data:"OLD FILE"}));
  await view.reply(oldOverview,toolResult({work_result:baseState}));
  assert.equal(view.nodes.previewContent.textContent,"");
  assert.equal(view.nodes.overview.children.length,0);
  assert.ok(view.sent.some(request=>request.method==="notifications/cancelled" && request.params.requestId===oldRead.id));
});

test("Session switch clears artifact rows and ignores old artifact search",async()=>{
  const view=await launch();await selectProject(view);view.nodes.tabArtifacts.onclick();
  view.nodes.sessionSelect.value=session_id;view.nodes.sessionSelect.onchange();await flush();
  const old=search(view,"artifact")[0];
  const newSession="wc_sess_"+"2".repeat(32);
  view.nodes.sessionSelect.value=newSession;view.nodes.sessionSelect.onchange();await flush();
  assert.equal(view.nodes.rows.children.length,0);
  await view.reply(old,toolResult(page([{...fileItem,_meta:{...fileItem._meta,kind:"artifact"}}])));
  assert.equal(view.nodes.rows.children.length,0);
  assert.equal(search(view,"artifact").at(-1).params.arguments.session_id,newSession);
});

test("area/search fences late replies, supports paging and never eagerly reads",async()=>{
  const view=await launch();await selectProject(view);view.nodes.tabFiles.onclick();await flush();
  const old=search(view,"file")[0];
  view.nodes.query.value="a";view.nodes.searchForm.onsubmit({preventDefault(){}});await flush();
  const current=search(view,"file")[1];
  await view.reply(old,toolResult(page([fileItem])));
  assert.equal(view.nodes.rows.children.length,0);
  await view.reply(current,toolResult({...page([fileItem],50),list_truncated:true}));
  assert.equal(view.nodes.more.hidden,false);
  assert.match(view.nodes.coverage.textContent,/Partial/);
  view.nodes.more.onclick();await flush();
  assert.equal(search(view,"file").at(-1).params.arguments.offset,50);
  assert.equal(search(view,"file").at(-1).params.arguments.query,"a");
  assert.equal(readRequests(view).length,0);
  view.nodes.tabGoals.onclick();await flush();
  assert.equal(search(view,"goal").length,1);
});

test("preview deduplicates only in-flight reads and observes latest bounded content on every later action",async()=>{
  const view=await launch();await files(view);
  rowActions(view)[0].onclick();await flush();
  rowActions(view)[0].onclick();await flush();
  assert.equal(readRequests(view).length,1);
  await view.reply(readRequests(view)[0],toolResult({uri:fileUri,kind:"file",version:"v1",data:"<script>"+"汉".repeat(30000),truncated:true}));
  assert.match(view.nodes.previewContent.textContent,/^<script>/);
  assert.ok(new TextEncoder().encode(view.nodes.previewContent.textContent).length<=24576);
  assert.match(view.nodes.previewContent.textContent,/truncated/);
  assert.match(view.nodes.previewMeta.textContent,/v1.*Partial/);
  rowActions(view)[0].onclick();await flush();
  assert.equal(readRequests(view).length,2);
  await view.reply(readRequests(view)[1],toolResult({uri:fileUri,kind:"file",version:"v2",data:"Current content"}));
  assert.equal(view.nodes.previewContent.textContent,"Current content");
  assert.match(view.nodes.previewMeta.textContent,/v2/);
});

test("resourceLink capability attaches references only after explicit action without reading or messaging",async()=>{
  const view=await launch({resourceLink:{}});await files(view);
  const add=rowActions(view)[1];add.onclick();await flush();
  assert.equal(refs(view).length,1);
  assert.equal(readRequests(view).length,0);
  assert.deepEqual(JSON.parse(JSON.stringify(refs(view)[0].params)),{content:[fileItem]});
  await view.reply(refs(view)[0],{});
  assert.equal(view.nodes.references.children.length,1);
  add.onclick();await flush();assert.equal(refs(view).length,1);
  await selectProject(view,secondProject);
  assert.equal(view.nodes.references.children.length,1);
  assert.equal(view.sent.filter(request=>request.method==="ui/message").length,0);
  assert.equal(view.calls("run_shell").length,0);
});

for (const modality of ["resource","text","none"]) test(`${modality} fallback materializes bounded content instead of relying on a URI`,async()=>{
  const view=await launch(modality==="none" ? {} : {[modality]:{}});await files(view);
  rowActions(view)[1].onclick();await flush();
  assert.equal(readRequests(view).length,1);
  assert.equal(refs(view).length,0);
  await view.reply(readRequests(view)[0],toolResult({uri:fileUri,kind:"file",data:{content:"actual file text " + "汉".repeat(20000)}}));
  if(modality==="none") {
    assert.equal(refs(view).length,0);
    assert.equal(view.nodes.copyLabel.hidden,false);
    assert.match(view.nodes.referenceText.value,/actual file text/);
    assert.ok(new TextEncoder().encode(view.nodes.referenceText.value).length<5000);
  } else {
    assert.equal(refs(view).length,1);
    const block=refs(view)[0].params.content[0];
    assert.equal(block.type,modality);
    const text=modality==="resource" ? block.resource.text : block.text;
    assert.match(text,/actual file text/);assert.match(text,/Preview truncated/);
    await view.reply(refs(view)[0],{});
  }
  assert.equal(view.sent.filter(request=>request.method==="ui/message").length,0);
});

test("context failure does not claim selection; remove replaces complete App content",async()=>{
  const view=await launch({resourceLink:{}});await files(view);
  rowActions(view)[1].onclick();await flush();await view.reject(refs(view)[0]);
  assert.equal(view.nodes.references.children.length,0);
  assert.match(view.nodes.referenceStatus.textContent,/Unavailable/);
  rowActions(view)[1].onclick();await flush();await view.reply(refs(view)[1],{});
  view.nodes.references.children[0].children[1].onclick();await flush();
  assert.equal(refs(view)[2].params.content.length,0);
  await view.reply(refs(view)[2],{});assert.equal(view.nodes.references.children.length,0);
});

const anotherFile={...fileItem,uri:"webcodex-resource://file/YWdlbnQ6ZGVtbw/cm9vdA/c3JjL2IucnM",name:"src/b.rs",_meta:{...fileItem._meta,path:"src/b.rs"}};
for (const modality of ["text","resource"]) test(`${modality} rapid reference selections preserve both resources across Host acknowledgements`,async()=>{
  const view=await launch({[modality]:{}});await files(view,[fileItem,anotherFile]);
  rowActions(view)[1].onclick();
  view.nodes.rows.children[1].children[1].children[1].onclick();await flush();
  assert.equal(readRequests(view).length,1);
  await view.reply(readRequests(view)[0],toolResult({uri:fileUri,kind:"file",data:{text:"First reference"}}));
  assert.equal(refs(view).length,1);
  await view.reply(refs(view)[0],{});
  assert.equal(readRequests(view).length,2);
  await view.reply(readRequests(view)[1],toolResult({uri:anotherFile.uri,kind:"file",data:{text:"Second reference"}}));
  assert.equal(refs(view).length,2);
  assert.equal(refs(view)[1].params.content.length,2);
  assert.match(JSON.stringify(refs(view)[1].params.content),/First reference/);
  assert.match(JSON.stringify(refs(view)[1].params.content),/Second reference/);
  await view.reply(refs(view)[1],{});
  assert.equal(view.nodes.references.children.length,2);
  assert.equal(view.sent.filter(request=>request.method==="ui/message").length,0);
});

test("resourceLink rapid selections serialize complete context updates without losing the second choice",async()=>{
  const view=await launch({resourceLink:{}});await files(view,[fileItem,anotherFile]);
  rowActions(view)[1].onclick();
  view.nodes.rows.children[1].children[1].children[1].onclick();await flush();
  assert.equal(refs(view).length,1);
  await view.reply(refs(view)[0],{});
  assert.equal(refs(view).length,2);
  assert.deepEqual(JSON.parse(JSON.stringify(refs(view)[1].params.content)),[fileItem,anotherFile]);
  await view.reply(refs(view)[1],{});
  assert.equal(view.nodes.references.children.length,2);
});

test("canonical Host removal invalidates queued reference selections as well as an in-flight update",async()=>{
  const view=app("mcp_workbench_app.html");view.toolInput({});view.toolResult({projects:page([projectItem])});
  await initialize(view,{resourceLink:{}},null,true);await files(view,[fileItem,anotherFile]);
  rowActions(view)[1].onclick();
  view.nodes.rows.children[1].children[1].children[1].onclick();await flush();
  view.notification("ui/notifications/host-context-changed",{"openai/modelContext":{updateId:"removed",content:[]}});await flush();
  await view.reply(refs(view)[0],{_meta:{"openai/modelContext":{updateId:"previous"}}});
  assert.equal(refs(view).length,1);
  assert.equal(view.nodes.references.children.length,0);
});

test("OpenAI canonical context restores remount references and follows host removal without messages",async()=>{
  const view=app("mcp_workbench_app.html");view.toolInput({});view.toolResult({projects:page([projectItem])});
  const stripped={...fileItem};delete stripped._meta;
  await initialize(view,{resourceLink:{}},{updateId:"restored",content:[stripped]},true);
  assert.equal(view.nodes.references.children.length,1);
  view.notification("ui/notifications/host-context-changed",{"openai/modelContext":{updateId:"removed",content:[]}});await flush();
  assert.equal(view.nodes.references.children.length,0);
  view.notification("ui/notifications/host-context-changed",{"openai/modelContext":null});await flush();
  assert.equal(refs(view).length,0);
  assert.equal(view.sent.filter(request=>request.method==="ui/message").length,0);
});

test("host canonical removal during an update wins over a stale acknowledgement",async()=>{
  const view=app("mcp_workbench_app.html");view.toolInput({});view.toolResult({projects:page([projectItem])});
  await initialize(view,{resourceLink:{}},null,true);await files(view);
  rowActions(view)[1].onclick();await flush();
  view.notification("ui/notifications/host-context-changed",{"openai/modelContext":{updateId:"removed",content:[]}});await flush();
  await view.reply(refs(view)[0],{_meta:{"openai/modelContext":{updateId:"previous"}}});
  assert.equal(view.nodes.references.children.length,0);
});

test("host brand never implies adapter support and forged messages cannot alter state",async()=>{
  const view=await launch({resourceLink:{}});await files(view);
  view.notification("ui/notifications/host-context-changed",{"openai/modelContext":{updateId:"fake",content:[fileItem]}});await flush();
  assert.equal(view.nodes.references.children.length,0);
  view.toolInput({project:secondProject},{});await flush();
  assert.equal(view.nodes.projectSelect.value,project);
});

test("overview refresh pauses while hidden, resumes on visibility, and teardown cancels pending work",async()=>{
  const view=await launch({resourceLink:{}});await selectProject(view);
  await view.reply(view.calls("get_work_result_state")[0],toolResult({work_result:baseState}));
  await view.reply(view.calls("list_sessions")[0],toolResult({sessions:[]}));
  for(const request of view.calls("read_workbench_state"))await view.reject(request);
  assert.equal(view.timers.size,1);
  await view.visibility(true);assert.equal(view.timers.size,0);
  await view.fireTimers(10000);assert.equal(view.calls("get_work_result_state").length,1);
  await view.visibility(false);assert.equal(view.calls("get_work_result_state").length,2);
  const pending=view.calls("get_work_result_state")[1];
  const old=view.nodes.overview.children;
  await view.teardown();assert.equal(view.timers.size,0);
  await view.reply(pending,toolResult({work_result:{...baseState,workspace:{...baseState.workspace,clean:true}}}));
  assert.equal(view.nodes.overview.children,old);
  assert.equal(view.nodes.refresh.disabled,true);
  assert.ok(view.sent.some(request=>request.method==="notifications/cancelled" && request.params.requestId===pending.id));
});

for(const failure of ["error","timeout"]) test(`initial handshake ${failure} blocks tools and reports failure`,async()=>{
  const view=app("mcp_workbench_app.html");view.toolInput({project});
  await view.initialize(failure);
  assert.equal(view.calls("get_work_result_state").length,0);
  assert.match(view.nodes.status.textContent,/Unavailable/);
  assert.equal(view.nodes.projectSelect.disabled,true);
});

test("malformed, oversized and unsuccessful tool results surface protocol errors",async()=>{
  for(const result of [
    toolResult({items:[{...fileItem,uri:"file:///etc/passwd"}]}),
    {content:[{type:"text",text:" ".repeat(262145)+JSON.stringify({success:true,output:page([fileItem])})}]},
    {structuredContent:{success:false,error:"Permission required"}},
  ]) {
    const view=await launch();await selectProject(view);view.nodes.tabFiles.onclick();await flush();
    await view.reply(search(view,"file")[0],result);
    assert.equal(view.nodes.rows.children.length,0);
    assert.match(view.nodes.status.textContent,/Unavailable/);
  }
});

test("Project chooser supports narrowed search and paging without selecting a result",async()=>{
  const view=app("mcp_workbench_app.html");view.toolInput({});
  view.toolResult({client_id:runnerId,runners:runnerPage,projects:page([projectItem],100)});await initialize(view);
  assert.equal(view.nodes.projectsMore.hidden,false);
  view.nodes.projectsMore.onclick();await flush();
  assert.equal(search(view,"project")[0].params.arguments.offset,100);
  await view.reply(search(view,"project")[0],toolResult(page([{...projectItem,uri:"webcodex-resource://project/b3RoZXI/cm9vdA",name:"Second Project",_meta:{kind:"project",project:`agent:${runnerId}:second`,client_id:runnerId}}])));
  assert.equal(view.nodes.projectSelect.children.length,3);
  assert.equal(view.nodes.projectSelect.value,"");
  view.nodes.projectQuery.value="汉".repeat(250);
  view.nodes.projectSearchForm.onsubmit({preventDefault(){}});await flush();
  assert.equal(search(view,"project").at(-1).params.arguments.query.length,200);
  assert.equal(search(view,"project").at(-1).params.arguments.offset,0);
});

test("Goals are owner-scoped and their search omits Project selectors",async()=>{
  const view=await launch();await selectProject(view);view.nodes.tabGoals.onclick();await flush();
  assert.deepEqual({...search(view,"goal")[0].params.arguments},{kind:"goal",offset:0,limit:50});
});

test("Project refs retain canonical identity after chooser search changes",async()=>{
  const view=app("mcp_workbench_app.html");view.toolInput({});
  const ref="p_ref_test";
  view.toolResult({projects:page([{...projectItem,_meta:{...projectItem._meta,project_ref:ref}}])});
  await initialize(view);await selectProject(view,ref);
  view.nodes.projectQuery.value="other";view.nodes.projectSearchForm.onsubmit({preventDefault(){}});await flush();
  await view.reply(search(view,"project")[0],toolResult(page([])));
  await view.reply(view.calls("get_work_result_state")[0],toolResult({work_result:baseState}));
  assert.equal(view.nodes.overview.children.length,4);
  assert.equal(view.nodes.projectReference.hidden,false);
});

test("malformed overview without an authoritative Project cannot be rendered",async()=>{
  const view=await launch();await selectProject(view,secondProject);
  const {project:_,...state}=baseState;
  await view.reply(view.calls("get_work_result_state")[0],toolResult({work_result:state}));
  assert.equal(view.nodes.overview.children.length,0);
  assert.match(view.nodes.status.textContent,/selection mismatch/);
});

test("canonical file text is previewed directly and a late fallback read never attaches after scope switch",async()=>{
  const view=await launch({text:{}});await files(view);
  rowActions(view)[0].onclick();await flush();
  await view.reply(readRequests(view)[0],toolResult({uri:fileUri,kind:"file",data:{text:"Readable source",read_revision:"rev"}}));
  assert.equal(view.nodes.previewContent.textContent,"Readable source");
  view.nodes.tabFiles.onclick();await flush();await view.reply(search(view,"file").at(-1),toolResult(page([{...fileItem,uri:"webcodex-resource://file/YWdlbnQ6ZGVtbw/cm9vdA/b3RoZXI"}])));
  rowActions(view)[1].onclick();await flush();const pending=readRequests(view).at(-1);
  await selectProject(view,secondProject);
  await view.reply(pending,toolResult({uri:pending.params.arguments.uri,kind:"file",data:"Previous Project"}));
  assert.equal(refs(view).length,0);
});

test("standard text and embedded references restore from canonical context on remount",async()=>{
  for(const block of [
    {type:"text",text:`WebCodex reference\nURI: ${fileUri}\nFile\ncontent`},
    {type:"resource",resource:{uri:fileUri,mimeType:"text/plain",text:"content"}},
  ]) {
    const view=app("mcp_workbench_app.html");view.toolInput({});view.toolResult({projects:page([projectItem])});
    await initialize(view,{text:{},resource:{}},{updateId:"remount",content:[block]},true);
    assert.equal(view.nodes.references.children.length,1);
  }
});

test("late canonical launcher scope resolves short selectors without overriding a user choice",async()=>{
  const view=app("mcp_workbench_app.html");view.toolInput({project:"p_short",session_id:"s_short"});
  await initialize(view);
  assert.equal(view.nodes.projectSelect.value,"p_short");
  view.toolResult({project,session_id});await flush();
  assert.equal(view.nodes.projectSelect.value,project);
  assert.equal(view.nodes.sessionSelect.value,session_id);
  await selectProject(view,secondProject);
  view.toolResult({project,session_id});await flush();
  assert.equal(view.nodes.projectSelect.value,secondProject);
});

test("empty result before a named input does not swallow the explicit launcher scope",async()=>{
  const view=app("mcp_workbench_app.html");await initialize(view);
  view.toolResult({});view.toolInput({project});await flush();
  assert.equal(view.nodes.projectSelect.value,project);
  assert.equal(view.calls("get_work_result_state").length,1);
});

test("Your Goals remains browsable and referenceable when Project discovery is unavailable",async()=>{
  const view=app("mcp_workbench_app.html");view.toolInput({});
  view.toolResult({projects:{...page([]),total:0,list_truncated:true,incomplete:"project_discovery_unavailable"}});
  await initialize(view,{resourceLink:{}});
  assert.match(view.nodes.selectionState.textContent,/Project discovery unavailable.*Goals/);
  assert.match(view.nodes.status.textContent,/Project discovery unavailable/);
  assert.equal(view.nodes.projectSelect.value,"");
  assert.equal(view.calls("get_work_result_state").length,0);
  view.nodes.tabGoals.onclick();await flush();
  assert.equal(view.nodes.browser.hidden,false);
  assert.match(view.nodes.selectionState.textContent,/Your Goals/);
  assert.deepEqual({...search(view,"goal")[0].params.arguments},{kind:"goal",offset:0,limit:50});
  const goalItem={type:"resource_link",uri:"webcodex-resource://goal/d2NfZ29hbF9hYmM",name:"Independent Goal",_meta:{kind:"goal",goal_id:"wc_goal_abc"}};
  await view.reply(search(view,"goal")[0],toolResult(page([goalItem])));
  assert.equal(view.nodes.rows.children.length,1);
  rowActions(view)[1].onclick();await flush();
  await view.reply(refs(view)[0],{});
  assert.equal(view.nodes.references.children.length,1);
  view.nodes.query.value="Independent";view.nodes.searchForm.onsubmit({preventDefault(){}});await flush();
  assert.equal(search(view,"goal").at(-1).params.arguments.query,"Independent");
  assert.equal("project" in search(view,"goal").at(-1).params.arguments,false);
  view.nodes.tabFiles.onclick();await flush();
  assert.equal(view.nodes.browser.hidden,true);
  assert.equal(search(view,"file").length,0);
});

test("no-Project Goals can refresh and resume after visibility changes",async()=>{
  const view=await launch();view.nodes.tabGoals.onclick();await flush();
  await view.reply(search(view,"goal")[0],toolResult(page([])));
  view.nodes.refresh.onclick();await flush();
  assert.equal(search(view,"goal").length,2);
  await view.visibility(true);await view.visibility(false);
  assert.equal(search(view,"goal").length,3);
  assert.equal(view.calls("list_sessions").length,0);
  assert.equal(view.calls("get_work_result_state").length,0);
});

test("overview uses canonical active_requests/events activity and leaves unknown workspace state unavailable",async()=>{
  const view=await launch();await selectProject(view);
  const activeActivity={available:true,active:true,truncated:false,active_requests:[{label:"Reading current files",started_at_ms:100}],events:[{label:"Finished earlier check",ended_at_ms:90}]};
  await view.reply(view.calls("get_work_result_state")[0],toolResult({work_result:{...baseState,workspace:{git_available:true},activity:activeActivity}}));
  assert.equal(view.nodes.overview.children[0].children[1].textContent,"Unavailable");
  assert.equal(view.nodes.overview.children[2].children[1].textContent,"Reading current files");
  view.nodes.refresh.onclick();await flush();
  await view.reply(view.calls("get_work_result_state").at(-1),toolResult({work_result:{...baseState,activity:{available:true,active:false,truncated:true,active_requests:[],events:[{label:"Newest review",ended_at_ms:300},{label:"Older edit",ended_at_ms:200}]}}}));
  assert.equal(view.nodes.overview.children[2].children[1].textContent,"Newest review");
});

test("text reference added after an earlier preview re-reads current resource content",async()=>{
  const view=await launch({text:{}});await files(view);
  rowActions(view)[0].onclick();await flush();
  await view.reply(readRequests(view)[0],toolResult({uri:fileUri,kind:"file",version:"old",data:{text:"Previous content"}}));
  rowActions(view)[1].onclick();await flush();
  assert.equal(readRequests(view).length,2);
  await view.reply(readRequests(view)[1],toolResult({uri:fileUri,kind:"file",version:"new",data:{text:"Latest reference content"}}));
  assert.match(refs(view)[0].params.content[0].text,/Latest reference content/);
  assert.doesNotMatch(refs(view)[0].params.content[0].text,/Previous content/);
  await view.reply(refs(view)[0],{});
});


test("Runner switch clears Project, Session and old results before scoped discovery",async()=>{
  const view=await launch();await selectProject(view);
  view.nodes.sessionSelect.value=session_id;view.nodes.sessionSelect.onchange();await flush();
  const oldOverview=view.calls("get_work_result_state").at(-1);
  view.nodes.runnerSelect.value="other";view.nodes.runnerSelect.onchange();await flush();
  assert.equal(view.nodes.projectSelect.value,"");assert.equal(view.nodes.sessionSelect.value,"");
  assert.equal(view.nodes.overview.children.length,0);
  const request=search(view,"project").at(-1);
  assert.equal(request.params.arguments.client_id,"other");
  await view.reply(oldOverview,toolResult({work_result:baseState}));
  assert.equal(view.nodes.overview.children.length,0);
  await view.reply(request,toolResult(page([projectItem])));
  assert.match(view.nodes.status.textContent,/mismatch/);
  assert.equal(view.nodes.projectSelect.children.length,1);
  view.toolResult({client_id:runnerId,project,session_id,runners:runnerPage,projects:page([projectItem])});
  assert.equal(view.nodes.runnerSelect.value,"other");assert.equal(view.nodes.projectSelect.value,"");
});

test("background overview reuses bounded observations but explicit refresh remains fresh",async()=>{
  const view=await launch();await selectProject(view);
  await view.reply(view.calls("list_sessions")[0],toolResult({sessions:[]}));
  await view.reply(view.calls("get_work_result_state")[0],toolResult({work_result:baseState}));
  await view.fireTimers(10000);
  const poll=view.calls("get_work_result_state").at(-1);
  assert.equal(poll.params.arguments.automatic,true);
  await view.reply(poll,toolResult({work_result:baseState}));
  view.nodes.refresh.onclick();await flush();
  assert.equal(view.calls("get_work_result_state").at(-1).params.arguments.automatic,undefined);
});

test("selected context is reauthorized and attached only after explicit click and Host acknowledgement",async()=>{
  const view=await launch({text:{}});await selectProject(view);
  view.nodes.sessionSelect.value=session_id;view.nodes.sessionSelect.onchange();await flush();
  assert.equal(refs(view).length,0);
  view.nodes.useContext.onclick();await flush();
  const check=view.calls("open_webcodex_workbench").at(-1);
  assert.deepEqual({...check.params.arguments},{client_id:runnerId,project,session_id});
  await view.reply(check,toolResult({client_id:runnerId,project,session_id,projects:page([projectItem])}));
  assert.equal(refs(view).length,1);
  assert.equal(view.nodes.references.children.length,0);
  assert.match(refs(view)[0].params.content[0].text,new RegExp(session_id));
  await view.reply(refs(view)[0],{});
  assert.equal(view.nodes.references.children.length,1);
  assert.equal(view.calls("work_on_project").length,0);
  assert.equal(view.sent.filter(request=>request.method==="ui/message").length,0);
});

test("resourceLink-only Host receives copyable exact Session context, not a false attachment claim",async()=>{
  const view=await launch({resourceLink:{}});await selectProject(view);
  view.nodes.useContext.onclick();await flush();
  await view.reply(view.calls("open_webcodex_workbench").at(-1),toolResult({client_id:runnerId,project,session_id:null,projects:page([projectItem])}));
  assert.equal(refs(view).length,0);
  assert.equal(view.nodes.copyLabel.hidden,false);
  assert.match(view.nodes.referenceText.value,/none selected; do not infer one/);
  assert.match(view.nodes.referenceStatus.textContent,/Host context was not changed/);
});

test("revoked selection and late context validation cannot inject stale model context",async()=>{
  const view=await launch({text:{}});await selectProject(view);
  view.nodes.useContext.onclick();await flush();
  const check=view.calls("open_webcodex_workbench").at(-1);
  view.nodes.runnerSelect.value="other";view.nodes.runnerSelect.onchange();await flush();
  await view.reply(check,toolResult({client_id:runnerId,project,session_id:null,projects:page([projectItem])}));
  assert.equal(refs(view).length,0);
});


test("selected context text survives Host remount and canonical removal",async()=>{
  const declaration={type:"text",text:"WebCodex selected context\nRunner: "+runnerId+"\nProject: "+project+"\nWorkflow Session: "+session_id+"\nURI: "+projectUri+"\nSelection is context only."};
  const view=app("mcp_workbench_app.html");view.toolInput({});view.toolResult({runners:runnerPage,projects:page([])});
  await initialize(view,{text:{}},{updateId:"restored-selection",content:[declaration]},true);
  assert.equal(view.nodes.references.children.length,1);
  assert.equal(refs(view).length,0);
  view.notification("ui/notifications/host-context-changed",{"openai/modelContext":{updateId:"removed-selection",content:[]}});await flush();
  assert.equal(view.nodes.references.children.length,0);
});

test("selected context rejects a replaced Project root instead of retargeting the old choice",async()=>{
  const view=await launch({text:{}});await selectProject(view);
  view.nodes.useContext.onclick();await flush();
  const replaced={...projectItem,uri:projectUri+"changed"};
  await view.reply(view.calls("open_webcodex_workbench").at(-1),toolResult({client_id:runnerId,project,session_id:null,projects:page([replaced])}));
  assert.equal(refs(view).length,0);
  assert.match(view.nodes.referenceStatus.textContent,/root changed/);
});

const serverText = value => ({ type: "text", text: JSON.stringify(value) });
const runnerValues = view => [...view.nodes.runnerSelect.children].map(node => node.value);

test("a canonical host projection does not mask the server runner inventory", async () => {
  const view = app("mcp_workbench_app.html");
  view.toolInput({});
  await initialize(view);
  await view.reply(view.calls("list_runners")[0], {
    structuredContent: { success: true, output: { projected: true } },
    content: [serverText({ success: true, output: runnerPage })],
  });
  assert.deepEqual(runnerValues(view), ["", runnerId, "other"]);
});

for (const key of ["result", "toolResult", "tool_result"]) {
  test(`an outer host projection does not mask a nested ${key} server runner envelope`, async () => {
    const view = app("mcp_workbench_app.html");
    view.toolInput({});
    await initialize(view);
    await view.reply(view.calls("list_runners")[0], {
      structuredContent: { success: true, output: { projected: true } },
      [key]: { content: [serverText({ success: true, output: runnerPage })] },
    });
    assert.deepEqual(runnerValues(view), ["", runnerId, "other"]);
  });
}

test("a nested server failure takes precedence over a successful host runner projection", async () => {
  const view = app("mcp_workbench_app.html");
  view.toolInput({});
  await initialize(view);
  await view.reply(view.calls("list_runners")[0], {
    structuredContent: { success: true, output: runnerPage },
    toolResult: { content: [serverText({ success: false, error: "Server rejected this inventory" })] },
  });
  assert.match(view.nodes.status.textContent, /Server rejected this inventory/);
  assert.deepEqual(runnerValues(view), [""]);
});

test("launcher text keeps projects when a canonical host projection drops them", async () => {
  const view = app("mcp_workbench_app.html");
  view.notification("ui/notifications/tool-result", {
    structuredContent: { success: true, output: { projected: true } },
    content: [serverText({ success: true, output: { runners: runnerPage, projects: page([projectItem]) } })],
  });
  await initialize(view);
  assert.deepEqual(runnerValues(view), ["", runnerId, "other"]);
  assert.equal(view.nodes.projectSelect.children[1].textContent, "Demo Project · /repo/demo");
});

// Upgrade coverage uses actual controller events and canonical tool envelopes.
const {workbenchState,goalDetail,agentId,goalId}=await import("./workbench_upgrade_fixture.mjs");
const formEvent={preventDefault(){}};
async function upgraded(options={}) {const view=await launch({resourceLink:{}});await view.reply(view.calls("read_workbench_state").at(-1),toolResult({...structuredClone(workbenchState),...options}));return view;}
const texts=node=>[node.textContent,...node.children.flatMap(child=>texts(child))].join("\n");

test("Home renders observed work and settings without choosing execution context",async()=>{
 const view=await upgraded();assert.equal(view.nodes.instanceName.textContent,"Personal workspace");assert.equal(view.document.title,"Personal workspace");assert.equal(view.nodes.homePanel.hidden,false);assert.match(texts(view.nodes.homeWork),/Review files/);assert.match(texts(view.nodes.homeWork),/Elapsed 1 min/);assert.equal(view.nodes.projectSelect.value,"");assert.equal(view.calls("submit_workbench_work").length,0);
});
test("Home can page the global Task overview with the same bounded request controls",async()=>{
 const view=await launch();const request=view.calls("read_workbench_state").at(-1);const first={...structuredClone(workbenchState),tasks:{tasks:[workbenchState.tasks.tasks[0]],total_count:2,next_offset:20,truncated:true}};await view.reply(request,toolResult(first));assert.equal(view.nodes.homeWorkMore.hidden,false);assert.equal(view.nodes.workMore.hidden,false);assert.match(view.nodes.homeWorkCoverage.textContent,/Partial results/);view.nodes.homeWorkMore.onclick();await flush();const next=view.calls("read_workbench_state").at(-1);assert.equal(next.params.arguments.offset,20);assert.equal(next.params.arguments.limit,20);await view.reply(next,toolResult({...workbenchState,tasks:{tasks:[{...workbenchState.tasks.tasks[0],task_id:"wc_task_secondtask000000"}],total_count:2,next_offset:null,truncated:false}}));assert.equal(view.nodes.homeWork.children.length,2);assert.equal(view.nodes.homeWorkMore.hidden,true);
});
test("section errors remain visible alongside successful canonical pages",async()=>{
 const view=await upgraded({errors:{tasks:{message:"communication.read denied"},providers:{message:"Runner offline"}}});assert.match(view.nodes.homeEmpty.textContent,/denied/);assert.equal(view.nodes.sharedAttention.hidden,false);assert.match(texts(view.nodes.sharedAttention),/Runner offline/);assert.doesNotMatch(texts(view.nodes.sharedAttention),/\[object Object\]/);assert.match(texts(view.nodes.homeWork),/Review files/);
});
test("permissions fail closed for all upgrade mutation controls",async()=>{
 const view=await upgraded({capabilities:{}});assert.equal(view.nodes.renameInstance.disabled,true);assert.equal(view.nodes.newGoal.disabled,true);assert.equal(view.nodes.newHomeWork.disabled,true);view.nodes.newWork.onclick();view.nodes.workForm.onsubmit(formEvent);view.nodes.newGoal.onclick();view.nodes.goalForm.onsubmit(formEvent);assert.equal(view.calls("submit_workbench_work").length,0);assert.equal(view.calls("create_goal").length,0);
});
test("late Workbench response cannot overwrite the selected project or settings",async()=>{
 const view=await launch();const old=view.calls("read_workbench_state").at(-1);await selectProject(view);const current=view.calls("read_workbench_state").at(-1);await view.reply(current,toolResult(workbenchState));await view.reply(old,toolResult({...workbenchState,settings:{instance_name:"stale"}}));assert.equal(view.nodes.instanceName.textContent,"Personal workspace");
});
test("Runner submission carries only explicitly selected project and actual route",async()=>{
 const view=await upgraded();view.nodes.newHomeWork.onclick();view.nodes.workProject.value=project;view.nodes.workProject.onchange();await flush();await view.reply(view.calls("read_workbench_state").at(-1),toolResult(workbenchState));view.nodes.workExecutor.value="1";view.nodes.workExecutor.onchange();view.nodes.workTitle.value="Check paths";view.document.getElementById("workInstruction").value="Inspect source safely";view.nodes.workGoal.value=goalId;view.nodes.workForm.onsubmit(formEvent);await flush();const request=view.calls("submit_workbench_work").at(-1);assert.deepEqual({...request.params.arguments.execution},{kind:"runner",agent_id:agentId,provider_id:"default"});assert.equal(request.params.arguments.project,project);assert.equal(request.params.arguments.goal_id,goalId);assert.ok(request.params.arguments.idempotency_key);await view.reject(request);assert.equal(view.nodes.workInstruction.value,"Inspect source safely");view.nodes.workForm.onsubmit(formEvent);await flush();assert.equal(view.calls("submit_workbench_work").at(-1).params.arguments.idempotency_key,request.params.arguments.idempotency_key);
});
test("uncertain Work submission freezes key and payload across edits, route changes, and close/reopen",async()=>{
 const view=await upgraded();view.nodes.newHomeWork.onclick();view.nodes.workProject.value=project;view.nodes.workProject.onchange();await flush();await view.reply(view.calls("read_workbench_state").at(-1),toolResult(workbenchState));view.nodes.workExecutor.value="1";view.nodes.workExecutor.onchange();view.nodes.workTitle.value="Frozen task";view.nodes.workInstruction.value="Do exactly this";view.nodes.workForm.onsubmit(formEvent);await flush();const first=view.calls("submit_workbench_work").at(-1),args=JSON.parse(JSON.stringify(first.params.arguments));await view.reply(first,{structuredContent:{success:false,error:"transport uncertain",output:{workbench_submission:{task_id:"wc_task_original",phase:"runner_start_uncertain"}}}});assert.match(view.nodes.workTaskIdentity.textContent,/wc_task_original/);assert.equal(view.nodes.submitWork.disabled,false);view.nodes.workInstruction.value="Different task";view.nodes.workInstruction.oninput();assert.equal(view.nodes.submitWork.disabled,true);view.nodes.workForm.onsubmit(formEvent);await flush();assert.equal(view.calls("submit_workbench_work").length,1);view.nodes.closeWork.onclick();view.nodes.newHomeWork.onclick();assert.equal(view.nodes.workInstruction.value,"Different task");view.nodes.workForm.onsubmit(formEvent);assert.equal(view.calls("submit_workbench_work").length,1);view.nodes.workInstruction.value="Do exactly this";view.nodes.workInstruction.oninput();assert.equal(view.nodes.submitWork.disabled,false);view.nodes.workForm.onsubmit(formEvent);await flush();const retry=view.calls("submit_workbench_work").at(-1);assert.deepEqual(JSON.parse(JSON.stringify(retry.params.arguments)),args);await view.reply(retry,toolResult({task_id:"wc_task_original"}));assert.match(view.nodes.workStatus.textContent,/wc_task_original/);assert.equal(view.nodes.workTitle.value,"");assert.equal(view.nodes.workInstruction.value,"");
});
test("general Agent readiness does not fabricate a callable chat route",async()=>{
 const view=await upgraded({agents:{agents:[{agent_id:agentId,production_auto_resume_available:true}]},providers:[],execution_options:[]});view.nodes.newHomeWork.onclick();assert.equal(view.nodes.workExecutor.children.length,1);assert.equal(view.nodes.submitWork.disabled,true);
});
test("executor inventory refresh never retargets a selected route by its old index",async()=>{
 const view=await upgraded();view.nodes.newHomeWork.onclick();view.nodes.workExecutor.value="1";view.nodes.workExecutor.onchange();view.nodes.homeNav.onclick();await flush();await view.reply(view.calls("read_workbench_state").at(-1),toolResult({...workbenchState,execution_options:[{kind:"runner",agent_id:agentId,provider_id:"different"}]}));assert.equal(view.nodes.workExecutor.value,"");assert.equal(view.nodes.submitWork.disabled,true);
});
test("Goal edits use observed revision; conflict reload preserves title objective and intent",async()=>{
 const view=await upgraded();view.nodes.goalsNav.onclick();await flush();const goalItem={type:"resource_link",uri:"webcodex-resource://goal/Z29hbA",name:"Goal",_meta:{kind:"goal",goal_id:goalId}};await view.reply(search(view,"goal").at(-1),toolResult(page([goalItem])));view.nodes.rows.children[0].children[1].children[2].onclick();await flush();await view.reply(view.calls("get_goal").at(-1),toolResult({goal:goalDetail}));assert.equal(view.nodes.goalConditions.value,"Directory navigation works");view.nodes.goalTitle.value="My draft";view.nodes.goalObjective.value="Revised objective";view.nodes.goalConditions.value="Revised condition";view.nodes.goalForm.onsubmit(formEvent);await flush();const update=view.calls("update_goal").at(-1);assert.equal(update.params.arguments.expected_revision,3);await view.reply(update,{structuredContent:{success:false,error:"goal_revision_conflict",output:{error_kind:"revision_conflict"}}});assert.equal(view.nodes.goalTitle.value,"My draft");assert.equal(view.nodes.reloadGoal.hidden,false);view.nodes.reloadGoal.onclick();await flush();await view.reply(view.calls("get_goal").at(-1),toolResult({goal:{...goalDetail,summary:{...goalDetail.summary,revision:4}}}));assert.equal(view.nodes.goalTitle.value,"My draft");assert.equal(view.nodes.goalObjective.value,"Revised objective");assert.equal(view.nodes.goalConditions.value,"Revised condition");view.nodes.goalForm.onsubmit(formEvent);await flush();assert.equal(view.calls("update_goal").at(-1).params.arguments.expected_revision,4);
});
test("failed Goal edit cannot silently replay the old payload after visible fields change",async()=>{
 const view=await upgraded();view.nodes.goalsNav.onclick();await flush();const goalItem={type:"resource_link",uri:"webcodex-resource://goal/Z29hbA",name:"Goal",_meta:{kind:"goal",goal_id:goalId}};await view.reply(search(view,"goal").at(-1),toolResult(page([goalItem])));view.nodes.rows.children[0].children[1].children[2].onclick();await flush();await view.reply(view.calls("get_goal").at(-1),toolResult({goal:goalDetail}));view.nodes.goalTitle.value="First edited title";view.nodes.goalForm.onsubmit(formEvent);await flush();const update=view.calls("update_goal").at(-1);await view.reject(update);assert.equal(view.nodes.saveGoal.disabled,false,"unchanged exact retry stays available");view.nodes.goalTitle.value="Different visible draft";view.nodes.goalTitle.oninput();assert.equal(view.nodes.saveGoal.disabled,true);view.nodes.goalForm.onsubmit(formEvent);await flush();assert.equal(view.calls("update_goal").length,1);assert.equal(view.nodes.goalTitle.value,"Different visible draft");
});
test("new Goal submission is bounded and global with no Project authority",async()=>{
 const view=await upgraded();view.nodes.newGoal.onclick();view.nodes.goalTitle.value="Goal";view.nodes.goalObjective.value="Complete feature";view.nodes.goalConditions.value=Array.from({length:9},(_,i)=>"Condition "+i).join("\n");view.nodes.goalForm.onsubmit(formEvent);assert.equal(view.calls("create_goal").length,0);view.nodes.goalConditions.value="Acceptance met";view.nodes.goalForm.onsubmit(formEvent);await flush();const request=view.calls("create_goal").at(-1);assert.equal(request.params.arguments.project,undefined);assert.deepEqual(Array.from(request.params.arguments.completion_conditions),["Acceptance met"]);await view.reply(request,toolResult({goal:goalDetail}));assert.match(view.nodes.goalStatus.textContent,/revision 3/);
});
test("uncertain Goal creation retries the same immutable key and requires inspection before discard",async()=>{
 const view=await upgraded();view.nodes.newGoal.onclick();view.nodes.goalTitle.value="One Goal";view.nodes.goalObjective.value="Preserve one creation";view.nodes.goalConditions.value="Done";view.nodes.goalForm.onsubmit(formEvent);await flush();const first=view.calls("create_goal").at(-1),args=JSON.parse(JSON.stringify(first.params.arguments));await view.reply(first,{structuredContent:{success:false,error:"transport uncertain",output:{goal_id:goalId}}});view.nodes.goalObjective.value="Different objective";view.nodes.goalObjective.oninput();assert.equal(view.nodes.saveGoal.disabled,true);view.nodes.goalForm.onsubmit(formEvent);assert.equal(view.calls("create_goal").length,1);view.nodes.closeGoal.onclick();view.nodes.newGoal.onclick();assert.equal(view.nodes.goalObjective.value,"Different objective");view.nodes.goalObjective.value="Preserve one creation";view.nodes.goalObjective.oninput();view.nodes.goalForm.onsubmit(formEvent);await flush();const retry=view.calls("create_goal").at(-1);assert.deepEqual(JSON.parse(JSON.stringify(retry.params.arguments)),args);await view.reply(retry,toolResult({goal:goalDetail}));assert.match(view.nodes.goalStatus.textContent,/Saved Goal/);
});
test("a new Goal cannot inherit a previous saved Goal identity after an uncertain response",async()=>{
 const view=await upgraded();view.nodes.newGoal.onclick();view.nodes.goalTitle.value="First Goal";view.nodes.goalObjective.value="First objective";view.nodes.goalForm.onsubmit(formEvent);await flush();await view.reply(view.calls("create_goal").at(-1),toolResult({goal:goalDetail}));assert.match(view.nodes.goalAttemptIdentity.textContent,new RegExp(goalId));
 view.nodes.newGoal.onclick();assert.equal(view.nodes.goalAttemptIdentity.hidden,true);view.nodes.goalTitle.value="Second Goal";view.nodes.goalObjective.value="Second objective";view.nodes.goalForm.onsubmit(formEvent);await flush();await view.reject(view.calls("create_goal").at(-1));assert.equal(view.nodes.goalAttemptIdentity.hidden,true);assert.equal(view.nodes.inspectGoalAttempt.hidden,true);
});
test("directory traversal is rejected locally and paging remains bounded",async()=>{
 const view=await upgraded();await selectProject(view);view.nodes.tabFiles.onclick();view.nodes.filePath.value="../private";view.nodes.pathForm.onsubmit(formEvent);assert.equal(view.calls("list_project_files").length,0);view.nodes.filePath.value="src";view.nodes.pathForm.onsubmit(formEvent);await flush();let request=view.calls("list_project_files").at(-1);assert.deepEqual({...request.params.arguments},{project,path:"src",offset:0,limit:50});await view.reply(request,toolResult({project,path:"src",entries:[{path:"src/a.rs",kind:"file"},{path:"src/nested",kind:"dir"}],next_offset:50,truncated:true}));assert.equal(view.nodes.rows.children.length,2);view.nodes.more.onclick();await flush();request=view.calls("list_project_files").at(-1);assert.equal(request.params.arguments.offset,50);view.nodes.parentPath.onclick();await flush();assert.equal(view.calls("list_project_files").at(-1).params.arguments.path,"");
});
test("multi-select references stays explicit and uses normal Host acknowledgement",async()=>{
 const view=await upgraded();await files(view,[fileItem,{...fileItem,uri:fileUri+"YQ",name:"b.rs"}]);for(const row of view.nodes.rows.children){const check=row.children[0].children[2];check.checked=true;check.onchange();}assert.equal(refs(view).length,0);view.nodes.fileSelection.onclick();await flush();assert.equal(refs(view).length,1);await view.reply(refs(view)[0],{});await flush();assert.equal(refs(view).length,2);await view.reply(refs(view)[1],{});assert.equal(view.nodes.references.children.length,2);
});
test("Markdown preview treats raw HTML and active links as inert text",async()=>{
 const view=await upgraded();await files(view,[{...fileItem,name:"README.md"}]);rowActions(view)[0].onclick();await flush();await view.reply(readRequests(view).at(-1),toolResult({kind:"file",uri:fileUri,data:{text:"# Title\n<script>danger()</script>\n[click](javascript:danger())"}}));assert.equal(view.nodes.markdownPreview.hidden,false);assert.match(texts(view.nodes.markdownPreview),/<script>danger/);assert.match(texts(view.nodes.markdownPreview),/javascript:danger/);assert.equal(view.nodes.markdownPreview.children.some(node=>["SCRIPT","A","IFRAME"].includes(node.tagName)),false);
});
test("instance default reset uses principal settings adapter",async()=>{
 const view=await upgraded();view.nodes.resetInstance.onclick();await flush();const request=view.calls("update_workbench_settings").at(-1);assert.equal(request.params.arguments.display_name,null);await view.reply(request,toolResult({instance_name:"Server instance",display_name:"Server instance",display_name_override:null}));assert.equal(view.nodes.instanceName.textContent,"Server instance");assert.equal(view.document.title,"Server instance");
});
test("on-demand task detail validates exact Task identity and ignores late selection responses",async()=>{
 const view=await upgraded();const details=view.nodes.homeWork.children[0].children.at(-1);assert.equal(view.calls("read_agent_task").length,0);details.open=true;details.ontoggle();await flush();const request=view.calls("read_agent_task").at(-1);const body=details.children[1];await selectProject(view);await view.reply(request,toolResult({task:{summary:workbenchState.tasks.tasks[0],instruction:"Stale instruction"}}));assert.doesNotMatch(texts(body),/Stale instruction/);
});
test("Task detail failure permits a fresh bounded read without executing work",async()=>{
 const view=await upgraded();const details=view.nodes.homeWork.children[0].children.at(-1);details.open=true;details.ontoggle();await flush();await view.reply(view.calls("read_agent_task").at(-1),toolResult({task:{summary:{task_id:"wrong"},instruction:"Other task"}}));assert.match(texts(details.children[1]),/identity mismatch/);details.ontoggle();await flush();assert.equal(view.calls("read_agent_task").length,2);assert.equal(view.calls("submit_workbench_work").length,0);
});
test("Task detail shows canonical instruction and attempt progress as readable rows",async()=>{
 const view=await upgraded(),details=view.nodes.homeWork.children[0].children.at(-1);details.open=true;details.ontoggle();await flush();await view.reply(view.calls("read_agent_task").at(-1),toolResult({task:{summary:{...workbenchState.tasks.tasks[0],execution_status:"outcome_unknown",latest_attempt:{attempt_number:2,state:"failed",assignee_agent_id:agentId,terminal_reason:"Runner state needs reconciliation"}},instruction:"Inspect the workspace safely."}}));assert.match(texts(details.children[1]),/Instruction/);assert.match(texts(details.children[1]),/Inspect the workspace safely/);assert.match(texts(details.children[1]),/Attempt 2 · failed/);assert.match(texts(details.children[1]),/Execution · outcome_unknown/);assert.match(texts(details.children[1]),/Runner state needs reconciliation/);assert.doesNotMatch(texts(details.children[1]),/\{"summary"/);
});
test("Goal preview renders canonical progress, criteria, and linked Task summaries safely",async()=>{
 const view=await upgraded();view.nodes.goalsNav.onclick();await flush();const item={type:"resource_link",uri:"webcodex-resource://goal/Z29hbA",name:"Improve workspace",_meta:{kind:"goal",goal_id:goalId}};await view.reply(search(view,"goal").at(-1),toolResult(page([item])));rowActions(view)[0].onclick();await flush();let request=view.calls("read_webcodex_resource").at(-1);await view.reply(request,toolResult({uri:item.uri,kind:"goal",data:{goal:{summary:goalDetail.summary,objective:goalDetail.objective},summary_only:true}}));await flush();request=view.calls("get_goal").at(-1);const goal={...goalDetail,plan:{completion_conditions:["Navigation is clear"],progress_summary:"One step is underway.",steps:[{id:"navigation",title:"Improve navigation",status:"in_progress",updated_at_unix_ms:1}]},correlations:[{kind:"agent_task",reference_id:workbenchState.tasks.tasks[0].task_id,created_at_unix_ms:1},{kind:"workflow_session",reference_id:session_id,created_at_unix_ms:1}]};await view.reply(request,toolResult({goal}));assert.equal(view.nodes.markdownPreview.hidden,false);assert.equal(view.nodes.previewContent.hidden,true);assert.match(texts(view.nodes.markdownPreview),/Improve workspace/);assert.match(texts(view.nodes.markdownPreview),/One step is underway/);assert.match(texts(view.nodes.markdownPreview),/Improve navigation · in_progress/);assert.match(texts(view.nodes.markdownPreview),/Navigation is clear/);assert.match(texts(view.nodes.markdownPreview),/Review files · active · wc_agent_task_abcdefghijklmnop/);assert.match(texts(view.nodes.markdownPreview),new RegExp(session_id));
});
test("shared attention only reports observed permission waits and session messages",async()=>{
 const view=await upgraded({tasks:{tasks:[{...workbenchState.tasks.tasks[0],execution_status:"waiting_permission"}]},todos:[{message:"Review the requested permission"}]});assert.equal(view.nodes.sharedAttention.hidden,false);assert.match(texts(view.nodes.sharedAttention),/Review the requested permission/);assert.match(texts(view.nodes.sharedAttention),/Review files/);
});
test("Home keeps global Tasks and separately scopes attention to the remembered Project and Session",async()=>{
 const view=await upgraded();await selectProject(view);view.nodes.sessionSelect.value=session_id;view.nodes.sessionSelect.onchange();await flush();view.nodes.homeNav.onclick();await flush();
 let request=view.calls("read_workbench_state").at(-1);assert.deepEqual({...request.params.arguments},{offset:0,limit:20});
 const global={...structuredClone(workbenchState),project:undefined,tasks:{...workbenchState.tasks,tasks:[{...workbenchState.tasks.tasks[0],title:"Global task"}]},todos:[]};await view.reply(request,toolResult(global));await flush();
 request=view.calls("read_workbench_state").at(-1);assert.deepEqual({...request.params.arguments},{project,session_id,offset:0,limit:20});
 const scoped={...structuredClone(workbenchState),project,tasks:{...workbenchState.tasks,tasks:[{...workbenchState.tasks.tasks[0],title:"Project-only task"}]},todos:[{session_id,message_id:"wc_msg_abcdefghijklmnop",message:"Session-specific attention"}]};await view.reply(request,toolResult(scoped));
 assert.match(texts(view.nodes.homeWork),/Global task/);assert.doesNotMatch(texts(view.nodes.homeWork),/Project-only task/);assert.equal(view.nodes.workCoverage.textContent,"All Projects · 1 work items shown");assert.match(texts(view.nodes.sharedAttention),/global Tasks \+ selected Project\/Session/);assert.match(texts(view.nodes.sharedAttention),/Session-specific attention/);
});
test("failed and uncertain canonical Tasks appear in attention without invented actions",async()=>{
 const view=await upgraded({tasks:{...workbenchState.tasks,tasks:[{task_id:"wc_task_failed_abcdefghijkl",title:"Failed task",state:"failed",execution_status:"terminal"},{task_id:"wc_task_unknown_abcdefghi",title:"Uncertain task",state:"active",execution_status:"outcome_unknown"}]}});assert.match(texts(view.nodes.sharedAttention),/Failed task · failed/);assert.match(texts(view.nodes.sharedAttention),/Uncertain task · outcome_unknown/);assert.equal(view.nodes.sharedAttention.children.filter(node=>node.tagName==="BUTTON").length,0);
});
test("projected work controls use exact identities and retain cancel-requested state",async()=>{
 const original=workbenchState.tasks.tasks[0],actionSet={cancel:{project,run_id:"wc_run_abcdefghijklmnop"},observe:{project,run_id:"wc_run_abcdefghijklmnop"},reconcile:{task_id:original.task_id,attempt_id:"wc_attempt_abcdefghijklmnop"}};
 const view=await upgraded({tasks:{...workbenchState.tasks,tasks:[{...original,project,workbench_actions:actionSet}]}});const card=view.nodes.homeWork.children[0],controls=card.children.find(n=>n.className==="toolbar");assert.deepEqual(controls.children.map(b=>b.textContent),["Cancel","Reconcile"]);const detail=card.children.find(n=>n.tagName==="DETAILS");assert.equal(detail.children[2].textContent,"Observe");
 controls.children[0].onclick();await flush();let request=view.calls("cancel_coding_agent").at(-1);assert.deepEqual({...request.params.arguments},{run_id:actionSet.cancel.run_id});await view.reply(request,toolResult({cancel_requested:true}));assert.equal(view.nodes.homeWork.children[0].children.find(n=>n.className==="toolbar").children[0].textContent,"Cancel");assert.match(texts(view.nodes.homeWork.children[0]),/awaiting terminal observation/);assert.equal(view.nodes.homeWork.children[0].children[0].children[1].textContent,"active");
 const current=view.nodes.homeWork.children[0],currentDetail=current.children.find(n=>n.tagName==="DETAILS");currentDetail.open=true;currentDetail.ontoggle();await flush();currentDetail.children[2].onclick();await flush();request=view.calls("observe_coding_agent").at(-1);assert.deepEqual({...request.params.arguments},{run_id:actionSet.observe.run_id,wait_secs:0});await view.reply(request,toolResult({run_id:actionSet.observe.run_id,events:[{text:"Observed safely"}]}));assert.match(texts(current),/Observed safely/);
 current.children.find(n=>n.className==="toolbar").children[1].onclick();await flush();request=view.calls("reconcile_agent_task_coding_run").at(-1);assert.deepEqual({...request.params.arguments},{task_id:original.task_id,attempt_id:actionSet.reconcile.attempt_id});await view.reply(request,toolResult({task_id:original.task_id}));await flush();assert.equal(view.calls("submit_workbench_work").length,0);
});
test("accepted cancellation receipt survives refresh and Project changes until terminal observation",async()=>{
 const original=workbenchState.tasks.tasks[0],actions={cancel:{project,run_id:"wc_run_abcdefghijklmnop"}},view=await upgraded({tasks:{...workbenchState.tasks,tasks:[{...original,project,workbench_actions:actions}]}});const getCard=()=>view.nodes.homeWork.children[0],toolbar=()=>getCard().children.find(node=>node.className==="toolbar");toolbar().children[0].onclick();await flush();const cancel=view.calls("cancel_coding_agent").at(-1);await view.reply(cancel,toolResult({cancel_requested:true}));await flush();let stateRead=view.calls("read_workbench_state").at(-1);await view.reply(stateRead,toolResult({...workbenchState,tasks:{...workbenchState.tasks,tasks:[{...original,project,cancel_requested:false,workbench_actions:actions}]}}));assert.match(texts(getCard()),/Cancellation requested · awaiting terminal observation/);await selectProject(view);view.nodes.homeNav.onclick();await flush();stateRead=view.calls("read_workbench_state").at(-1);await view.reply(stateRead,toolResult({...workbenchState,tasks:{...workbenchState.tasks,tasks:[{...original,project,cancel_requested:false,workbench_actions:actions}]}}));assert.match(texts(getCard()),/Cancellation requested · awaiting terminal observation/);view.nodes.refresh.onclick();await flush();stateRead=view.calls("read_workbench_state").at(-1);await view.reply(stateRead,toolResult({...workbenchState,tasks:{...workbenchState.tasks,tasks:[{...original,project,cancel_requested:false,execution_observation:{state:"cancelled"},workbench_actions:actions}]}}));assert.doesNotMatch(texts(getCard()),/Cancellation requested · awaiting terminal observation/);
});
test("unprojected work operations are unavailable and stale-card controls do nothing",async()=>{
 const original=workbenchState.tasks.tasks[0],view=await upgraded();const card=view.nodes.homeWork.children[0];assert.equal(card.children.some(n=>n.className==="toolbar"),false);const projected=await upgraded({tasks:{...workbenchState.tasks,tasks:[{...original,workbench_actions:{cancel:{project,run_id:"wc_run_abcdefghijklmnop"}}}]}});const stale=projected.nodes.homeWork.children[0].children.find(n=>n.className==="toolbar").children[0];await selectProject(projected);stale.onclick();await flush();assert.equal(projected.calls("cancel_coding_agent").length,0);assert.equal(view.calls("cancel_coding_agent").length,0);
});
test("todo completion uses the projected message IDs, opaque fence, and immutable retry payload",async()=>{
 const todo={session_id:"wc_sess_abcdefghijklmnop",message_id:"wc_msg_abcdefghijklmnop",message:"Review this request"},view=await upgraded({todos:[todo]});const review=view.nodes.sharedAttention.children[1].children[1];review.onclick();await flush();let assignment=view.calls("get_session_assignment").at(-1);assert.deepEqual({...assignment.params.arguments},{session_id:todo.session_id,message_id:todo.message_id});const fence="wsa2_abcdefghijklmnopqrstuv";await view.reply(assignment,toolResult({session_id:todo.session_id,message_id:todo.message_id,assignment_fence:fence,todo:{message_id:todo.message_id}}));const panel=view.nodes.todoDetail;panel.children[2].value="Completed with the requested answer";panel.children[5].onclick();await flush();let completion=view.calls("complete_session_message").at(-1);assert.deepEqual({...completion.params.arguments},{session_id:todo.session_id,message_id:todo.message_id,answer:panel.children[2].value,completion_key:completion.params.arguments.completion_key,expected_assignment_fence:fence});const immutable=structuredClone(completion.params.arguments);await view.reject(completion);assert.equal(panel.children[2].disabled,true);panel.children[5].onclick();await flush();completion=view.calls("complete_session_message").at(-1);assert.deepEqual({...completion.params.arguments},immutable);await view.reply(completion,toolResult({completed:true}));await flush();assert.equal(view.nodes.sharedAttention.children.some(node=>node.className==="editor"),false);assert.equal(view.calls("submit_workbench_work").length,0);
});
test("attention refresh preserves a reply draft and an uncertain completion retry",async()=>{
 const todo={session_id:"wc_sess_abcdefghijklmnop",message_id:"wc_msg_abcdefghijklmnop",message:"Review the request"},other={...todo,message_id:"wc_msg_othermessage000000",message:"Other request"};const state={...workbenchState,todos:[todo,other]},view=await upgraded(state);view.nodes.sharedAttention.children[1].children[1].onclick();await flush();await view.reply(view.calls("get_session_assignment").at(-1),toolResult({session_id:todo.session_id,message_id:todo.message_id,assignment_fence:"wsa2_abcdefghijklmnopqrstuv"}));const panel=view.nodes.todoDetail,answer=panel.children[2];answer.value="Keep this answer";
 view.nodes.refresh.onclick();await flush();await view.reply(view.calls("read_workbench_state").at(-1),toolResult(state));assert.equal(view.nodes.todoDetail,panel);assert.equal(panel.hidden,false);assert.equal(answer.value,"Keep this answer");
 panel.children[5].onclick();await flush();const first=view.calls("complete_session_message").at(-1),payload=structuredClone(first.params.arguments);await view.reject(first);view.nodes.refresh.onclick();await flush();await view.reply(view.calls("read_workbench_state").at(-1),toolResult(state));assert.equal(panel.children[2].value,"Keep this answer");view.nodes.sharedAttention.children[2].children[1].onclick();assert.equal(view.nodes.todoDetail,panel);assert.match(panel.children[3].textContent,/original completion/);panel.children[5].onclick();await flush();assert.deepEqual(JSON.parse(JSON.stringify(view.calls("complete_session_message").at(-1).params.arguments)),payload);
});


test("chat submission preserves its exact endpoint generation and malformed routes stay unavailable",async()=>{
 const view=await upgraded();view.nodes.newHomeWork.onclick();view.nodes.workProject.value=project;view.nodes.workProject.onchange();await flush();await view.reply(view.calls("read_workbench_state").at(-1),toolResult(workbenchState));view.nodes.workExecutor.value="0";view.nodes.workExecutor.onchange();view.nodes.workTitle.value="Use current chat";view.nodes.workInstruction.value="Continue the selected task";view.nodes.workForm.onsubmit(formEvent);await flush();const request=view.calls("submit_workbench_work").at(-1);assert.deepEqual(JSON.parse(JSON.stringify(request.params.arguments.execution)),workbenchState.execution_options[0]);
 const invalid=await upgraded({execution_options:[{kind:"chat",agent_id:agentId},{kind:"runner",agent_id:agentId,provider_id:"default",endpoint_id:"wc_endpoint_abcdefghijklmnop"}]});invalid.nodes.newHomeWork.onclick();assert.equal(invalid.nodes.workExecutor.children.length,1);assert.equal(invalid.nodes.submitWork.disabled,true);
});
