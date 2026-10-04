import { afterEach, describe, expect, test } from "vitest";
import { Channel } from "@tauri-apps/api/core";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";

import { createTauriAtlasApi } from "./atlas-api";
import type {
  ChatEvent,
  Conversation,
  ConversationAskResult,
  ConversationMetadata,
} from "./types";

const metadata: ConversationMetadata = {
  id: "00000000-0000-4000-8000-000000000001",
  title: "Desktop chat",
  titleAttempted: true,
  createdAt: "2026-10-04T10:00:00Z",
  updatedAt: "2026-10-04T10:00:01Z",
  messageCount: 2,
};

const conversation: Conversation = {
  metadata,
  messages: [
    { role: "user", content: "hello", createdAt: metadata.createdAt },
    { role: "assistant", content: "hi", createdAt: metadata.updatedAt },
  ],
};

afterEach(() => clearMocks());

describe("Tauri Atlas API", () => {
  test("uses the exact CRUD command names and camelCase payloads", async () => {
    const calls: Array<{ command: string; payload: unknown }> = [];
    mockIPC((command, payload) => {
      calls.push({ command, payload });
      switch (command) {
        case "list_conversations":
          return [metadata];
        case "load_conversation":
          return conversation;
        case "rename_conversation":
          return { ...metadata, title: "Renamed" };
        case "delete_conversation":
          return { id: metadata.id };
        default:
          throw new Error(`unexpected command: ${command}`);
      }
    });
    const api = createTauriAtlasApi();

    await api.listConversations();
    await api.loadConversation(metadata.id);
    await api.renameConversation(metadata.id, "Renamed");
    await api.deleteConversation(metadata.id);

    expect(calls).toEqual([
      { command: "list_conversations", payload: {} },
      { command: "load_conversation", payload: { id: metadata.id } },
      {
        command: "rename_conversation",
        payload: { id: metadata.id, title: "Renamed" },
      },
      { command: "delete_conversation", payload: { id: metadata.id } },
    ]);
  });

  test("delivers ordered ask channel deltas and returns metadata", async () => {
    const result: ConversationAskResult = {
      answer: "Hello desktop",
      metadata,
      created: true,
    };
    mockIPC((command, payload) => {
      expect(command).toBe("ask");
      const args = payload as {
        prompt: string;
        conversationId: string | null;
        channel: Channel<ChatEvent>;
      };
      expect(args.prompt).toBe("hello");
      expect(args.conversationId).toBeNull();
      args.channel.onmessage({ event: "delta", data: { content: "Hello" } });
      args.channel.onmessage({ event: "delta", data: { content: " desktop" } });
      return result;
    });
    const deltas: string[] = [];

    const completed = await createTauriAtlasApi().ask(
      "hello",
      null,
      (delta) => deltas.push(delta),
    );

    expect(deltas).toEqual(["Hello", " desktop"]);
    expect(completed).toEqual(result);
  });
});
