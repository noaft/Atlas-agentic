import {test,expect,vi} from "vitest";
import {createProcessesPanel} from "./processes-panel";
import type {ProcessInfo,ProcessOutput} from "./process-types";
const settle=async()=>{for(let i=0;i<15;i++)await Promise.resolve();};
const info:ProcessInfo={sessionId:"owned",kind:"process",lifetime:"background",status:"running",running:true,workspacePath:"/project",startedAt:"2026-11-09T00:00:00Z",timeoutMs:3600000,exitCode:null};
const output:ProcessOutput={session_id:"owned",running:true,stdout:"ready\n",stderr:"",stdout_truncated:false,stderr_truncated:false,next_cursor:{stdout:6,stderr:0},oldest_cursor:{stdout:0,stderr:0},exit_code:null,timed_out:false,stopped:false,stop_requested:false,error:null};
function fixture(){const api={listProcesses:vi.fn(async(_owner:string)=>[info]),readProcessOutput:vi.fn(async(_owner:string,_id:string,_cursor:unknown)=>output),stopProcess:vi.fn(async(_owner:string,_id:string)=>({...output,running:false,stopped:true}))};const panel=createProcessesPanel(api);panel.updateContext("chat-a",true);return{api,panel};}
test("viewer fetches only open logs with independent cursor and stable nodes",async()=>{const{api,panel}=fixture();await settle();panel.element.open=true;panel.element.dispatchEvent(new Event("toggle"));await settle();expect(api.readProcessOutput).not.toHaveBeenCalled();const details=panel.element.querySelector<HTMLDetailsElement>("details")!;details.open=true;details.dispatchEvent(new Event("toggle"));await settle();expect(api.readProcessOutput).toHaveBeenCalledWith("chat-a","owned",{stdout:0,stderr:0});expect(panel.element.textContent).toContain("ready");const pre=panel.element.querySelector("pre");await panel.refresh();expect(panel.element.querySelector("pre")).toBe(pre);expect(api.readProcessOutput).toHaveBeenLastCalledWith("chat-a","owned",{stdout:6,stderr:0});panel.destroy();});
test("switching chats rejects late lists and stop is explicit",async()=>{const{api,panel}=fixture();await settle();let resolve!:(items:ProcessInfo[])=>void;api.listProcesses.mockImplementationOnce(()=>new Promise(done=>{resolve=done;}));void panel.refresh();panel.updateContext("chat-b",true);await settle();resolve([{...info,sessionId:"stale"}]);await settle();expect(panel.element.textContent).not.toContain("stale");expect(api.stopProcess).not.toHaveBeenCalled();panel.element.querySelector<HTMLButtonElement>('[data-process-stop]')!.click();await settle();expect(api.stopProcess).toHaveBeenCalledWith("chat-b","owned");panel.destroy();});

test("an older list response cannot restore Running after a confirmed Stop",async()=>{
 const{api,panel}=fixture();await settle();let resolve!:(items:ProcessInfo[])=>void;
 api.listProcesses.mockImplementationOnce(()=>new Promise(done=>{resolve=done;}));void panel.refresh();
 api.stopProcess.mockImplementation(async()=>{api.listProcesses.mockResolvedValue([{...info,running:false,status:"stopped"}]);return{...output,running:false,stopped:true};});
 panel.element.querySelector<HTMLButtonElement>('[data-process-stop]')!.click();await settle();
 resolve([{...info}]);await settle();expect(panel.element.textContent).toContain("Stopped");expect(panel.element.querySelector<HTMLButtonElement>('[data-process-stop]')!.disabled).toBe(true);panel.destroy();
});
