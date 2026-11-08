import type { AtlasApi } from "./atlas-api";
import type { McpConnectionStatus } from "./types";
import type { McpContext, McpCheckEvent, McpCheckResult, McpServerConfig, McpSettingsView } from "./mcp-types";

export type McpSettingsApi = Pick<AtlasApi, "loadMcpSettings" | "addMcpServer" | "inspectMcpServer" | "resolveMcpInspectionApproval" | "cancelMcpInspection" | "connectMcpServer" | "disconnectMcpServer" | "cancelMcpConnection">;
export interface McpSettingsPanel extends HTMLElement { updateContext(context: McpContext, owner: string): void }
type Check = {id: string; status: "checking" | "cancelling" | "verified" | "failed"; approval?: Extract<McpCheckEvent,{event:"approvalRequested"}>["data"]; deciding?: boolean; invalidated?: boolean; result?: McpCheckResult; error?: string};

/** The editor stays mounted while independent connection workers update their rows. */
export function createMcpSettingsPanel(api: McpSettingsApi): McpSettingsPanel {
  const panel = document.createElement("section") as unknown as McpSettingsPanel;
  panel.className = "mcp-settings"; panel.setAttribute("aria-label", "MCP connections");
  let view: McpSettingsView | null = null;
  let viewEpoch = 0;
  let loading = false, refreshAgain = false, saving = false, baseline = "";
  let context: McpContext = {conversationId:null,workspacePath:null,permissionMode:"ask"}, owner = "";
  const busy = new Map<string, "connecting" | "disconnecting">();
  const cancelling = new Set<string>(), errors = new Map<string,string>(), checks = new Map<string,Check>();
  const text = (error: unknown) => error instanceof Error ? error.message : String(error);
  const button = (label: string, action: () => void, disabled = false) => {
    const b = document.createElement("button"); b.type="button"; b.className="secondary-button"; b.textContent=label; b.disabled=disabled; b.addEventListener("click",action); return b;
  };
  const paragraph = (value: string, className = "settings-hint") => {const p=document.createElement("p");p.className=className;p.textContent=value;return p;};
  const alert = (value: string) => {const p=paragraph(value,"settings-error");p.setAttribute("role","alert");return p;};
  const heading=document.createElement("h2");heading.textContent="MCP connections";
  const hint=paragraph("Add a service or a local tool server. Saved connections apply to new requests. Sign-in and tool checks run only when you choose them.");
  const toolbar=document.createElement("div");toolbar.className="mcp-toolbar";
  const editor=document.createElement("div");editor.className="settings-card mcp-editor";editor.hidden=true;
  const list=document.createElement("div");list.className="mcp-list";
  const notice=document.createElement("div"), editorNotice=document.createElement("div");
  const configPath=paragraph("");configPath.classList.add("mcp-config-path");
  const name=document.createElement("input"), url=document.createElement("input"), command=document.createElement("input"), args=document.createElement("textarea");
  const transport=document.createElement("select");for(const [value,label] of [["http","Web service (HTTP)"],["stdio","Local program (stdio)"]]) {const option=document.createElement("option");option.value=value;option.textContent=label;transport.append(option);}
  const field=(label: string,input: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement)=>{const wrapper=document.createElement("label");wrapper.className="mcp-field";input.setAttribute("aria-label",label);const caption=document.createElement("span");caption.textContent=label;wrapper.append(caption,input);return wrapper;};
  name.placeholder="e.g. docs";name.maxLength=64;url.placeholder="https://example.com/mcp";command.placeholder="e.g. npx";args.placeholder="One argument per line";args.rows=3;
  for(const input of [name,url,command,args]) input.spellcheck=false;
  const httpFields=document.createElement("div"),localFields=document.createElement("div");
  httpFields.append(field("Server URL",url));localFields.append(field("Program",command),field("Arguments",args),paragraph("Runs with Atlas’s permissions, environment and the current chat’s starting folder when you check or use it."));localFields.hidden=true;
  const advanced=document.createElement("details");const advancedTitle=document.createElement("summary");advancedTitle.textContent="Advanced options";
  const timeout=document.createElement("input");timeout.type="number";timeout.min="1";timeout.max="300000";timeout.value="30000";
  const oauth=document.createElement("input");oauth.type="checkbox";
  const client=document.createElement("input"), scopes=document.createElement("input"), port=document.createElement("input");port.type="number";port.min="1";port.max="65535";
  const oauthFields=document.createElement("div");oauthFields.hidden=true;oauthFields.append(field("Public client ID (optional)",client),field("Scopes (space separated)",scopes),field("Loopback port (optional)",port));
  const authGroup=document.createElement("div");authGroup.append(field("Browser sign-in (OAuth)",oauth),oauthFields);
  oauth.addEventListener("change",()=>{oauthFields.hidden=!oauth.checked;});
  transport.addEventListener("change",()=>{const http=transport.value==="http";httpFields.hidden=!http;localFields.hidden=http;authGroup.hidden=!http;});
  advanced.append(advancedTitle,field("Timeout (milliseconds)",timeout),authGroup);
  editor.addEventListener("keydown",event=>{
    if(event.key==="Enter"&&!event.isComposing&&event.keyCode!==229&&event.target instanceof HTMLInputElement){event.preventDefault();void add();}
  });
  const save=button("Save connection",()=>void add());
  const close=button("Cancel",()=>{if(!saving)editor.hidden=true;});
  const editorActions=document.createElement("div");editorActions.className="mcp-toolbar";editorActions.append(save,close);
  editor.append(field("Connection name",name),field("Connection type",transport),httpFields,localFields,advanced,editorNotice,editorActions);
  const addButton=button("Add connection",()=>{if(!view)return;baseline=view.fingerprint;editor.hidden=false;name.focus();});
  const refreshButton=button("Refresh connections",()=>void refresh());toolbar.append(addButton,refreshButton);
  panel.append(heading,hint,toolbar,editor,notice,list,configPath);

  const cancelCheck = async (server: string, check: Check) => {
    check.status="cancelling";check.approval=undefined;draw();
    try {await api.cancelMcpInspection(check.id);} catch(error) {if(checks.get(server)===check) {check.error=text(error);if(!check.invalidated)check.status="checking";draw();}}
    // Keep it busy until its original worker finishes, including a synchronous RPC.
  };
  const invalidate=()=>{
    for(const [server,check] of checks){
      if(check.status==="checking"||check.status==="cancelling"){
        check.invalidated=true;check.approval=undefined;check.result=undefined;
        if(check.status!=="cancelling")void cancelCheck(server,check);
      }else checks.delete(server);
    }
  };
  panel.updateContext=(next,nextOwner)=>{if(owner!==nextOwner||JSON.stringify(context)!==JSON.stringify(next)){invalidate();context={...next};owner=nextOwner;draw();}};
  const apply=(next:McpSettingsView)=>{if(view&&view.fingerprint!==next.fingerprint)invalidate();view=next;draw();};
  const decide=async(server:string,check:Check,allow:boolean)=>{
    if(!check.approval||check.deciding||check.status!=="checking")return;
    const approval=check.approval;check.deciding=true;draw();
    try {await api.resolveMcpInspectionApproval(check.id,approval.id,allow);if(checks.get(server)===check)check.approval=undefined;}
    catch(error){if(checks.get(server)===check)check.error=text(error);}
    finally {check.deciding=false;if(checks.get(server)===check)draw();}
  };
  const checkTools=async(server:string)=>{
    if(!view||busy.has(server)||["checking","cancelling"].includes(checks.get(server)?.status??""))return;
    const fingerprint=view.fingerprint;
    const check:Check={id:crypto.randomUUID(),status:"checking"};checks.set(server,check);draw();
    try {
      const result=await api.inspectMcpServer({id:check.id,server,fingerprint,context:{...context}},event=>{
        if(checks.get(server)!==check||check.status!=="checking")return;
        if(event.event==="approvalRequested"){check.approval=event.data;draw();}
      });
      if(checks.get(server)!==check)return;
      if(check.status==="cancelling"){checks.delete(server);return;}
      if(result.fingerprint!==fingerprint||view?.fingerprint!==fingerprint){checks.delete(server);return;}
      check.status="verified";check.result=result;check.approval=undefined;
    }catch(error){if(checks.get(server)!==check)return;if(check.status==="cancelling")checks.delete(server);else{check.status="failed";check.error=text(error);check.approval=undefined;}}
    finally {if(checks.get(server)===check||!checks.has(server))draw();}
  };
  const draw=()=>{
    const expanded=new Set([...list.querySelectorAll<HTMLDetailsElement>("details[open]")].map(d=>d.dataset.server));
    list.replaceChildren();addButton.disabled=!view||saving;refreshButton.disabled=loading;refreshButton.textContent=loading?"Loading connections…":"Refresh connections";
    save.disabled=saving;save.textContent=saving?"Saving…":"Save connection";close.disabled=saving;
    configPath.textContent=view?`Configuration: ${view.path}`:"";
    if(view&&!view.connections.length&&!loading)list.append(paragraph("No MCP servers configured. Add your first connection above."));
    for(const connection of view?.connections??[]){
      const server=connection.name,operation=busy.get(server),check=checks.get(server);
      const row=document.createElement("div");row.className="mcp-connection";
      const label=document.createElement("strong");label.textContent=server;
      const transportLabel=paragraph(connection.transport==="http"?"HTTP":"Standard input/output");
      const auth=paragraph(operation==="connecting"?"Waiting for browser authorization…":operation==="disconnecting"?"Signing out…":connection.error?"Connection needs attention":connection.oauth?(connection.connected?"Authorized":"Sign-in required"):"No sign-in needed");auth.setAttribute("role","status");
      const status=paragraph(check?.status==="verified"?`Verified · ${check.result?.tools.length??0} ${check.result?.tools.length===1?"tool":"tools"}`:check?.status==="checking"?"Checking tools…":check?.status==="cancelling"?"Cancelling…":check?.status==="failed"?"Check failed":"Not checked");status.setAttribute("role","status");
      const actions=document.createElement("div");actions.className="mcp-toolbar";
      const checking=check?.status==="checking"||check?.status==="cancelling";
      if(checking)actions.append(button(check?.status==="cancelling"?"Cancelling…":"Cancel check",()=>void cancelCheck(server,check!),check?.status==="cancelling"));
      else actions.append(button("Check tools",()=>void checkTools(server),!!operation||!!connection.error||(connection.oauth&&!connection.connected)));
      if(operation==="connecting")actions.append(button(cancelling.has(server)?"Cancelling…":"Cancel connection",()=>void cancel(server),cancelling.has(server)));
      else if(connection.oauth&&connection.error)actions.append(button("Sign in",()=>void run(connection,false),!!operation||checking),button("Sign out",()=>void run(connection,true),!!operation||checking));
      else if(connection.oauth)actions.append(button(connection.connected?"Sign out":"Sign in",()=>void run(connection),!!operation||checking));
      row.append(label,transportLabel,auth,status,actions);
      if(check?.approval){const approval=document.createElement("div");approval.className="mcp-approval";approval.append(paragraph(`Allow this check? Atlas ${check.approval.reason}. It initializes the server and reads its tool list using the current chat’s folder and permissions.`),button("Allow check",()=>void decide(server,check,true),check.deciding),button("Deny",()=>void decide(server,check,false),check.deciding));row.append(approval);}
      if(connection.error)row.append(alert(connection.error));if(errors.has(server))row.append(alert(errors.get(server)!));if(check?.error)row.append(alert(check.error));
      if(check?.result){const details=document.createElement("details");details.dataset.server=server;details.open=expanded.has(server);const title=document.createElement("summary");title.textContent="Tool details";details.append(title,paragraph(`Checked in ${check.result.workspacePath}. Tool descriptions are supplied by the server. Checks close the connection after discovery.`));for(const tool of check.result.tools){const toolName=document.createElement("strong");toolName.textContent=tool.name;details.append(toolName,paragraph(tool.description));}row.append(details);}
      list.append(row);
    }
  };
  const refresh=async()=>{
    if(loading){refreshAgain=true;return;}loading=true;notice.replaceChildren();draw();
    const epoch=viewEpoch;
    try{const next=await api.loadMcpSettings();if(epoch===viewEpoch)apply(next);}catch(error){if(epoch===viewEpoch)notice.replaceChildren(alert(text(error)));}finally{loading=false;draw();if(refreshAgain){refreshAgain=false;void refresh();}}
  };
  const add=async()=>{
    if(saving||!view)return;editorNotice.replaceChildren();
    const timeoutMs=Number(timeout.value);if(!Number.isInteger(timeoutMs)||timeoutMs<1||timeoutMs>300000){editorNotice.append(alert("Timeout must be between 1 and 300000 milliseconds."));return;}
    if(!/^[A-Za-z0-9_-]{1,64}$/.test(name.value.trim())){editorNotice.append(alert("Use 1–64 letters, digits, underscores or hyphens for the connection name."));return;}
    const config:McpServerConfig={command:"",args:[],timeout_ms:timeoutMs};
    if(transport.value==="http"){
      if(!url.value.trim()){editorNotice.append(alert("Enter a server URL."));return;}config.url=url.value.trim();
      if(oauth.checked){const portValue=port.value?Number(port.value):undefined;if(portValue!==undefined&&(!Number.isInteger(portValue)||portValue<1||portValue>65535)){editorNotice.append(alert("Loopback port must be between 1 and 65535."));return;}
        config.oauth={scopes:scopes.value.split(/\s+/).filter(Boolean),...(client.value.trim()?{client_id:client.value.trim()}:{}),...(portValue===undefined?{}:{redirect_port:portValue})};}
    }else{config.command=command.value.trim();config.args=args.value.split(/\r?\n/).filter(v=>v.length>0);if(!config.command){editorNotice.append(alert("Enter a program to run."));return;}}
    saving=true;for(const input of editor.querySelectorAll<HTMLInputElement>("input, textarea, select"))input.disabled=true;draw();
    try{const next=await api.addMcpServer(baseline,name.value.trim(),config);viewEpoch++;apply(next);editor.hidden=true;name.value="";url.value="";command.value="";args.value="";client.value="";scopes.value="";port.value="";oauth.checked=false;oauthFields.hidden=true;timeout.value="30000";}
    catch(error){editorNotice.append(alert(text(error)),button("Reload configuration",()=>{void (async()=>{await refresh();if(view)baseline=view.fingerprint;})();}));}
    finally{saving=false;for(const input of editor.querySelectorAll<HTMLInputElement>("input, textarea, select"))input.disabled=false;draw();}
  };
  const run=async(connection:McpConnectionStatus,disconnect=connection.connected)=>{
    const server=connection.name;if(busy.has(server))return;checks.delete(server);busy.set(server,disconnect?"disconnecting":"connecting");errors.delete(server);draw();
    try{if(disconnect)await api.disconnectMcpServer(server);else await api.connectMcpServer(server);}catch(error){if(!cancelling.has(server))errors.set(server,text(error));}
    finally{busy.delete(server);cancelling.delete(server);draw();await refresh();}
  };
  const cancel=async(server:string)=>{if(cancelling.has(server))return;cancelling.add(server);errors.delete(server);draw();try{await api.cancelMcpConnection(server);}catch(error){cancelling.delete(server);errors.set(server,text(error));draw();}};
  void refresh();return panel;
}
