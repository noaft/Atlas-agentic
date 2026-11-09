import type {ProcessInfo,ProcessOutput,ProcessCursor} from "./process-types";
interface PanelApi{listProcesses(owner:string):Promise<ProcessInfo[]>;readProcessOutput(owner:string,id:string,cursor:ProcessCursor):Promise<ProcessOutput>;stopProcess(owner:string,id:string):Promise<ProcessOutput>}
interface Viewer{cursor:ProcessCursor;text:string;warning:string}
interface Row{element:HTMLElement;status:HTMLElement;meta:HTMLElement;stop:HTMLButtonElement;details:HTMLDetailsElement;pre:HTMLElement;error:HTMLElement;info:ProcessInfo;viewer:Viewer;operationError:string}
export function createProcessesPanel(api:PanelApi){
 const element=document.createElement("details");element.className="processes-panel";element.setAttribute("aria-label","Processes");
 const summary=document.createElement("summary");summary.textContent="Processes";
 const body=document.createElement("div");body.className="processes-body";
 const hint=document.createElement("p");hint.className="settings-hint";hint.textContent="Only this chat. Background processes keep running between messages; jobs end with their request.";
 const error=document.createElement("p");error.setAttribute("role","alert");error.className="settings-error";
 const refreshButton=document.createElement("button");refreshButton.type="button";refreshButton.className="secondary-button";refreshButton.textContent="Refresh processes";
 const list=document.createElement("div");list.className="process-list";body.append(hint,error,refreshButton,list);element.append(summary,body);
 let owner="",visible=false,generation=0,mutation=0,fetchCounter=0,inFlight:number|null=null,timer:ReturnType<typeof setTimeout>|null=null,destroyed=false;
 const rows=new Map<string,Row>(),viewers=new Map<string,Map<string,Viewer>>(),stopping=new Set<string>();
 const message=(e:unknown)=>e instanceof Error?e.message:String(e);
 const cancelTimer=()=>{if(timer!==null)clearTimeout(timer);timer=null;};
 const schedule=()=>{cancelTimer();if(visible&&!destroyed)timer=setTimeout(()=>void refresh(),2000);};
 const read=async(row:Row,token:number,target:string,epoch:number)=>{
  if(!element.open||!row.details.open)return;
  try{const result=await api.readProcessOutput(target,row.info.sessionId,{...row.viewer.cursor});if(token!==generation||epoch!==mutation||target!==owner||rows.get(row.info.sessionId)!==row)return;
   row.viewer.cursor=result.next_cursor;if(result.stdout_truncated||result.stderr_truncated)row.viewer.warning="Earlier output was dropped from the bounded buffer.";
   const chunk=result.stdout+(result.stderr?`\n[stderr]\n${result.stderr}`:"");if(chunk){const follow=row.pre.scrollHeight-row.pre.clientHeight-row.pre.scrollTop<20;let combined=row.viewer.text+chunk;
    if(combined.length>65536){combined=combined.slice(-65536);const first=combined.charCodeAt(0);if(first>=0xDC00&&first<=0xDFFF)combined=combined.slice(1);row.viewer.warning="Viewer keeps only the latest output; earlier display text was dropped.";}
    row.viewer.text=combined;row.pre.textContent=row.viewer.text;if(follow)row.pre.scrollTop=row.pre.scrollHeight;}
   row.error.textContent=row.operationError||result.error||row.viewer.warning;
  }catch(e){if(token===generation&&epoch===mutation&&target===owner)row.error.textContent=message(e);}
 };
 const stop=async(row:Row)=>{
  const token=generation,target=owner,id=row.info.sessionId;if(stopping.has(id))return;stopping.add(id);row.operationError="";row.error.textContent="";row.stop.disabled=true;row.status.textContent="Stopping…";
  try{await api.stopProcess(target,id);}catch(e){if(token===generation){row.operationError=message(e);row.error.textContent=row.operationError;}}finally{if(token===generation&&target===owner){stopping.delete(id);mutation++;inFlight=null;await refresh();}}
 };
 const draw=(items:ProcessInfo[])=>{
  summary.textContent=`Processes${items.length?` · ${items.filter(p=>p.running).length} running`:""}`;
  for(const[id,row]of rows)if(!items.some(info=>info.sessionId===id)){row.element.remove();rows.delete(id);}
  if(!items.length){list.replaceChildren();const empty=document.createElement("p");empty.className="settings-hint";empty.textContent="No processes in this chat yet.";list.append(empty);return;}
  if(!rows.size)list.replaceChildren();
  let cache=viewers.get(owner);if(!cache){if(viewers.size>=16)viewers.delete(viewers.keys().next().value!);cache=new Map();viewers.set(owner,cache);}
  for(const info of items){let row=rows.get(info.sessionId);if(!row){
   if(cache.size>=32)cache.delete(cache.keys().next().value!);
   let viewer=cache.get(info.sessionId);if(!viewer){viewer={cursor:{stdout:0,stderr:0},text:"",warning:""};cache.set(info.sessionId,viewer);}
   const item=document.createElement("section");item.className="process-row";
   const title=document.createElement("strong");title.textContent=`Process ${info.sessionId.slice(0,8)}`;
   const status=document.createElement("span");status.setAttribute("role","status");
   const meta=document.createElement("p");meta.className="settings-hint";
   const stopButton=document.createElement("button");stopButton.type="button";stopButton.className="secondary-button";stopButton.textContent="Stop";stopButton.dataset.processStop="";
   const details=document.createElement("details");const titleLog=document.createElement("summary");titleLog.textContent="View output";const pre=document.createElement("pre");pre.className="process-output";pre.tabIndex=0;pre.textContent=viewer.text;const rowError=document.createElement("p");rowError.className="settings-error";rowError.setAttribute("role","alert");details.append(titleLog,pre,rowError);
   row={element:item,status,meta,stop:stopButton,details,pre,error:rowError,info,viewer,operationError:""};rows.set(info.sessionId,row);const owned=row;
   stopButton.addEventListener("click",()=>void stop(owned));details.addEventListener("toggle",()=>{if(details.open)void refresh();});item.append(title,status,meta,stopButton,details);list.append(item);
  }
  row.info=info;row.meta.textContent=`${info.lifetime} · ${info.workspacePath}`;row.status.textContent=stopping.has(info.sessionId)?"Stopping…":({starting:"Starting…",running:"Running",stopping:"Stopping…",exited:`Exited (${info.exitCode??"—"})`,stopped:"Stopped",timedOut:"Timed out",failed:`Failed${info.exitCode===null?"":` (${info.exitCode})`}`}[info.status]);row.stop.disabled=!info.running||stopping.has(info.sessionId);
  }
 };
 async function refresh(){
  if(!visible||destroyed||inFlight!==null)return;const token=generation,target=owner,epoch=mutation,fetch=++fetchCounter;inFlight=fetch;refreshButton.disabled=true;error.textContent="";cancelTimer();
  try{const items=await api.listProcesses(target);if(token!==generation||epoch!==mutation||target!==owner)return;draw(items);if(element.open)for(const row of rows.values())await read(row,token,target,epoch);}
  catch(e){if(token===generation&&epoch===mutation&&target===owner)error.textContent=message(e);}
  finally{if(token===generation&&target===owner&&inFlight===fetch){inFlight=null;refreshButton.disabled=false;schedule();}}
 }
 element.addEventListener("toggle",()=>{if(element.open)void refresh();});refreshButton.addEventListener("click",()=>void refresh());
 return{element,refresh,updateContext(nextOwner:string,nextVisible:boolean){if(owner===nextOwner&&visible===nextVisible)return;generation++;owner=nextOwner;visible=nextVisible;inFlight=null;stopping.clear();cancelTimer();rows.clear();list.replaceChildren();error.textContent="";summary.textContent="Processes";element.hidden=!nextVisible;if(visible)void refresh();},destroy(){destroyed=true;generation++;cancelTimer();rows.clear();}};
}
