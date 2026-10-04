import { expect, test } from "vitest";

import type { AtlasApi } from "./atlas-api";
import { ChatStore } from "./chat-state";
import type {
  Conversation,
  ConversationAskResult,
  ConversationMetadata,
} from "./types";

const first: ConversationMetadata = {
  id: "00000000-0000-4000-8000-000000000001",
  title: "First",
  titleAttempted: true,
  createdAt: "2026-10-04T10:00:00Z",
  updatedAt: "2026-10-04T10:00:01Z",
  messageCount: 2,
};

const second: ConversationMetadata = {
  id: "00000000-0000-4000-8000-000000000002",
  title: "Second",
  titleAttempted: true,
  createdAt: "2026-10-04T11:00:00Z",
  updatedAt: "2026-10-04T11:00:01Z",
  messageCount: 2,
};

function savedConversation(metadata: ConversationMetadata): Conversation {
  return {
    metadata,
    messages: [
      { role: "user", content: `${metadata.title} question`, createdAt: metadata.createdAt },
      {
        role: "assistant",
        content: `${metadata.title} answer`,
        createdAt: metadata.updatedAt,
      },
    ],
  };
}

class FakeApi implements AtlasApi {
  listCalls = 0;
  loadCalls: string[] = [];
  askCalls: Array<{ prompt: string; conversationId: string | null }> = [];
  renameCalls: Array<{ id: string; title: string }> = [];
  deleteCalls: string[] = [];
  conversations: ConversationMetadata[] = [];
  loads = new Map<string, () => Promise<Conversation>>();
  asks: Array<
    (
      prompt: string,
      conversationId: string | null,
      onDelta: (text: string) => void,
    ) => Promise<ConversationAskResult>
  > = [];
  rename: (id: string, title: string) => Promise<ConversationMetadata> = async (
    id,
    title,
  ) => ({ ...this.conversations.find((item) => item.id === id)!, title });
  remove: (id: string) => Promise<void> = async () => {};

  async listConversations(): Promise<ConversationMetadata[]> {
    this.listCalls += 1;
    return this.conversations;
  }

  loadConversation(id: string): Promise<Conversation> {
    this.loadCalls.push(id);
    const load = this.loads.get(id);
    if (!load) throw new Error(`missing fake conversation: ${id}`);
    return load();
  }

  ask(
    prompt: string,
    conversationId: string | null,
    onDelta: (text: string) => void,
  ): Promise<ConversationAskResult> {
    this.askCalls.push({ prompt, conversationId });
    const ask = this.asks.shift();
    if (!ask) throw new Error("missing fake ask");
    return ask(prompt, conversationId, onDelta);
  }

  renameConversation(id: string, title: string): Promise<ConversationMetadata> {
    this.renameCalls.push({ id, title });
    return this.rename(id, title);
  }

