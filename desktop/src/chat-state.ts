import { taskUnavailableReason } from "./slash-commands";
import type { AtlasApi } from "./atlas-api";
import type { SkillScope, SkillSource, SkillWorkspaceContext } from "./skills-types";
import type {
  TaskDescriptor,
  TaskResult,
  ChatState,
  ChatEvent,
  ChatSessionSummary,
  ConversationGroup,
  ConversationMessage,
  ConversationMetadata,
  DisplayMessage,
  ModelInfo,
  ProviderId,
  ProviderDraft,
  ProviderSettingsPatch,
  PermissionMode,
  SettingsDraft,
  SettingsView,
  SettingsSection,
  Workspace,
} from "./types";

type Listener = (state: Readonly<ChatState>) => void;

const newestFirst = (left: ConversationMetadata, right: ConversationMetadata): number =>
  right.updatedAt.localeCompare(left.updatedAt);

export function groupConversations(
  conversations: ConversationMetadata[],
): ConversationGroup[] {
  const grouped = new Map<string, ConversationGroup>();
  for (const conversation of [...conversations].sort(newestFirst)) {
    const key = conversation.workspace?.path ?? "__unassigned__";
    const existing = grouped.get(key);
    if (existing) existing.conversations.push(conversation);
    else {
      grouped.set(key, {
        key,
        workspace: conversation.workspace,
        conversations: [conversation],
        updatedAt: conversation.updatedAt,
      });
    }
  }
  return [...grouped.values()].sort((left, right) =>
    right.updatedAt.localeCompare(left.updatedAt),
  );
}

const initialState = (): ChatState => ({
  tasks: [], taskCatalogStatus: "idle", taskCatalogError: null,
  compactionFailure: null, memory: null, compactionProgress: null, contextStatus: null, compactionMessage: null,
  activeSessionId: "local-1",
  sessions: [],
  conversations: [],
  conversationGroups: [],
  activeConversationId: null,
  activeWorkspace: null,
  workspaceBusy: false,
  activePermissionMode: "ask",
  permissionBusy: false,
  pendingApproval: null,
  toolActivity: [],
  messages: [],
  historyLoaded: true,
  contextUsage: null,
  draftText: "",
  status: "idle",
  errorMessage: null,
  listStatus: "idle",
  listErrorMessage: null,
  screen: "chat",
  settingsSection: "models",
  settings: null,
  settingsDraft: null,
  settingsStatus: "idle",
  settingsErrorMessage: null,
  availableModels: [],
  modelListStatus: "idle",
  modelListErrorMessage: null,
  modelListRetryAvailable: false,
});

const displaySavedMessage = (message: ConversationMessage): DisplayMessage => ({
  ...message,
  delivery: "saved",
});

const displaySavedMessages = (conversation: import("./types").Conversation): DisplayMessage[] => {
  const activityByMessage = new Map(
    (conversation.activities ?? []).map((turn) => [turn.assistantMessageIndex, turn]),
  );
  return conversation.messages.map((message, index) => {
    const activity = activityByMessage.get(index);
    return {
      ...displaySavedMessage(message),
      activity: activity?.records,
      activityDurationMs: activity?.durationMs,
    };
  });
};

const errorMessage = (error: unknown): string =>
  error instanceof Error ? error.message : String(error);

const moveToTop = (
  conversations: ConversationMetadata[],
  metadata: ConversationMetadata,
): ConversationMetadata[] => [
  metadata,
  ...conversations.filter((conversation) => conversation.id !== metadata.id),
];

const normalizedEndpoint = (value: string): string => {
  try { return new URL(value.trim()).toString().replace(/\/+$/, ""); }
  catch { return value.trim().replace(/\/+$/, ""); }
};

const draftFromSettings = (settings: SettingsView): SettingsDraft => ({
  ...(settings.provider ? { selectedProvider: settings.provider, profileDrafts: {}, editedProviders: [] } : {}),
  limitsSourceId: settings.source?.sourceId ?? null,
  apiKey: "",
  clearApiKey: false,
  baseUrl: settings.baseUrl,
  model: settings.model,
  historyLimit: settings.historyLimit,
  ...(settings.modelLimits ? { modelLimits: structuredClone(settings.modelLimits) } : {}),
  defaultWorkspace: settings.defaultWorkspaceAutomatic ? null : settings.defaultWorkspace,
  defaultPermissionMode: settings.defaultPermissionMode,
  capabilities: settings.providers?.find(p => p.provider === settings.provider)?.capabilities,
  allowKeyless: settings.providers?.find(p => p.provider === settings.provider)?.allowKeyless,
  requestOutputTokens: settings.providers?.find(p => p.provider === settings.provider)?.requestOutputTokens,
});

const usableDefaultWorkspace = (settings: SettingsView | null): Workspace | null =>
  settings && !settings.defaultWorkspaceError ? settings.defaultWorkspace : null;

const sameWorkspace = (left: Workspace | null, right: Workspace | null): boolean =>
  left?.path === right?.path;

const withConfiguredModel = (models: ModelInfo[], configured: string): ModelInfo[] =>
  models.some((model) => model.id === configured)
    ? models
    : configured ? [{ id: configured, name: configured }, ...models] : models;

const sessionKeys = [
  "compactionFailure", "memory", "compactionProgress", "contextStatus", "compactionMessage",
  "activeConversationId", "messages", "historyLoaded", "contextUsage", "draftText", "status", "errorMessage",
  "activeWorkspace", "workspaceBusy", "activePermissionMode", "permissionBusy",
  "pendingApproval", "toolActivity",
] as const;
type SessionState = Pick<ChatState, typeof sessionKeys[number]>;
interface ChatSession {
  state: SessionState;
  generation: number;
}
const sessionState = (state: ChatState): SessionState =>
  Object.fromEntries(sessionKeys.map((key) => [key, state[key]])) as SessionState;

