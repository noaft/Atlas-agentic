export interface MemoryBlock { startIndex: number; endIndexExclusive: number; content: string }
export interface MemoryCheckpoint { version: number; conversationId: string; checkpointId: string; createdAt: string; model: string; compactedMessageCount: number; blocks: MemoryBlock[] }
export interface CompactionProgress { completedBlocks: number; totalBlocks: number }
export interface CompactionResult { memory: MemoryCheckpoint | null; message: string }
export interface ContextStatus { available: boolean; message: string; estimated: boolean; inputTokens: number | null; maxTokens: number | null }
export interface ModelLimits { contextWindow: number; maxOutputTokens: number }
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
  memory?: MemoryCheckpoint | null;
  metadata: ConversationMetadata;
  messages: ConversationMessage[];
  activities?: ActivityTurn[];
}

export interface ConversationAskResult {
  memory?: MemoryCheckpoint | null;
  answer: string;
  metadata: ConversationMetadata;
  activities?: ActivityRecord[];
  activityDurationMs?: number;
  created: boolean;
  usage?: ModelUsage | null;
}

export type ActivityKind = "status" | "toolCall" | "toolResult" | "progress";
export type ActivityOutcome = "succeeded" | "failed" | "denied";

export interface ActivityRecord {
  sequence: number;
  kind: ActivityKind;
  tool: string | null;
  arguments: string | null;
  outcome: ActivityOutcome | null;
  summary: string;
  details: string | null;
  createdAt: string;
}

export interface ActivityTurn {
  turnId: string;
  assistantMessageIndex: number;
  durationMs?: number | null;
  usage?: ModelUsage | null;
  records: ActivityRecord[];
}

export type ProviderId = "deepseek" | "openai" | "anthropic" | "gemini" | "openai-compatible";
export interface ModelSource { provider: ProviderId; sourceId: string }
export interface ProviderCapabilities { temperature?: boolean | null; topP?: boolean | null; jsonMode?: boolean | null; streamUsage?: boolean | null; imageInput?: boolean | null }
export interface ProviderSettingsView {
  provider: ProviderId; name: string; envPrefix: string; source: ModelSource | null;
  apiKeySource: ApiKeySource; baseUrl: string; model: string; activeModel: string;
  environmentOverrides: string[]; modelLimits: Record<string, ModelLimits>;
  capabilities: ProviderCapabilities; allowKeyless: boolean; requestOutputTokens: number;
}
export interface ProviderDraft {
  limitsSourceId?: string | null;
  apiKey: string; clearApiKey: boolean; baseUrl: string; model: string;
  modelLimits?: Record<string, ModelLimits>; capabilities?: ProviderCapabilities;
  allowKeyless?: boolean; requestOutputTokens?: number;
}
export interface ProviderSettingsPatch extends Omit<ProviderDraft, "apiKey"> { provider: ProviderId; apiKey: string | null; limitsSourceId?: string | null }
export interface ModelDiscoveryInput { provider: ProviderId; patch?: ProviderSettingsPatch }

export interface ModelUsage {
  source?: ModelSource | null;
  model: string;
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
}

export interface ModelInfo {
  source?: ModelSource | null;
  inputTokenLimit?: number | null;
  capabilities?: ProviderCapabilities;
  id: string;
  name: string;
  contextWindow?: number | null;
  maxOutputTokens?: number | null;
}

export interface AuthorizationRequest {
  id: string;
  tool: string;
  arguments: string;
  reason: string;
}

export type ChatEvent =
  | { event: "progress"; data: { message: string } }
  | { event: "progressPreview"; data: { message: string | null } }
  | { event: "responseReset" }
  | { event: "compaction"; data: { progress: CompactionProgress } }
  | { event: "contextStatus"; data: { status: ContextStatus } }
  | { event: "usage"; data: { usage: ModelUsage | null } }
  | { event: "delta"; data: { content: string } }
  | { event: "runtimeStatus"; data: { status: "thinking" | "preparingAnswer" } }
  | {
      event: "toolRequested";
      data: { sequence: number; tool: string; arguments: string };
    }
  | {
      event: "approvalRequested";
      data: { sequence: number; request: AuthorizationRequest };
    }
  | { event: "approvalWaiting"; data: { id: string } }
  | {
      event: "toolDenied";
      data: { sequence: number; tool: string; summary: string };
    }
  | { event: "toolStarted"; data: { sequence: number; tool: string } }
  | {
      event: "toolFinished";
      data: {
        sequence: number;
        tool: string;
        success: boolean;
        summary: string;
        details: string | null;
      };
    };