  deleteConversation(id: string): Promise<void> {
    this.deleteCalls.push(id);
    return this.remove(id);
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function result(metadata: ConversationMetadata, answer: string, created: boolean) {
  return { answer, metadata, created } satisfies ConversationAskResult;
}

test("new chat never calls the backend", async () => {
  const api = new FakeApi();
  api.conversations = [first];
  const store = new ChatStore(api);
  await store.initialize();

  store.newChat();

  expect(api.listCalls).toBe(1);
  expect(api.loadCalls).toEqual([]);
  expect(api.askCalls).toEqual([]);
  expect(store.getState().activeConversationId).toBeNull();
  expect(store.getState().messages).toEqual([]);
});

test("first success adopts the returned id and sidebar metadata", async () => {
  const api = new FakeApi();
  api.asks.push(async (_prompt, _id, onDelta) => {
    onDelta("Hello");
    onDelta(" desktop");
    return result(first, "Hello desktop", true);
  });
  const store = new ChatStore(api);
  store.setDraft("  hello  ");

  await store.send();

  const state = store.getState();
  expect(api.askCalls).toEqual([{ prompt: "hello", conversationId: null }]);
  expect(state.activeConversationId).toBe(first.id);
  expect(state.conversations).toEqual([first]);
  expect(state.messages.map(({ role, content, delivery }) => ({ role, content, delivery }))).toEqual([
    { role: "user", content: "hello", delivery: "saved" },
    { role: "assistant", content: "Hello desktop", delivery: "saved" },
  ]);
  expect(state.status).toBe("idle");
});

test("failed first send stays unsaved, marks partial failed, and restores prompt", async () => {
  const api = new FakeApi();
  api.asks.push(async (_prompt, _id, onDelta) => {
    onDelta("partial");
    throw new Error("provider missing");
  });
  const store = new ChatStore(api);
  store.setDraft("hello");

  await store.send();

  const state = store.getState();
  expect(state.activeConversationId).toBeNull();
  expect(state.draftText).toBe("hello");
  expect(state.status).toBe("error");
  expect(state.errorMessage).toBe("provider missing");
  expect(state.messages[1]).toMatchObject({
    role: "assistant",
    content: "partial",
    delivery: "failed",
  });
});

test("retry removes the previous failed attempt", async () => {
  const api = new FakeApi();
  api.asks.push(async (_prompt, _id, onDelta) => {
    onDelta("partial");
    throw new Error("temporary failure");
  });
  api.asks.push(async (_prompt, _id, onDelta) => {
    onDelta("complete");
    return result(first, "complete", true);
  });
  const store = new ChatStore(api);
  store.setDraft("hello");
  await store.send();

  await store.send();

  expect(store.getState().messages.map((message) => message.content)).toEqual([
    "hello",
    "complete",
  ]);
  expect(store.getState().messages.every((message) => message.delivery === "saved")).toBe(true);
});

test("continue keeps the id and moves updated metadata to the top", async () => {
  const api = new FakeApi();
  api.conversations = [second, first];
  api.loads.set(first.id, async () => savedConversation(first));
  const updated = { ...first, updatedAt: "2026-10-04T12:00:00Z", messageCount: 4 };
  api.asks.push(async (_prompt, _id, onDelta) => {
    onDelta("next answer");
    return result(updated, "next answer", false);
  });
  const store = new ChatStore(api);
  await store.initialize();
  await store.selectConversation(first.id);
  store.setDraft("next question");

  await store.send();

  expect(api.askCalls).toEqual([{ prompt: "next question", conversationId: first.id }]);
  expect(store.getState().activeConversationId).toBe(first.id);
  expect(store.getState().conversations.map((item) => item.id)).toEqual([first.id, second.id]);
  expect(store.getState().messages).toHaveLength(4);
});

test("rename and delete commit state only after success", async () => {
  const api = new FakeApi();
  api.conversations = [first];
  api.loads.set(first.id, async () => savedConversation(first));
  const store = new ChatStore(api);
  await store.initialize();
  await store.selectConversation(first.id);
  api.rename = async () => {
    throw new Error("rename failed");
  };

  await store.renameActive("  Renamed  ");
  expect(store.getState().conversations[0].title).toBe("First");
  expect(api.renameCalls[0]).toEqual({ id: first.id, title: "Renamed" });

  api.rename = async (_id, title) => ({ ...first, title });
  await store.renameActive("Renamed");
  expect(store.getState().conversations[0].title).toBe("Renamed");

  api.remove = async () => {
    throw new Error("delete failed");
  };
  await store.deleteActive();
  expect(store.getState().activeConversationId).toBe(first.id);

  api.remove = async () => {};
  await store.deleteActive();
  expect(store.getState().activeConversationId).toBeNull();
  expect(store.getState().conversations).toEqual([]);
});

test("missing provider keeps CRUD and retry available", async () => {
  const api = new FakeApi();
  api.conversations = [first];
  api.loads.set(first.id, async () => savedConversation(first));
  api.asks.push(async () => {
    throw new Error("DEEPSEEK_API_KEY is required");
  });
  const store = new ChatStore(api);
  await store.initialize();
  await store.selectConversation(first.id);
  store.setDraft("hello");
  await store.send();

  api.rename = async (_id, title) => ({ ...first, title });
  await store.renameActive("Still editable");

  expect(store.getState().conversations[0].title).toBe("Still editable");
  expect(store.getState().draftText).toBe("hello");
});

test("latest conversation selection wins when loads resolve out of order", async () => {
  const api = new FakeApi();
  api.conversations = [first, second];
  const firstLoad = deferred<Conversation>();
  const secondLoad = deferred<Conversation>();
  api.loads.set(first.id, () => firstLoad.promise);
  api.loads.set(second.id, () => secondLoad.promise);
  const store = new ChatStore(api);
  await store.initialize();

  const selectingFirst = store.selectConversation(first.id);
  const selectingSecond = store.selectConversation(second.id);
  secondLoad.resolve(savedConversation(second));
  await selectingSecond;
  firstLoad.resolve(savedConversation(first));
  await selectingFirst;

  expect(store.getState().activeConversationId).toBe(second.id);
  expect(store.getState().messages[0].content).toBe("Second question");
});

test("late delta after settlement is ignored", async () => {
  const api = new FakeApi();
  let emit!: (text: string) => void;
  api.asks.push(async (_prompt, _id, onDelta) => {
    emit = onDelta;
    onDelta("complete");
    return result(first, "complete", true);
  });
  const store = new ChatStore(api);
  store.setDraft("hello");
  await store.send();

  emit(" late");

  expect(store.getState().messages.at(-1)?.content).toBe("complete");
});
