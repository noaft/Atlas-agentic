import type { AtlasApi } from "./atlas-api";
import type {
  ChatState,
  ConversationMessage,
  ConversationMetadata,
  DisplayMessage,
} from "./types";

type Listener = (state: Readonly<ChatState>) => void;

const initialState = (): ChatState => ({
  conversations: [],
  activeConversationId: null,
  messages: [],
  draftText: "",
  status: "idle",
  errorMessage: null,
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

export class ChatStore {
  private state: ChatState = initialState();
  private readonly listeners = new Set<Listener>();
  private selectionToken = 0;
  private askToken = 0;

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
    this.update({ status: "loading", errorMessage: null });
    try {
      const conversations = await this.api.listConversations();
      this.update({ conversations, status: "idle", errorMessage: null });
    } catch (error) {
      this.update({ status: "error", errorMessage: errorMessage(error) });
    }
  }

  newChat(): void {
    if (this.state.status === "streaming") return;
    this.selectionToken += 1;
    this.askToken += 1;
    this.state = {
      ...this.state,
      activeConversationId: null,
      messages: [],
      draftText: "",
      status: "idle",
      errorMessage: null,
    };
    this.emit();
  }

  async selectConversation(id: string): Promise<void> {
    if (this.state.status === "streaming") return;
    const token = ++this.selectionToken;
    this.update({ status: "loading", errorMessage: null });
    try {
      const conversation = await this.api.loadConversation(id);
      if (token !== this.selectionToken) return;
      this.state = {
        ...this.state,
        activeConversationId: conversation.metadata.id,
        messages: conversation.messages.map(displaySavedMessage),
        status: "idle",
        errorMessage: null,
      };
      this.emit();
    } catch (error) {
      if (token !== this.selectionToken) return;
      this.update({ status: "error", errorMessage: errorMessage(error) });
    }
  }

  setDraft(text: string): void {
    this.update({ draftText: text });
  }

  async send(): Promise<void> {
    if (this.state.status === "streaming" || this.state.status === "loading") return;
    const prompt = this.state.draftText.trim();
    if (!prompt) return;

    const token = ++this.askToken;
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
    this.state = {
      ...this.state,
      messages: [...stableMessages, userMessage, assistantMessage],
      draftText: "",
      status: "streaming",
      errorMessage: null,
    };
    this.emit();

    try {
      const completed = await this.api.ask(
        prompt,
        this.state.activeConversationId,
        (delta) => this.appendDelta(token, delta),
      );
      if (token !== this.askToken) return;
      const messages = this.state.messages.map((message, index, all) => ({
        ...message,
        content:
          index === all.length - 1 && message.role === "assistant"
            ? completed.answer
            : message.content,
        delivery: message.delivery === "pending" ? ("saved" as const) : message.delivery,
      }));
      this.state = {
        ...this.state,
        conversations: moveToTop(this.state.conversations, completed.metadata),
        activeConversationId: completed.metadata.id,
        messages,
        status: "idle",
        errorMessage: null,
      };
      this.emit();
    } catch (error) {
      if (token !== this.askToken) return;
      this.state = {
        ...this.state,
        messages: this.state.messages.map((message) => ({
          ...message,
          delivery: message.delivery === "pending" ? "failed" : message.delivery,
        })),
        draftText: prompt,
        status: "error",
        errorMessage: errorMessage(error),
      };
      this.emit();
    }
  }

  async renameActive(title: string): Promise<void> {
    const id = this.state.activeConversationId;
    const normalized = title.trim();
    if (!id || !normalized || this.state.status === "streaming") return;
    this.update({ status: "loading", errorMessage: null });
    try {
      const metadata = await this.api.renameConversation(id, normalized);
      this.update({
        conversations: this.state.conversations.map((conversation) =>
          conversation.id === metadata.id ? metadata : conversation,
        ),
        status: "idle",
        errorMessage: null,
      });
    } catch (error) {
      this.update({ status: "error", errorMessage: errorMessage(error) });
    }
  }

  async deleteActive(): Promise<void> {
    const id = this.state.activeConversationId;
    if (!id || this.state.status === "streaming") return;
    this.update({ status: "loading", errorMessage: null });
    try {
      await this.api.deleteConversation(id);
      this.selectionToken += 1;
      this.askToken += 1;
      this.state = {
        conversations: this.state.conversations.filter(
          (conversation) => conversation.id !== id,
        ),
        activeConversationId: null,
        messages: [],
        draftText: "",
        status: "idle",
        errorMessage: null,
      };
      this.emit();
    } catch (error) {
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
    this.state = { ...this.state, ...patch };
    this.emit();
  }

  private emit(): void {
    for (const listener of this.listeners) listener(this.state);
  }
}
