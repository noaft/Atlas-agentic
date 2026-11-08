import { expect, test, vi } from "vitest";
import { createMcpSettingsPanel } from "./mcp-settings";
import type { McpCheckEvent, McpCheckResult, McpContext, McpSettingsView } from "./mcp-types";
import { EmptySkillsApi } from "./skills-api.test-support";
const settle = async () => {for (let i=0;i<12;i++) await Promise.resolve();};
const button = (p: HTMLElement,t:string) => [...p.querySelectorAll("button")].find(x=>x.textContent===t)!;
const context: McpContext = {conversationId:null,workspacePath:"/one",permissionMode:"ask"};
function fixture() {
 const api = {...new EmptySkillsApi(),
  loadMcpSettings: vi.fn(async(): Promise<McpSettingsView> =>({path:"/config/mcp.toml",fingerprint:"first",connections:[{name:"local",transport:"stdio" as const,oauth:false,connected:true}]})),
  addMcpServer: vi.fn(async(_fingerprint: string,_name: string,_server: import("./mcp-types").McpServerConfig): Promise<McpSettingsView> =>({path:"/config/mcp.toml",fingerprint:"second",connections:[]})),
  inspectMcpServer: vi.fn(async (_request: unknown,_event: (e:McpCheckEvent)=>void):Promise<McpCheckResult> => ({fingerprint:"first",workspacePath:"/one",tools:[]})),
  resolveMcpInspectionApproval: vi.fn(async()=>{}), cancelMcpInspection:vi.fn(async()=>{}),
  listMcpConnections: vi.fn(async()=>[]),connectMcpServer:vi.fn(),disconnectMcpServer:vi.fn(),cancelMcpConnection:vi.fn()};
 const panel = createMcpSettingsPanel(api); panel.updateContext(context,"owner"); return {api,panel};
}
test("adding is explicit, keeps draft focus across refresh and never probes",async()=>{
 const {api,panel}=fixture();await settle();document.body.append(panel);
 button(panel,"Add connection").click();
 const name=panel.querySelector<HTMLInputElement>('[aria-label="Connection name"]')!;
 const url=panel.querySelector<HTMLInputElement>('[aria-label="Server URL"]')!;
 name.value="docs";url.value="https://mcp.example.com/mcp";url.focus();
 button(panel,"Refresh connections").click();await settle();
 expect(document.activeElement).toBe(url);expect(url.value).toContain("example.com");
 button(panel,"Save connection").click();await settle();
 expect(api.addMcpServer).toHaveBeenCalledWith("first","docs",{command:"",args:[],url:"https://mcp.example.com/mcp",timeout_ms:30000});
 expect(api.inspectMcpServer).not.toHaveBeenCalled();expect(api.connectMcpServer).not.toHaveBeenCalled();panel.remove();
});
test("explicit check owns approval and ignores results after context change",async()=>{
 const {api,panel}=fixture();await settle();
 let done!: (v:McpCheckResult)=>void;let emit!: (e:McpCheckEvent)=>void;
 api.inspectMcpServer.mockImplementation((_request,onEvent)=>{emit=onEvent;return new Promise(resolve=>{done=resolve;});});
 expect(panel.textContent).toContain("Not checked");button(panel,"Check tools").click();await settle();
 emit({event:"approvalRequested",data:{id:"approval",tool:"mcp_list_tools",arguments:'{"server":"local"}',reason:"launches a process"}});
 button(panel,"Allow check").click();await settle();
 expect(api.resolveMcpInspectionApproval).toHaveBeenCalledWith(expect.any(String),"approval",true);
 panel.updateContext({...context,workspacePath:"/two"},"owner");await settle();
 expect(api.cancelMcpInspection).toHaveBeenCalledTimes(1);
 expect(button(panel,"Check tools")).toBeUndefined();expect(panel.textContent).toContain("Cancelling…");
 done({fingerprint:"first",workspacePath:"/one",tools:[{name:"stale",description:"",inputSchema:{}}]});await settle();
 expect(panel.textContent).not.toContain("stale");expect(panel.textContent).not.toContain("Verified");
});
test("authorization status alone does not verify tools",async()=>{
 const {api,panel}=fixture();await settle();api.loadMcpSettings.mockResolvedValue({path:"/config/mcp.toml",fingerprint:"first",connections:[{name:"remote",transport:"http",oauth:true,connected:true}]});
 button(panel,"Refresh connections").click();await settle();
 expect(panel.textContent).toContain("Authorized");expect(panel.textContent).toContain("Not checked");expect(api.inspectMcpServer).not.toHaveBeenCalled();
});
test("cancel waits for the check worker and rejects a late successful result",async()=>{
 const {api,panel}=fixture();await settle();let done!: (v:McpCheckResult)=>void;
 api.inspectMcpServer.mockImplementation(()=>new Promise(resolve=>{done=resolve;}));
 button(panel,"Check tools").click();await settle();button(panel,"Cancel check").click();await settle();
 expect(panel.textContent).toContain("Cancelling…");expect(button(panel,"Check tools")).toBeUndefined();
 done({fingerprint:"first",workspacePath:"/one",tools:[]});await settle();
 expect(panel.textContent).toContain("Not checked");expect(panel.textContent).not.toContain("Verified");expect(button(panel,"Check tools").disabled).toBe(false);
});
test("tool descriptions are inert text and check failures can retry",async()=>{
 const {api,panel}=fixture();await settle();
 api.inspectMcpServer.mockRejectedValueOnce(new Error("Server unavailable"));
 button(panel,"Check tools").click();await settle();expect(panel.textContent).toContain("Server unavailable");
 api.inspectMcpServer.mockResolvedValueOnce({fingerprint:"first",workspacePath:"/one",tools:[{name:"<img src=x>",description:"<script>boom()</script>",inputSchema:{}}]});
 button(panel,"Check tools").click();await settle();expect(panel.textContent).toContain("Verified · 1 tool");expect(panel.querySelector("img,script")).toBeNull();
 const details=panel.querySelector<HTMLDetailsElement>("details[data-server]")!;details.open=true;
 button(panel,"Refresh connections").click();await settle();expect(panel.querySelector<HTMLDetailsElement>("details[data-server]")?.open).toBe(true);
});
test("stale save keeps the draft and does not silently rebind its fingerprint",async()=>{
 const {api,panel}=fixture();await settle();button(panel,"Add connection").click();
 panel.querySelector<HTMLInputElement>('[aria-label="Connection name"]')!.value="new";
 panel.querySelector<HTMLInputElement>('[aria-label="Server URL"]')!.value="https://example.com/mcp";
 api.loadMcpSettings.mockResolvedValue({path:"/config/mcp.toml",fingerprint:"external-edit",connections:[]});
 button(panel,"Refresh connections").click();await settle();
 api.addMcpServer.mockRejectedValueOnce(new Error("MCP configuration changed"));
 button(panel,"Save connection").click();await settle();expect(api.addMcpServer.mock.calls[0][0]).toBe("first");
 expect(panel.querySelector<HTMLInputElement>('[aria-label="Connection name"]')!.value).toBe("new");
 button(panel,"Reload configuration").click();await settle();button(panel,"Save connection").click();await settle();
 expect(api.addMcpServer.mock.calls[1][0]).toBe("external-edit");
});

test("a pre-save refresh cannot replace successfully saved connections",async()=>{
 const {api,panel}=fixture();await settle();button(panel,"Add connection").click();
 panel.querySelector<HTMLInputElement>('[aria-label="Connection name"]')!.value="new";
 panel.querySelector<HTMLInputElement>('[aria-label="Server URL"]')!.value="https://example.com/mcp";
 let finish!: (value:Awaited<ReturnType<typeof api.loadMcpSettings>>)=>void;
 api.loadMcpSettings.mockImplementationOnce(()=>new Promise(resolve=>{finish=resolve;}));
 button(panel,"Refresh connections").click();await settle();
 api.addMcpServer.mockResolvedValueOnce({path:"/config/mcp.toml",fingerprint:"saved",connections:[{name:"new",transport:"http",oauth:false,connected:true}]});
 button(panel,"Save connection").click();await settle();
 finish({path:"/config/mcp.toml",fingerprint:"first",connections:[]});await settle();
 expect(panel.querySelector(".mcp-connection strong")?.textContent).toBe("new");
});
