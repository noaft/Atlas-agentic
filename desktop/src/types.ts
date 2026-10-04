export type MessageRole = "user" | "assistant";

export interface ConversationMetadata {
  id: string;
  title: string;
  titleAttempted: boolean;
  createdAt: string;
  updatedAt: string;
  messageCount: number;
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
}