export interface ToolActivity {
  id: string;
  sequence: number | null;
  kind: "status" | "tool";
  tool: string | null;
  arguments: string | null;
  summary: string;
  details: string | null;
  status: "status" | "requested" | "waiting" | "running" | ActivityOutcome;
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
  failureReason?: string;
  progressMessage?: string;
  progressPreview?: string;
  activity?: ActivityRecord[];
  activityDurationMs?: number | null;
}

export interface ChatSessionSummary {
  id: string;
  conversationId: string | null;
  title: string;
  status: ChatStatus;
  needsApproval: boolean;
}

export interface CompactionFailure {
  message: string;
  task: {id: string; arguments: Record<string, unknown>} | null;
}

export type SettingsSection = "models" | "connections" | "skills" | "workspace" | "permissions";

export interface ChatState {
  compactionFailure: CompactionFailure | null;
  tasks: TaskDescriptor[];
  taskCatalogStatus: "idle" | "loading" | "ready" | "error";
  taskCatalogError: string | null;
  memory: MemoryCheckpoint | null;
  compactionProgress: CompactionProgress | null;
  contextStatus: ContextStatus | null;
  compactionMessage: string | null;
  activeSessionId: string;
  sessions: ChatSessionSummary[];
  conversations: ConversationMetadata[];
  activeConversationId: string | null;
  messages: DisplayMessage[];
  historyLoaded: boolean;
  contextUsage: ModelUsage | null;
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
  settingsSection: SettingsSection;
  settings: SettingsView | null;
  settingsDraft: SettingsDraft | null;
  settingsModels?: ModelInfo[];
  settingsModelsStatus?: "idle" | "loading" | "error";
  settingsModelsError?: string | null;
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
  provider?: ProviderId;
  source?: ModelSource | null;
  providers?: ProviderSettingsView[];
  modelLimits?: Record<string, ModelLimits>;
  apiKeySource: ApiKeySource;
  baseUrl: string;
  model: string;
  activeModel: string;
  historyLimit: number;
  defaultWorkspace: Workspace | null;
  defaultWorkspaceAutomatic: boolean;
  defaultWorkspaceError: string | null;
  defaultPermissionMode: PermissionMode;
  environmentOverrides: Array<"provider" | "apiKey" | "baseUrl" | "model" | "historyLimit">;
}

export interface SettingsDraft extends ProviderDraft {
  selectedProvider?: ProviderId;
  profileDrafts?: Partial<Record<ProviderId, ProviderDraft>>;
  editedProviders?: ProviderId[];
  modelLimits?: Record<string, ModelLimits>;
  apiKey: string;
  clearApiKey: boolean;
  baseUrl: string;
  model: string;
  historyLimit: number;
  defaultWorkspace: Workspace | null;
  defaultPermissionMode: PermissionMode;
}

export interface SaveSettingsInput {
  selectedProvider?: ProviderId;
  profilePatches?: ProviderSettingsPatch[];
  modelLimits?: Record<string, ModelLimits>;
  apiKey: string | null;
  clearApiKey: boolean;
  baseUrl: string;
  model: string;
  historyLimit: number;
  defaultWorkspacePath: string | null;
  defaultPermissionMode: PermissionMode;
}

export interface TaskDescriptor {
  id: string;
  command: string;
  title: string;
  description: string;
  argumentSchema: {type: string; properties?: Record<string, unknown>; additionalProperties?: boolean};
  requiresConversation: boolean;
  requiresModel: boolean;
  cancellable: boolean;
}
export interface TaskResult {
  taskId: string;
  outcome: {kind: "compaction"; result: CompactionResult};
}

export interface McpConnectionStatus {
  name: string;
  transport: "stdio" | "http";
  oauth: boolean;
  connected: boolean;
  error?: string;
}
