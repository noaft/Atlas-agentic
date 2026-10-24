import { expect, test, vi } from "vitest";
import { createMcpSettingsPanel } from "./mcp-settings";

const entry = {name: "remote", transport: "http" as const, oauth: true, connected: false};
const settle = async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); };
const button = (node: HTMLElement, text: string) => [...node.querySelectorAll("button")].find(b => b.textContent === text)!;
function fixture() {
  const api = {
    listMcpConnections: vi.fn(async () => [entry]),
    connectMcpServer: vi.fn(async (_server: string) => {}),
    disconnectMcpServer: vi.fn(async (_server: string) => {}),
    cancelMcpConnection: vi.fn(async (_server: string) => {}),
  };
  return {api, panel: createMcpSettingsPanel(api)};
}
test("loads status without authorizing and connects only on explicit click", async () => {
  const {api, panel} = fixture(); await settle();
  expect(api.connectMcpServer).not.toHaveBeenCalled();
  api.listMcpConnections.mockResolvedValue([{...entry, connected: true}]);
  button(panel,"Connect").click(); await settle();
  expect(api.connectMcpServer).toHaveBeenCalledWith("remote");
  expect(panel.textContent).toContain("Connected");
  button(panel,"Disconnect").click(); await settle();
  expect(api.disconnectMcpServer).toHaveBeenCalledWith("remote");
});
test("cancels a pending browser connection and disables duplicate requests", async () => {
  const {api, panel} = fixture(); await settle();
  let finish!: () => void;
  api.connectMcpServer.mockImplementation(() => new Promise<void>(resolve => {finish = resolve;}));
  button(panel,"Connect").click(); await settle();
  expect(panel.textContent).toContain("Waiting for browser");
  button(panel,"Cancel connection").click(); await settle();
  expect(api.cancelMcpConnection).toHaveBeenCalledWith("remote");
  expect(button(panel,"Connect")).toBeUndefined();
  finish(); await settle();
  expect(button(panel,"Connect")).toBeDefined();
});
test("discovery failures can retry and action failures stay local", async () => {
  const api = {
    listMcpConnections: vi.fn().mockRejectedValueOnce(new Error("Invalid MCP config")).mockResolvedValue([entry]),
    connectMcpServer: vi.fn().mockRejectedValue(new Error("Connection timed out")),
    disconnectMcpServer: vi.fn(), cancelMcpConnection: vi.fn(),
  };
  const panel = createMcpSettingsPanel(api); await settle();
  expect(panel.textContent).toContain("Invalid MCP config");
  button(panel,"Refresh connections").click(); await settle();
  button(panel,"Connect").click(); await settle();
  expect(panel.textContent).toContain("Connection timed out");
  expect(button(panel,"Connect").disabled).toBe(false);
});
test("independent server operations preserve other status and surrounding inputs", async () => {
  const {api, panel} = fixture(); await settle();
  const host = document.createElement("div"); const key = document.createElement("input");
  key.type = "password"; key.value = "draft-key"; host.append(key,panel); document.body.append(host);
  key.focus(); key.setSelectionRange(2,4); host.scrollTop = 80;
  button(panel,"Refresh connections").click(); await settle();
  expect(host.querySelector("input")).toBe(key); expect(key.value).toBe("draft-key");
  expect(document.activeElement).toBe(key); expect(key.selectionStart).toBe(2); expect(host.scrollTop).toBe(80);
  expect(api.listMcpConnections).toHaveBeenCalledTimes(2); host.remove();
});

test("one pending connection leaves another server usable", async () => {
  let finish!: () => void;
  const api = {
    listMcpConnections: vi.fn(async () => [entry,{...entry,name:"second",connected:true}]),
    connectMcpServer: vi.fn(() => new Promise<void>(resolve => {finish=resolve;})),
    disconnectMcpServer: vi.fn(async () => {}), cancelMcpConnection: vi.fn(async () => {}),
  };
  const panel = createMcpSettingsPanel(api); await settle();
  button(panel,"Connect").click(); await settle();
  button(panel,"Disconnect").click(); await settle();
  expect(api.disconnectMcpServer).toHaveBeenCalledWith("second");
  expect(button(panel,"Cancel connection")).toBeDefined();
  finish(); await settle();
});

test("cancellation errors allow retry while the connection remains pending", async () => {
  const {api,panel} = fixture(); await settle();
  let finish!: () => void;
  api.connectMcpServer.mockImplementation(() => new Promise<void>(resolve => {finish=resolve;}));
  api.cancelMcpConnection.mockRejectedValueOnce(new Error("Cancellation unavailable"));
  button(panel,"Connect").click(); await settle(); button(panel,"Cancel connection").click(); await settle();
  expect(panel.textContent).toContain("Cancellation unavailable");
  expect(button(panel,"Cancel connection").disabled).toBe(false);
  finish(); await settle();
});

test("renders stdio and unauthenticated servers without authorization actions", async () => {
  const api = {
    listMcpConnections: async () => [{...entry,name:"<img src=x>",transport:"stdio" as const,oauth:false}],
    connectMcpServer: vi.fn(), disconnectMcpServer: vi.fn(), cancelMcpConnection: vi.fn(),
  };
  const panel = createMcpSettingsPanel(api); await settle();
  expect(panel.textContent).toContain("<img src=x>");
  expect(panel.textContent).toContain("No OAuth connection required");
  expect(panel.querySelector("img")).toBeNull();
  expect(button(panel,"Connect")).toBeUndefined();
  expect(api.connectMcpServer).not.toHaveBeenCalled();
});

test("credential errors keep recovery actions and other configured servers visible", async () => {
  const api = {
    listMcpConnections: vi.fn(async () => [{...entry,error:"Stored credential could not be read"},{...entry,name:"healthy",connected:true}]),
    connectMcpServer: vi.fn(async (_server: string) => {}),
    disconnectMcpServer: vi.fn(async (_server: string) => {}),
    cancelMcpConnection: vi.fn(async (_server: string) => {}),
  };
  const panel = createMcpSettingsPanel(api); await settle();
  const rows = panel.querySelectorAll<HTMLElement>(".mcp-connection");
  expect(rows).toHaveLength(2);
  expect(rows[0].textContent).toContain("Connection needs attention");
  expect(rows[0].textContent).toContain("Stored credential could not be read");
  expect(button(rows[0],"Connect")).toBeDefined();
  button(rows[0],"Disconnect").click(); await settle();
  expect(api.disconnectMcpServer).toHaveBeenCalledWith("remote");
  expect(panel.textContent).toContain("healthy");
  expect(panel.textContent).toContain("Connected");
});