export class ChatStore {
  private state: ChatState = initialState();
  private readonly listeners = new Set<Listener>();
  private selectionToken = 0;
  private nextSession = 1;
  private readonly chats = new Map<string, ChatSession>();
  private workspaceToken = 0;
  private settingsToken = 0;
  private modelListToken = 0;
  private taskCatalogToken = 0;

  constructor(private readonly api: AtlasApi) {
    this.chats.set(this.state.activeSessionId, {
      state: sessionState(this.state), generation: 0,
    });
  }

  listMcpConnections = () => this.api.listMcpConnections();
  skillContext = (): SkillWorkspaceContext => ({
    conversationId: this.state.activeConversationId,
    workspacePath: this.state.activeWorkspace?.path ?? null,
  });
  listSkills = (context = this.skillContext()) => this.api.listSkills(context);
  readSkill = (id: string, context = this.skillContext()) => this.api.readSkill(id, context);
  previewSkillImport = (url: string) => this.api.previewSkillImport(url);
  installSkill = (source: SkillSource, scope: SkillScope, replace = false, context = this.skillContext()) =>
    this.api.installSkill(source, scope, replace, context);
  setSkillEnabled = (id: string, enabled: boolean, context = this.skillContext()) =>
    this.api.setSkillEnabled(id, enabled, context);
  connectMcpServer = (server: string) => this.api.connectMcpServer(server);
  disconnectMcpServer = (server: string) => this.api.disconnectMcpServer(server);
  cancelMcpConnection = (server: string) => this.api.cancelMcpConnection(server);

