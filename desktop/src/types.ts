export type MessageRole = "user" | "assistant";

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

export type ChatEvent = {
  event: "delta";
  data: { content: string };
};

export type ChatStatus = "idle" | "loading" | "streaming" | "error";
export type ConversationListStatus = "idle" | "loading" | "error";
export type SettingsStatus = "idle" | "loading" | "saving" | "error";
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
  screen: "chat" | "settings";
  settings: SettingsView | null;
  settingsDraft: SettingsDraft | null;
  settingsStatus: SettingsStatus;
  settingsErrorMessage: string | null;
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
  historyLimit: number;
  defaultWorkspace: Workspace;
  defaultWorkspaceAutomatic: boolean;
  environmentOverrides: Array<"apiKey" | "baseUrl" | "model" | "historyLimit">;
}

export interface SettingsDraft {
  apiKey: string;
  clearApiKey: boolean;
  baseUrl: string;
  model: string;
  historyLimit: number;
  defaultWorkspace: Workspace | null;
}

export interface SaveSettingsInput {
  apiKey: string | null;
  clearApiKey: boolean;
  baseUrl: string;
  model: string;
  historyLimit: number;
  defaultWorkspacePath: string | null;
}
