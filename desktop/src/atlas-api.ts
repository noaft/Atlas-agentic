import { Channel, invoke } from "@tauri-apps/api/core";

import type {
  ChatEvent,
  Conversation,
  ConversationAskResult,
  ConversationMetadata,
} from "./types";

export interface AtlasApi {
  listConversations(): Promise<ConversationMetadata[]>;
  loadConversation(id: string): Promise<Conversation>;
  ask(
    prompt: string,
    conversationId: string | null,
    onDelta: (text: string) => void,
  ): Promise<ConversationAskResult>;
  renameConversation(id: string, title: string): Promise<ConversationMetadata>;
  deleteConversation(id: string): Promise<void>;
}

export function createTauriAtlasApi(): AtlasApi {
  return {
    listConversations: () => invoke<ConversationMetadata[]>("list_conversations"),
    loadConversation: (id) => invoke<Conversation>("load_conversation", { id }),
    ask: (prompt, conversationId, onDelta) => {
      const channel = new Channel<ChatEvent>((message) => {
        if (message.event === "delta") {
          onDelta(message.data.content);
        }
      });
      return invoke<ConversationAskResult>("ask", {
        prompt,
        conversationId,
        channel,
      });
    },
    renameConversation: (id, title) =>
      invoke<ConversationMetadata>("rename_conversation", { id, title }),
    deleteConversation: async (id) => {
      await invoke("delete_conversation", { id });
    },
  };
}
