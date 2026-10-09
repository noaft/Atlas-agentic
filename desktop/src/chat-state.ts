import type { AtlasApi } from "./atlas-api";
import type {
  ChatState,
  ConversationGroup,
  ConversationMessage,
  ConversationMetadata,
  DisplayMessage,
  SettingsDraft,
  SettingsView,
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
  conversations: [],
  conversationGroups: [],
  activeConversationId: null,
  activeWorkspace: null,
  workspaceBusy: false,
  messages: [],
  draftText: "",
  status: "idle",
  errorMessage: null,
  listStatus: "idle",
  listErrorMessage: null,
  screen: "chat",
  settings: null,
  settingsDraft: null,
  settingsStatus: "idle",
  settingsErrorMessage: null,
});

const displaySavedMessage = (message: ConversationMessage): DisplayMessage => ({
  ...message,
  delivery: "saved",
});

const errorMessage = (error: unknown): string =>
  error instanceof Error ? error.message : String(error);

const moveToTop = (
  conversations: ConversationMetadata[],
  metadata: ConversationMetadata,
): ConversationMetadata[] => [
  metadata,
  ...conversations.filter((conversation) => conversation.id !== metadata.id),
];

const draftFromSettings = (settings: SettingsView): SettingsDraft => ({
  apiKey: "",
  clearApiKey: false,
  baseUrl: settings.baseUrl,
  model: settings.model,
  historyLimit: settings.historyLimit,
  defaultWorkspace: settings.defaultWorkspaceAutomatic ? null : settings.defaultWorkspace,
});

const usableDefaultWorkspace = (settings: SettingsView | null): Workspace | null =>
  settings && !settings.defaultWorkspaceError ? settings.defaultWorkspace : null;

const sameWorkspace = (left: Workspace | null, right: Workspace | null): boolean =>
  left?.path === right?.path;

export class ChatStore {
  private state: ChatState = initialState();
  private readonly listeners = new Set<Listener>();
  private selectionToken = 0;
  private askToken = 0;
  private mutationToken = 0;
  private workspaceToken = 0;
  private settingsToken = 0;

