import { Channel, invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";

import type {
  ChatEvent,
  Conversation,
  ConversationAskResult,
  ConversationMetadata,
  SaveSettingsInput,
  SettingsView,
  Workspace,
} from "./types";

export interface AtlasApi {
  listConversations(): Promise<ConversationMetadata[]>;
  loadConversation(id: string): Promise<Conversation>;
  ask(
    prompt: string,
    conversationId: string | null,
    workspacePath: string | null,
    onDelta: (text: string) => void,
  ): Promise<ConversationAskResult>;
  renameConversation(id: string, title: string): Promise<ConversationMetadata>;
  deleteConversation(id: string): Promise<void>;
  loadSettings(): Promise<SettingsView>;
  saveSettings(input: SaveSettingsInput): Promise<SettingsView>;
  chooseWorkspace(): Promise<string | null>;
  validateWorkspace(path: string): Promise<Workspace>;
  assignConversationWorkspace(id: string, path: string): Promise<ConversationMetadata>;
  toggleFullscreen(): Promise<boolean>;
}

export function createTauriAtlasApi(): AtlasApi {
  return {
    listConversations: () => invoke<ConversationMetadata[]>("list_conversations"),
    loadConversation: (id) => invoke<Conversation>("load_conversation", { id }),
    ask: (prompt, conversationId, workspacePath, onDelta) => {
      const channel = new Channel<ChatEvent>((message) => {
        if (message.event === "delta") {
          onDelta(message.data.content);
        }
      });
      return invoke<ConversationAskResult>("ask", {
        prompt,
        conversationId,
        workspacePath,
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
    chooseWorkspace: async () => {
      const selected = await open({ directory: true, multiple: false });
      return typeof selected === "string" ? selected : null;
    },
    validateWorkspace: (path) => invoke<Workspace>("validate_workspace", { path }),
    assignConversationWorkspace: (id, path) =>
      invoke<ConversationMetadata>("assign_conversation_workspace", { id, path }),
    toggleFullscreen: async () => {
      const window = getCurrentWindow();
      const fullscreen = !(await window.isFullscreen());
      await window.setFullscreen(fullscreen);
      return fullscreen;
    },
  };
}