  getState(): Readonly<ChatState> {
    return this.state;
  }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    listener(this.state);
    return () => this.listeners.delete(listener);
  }

  async initialize(): Promise<void> {
    this.update({
      listStatus: "loading",
      listErrorMessage: null,
      settingsStatus: "loading",
      settingsErrorMessage: null,
    });
    await Promise.all([this.loadConversationList(), this.loadInitialSettings(), this.retryTasks()]);
  }

  async retryTasks(): Promise<void> {
    const token = ++this.taskCatalogToken;
    this.update({taskCatalogStatus: "loading", taskCatalogError: null});
    try {
      const tasks = await this.api.listTasks();
      if (token === this.taskCatalogToken) this.update({tasks, taskCatalogStatus: "ready"});
    } catch (error) {
      if (token === this.taskCatalogToken) this.update({taskCatalogStatus: "error", taskCatalogError: errorMessage(error)});
    }
  }

  private async loadConversationList(): Promise<void> {
    try {
      const conversations = await this.api.listConversations();
      const knownIds = new Set(
        this.state.conversations.map((conversation) => conversation.id),
      );
      this.update({
        conversations: [
          ...this.state.conversations,
          ...conversations.filter((conversation) => !knownIds.has(conversation.id)),
        ],
        listStatus: "idle",
        listErrorMessage: null,
      });
    } catch (error) {
      this.update({ listStatus: "error", listErrorMessage: errorMessage(error) });
    }
  }

  private async loadInitialSettings(): Promise<void> {
    const token = ++this.settingsToken;
    try {
      const settings = await this.api.loadSettings();
      if (token !== this.settingsToken) return;
      this.update({ settings, settingsStatus: "idle", settingsErrorMessage: null });
      if (!this.state.activeConversationId && this.state.messages.length === 0) {
        this.update({ activePermissionMode: settings.defaultPermissionMode });
      }
      if (!this.state.activeWorkspace) {
        const active = this.state.activeConversationId
          ? this.state.conversations.find(
              (conversation) => conversation.id === this.state.activeConversationId,
            )
          : null;
        if (!active || !active.workspace) {
          this.update({ activeWorkspace: usableDefaultWorkspace(settings) });
        }
      }
      if (settings.apiKeySource === "missing" && !settings.providers?.find(p => p.provider === settings.provider)?.allowKeyless) {
        this.modelListToken += 1;
        this.update({
          availableModels: [],
          modelListStatus: "idle",
          modelListErrorMessage: null,
          modelListRetryAvailable: false,
        });
      } else {
        await this.loadModels();
      }
    } catch (error) {
      if (token !== this.settingsToken) return;
      this.update({
        settingsStatus: "error",
        settingsErrorMessage: errorMessage(error),
      });
    }
  }

  async newChat(): Promise<void> {
    this.settingsDiscoveryToken++;
    this.selectionToken += 1;
    this.workspaceToken += 1;
    const id = `local-${++this.nextSession}`;
    const fresh = {
      ...sessionState(initialState()),
      activeWorkspace: usableDefaultWorkspace(this.state.settings),
      activePermissionMode: this.state.settings?.defaultPermissionMode ?? "ask",
    };
    this.chats.set(id, { state: fresh, generation: 0 });
    this.replace({ ...this.state, ...fresh, activeSessionId: id, screen: "chat" });
  }

  async selectSession(id: string): Promise<void> {
    this.settingsDiscoveryToken++;
    const chat = this.chats.get(id);
    if (!chat) return;
    this.selectionToken += 1;
    this.workspaceToken += 1;
    this.replace({ ...this.state, ...chat.state, activeSessionId: id, screen: "chat" });
  }

  async selectConversation(id: string): Promise<void> {
    this.settingsDiscoveryToken++;
    const existing = [...this.chats].find(([, chat]) => chat.state.activeConversationId === id);
    if (existing?.[1].state.historyLoaded) return this.selectSession(existing[0]);
    const sessionId = existing?.[0] ?? `local-${++this.nextSession}`;
    const metadata = this.state.conversations.find((item) => item.id === id);
    if (!existing) {
      this.chats.set(sessionId, {
        generation: 0,
        state: {
          ...sessionState(initialState()), activeConversationId: id, status: "loading", historyLoaded: false,
          activeWorkspace: metadata?.workspace ?? usableDefaultWorkspace(this.state.settings),
          activePermissionMode: metadata?.permissionMode ?? "ask",
        },
      });
    }
    await this.selectSession(sessionId);
    const token = this.selectionToken;
    try {
      const conversation = await this.api.loadConversation(id);
      if (token !== this.selectionToken) return;
      const latestActivity = conversation.activities?.at(-1);
      this.updateSession(sessionId, {
        memory: conversation.memory ?? null, contextStatus: null, compactionMessage: null, compactionFailure: null,
        contextUsage: latestActivity?.assistantMessageIndex === conversation.messages.length - 1
          ? latestActivity.usage ?? null : null,
        activeConversationId: conversation.metadata.id,
        activeWorkspace: conversation.metadata.workspace ?? usableDefaultWorkspace(this.state.settings),
        activePermissionMode: conversation.metadata.permissionMode ?? "ask",
        messages: displaySavedMessages(conversation), historyLoaded: true, status: "idle", errorMessage: null,
      });
    } catch (error) {
      if (token !== this.selectionToken) return;
      this.updateSession(sessionId, { status: "error", errorMessage: errorMessage(error) });
    }
  }

  setDraft(text: string): void {
    this.state = { ...this.state, draftText: text };
    this.chats.get(this.state.activeSessionId)!.state = sessionState(this.state);
  }

  async chooseWorkspaceForDraft(): Promise<void> {
    if (
      this.state.status === "streaming" ||
      this.state.activeConversationId ||
      this.state.workspaceBusy
    )
      return;
    const owner = this.state.activeSessionId;
    const token = ++this.workspaceToken;
    this.update({ workspaceBusy: true, errorMessage: null });
    try {
      const selected = await this.api.chooseWorkspace();
      if (!selected || token !== this.workspaceToken || this.state.activeConversationId) return;
      const workspace = await this.api.validateWorkspace(selected);
      if (token !== this.workspaceToken || this.state.activeConversationId) return;
      this.update({ activeWorkspace: workspace, errorMessage: null });
    } catch (error) {
      if (token !== this.workspaceToken) return;
      this.update({ errorMessage: errorMessage(error) });
    } finally {
      this.updateSession(owner, { workspaceBusy: false });
    }
  }

  async changeActiveWorkspace(): Promise<void> {
    if (this.state.status === "streaming" || this.state.workspaceBusy) return;
    const owner = this.state.activeSessionId;
    const token = ++this.workspaceToken;
    this.update({ workspaceBusy: true, errorMessage: null });
    try {
      const selected = await this.api.chooseWorkspace();
      if (!selected || token !== this.workspaceToken) return;
      const workspace = await this.api.validateWorkspace(selected);
      if (token !== this.workspaceToken) return;
      const id = this.state.activeConversationId;
      if (!id) {
        this.update({ activeWorkspace: workspace, errorMessage: null });
        return;
      }
      const metadata = await this.api.assignConversationWorkspace(id, workspace.path);
      this.update({ conversations: moveToTop(this.state.conversations, metadata) });
      this.updateSession(owner, { activeWorkspace: metadata.workspace, errorMessage: null });
    } catch (error) {
      if (token !== this.workspaceToken) return;
      this.update({ errorMessage: errorMessage(error) });
    } finally {
      this.updateSession(owner, { workspaceBusy: false });
    }
  }

  async send(): Promise<void> {
    if (
      !this.state.historyLoaded ||
      this.state.status === "streaming" ||
      this.state.status === "loading" ||
      this.state.workspaceBusy ||
      this.state.permissionBusy ||
      this.state.modelListStatus === "saving"
    )
      return;
    const prompt = this.state.draftText.trim();
    if (!prompt) return;
    const workspace = this.state.activeWorkspace;
    if (!workspace) {
      this.update({ errorMessage: "Choose a workspace folder before sending." });
      return;
    }
    this.selectionToken += 1;
    const owner = this.state.activeSessionId;
    const chat = this.chats.get(owner)!;
    const generation = ++chat.generation;
    const conversationId = this.state.activeConversationId;
    const permissionMode = this.state.activePermissionMode;
    const stableMessages = this.state.messages.filter(
      (message) => message.delivery !== "failed",
    );
    const userMessage: DisplayMessage = {
      role: "user",
      content: prompt,
      createdAt: null,
      delivery: "pending",
    };
    const assistantMessage: DisplayMessage = {
      role: "assistant",
      content: "",
      createdAt: null,
      delivery: "pending",
    };
    this.replace({
      ...this.state,
      messages: [...stableMessages, userMessage, assistantMessage],
      draftText: "",
      compactionMessage: null, contextStatus: null, compactionFailure: null,
      status: "streaming",
      errorMessage: null,
      pendingApproval: null,
      toolActivity: [],
    });
    try {
      const completed = await this.api.ask(
        prompt,
        conversationId,
        conversationId ? null : workspace.path,
        permissionMode,
        (event) => this.handleChatEvent(owner, generation, event),
      );
      if (generation !== chat.generation) return;
      const messages = chat.state.messages.map((message, index, all) => {
        const completedAssistant =
          index === all.length - 1 && message.role === "assistant";
        return {
          ...message,
          content: completedAssistant ? completed.answer : message.content,
          progressMessage: completedAssistant ? undefined : message.progressMessage,
          progressPreview: completedAssistant ? undefined : message.progressPreview,
          delivery:
            message.delivery === "pending" ? ("saved" as const) : message.delivery,
          activity: completedAssistant
            ? (completed.activities ?? [])
            : message.activity,
          activityDurationMs: completedAssistant
            ? completed.activityDurationMs
            : message.activityDurationMs,
        };
      });
      const conversations = moveToTop(this.state.conversations, completed.metadata);
      this.state = { ...this.state, conversations, conversationGroups: groupConversations(conversations) };
      this.updateSession(owner, {
        memory: completed.memory ?? chat.state.memory, compactionProgress: null,
        contextUsage: completed.usage ?? null,
        activeConversationId: completed.metadata.id,
        activeWorkspace: completed.metadata.workspace ?? workspace,
        activePermissionMode: completed.metadata.permissionMode ?? permissionMode,
        messages,
        status: "idle",
        errorMessage: null,
        pendingApproval: null,
        toolActivity: [],
      });
    } catch (error) {
      if (generation !== chat.generation) return;
      this.updateSession(owner, {
        compactionFailure: chat.state.compactionProgress ? {message: errorMessage(error), task: null} : null,
        contextUsage: null, compactionProgress: null,
        messages: chat.state.messages.map((message) => ({
          ...message,
          delivery: message.delivery === "pending" ? "failed" : message.delivery,
          progressPreview: undefined,
          failureReason: message.delivery === "pending" && message.role === "assistant"
            ? errorMessage(error)
            : message.failureReason,
        })),
        draftText: chat.state.draftText || prompt,
        status: "error",
        errorMessage: errorMessage(error),
        pendingApproval: null,
      });
      if (conversationId) {
        try { const saved = await this.api.loadConversation(conversationId);
          if (generation === chat.generation) this.updateSession(owner, { memory: saved.memory ?? chat.state.memory });
        } catch { /* Preserve prior local memory if the reload also fails. */ }
      }
    }
  }

  async cancelCompaction(): Promise<void> {
    const owner = this.state.activeSessionId;
    const chat = this.chats.get(owner)!;
    const generation = chat.generation;
    const id = chat.state.activeConversationId;
    if (!id || !chat.state.compactionProgress) return;
    try { await this.api.cancelCompaction(id); }
    catch (error) {
      if (chat.generation === generation && chat.state.compactionProgress) {
        this.updateSession(owner, {errorMessage: errorMessage(error)});
      }
    }
  }
  async retryCompaction(): Promise<void> {
    const failed = this.state.compactionFailure?.task;
    if (!failed) return;
    const task = this.state.tasks.find(task => task.id === failed.id);
    if (!task) return;
    await this.runTask(task, failed.arguments, {preserveDraft: true});
  }

  async runTask(task: TaskDescriptor, args: Record<string, unknown>, options: {preserveDraft?: boolean} = {}): Promise<void> {
    if (!this.state.historyLoaded || this.state.status === "streaming" || this.state.status === "loading" ||
        this.state.workspaceBusy || this.state.permissionBusy || this.state.modelListStatus === "saving") return;
    const unavailable = taskUnavailableReason(task, this.state);
    if (unavailable) { this.update({errorMessage: unavailable}); return; }
    const owner = this.state.activeSessionId;
    const chat = this.chats.get(owner)!;
    const submittedDraft = chat.state.draftText;
    const generation = ++chat.generation;
    this.updateSession(owner, {status: "streaming", draftText: options.preserveDraft ? submittedDraft : "", errorMessage: null, compactionMessage: null, compactionFailure: null,
      compactionProgress: task.cancellable ? {completedBlocks: 0, totalBlocks: 0} : null});
    try {
      const result = await this.api.runTask(task.id, args, chat.state.activeConversationId,
        event => this.handleChatEvent(owner, generation, event));
      if (generation !== chat.generation) return;
      if (result.taskId !== task.id) throw new Error("Task returned an unexpected result.");
      this.applyTaskResult(owner, result);
    } catch (error) {
      if (generation !== chat.generation) return;
      this.updateSession(owner, {compactionFailure: {message: errorMessage(error), task: {id: task.id, arguments: args}}, status: "error", compactionProgress: null, errorMessage: errorMessage(error),
        draftText: chat.state.draftText || submittedDraft});
    }
  }

  private applyTaskResult(owner: string, result: TaskResult): void {
    switch (result.outcome.kind) {
      case "compaction": {
        const {memory, message} = result.outcome.result;
        this.updateSession(owner, {memory, status: "idle", compactionProgress: null, compactionMessage: message, compactionFailure: null, errorMessage: null});
        break;
      }
    }
  }

  async selectPermissionMode(permissionMode: PermissionMode): Promise<void> {
    if (
      permissionMode === this.state.activePermissionMode ||
      this.state.status === "streaming" ||
      this.state.status === "loading" ||
      this.state.permissionBusy
    )
      return;
    const id = this.state.activeConversationId;
    if (!id) {
      this.update({ activePermissionMode: permissionMode });
      return;
    }
    const owner = this.state.activeSessionId;
    this.update({ permissionBusy: true, errorMessage: null });
    try {
      const metadata = await this.api.setConversationPermissionMode(id, permissionMode);
      this.update({ conversations: moveToTop(this.state.conversations, metadata) });
      this.updateSession(owner, {
        activePermissionMode: metadata.permissionMode ?? permissionMode,
        permissionBusy: false,
      });
    } catch (error) {
      this.updateSession(owner, { permissionBusy: false, errorMessage: errorMessage(error) });
    }
  }

  async resolveToolApproval(allow: boolean): Promise<void> {
    const owner = this.state.activeSessionId;
    const chat = this.chats.get(owner)!;
    const approval = chat.state.pendingApproval;
    if (!approval) return;
    try {
      await this.api.resolveToolApproval(approval.id, allow);
      if (chat.state.pendingApproval?.id === approval.id) {
        this.updateSession(owner, { pendingApproval: null });
      }
    } catch (error) {
      if (chat.state.pendingApproval?.id === approval.id) {
        this.updateSession(owner, { errorMessage: errorMessage(error) });
      }
    }
  }

  openSettings(): void {
    if (
      this.state.workspaceBusy ||
      this.state.modelListStatus === "saving" ||
      this.state.settingsStatus === "saving" ||
      !this.state.settings
    )
      return;
    this.settingsToken += 1;
    this.settingsDiscoveryToken += 1;
    if (this.state.settingsDraft) {
      this.update({screen:"settings", ...(this.state.settingsModelsStatus === "loading" ? {settingsModelsStatus:"idle" as const} : {})});
      return;
    }
    this.update({
      screen: "settings",
      settingsDraft: draftFromSettings(this.state.settings),
      settingsModels: [], settingsModelsStatus: "idle", settingsModelsError: null,
      settingsStatus: "idle",
      settingsErrorMessage: null,
    });
  }

  async retrySettings(): Promise<void> {
    if (
      this.state.workspaceBusy ||
      this.state.settingsStatus === "loading" ||
      this.state.settingsStatus === "saving"
    )
      return;
    this.update({ settingsStatus: "loading", settingsErrorMessage: null });
    await this.loadInitialSettings();
  }

  cancelSettings(): void {
    if (this.state.settingsStatus === "saving") return;
    this.settingsToken += 1;
    this.settingsDiscoveryToken += 1;
    this.update({
      screen: "chat",
      settingsDraft: null,
      settingsStatus: "idle",
      settingsErrorMessage: null,
    });
  }

  backToChat(): void {
    if (this.state.settingsStatus === "saving") return;
    this.settingsToken++;
    this.settingsDiscoveryToken++;
    this.update({screen:"chat", ...(this.state.settingsModelsStatus === "loading" ? {settingsModelsStatus:"idle" as const} : {})});
  }

  selectSettingsSection(section: SettingsSection): void {
    if (this.state.screen !== "settings") return;
    this.update({settingsSection:section});
  }

  discardSettingsChanges(): void {
    if (!this.state.settings || this.state.settingsStatus === "saving") return;
    this.settingsToken++;
    this.settingsDiscoveryToken++;
    this.update({settingsDraft:draftFromSettings(this.state.settings), settingsStatus:"idle", settingsErrorMessage:null,
      settingsModels:[],settingsModelsStatus:"idle",settingsModelsError:null});
  }

  hasSettingsChanges(): boolean {
    const settings = this.state.settings;
    const draft = this.state.settingsDraft;
    if (!settings || !draft) return false;
    const baseline = draftFromSettings(settings);
    if (draft.selectedProvider !== baseline.selectedProvider ||
        draft.historyLimit !== baseline.historyLimit || draft.defaultPermissionMode !== baseline.defaultPermissionMode ||
        draft.defaultWorkspace?.path !== baseline.defaultWorkspace?.path) return true;
    const differs = (current:ProviderDraft, expected:ProviderDraft) => {
      // Ignore discovery bookkeeping and cached, merely visited provider drafts.
      const fields = (value:ProviderDraft) => ({apiKey:value.apiKey.trim(), clearApiKey:value.clearApiKey,
        baseUrl:value.baseUrl,model:value.model,modelLimits:value.modelLimits ?? {},capabilities:value.capabilities,
        allowKeyless:value.allowKeyless,requestOutputTokens:value.requestOutputTokens});
      return JSON.stringify(fields(current)) !== JSON.stringify(fields(expected));
    };
    if (!settings.providers || !draft.selectedProvider) return differs(draft,baseline);
    return settings.providers.some(profile => {
      const current = profile.provider === draft.selectedProvider ? this.profileDraft(draft) : draft.profileDrafts?.[profile.provider];
      if (!current) return false;
      return differs(current,{apiKey:"",clearApiKey:false,baseUrl:profile.baseUrl,model:profile.model,
        modelLimits:profile.modelLimits,capabilities:profile.capabilities,allowKeyless:profile.allowKeyless,requestOutputTokens:profile.requestOutputTokens});
    });
  }

  updateSettingsDraft(patch: Partial<SettingsDraft>): void {
    if (!this.state.settingsDraft || this.state.settingsStatus === "saving") return;
    const draft = this.state.settingsDraft;
    const changedEndpoint = patch.baseUrl !== undefined && normalizedEndpoint(patch.baseUrl) !== normalizedEndpoint(draft.baseUrl);
    if (changedEndpoint) patch = { ...patch, modelLimits: {}, limitsSourceId: null };
    const profileFields = ["apiKey", "clearApiKey", "baseUrl", "model", "modelLimits", "capabilities", "allowKeyless", "requestOutputTokens"];
    const edited = draft.selectedProvider && Object.keys(patch).some(key => profileFields.includes(key))
      ? [...new Set([...(draft.editedProviders ?? []), draft.selectedProvider])] : draft.editedProviders;
    const invalidatesCatalog = Object.keys(patch).some(key => ["baseUrl", "apiKey", "clearApiKey", "allowKeyless"].includes(key));
    if (invalidatesCatalog) this.settingsDiscoveryToken++;
    const resetCatalog = invalidatesCatalog && (this.state.settingsModelsStatus === "loading" || !!this.state.settingsModels?.length || !!this.state.settingsModelsError);
    this.update({ settingsDraft: { ...draft, ...patch, editedProviders: edited },
      ...(resetCatalog ? { settingsModels: [], settingsModelsStatus: "idle" as const, settingsModelsError: null } : {}) });
  }

  private settingsDiscoveryToken = 0;

  private profileDraft(draft: SettingsDraft): ProviderDraft {
    return { limitsSourceId: draft.limitsSourceId, apiKey: draft.apiKey, clearApiKey: draft.clearApiKey, baseUrl: draft.baseUrl, model: draft.model,
      modelLimits: draft.modelLimits, capabilities: draft.capabilities, allowKeyless: draft.allowKeyless, requestOutputTokens: draft.requestOutputTokens };
  }

  selectSettingsProvider(provider: ProviderId): void {
    const draft = this.state.settingsDraft;
    const profile = this.state.settings?.providers?.find(p => p.provider === provider);
    if (!draft || !profile || this.state.settingsStatus === "saving" || this.state.settings?.environmentOverrides.includes("provider")) return;
    const profiles = { ...draft.profileDrafts };
    if (draft.selectedProvider) profiles[draft.selectedProvider] = this.profileDraft(draft);
    const next = profiles[provider] ?? { apiKey: "", clearApiKey: false, baseUrl: profile.baseUrl, model: profile.model,
      limitsSourceId: profile.source?.sourceId ?? null, modelLimits: structuredClone(profile.modelLimits), capabilities: structuredClone(profile.capabilities), allowKeyless: profile.allowKeyless, requestOutputTokens: profile.requestOutputTokens };
    this.settingsDiscoveryToken++;
    this.update({ settingsDraft: { ...draft, ...next, selectedProvider: provider, profileDrafts: profiles }, settingsModels: [], settingsModelsStatus: "idle", settingsModelsError: null });
  }

  private profilePatch(provider: ProviderId, draft: ProviderDraft): ProviderSettingsPatch {
    return { ...draft, provider, apiKey: draft.apiKey.trim() || null, limitsSourceId: draft.limitsSourceId ?? null };
  }

  async refreshSettingsModels(): Promise<void> {
    const draft = this.state.settingsDraft;
    if (!draft?.selectedProvider || this.state.settingsStatus === "saving") return;
    const token = ++this.settingsDiscoveryToken;
    const provider = draft.selectedProvider;
    this.update({ settingsModelsStatus: "loading", settingsModelsError: null });
    try {
      const models = await this.api.listModels({ provider, patch: this.profilePatch(provider, this.profileDraft(draft)) });
      if (token !== this.settingsDiscoveryToken || this.state.screen !== "settings" || this.state.settingsDraft?.selectedProvider !== provider) return;
      this.update({ settingsModels: models, settingsModelsStatus: "idle" });
    } catch (error) {
      if (token !== this.settingsDiscoveryToken || this.state.screen !== "settings") return;
      this.update({ settingsModelsStatus: "error", settingsModelsError: errorMessage(error) });
    }
  }

  async chooseDefaultWorkspace(): Promise<void> {
    if (!this.state.settingsDraft || this.state.settingsStatus === "saving") return;
    const token = ++this.settingsToken;
    try {
      const selected = await this.api.chooseWorkspace();
      if (!selected || token !== this.settingsToken || !this.state.settingsDraft) return;
      const workspace = await this.api.validateWorkspace(selected);
      if (token !== this.settingsToken || !this.state.settingsDraft) return;
      this.updateSettingsDraft({ defaultWorkspace: workspace });
    } catch (error) {
      if (token !== this.settingsToken) return;
      this.update({ settingsStatus: "error", settingsErrorMessage: errorMessage(error) });
    }
  }

  async saveSettings(options: {stayInSettings?:boolean} = {}): Promise<void> {
    const draft = this.state.settingsDraft;
    if (!draft || this.state.settingsStatus === "saving") {
      return;
    }
    const token = ++this.settingsToken;
    const settingsOwner = this.state.activeSessionId;
    const previousDefault = usableDefaultWorkspace(this.state.settings);
    const activeWorkspaceBeforeSave = this.state.activeWorkspace;
    this.update({ settingsStatus: "saving", settingsErrorMessage: null });
    try {
      const drafts = { ...draft.profileDrafts };
      if (draft.selectedProvider) drafts[draft.selectedProvider] = this.profileDraft(draft);
      const settings = await this.api.saveSettings({
        ...(draft.selectedProvider ? { selectedProvider: draft.selectedProvider, profilePatches: (draft.editedProviders ?? []).map(provider => this.profilePatch(provider, drafts[provider]!)) } : {}),
        apiKey: draft.apiKey.trim() || null,
        clearApiKey: draft.clearApiKey,
        baseUrl: draft.baseUrl,
        model: draft.model,
        historyLimit: draft.historyLimit,
        ...(draft.modelLimits ? { modelLimits: draft.modelLimits } : {}),
        defaultWorkspacePath: draft.defaultWorkspace?.path ?? null,
        defaultPermissionMode: draft.defaultPermissionMode,
      });
      if (token !== this.settingsToken) return;
      const nextDefault = usableDefaultWorkspace(settings);
      const activeConversation = this.state.activeConversationId
        ? this.state.conversations.find(
            (conversation) => conversation.id === this.state.activeConversationId,
          )
        : null;
      const activeWorkspace = this.state.activeSessionId !== settingsOwner || this.state.status === "streaming"
        ? this.state.activeWorkspace
        : activeConversation
          ? activeConversation.workspace ?? nextDefault
          : !activeWorkspaceBeforeSave || sameWorkspace(activeWorkspaceBeforeSave, previousDefault)
            ? nextDefault
            : activeWorkspaceBeforeSave;
      this.update({
        screen: options.stayInSettings ? "settings" : "chat",
        settings,
        settingsDraft: options.stayInSettings ? draftFromSettings(settings) : null,
        settingsStatus: "idle",
        settingsErrorMessage: null,
        activeWorkspace,
      });
      if (settings.apiKeySource === "missing" && !settings.providers?.find(p => p.provider === settings.provider)?.allowKeyless) {
        this.modelListToken += 1;
        this.update({
          availableModels: [],
          modelListStatus: "idle",
          modelListErrorMessage: null,
          modelListRetryAvailable: false,
        });
      } else {
        await this.loadModels();
      }
    } catch (error) {
      if (token !== this.settingsToken) return;
      this.update({
        settingsStatus: "error",
        settingsErrorMessage: errorMessage(error),
      });
    }
  }

  async retryModels(): Promise<void> {
    if (!this.state.settings || (this.state.settings.apiKeySource === "missing" && !this.state.settings.providers?.find(p => p.provider === this.state.settings?.provider)?.allowKeyless)) return;
    await this.loadModels();
  }

  async selectModel(model: string): Promise<void> {
    const settings = this.state.settings;
    if (
      !settings ||
      this.state.status === "streaming" ||
      this.state.modelListStatus === "loading" ||
      this.state.modelListStatus === "saving" ||
      settings.environmentOverrides.includes("model") ||
      !this.state.availableModels.some((available) => available.id === model)
    )
      return;
    const token = ++this.modelListToken;
    this.update({
      modelListStatus: "saving",
      modelListErrorMessage: null,
      modelListRetryAvailable: false,
    });
    try {
      const updated = await this.api.setModel(model, settings.source);
      if (token !== this.modelListToken) return;
      // Retained Settings drafts follow chat model choices unless that field was edited.
      let settingsDraft = this.state.settingsDraft;
      if (settingsDraft) {
        const provider = settings.provider;
        const previousModel = settings.providers?.find(profile => profile.provider === provider)?.model ?? settings.model;
        const nextModel = updated.providers?.find(profile => profile.provider === provider)?.model ?? updated.model;
        if (settingsDraft.selectedProvider === provider && settingsDraft.model === previousModel) {
          settingsDraft = { ...settingsDraft, model: nextModel };
        }
        const cached = provider && settingsDraft.profileDrafts?.[provider];
        if (provider && cached?.model === previousModel) {
          settingsDraft = { ...settingsDraft, profileDrafts: { ...settingsDraft.profileDrafts, [provider]: { ...cached, model: nextModel } } };
        }
      }
      this.update({
        settings: updated,
        settingsDraft,
        modelListStatus: "idle",
        modelListErrorMessage: null,
        modelListRetryAvailable: false,
      });
    } catch (error) {
      if (token !== this.modelListToken) return;
      this.update({
        modelListStatus: "error",
        modelListErrorMessage: errorMessage(error),
        modelListRetryAvailable: false,
      });
    }
  }

  private async loadModels(): Promise<void> {
    const settings = this.state.settings;
    if (!settings) return;
    const token = ++this.modelListToken;
    this.update({
      modelListStatus: "loading",
      modelListErrorMessage: null,
      modelListRetryAvailable: false,
    });
    try {
      const models = await this.api.listModels(settings.provider ? { provider: settings.provider } : undefined);
      if (token !== this.modelListToken || this.state.settings?.source?.sourceId !== settings.source?.sourceId) return;
      this.update({
        availableModels: withConfiguredModel(models, settings.activeModel),
        modelListStatus: "idle",
        modelListErrorMessage: null,
        modelListRetryAvailable: false,
      });
    } catch (error) {
      if (token !== this.modelListToken || this.state.settings?.source?.sourceId !== settings.source?.sourceId) return;
      this.update({
        availableModels: withConfiguredModel([], settings.activeModel),
        modelListStatus: "error",
        modelListErrorMessage: errorMessage(error),
        modelListRetryAvailable: true,
      });
    }
  }

  async renameActive(title: string): Promise<void> {
    const id = this.state.activeConversationId;
    const normalized = title.trim();
    if (!id || !normalized || this.state.status === "streaming" || this.state.status === "loading") return;
    const owner = this.state.activeSessionId;
    this.update({ status: "loading", errorMessage: null });
    try {
      const metadata = await this.api.renameConversation(id, normalized);
      this.update({ conversations: this.state.conversations.map((item) => item.id === id ? metadata : item) });
      this.updateSession(owner, { status: "idle", errorMessage: null });
    } catch (error) {
      this.updateSession(owner, { status: "error", errorMessage: errorMessage(error) });
    }
  }

  async deleteActive(): Promise<void> {
    const id = this.state.activeConversationId;
    if (!id || this.state.status === "streaming" || this.state.status === "loading") return;
    const owner = this.state.activeSessionId;
    this.update({ status: "loading", errorMessage: null });
    try {
      await this.api.deleteConversation(id);
      if (this.state.activeSessionId === owner) await this.newChat();
      this.chats.delete(owner);
      this.update({ conversations: this.state.conversations.filter((item) => item.id !== id) });
    } catch (error) {
      this.updateSession(owner, { status: "error", errorMessage: errorMessage(error) });
    }
  }

  async toggleFullscreen(): Promise<void> {
    try {
      await this.api.toggleFullscreen();
    } catch (error) {
      this.update({ errorMessage: errorMessage(error) });
    }
  }

  private appendDelta(owner: string, delta: string): void {
    const state = this.chats.get(owner)!.state;
    const messages = [...state.messages];
    const last = messages.at(-1);
    if (!last || last.role !== "assistant" || last.delivery !== "pending") return;
    messages[messages.length - 1] = { ...last, content: last.content + delta };
    this.updateSession(owner, { messages });
  }

  private handleChatEvent(owner: string, generation: number, event: ChatEvent): void {
    const chat = this.chats.get(owner);
    if (!chat || generation !== chat.generation || chat.state.status !== "streaming") return;
    const state = chat.state;
    if (event.event === "compaction") { this.updateSession(owner, { compactionProgress: event.data.progress }); return; }
    if (event.event === "contextStatus") { this.updateSession(owner, { contextStatus: event.data.status, compactionProgress: null, compactionMessage: null }); return; }
    if (event.event === "usage") {
      this.updateSession(owner, { contextUsage: event.data.usage });
      return;
    }
    if (event.event === "delta") {
      this.appendDelta(owner, event.data.content);
      return;
    }
    if (event.event === "progressPreview") {
      const messages = [...state.messages];
      const last = messages.at(-1);
      if (last?.role === "assistant" && last.delivery === "pending") {
        messages[messages.length - 1] = { ...last, progressPreview: event.data.message ?? undefined };
        this.updateSession(owner, { messages });
      }
      return;
    }
    if (event.event === "progress") {
      const messages = [...state.messages];
      const last = messages.at(-1);
      if (last?.role === "assistant" && last.delivery === "pending") {
        messages[messages.length - 1] = { ...last, content: "", progressMessage: event.data.message, progressPreview: undefined };
        this.updateSession(owner, { messages });
      }
      return;
    }
    if (event.event === "responseReset") {
      const messages = [...state.messages];
      const last = messages.at(-1);
      if (last?.role === "assistant" && last.delivery === "pending") {
        messages[messages.length - 1] = { ...last, content: "" };
        this.updateSession(owner, { messages });
      }
      return;
    }
    if (event.event === "runtimeStatus") {
      if (event.data.status === "preparingAnswer") return;
      this.updateSession(owner, {
        toolActivity: [
          ...state.toolActivity,
          {
            id: `status-${state.toolActivity.length}`,
            sequence: null,
            kind: "status",
            tool: null,
            arguments: null,
            summary: "Thinking",
            details: null,
            status: "status",
          },
        ],
      });
      return;
    }
    if (event.event === "toolRequested") {
      this.updateSession(owner, {
        toolActivity: [
          ...state.toolActivity,
          {
            id: `tool-${event.data.sequence}`,
            sequence: event.data.sequence,
            kind: "tool",
            tool: event.data.tool,
            arguments: event.data.arguments,
            summary: `Requested ${event.data.tool}`,
            details: null,
            status: "requested",
          },
        ],
      });
      return;
    }
    if (event.event === "approvalRequested") {
      this.updateSession(owner, {
        pendingApproval: event.data.request,
        toolActivity: this.updateToolActivity(state, event.data.sequence, {
          status: "waiting",
          summary: `Waiting for approval: ${event.data.request.tool}`,
        }),
      });
      return;
    }
    if (event.event === "approvalWaiting") return;
    if (event.event === "toolDenied") {
      this.updateSession(owner, {
        pendingApproval: null,
        toolActivity: this.updateToolActivity(state, event.data.sequence, {
          status: "denied",
          summary: event.data.summary,
        }),
      });
      return;
    }
    if (event.event === "toolStarted") {
      this.updateSession(owner, {
        pendingApproval: null,
        toolActivity: this.updateToolActivity(state, event.data.sequence, {
          status: "running",
          summary: `Running ${event.data.tool}`,
        }),
      });
      return;
    }
    this.updateSession(owner, {
      toolActivity: this.updateToolActivity(state, event.data.sequence, {
        status: event.data.success ? "succeeded" : "failed",
        summary: event.data.summary,
        details: event.data.details,
      }),
    });
  }

  private updateToolActivity(
    state: SessionState,
    sequence: number,
    patch: Partial<import("./types").ToolActivity>,
  ): import("./types").ToolActivity[] {
    return state.toolActivity.map((activity) =>
      activity.sequence === sequence ? { ...activity, ...patch } : activity,
    );
  }

  private update(patch: Partial<ChatState>): void {
    this.replace({ ...this.state, ...patch });
  }

  private updateSession(id: string, patch: Partial<SessionState>): void {
    const chat = this.chats.get(id);
    if (!chat) return;
    chat.state = { ...chat.state, ...patch };
    if (id === this.state.activeSessionId) this.replace({ ...this.state, ...chat.state });
    else {
      const sessions = this.sessionSummaries();
      if (sessions !== this.state.sessions) {
        this.state = { ...this.state, sessions };
        this.emit();
      }
    }
  }

  private sessionSummaries(): ChatSessionSummary[] {
    const summaries: ChatSessionSummary[] = [];
    for (const [id, { state }] of this.chats) {
      if (!state.activeConversationId && !state.messages.length && !state.draftText.trim()) continue;
      const metadata = this.state.conversations.find((item) => item.id === state.activeConversationId);
      summaries.push({ id, conversationId: state.activeConversationId,
        title: metadata?.title ?? [...(state.messages[0]?.content || state.draftText || "New chat")].slice(0, 60).join(""),
        status: state.status, needsApproval: state.pendingApproval !== null,
      });
    }
    const previous = this.state.sessions;
    return JSON.stringify(previous) === JSON.stringify(summaries) ? previous : summaries;
  }

  private replace(next: ChatState): void {
    const chat = this.chats.get(next.activeSessionId);
    if (chat) chat.state = sessionState(next);
    this.state = {
      ...next,
      conversationGroups:
        next.conversations === this.state.conversations
          ? this.state.conversationGroups
          : groupConversations(next.conversations),
    };
    this.state = { ...this.state, sessions: this.sessionSummaries() };
    this.emit();
  }

  private emit(): void {
    for (const listener of this.listeners) listener(this.state);
  }
}
