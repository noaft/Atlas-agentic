import { Channel, invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";

import type {
  McpConnectionStatus,
  TaskDescriptor,
  TaskResult,
  ChatEvent,
  Conversation,
  ConversationAskResult,
  ConversationMetadata,
  ModelInfo,
  ModelDiscoveryInput,
  ModelSource,
  PermissionMode,
  SaveSettingsInput,
  SettingsView,
  Workspace,
} from "./types";

export interface AtlasApi {
  listMcpConnections(): Promise<McpConnectionStatus[]>;
  connectMcpServer(server: string): Promise<void>;
  disconnectMcpServer(server: string): Promise<void>;
  cancelMcpConnection(server: string): Promise<void>;
  listTasks(): Promise<TaskDescriptor[]>;
  runTask(taskId: string, args: Record<string, unknown>, conversationId: string | null, onEvent: (event: ChatEvent) => void): Promise<TaskResult>;
  cancelCompaction(id: string): Promise<void>;
  compactConversation(id: string, onEvent: (event: ChatEvent) => void): Promise<import("./types").CompactionResult>;
  listConversations(): Promise<ConversationMetadata[]>;
  loadConversation(id: string): Promise<Conversation>;
  ask(
    prompt: string,
    conversationId: string | null,
    workspacePath: string | null,
    permissionMode: PermissionMode,
    onEvent: (event: ChatEvent) => void,
  ): Promise<ConversationAskResult>;
  renameConversation(id: string, title: string): Promise<ConversationMetadata>;
  deleteConversation(id: string): Promise<void>;
  loadSettings(): Promise<SettingsView>;
  saveSettings(input: SaveSettingsInput): Promise<SettingsView>;
  listModels(request?: ModelDiscoveryInput): Promise<ModelInfo[]>;
  setModel(model: string, source?: ModelSource | null): Promise<SettingsView>;
  chooseWorkspace(): Promise<string | null>;
  validateWorkspace(path: string): Promise<Workspace>;
  assignConversationWorkspace(id: string, path: string): Promise<ConversationMetadata>;
  setConversationPermissionMode(
    id: string,
    permissionMode: PermissionMode,
  ): Promise<ConversationMetadata>;
  resolveToolApproval(id: string, allow: boolean): Promise<void>;
  cancelToolApprovals(): Promise<void>;
  toggleFullscreen(): Promise<boolean>;
}

export function createTauriAtlasApi(): AtlasApi {
  return {
    listMcpConnections: () => invoke<McpConnectionStatus[]>("list_mcp_connections"),
    connectMcpServer: (server) => invoke<void>("connect_mcp_server", { server }),
    disconnectMcpServer: (server) => invoke<void>("disconnect_mcp_server", { server }),
    cancelMcpConnection: (server) => invoke<void>("cancel_mcp_connection", { server }),
    listTasks: () => invoke<TaskDescriptor[]>("list_tasks"),
    runTask: (taskId, args, conversationId, onEvent) => {
      const channel = new Channel<ChatEvent>(onEvent);
      return invoke<TaskResult>("run_task", {request: {taskId, arguments: args, conversationId}, channel});
    },
    cancelCompaction: (id) => invoke<void>("cancel_compaction", { id }),
    compactConversation: (id, onEvent) => {
      const channel = new Channel<ChatEvent>(onEvent);
      return invoke<import("./types").CompactionResult>("compact_conversation", { id, channel });
    },
    listConversations: () => invoke<ConversationMetadata[]>("list_conversations"),
    loadConversation: (id) => invoke<Conversation>("load_conversation", { id }),
    ask: (prompt, conversationId, workspacePath, permissionMode, onEvent) => {
      const channel = new Channel<ChatEvent>((message) => {
        onEvent(message);
      });
      return invoke<ConversationAskResult>("ask", {
        prompt,
        conversationId,
        workspacePath,
        permissionMode,
        channel,
      });
    },
    renameConversation: (id, title) =>
      invoke<ConversationMetadata>("rename_conversation", { id, title }),
    deleteConversation: async (id) => {
      await invoke("delete_conversation", { id });
    },
    loadSettings: () => invoke<SettingsView>("load_settings"),
    saveSettings: (input) => invoke<SettingsView>("save_settings", { input }),
    listModels: (request) => invoke<ModelInfo[]>("list_models", request ? { request } : undefined),
    setModel: (model, source) => invoke<SettingsView>("set_model", source ? { model, source } : { model }),
    chooseWorkspace: async () => {
      const selected = await open({ directory: true, multiple: false });
      return typeof selected === "string" ? selected : null;
    },
    validateWorkspace: (path) => invoke<Workspace>("validate_workspace", { path }),
    assignConversationWorkspace: (id, path) =>
      invoke<ConversationMetadata>("assign_conversation_workspace", { id, path }),
    setConversationPermissionMode: (id, permissionMode) =>
      invoke<ConversationMetadata>("set_conversation_permission_mode", {
        id,
        permissionMode,
      }),
    resolveToolApproval: async (id, allow) => {
      await invoke("resolve_tool_approval", { id, allow });
    },
    cancelToolApprovals: async () => {
      await invoke("cancel_tool_approvals");
    },
    toggleFullscreen: async () => {
      const window = getCurrentWindow();
      const fullscreen = !(await window.isFullscreen());
      await window.setFullscreen(fullscreen);
      return fullscreen;
    },
  };
}
