export type MessageRole = "user" | "assistant";
export type PermissionMode = "ask" | "workspace" | "fullAccess";

export interface Workspace {
  path: string;
  name: string;
}

export interface ConversationMetadata {
  id: string;
  title: string;
  titleAttempted: boolean;
  createdAt: string;
  updatedAt: string;
  messageCount: number;
  workspace: Workspace | null;
  permissionMode: PermissionMode | null;
}

export interface ConversationMessage {
  role: MessageRole;
  content: string;
  createdAt: string;
}

export interface Conversation {
  metadata: ConversationMetadata;
  messages: ConversationMessage[];
}

export interface ConversationAskResult {
  answer: string;
  metadata: ConversationMetadata;
  created: boolean;
}

export interface ModelInfo {
  id: string;
  name: string;
}

export interface AuthorizationRequest {
  id: string;
  tool: string;
  arguments: string;
  reason: string;
}

export type ChatEvent =
  | { event: "delta"; data: { content: string } }
  | { event: "approvalRequested"; data: { request: AuthorizationRequest } }
  | { event: "approvalWaiting"; data: { id: string } }
  | { event: "toolStarted"; data: { tool: string } }
  | { event: "toolFinished"; data: { tool: string; success: boolean } };

export interface ToolActivity {
  id: number;
  tool: string;
  status: "running" | "succeeded" | "failed";
}

export type ChatStatus = "idle" | "loading" | "streaming" | "error";
export type ConversationListStatus = "idle" | "loading" | "error";
export type SettingsStatus = "idle" | "loading" | "saving" | "error";
export type ModelListStatus = "idle" | "loading" | "saving" | "error";
export type MessageDelivery = "saved" | "pending" | "failed";

export interface DisplayMessage {
  role: MessageRole;
  content: string;
  createdAt: string | null;
  delivery: MessageDelivery;
}

export interface ChatState {
  conversations: ConversationMetadata[];
  activeConversationId: string | null;
  messages: DisplayMessage[];
  draftText: string;
  status: ChatStatus;
  errorMessage: string | null;
  listStatus: ConversationListStatus;
  listErrorMessage: string | null;
  activeWorkspace: Workspace | null;
  workspaceBusy: boolean;
  activePermissionMode: PermissionMode;
  permissionBusy: boolean;
  pendingApproval: AuthorizationRequest | null;
  toolActivity: ToolActivity[];
  screen: "chat" | "settings";
  settings: SettingsView | null;
  settingsDraft: SettingsDraft | null;
  settingsStatus: SettingsStatus;
  settingsErrorMessage: string | null;
  availableModels: ModelInfo[];
  modelListStatus: ModelListStatus;
  modelListErrorMessage: string | null;
  modelListRetryAvailable: boolean;
  conversationGroups: ConversationGroup[];
}

export interface ConversationGroup {
  key: string;
  workspace: Workspace | null;
  conversations: ConversationMetadata[];
  updatedAt: string;
}

export type ApiKeySource = "missing" | "settings" | "environment";

export interface SettingsView {
  apiKeySource: ApiKeySource;
  baseUrl: string;
  model: string;
  activeModel: string;
  historyLimit: number;
  defaultWorkspace: Workspace | null;
  defaultWorkspaceAutomatic: boolean;
  defaultWorkspaceError: string | null;
  defaultPermissionMode: PermissionMode;
  environmentOverrides: Array<"apiKey" | "baseUrl" | "model" | "historyLimit">;
}

export interface SettingsDraft {
  apiKey: string;
  clearApiKey: boolean;
  baseUrl: string;
  model: string;
  historyLimit: number;
  defaultWorkspace: Workspace | null;
  defaultPermissionMode: PermissionMode;
}

export interface SaveSettingsInput {
  apiKey: string | null;
  clearApiKey: boolean;
  baseUrl: string;
  model: string;
  historyLimit: number;
  defaultWorkspacePath: string | null;
  defaultPermissionMode: PermissionMode;
}