  constructor(private readonly api: AtlasApi) {}

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
    await Promise.all([this.loadConversationList(), this.loadInitialSettings()]);
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
    } catch (error) {
      if (token !== this.settingsToken) return;
      this.update({
        settingsStatus: "error",
        settingsErrorMessage: errorMessage(error),
      });
    }
  }

  async newChat(): Promise<void> {
    if (this.state.status === "streaming" || this.state.workspaceBusy) return;
    this.selectionToken += 1;
    this.askToken += 1;
    this.mutationToken += 1;
    this.workspaceToken += 1;
    this.replace({
      ...this.state,
      screen: "chat",
      activeConversationId: null,
      activeWorkspace: null,
      messages: [],
      draftText: "",
      status: "idle",
      errorMessage: null,
    });
    const defaultWorkspace = usableDefaultWorkspace(this.state.settings);
    this.update({ activeWorkspace: defaultWorkspace });
  }

  async selectConversation(id: string): Promise<void> {
    if (this.state.status === "streaming" || this.state.workspaceBusy) return;
    const token = ++this.selectionToken;
    this.workspaceToken += 1;
    this.mutationToken += 1;
    this.update({ screen: "chat", status: "loading", errorMessage: null });
    try {
      const conversation = await this.api.loadConversation(id);
      if (token !== this.selectionToken) return;
      this.replace({
        ...this.state,
        activeConversationId: conversation.metadata.id,
        activeWorkspace:
          conversation.metadata.workspace ?? usableDefaultWorkspace(this.state.settings),
        messages: conversation.messages.map(displaySavedMessage),
        status: "idle",
        errorMessage: null,
      });
    } catch (error) {
      if (token !== this.selectionToken) return;
      this.update({ status: "error", errorMessage: errorMessage(error) });
    }
  }

  setDraft(text: string): void {
    this.state = { ...this.state, draftText: text };
  }

  async chooseWorkspaceForDraft(): Promise<void> {
    if (
      this.state.status === "streaming" ||
      this.state.activeConversationId ||
      this.state.workspaceBusy
    )
      return;
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
      if (token === this.workspaceToken) this.update({ workspaceBusy: false });
    }
  }

  async changeActiveWorkspace(): Promise<void> {
    if (this.state.status === "streaming" || this.state.workspaceBusy) return;
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
      if (token !== this.workspaceToken || id !== this.state.activeConversationId) return;
      this.update({
        activeWorkspace: metadata.workspace,
        conversations: moveToTop(this.state.conversations, metadata),
        errorMessage: null,
      });
    } catch (error) {
      if (token !== this.workspaceToken) return;
      this.update({ errorMessage: errorMessage(error) });
    } finally {
      if (token === this.workspaceToken) this.update({ workspaceBusy: false });
    }
  }

  async send(): Promise<void> {
    if (
      this.state.status === "streaming" ||
      this.state.status === "loading" ||
      this.state.workspaceBusy
    )
      return;
    const prompt = this.state.draftText.trim();
    if (!prompt) return;
    const workspace = this.state.activeWorkspace;
    if (!workspace) {
      this.update({ errorMessage: "Choose a workspace folder before sending." });
      return;
    }
    const token = ++this.askToken;
    const conversationId = this.state.activeConversationId;
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
      status: "streaming",
      errorMessage: null,
    });
    try {
      const completed = await this.api.ask(
        prompt,
        conversationId,
        conversationId ? null : workspace.path,
        (delta) => this.appendDelta(token, delta),
      );
      if (token !== this.askToken) return;
      const messages = this.state.messages.map((message, index, all) => ({
        ...message,
        content:
          index === all.length - 1 && message.role === "assistant"
            ? completed.answer
            : message.content,
        delivery:
          message.delivery === "pending" ? ("saved" as const) : message.delivery,
      }));
      this.replace({
        ...this.state,
        conversations: moveToTop(this.state.conversations, completed.metadata),
        activeConversationId: completed.metadata.id,
        activeWorkspace: completed.metadata.workspace ?? workspace,
        messages,
        status: "idle",
        errorMessage: null,
      });
    } catch (error) {
      if (token !== this.askToken) return;
      this.replace({
        ...this.state,
        messages: this.state.messages.map((message) => ({
          ...message,
          delivery: message.delivery === "pending" ? "failed" : message.delivery,
        })),
        draftText: prompt,
        status: "error",
        errorMessage: errorMessage(error),
      });
    }
  }

  openSettings(): void {
    if (this.state.status === "streaming" || this.state.workspaceBusy || !this.state.settings)
      return;
    this.settingsToken += 1;
    this.update({
      screen: "settings",
      settingsDraft: draftFromSettings(this.state.settings),
      settingsStatus: "idle",
      settingsErrorMessage: null,
    });
  }

  async retrySettings(): Promise<void> {
    if (
      this.state.status === "streaming" ||
      this.state.workspaceBusy ||
      this.state.settingsStatus === "loading" ||
      this.state.settingsStatus === "saving"
    )
      return;
    this.update({ settingsStatus: "loading", settingsErrorMessage: null });
    await this.loadInitialSettings();
  }

  cancelSettings(): void {
    if (this.state.status === "streaming" || this.state.settingsStatus === "saving") return;
    this.settingsToken += 1;
    this.update({
      screen: "chat",
      settingsDraft: null,
      settingsStatus: "idle",
      settingsErrorMessage: null,
    });
  }

  updateSettingsDraft(patch: Partial<SettingsDraft>): void {
    if (!this.state.settingsDraft || this.state.settingsStatus === "saving") return;
    this.update({ settingsDraft: { ...this.state.settingsDraft, ...patch } });
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

  async saveSettings(): Promise<void> {
    const draft = this.state.settingsDraft;
    if (!draft || this.state.status === "streaming" || this.state.settingsStatus === "saving") {
      return;
    }
    const token = ++this.settingsToken;
    const previousDefault = usableDefaultWorkspace(this.state.settings);
    const activeWorkspaceBeforeSave = this.state.activeWorkspace;
    this.update({ settingsStatus: "saving", settingsErrorMessage: null });
    try {
      const settings = await this.api.saveSettings({
        apiKey: draft.apiKey.trim() || null,
        clearApiKey: draft.clearApiKey,
        baseUrl: draft.baseUrl,
        model: draft.model,
        historyLimit: draft.historyLimit,
        defaultWorkspacePath: draft.defaultWorkspace?.path ?? null,
      });
      if (token !== this.settingsToken) return;
      const nextDefault = usableDefaultWorkspace(settings);
      const activeConversation = this.state.activeConversationId
        ? this.state.conversations.find(
            (conversation) => conversation.id === this.state.activeConversationId,
          )
        : null;
      const activeWorkspace = activeConversation
        ? activeConversation.workspace ?? nextDefault
        : !activeWorkspaceBeforeSave || sameWorkspace(activeWorkspaceBeforeSave, previousDefault)
          ? nextDefault
          : activeWorkspaceBeforeSave;
      this.update({
        screen: "chat",
        settings,
        settingsDraft: null,
        settingsStatus: "idle",
        settingsErrorMessage: null,
        activeWorkspace,
      });
    } catch (error) {
      if (token !== this.settingsToken) return;
      this.update({
        settingsStatus: "error",
        settingsErrorMessage: errorMessage(error),
      });
    }
  }

  async renameActive(title: string): Promise<void> {
    const id = this.state.activeConversationId;
    const normalized = title.trim();
    if (!id || !normalized || this.state.status === "streaming") return;
    const token = ++this.mutationToken;
    this.update({ status: "loading", errorMessage: null });
    try {
      const metadata = await this.api.renameConversation(id, normalized);
      if (token !== this.mutationToken) return;
      this.update({
        conversations: this.state.conversations.map((conversation) =>
          conversation.id === metadata.id ? metadata : conversation,
        ),
        activeWorkspace: metadata.workspace ?? usableDefaultWorkspace(this.state.settings),
        status: "idle",
        errorMessage: null,
      });
    } catch (error) {
      if (token !== this.mutationToken) return;
      this.update({ status: "error", errorMessage: errorMessage(error) });
    }
  }

  async deleteActive(): Promise<void> {
    const id = this.state.activeConversationId;
    if (!id || this.state.status === "streaming") return;
    const token = ++this.mutationToken;
    this.update({ status: "loading", errorMessage: null });
    try {
      await this.api.deleteConversation(id);
      if (token !== this.mutationToken) {
        this.update({
          conversations: this.state.conversations.filter(
            (conversation) => conversation.id !== id,
          ),
        });
        return;
      }
      this.selectionToken += 1;
      this.askToken += 1;
      this.replace({
        ...this.state,
        conversations: this.state.conversations.filter(
          (conversation) => conversation.id !== id,
        ),
        activeConversationId: null,
        activeWorkspace: usableDefaultWorkspace(this.state.settings),
        messages: [],
        draftText: "",
        status: "idle",
        errorMessage: null,
      });
    } catch (error) {
      if (token !== this.mutationToken) return;
      this.update({ status: "error", errorMessage: errorMessage(error) });
    }
  }

  private appendDelta(token: number, delta: string): void {
    if (token !== this.askToken || this.state.status !== "streaming") return;
    const messages = [...this.state.messages];
    const last = messages.at(-1);
    if (!last || last.role !== "assistant" || last.delivery !== "pending") return;
    messages[messages.length - 1] = { ...last, content: last.content + delta };
    this.update({ messages });
  }

  private update(patch: Partial<ChatState>): void {
    this.replace({ ...this.state, ...patch });
  }

  private replace(next: ChatState): void {
    this.state = {
      ...next,
      conversationGroups: groupConversations(next.conversations),
    };
    this.emit();
  }

  private emit(): void {
    for (const listener of this.listeners) listener(this.state);
  }
}
