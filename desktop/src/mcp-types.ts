import type { McpConnectionStatus, PermissionMode } from "./types";
export interface McpSettingsView { path: string; fingerprint: string; connections: McpConnectionStatus[] }
export interface McpServerConfig {
  command: string; args: string[]; url?: string;
  timeout_ms: number;
  oauth?: { client_id?: string; scopes: string[]; redirect_port?: number };
}
export interface McpContext { conversationId: string | null; workspacePath: string | null; permissionMode: PermissionMode }
export interface McpCheckRequest { id: string; server: string; fingerprint: string; context: McpContext }
export type McpCheckEvent = {event: "approvalRequested"; data: {id: string; tool: string; arguments: string; reason: string}} | {event: "waiting"};
export interface McpCheckResult { fingerprint: string; workspacePath: string; tools: {name: string; description: string; inputSchema: unknown}[] }
export interface McpOnboardingApi {
  loadMcpSettings(): Promise<McpSettingsView>;
  addMcpServer(fingerprint: string, name: string, server: McpServerConfig): Promise<McpSettingsView>;
  inspectMcpServer(request: McpCheckRequest, onEvent: (event: McpCheckEvent) => void): Promise<McpCheckResult>;
  resolveMcpInspectionApproval(id: string, approvalId: string, allow: boolean): Promise<void>;
  cancelMcpInspection(id: string): Promise<void>;
}
