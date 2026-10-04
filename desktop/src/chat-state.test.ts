import { expect, test } from "vitest";

import type { AtlasApi } from "./atlas-api";
import { ChatStore, groupConversations } from "./chat-state";
import type {
  Conversation,
  ConversationAskResult,
  ConversationMetadata,
  SaveSettingsInput,
  SettingsView,
} from "./types";

const workspace = { path: "/work/atlas", name: "atlas" };
const settings: SettingsView = {
  apiKeySource: "missing",
  baseUrl: "https://api.deepseek.com",
  model: "deepseek-flash",
  historyLimit: 40,
  defaultWorkspace: null,
  environmentOverrides: [],
};

const first: ConversationMetadata = {
  id: "00000000-0000-4000-8000-000000000001",
  title: "First",
  titleAttempted: true,
  createdAt: "2026-10-04T10:00:00Z",
  updatedAt: "2026-10-04T10:00:01Z",
  messageCount: 2,
  workspace,
};

const second: ConversationMetadata = {
  id: "00000000-0000-4000-8000-000000000002",
  title: "Second",
  titleAttempted: true,
  createdAt: "2026-10-04T11:00:00Z",
  updatedAt: "2026-10-04T11:00:01Z",
  messageCount: 2,
  workspace: { path: "/work/second", name: "second" },
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
  askCalls: Array<{
    prompt: string;
    conversationId: string | null;
    workspacePath: string | null;
  }> = [];
  renameCalls: Array<{ id: string; title: string }> = [];
  deleteCalls: string[] = [];
  conversations: ConversationMetadata[] = [];
  loads = new Map<string, () => Promise<Conversation>>();
  asks: Array<
    (
      prompt: string,
      conversationId: string | null,
      workspacePath: string | null,
      onDelta: (text: string) => void,
    ) => Promise<ConversationAskResult>
  > = [];
  rename: (id: string, title: string) => Promise<ConversationMetadata> = async (
    id,
    title,
  ) => ({ ...this.conversations.find((item) => item.id === id)!, title });
  remove: (id: string) => Promise<void> = async () => {};
  settings: SettingsView = settings;
  chosenWorkspace: string | null = workspace.path;
  savedSettings: SaveSettingsInput[] = [];
  assignments: Array<{ id: string; path: string }> = [];
  chooseCalls = 0;
  validateCalls: string[] = [];
  save: (input: SaveSettingsInput) => Promise<SettingsView> = async () => this.settings;
  assign: (id: string, path: string) => Promise<ConversationMetadata> = async (
    id,
    path,
  ) => {
    const existing = this.conversations.find((item) => item.id === id) ?? first;
    return { ...existing, workspace: await this.validateWorkspace(path) };
  };

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
    workspacePath: string | null,
    onDelta: (text: string) => void,
  ): Promise<ConversationAskResult> {
    this.askCalls.push({ prompt, conversationId, workspacePath });
    const ask = this.asks.shift();
    if (!ask) throw new Error("missing fake ask");
    return ask(prompt, conversationId, workspacePath, onDelta);
  }

  renameConversation(id: string, title: string): Promise<ConversationMetadata> {
    this.renameCalls.push({ id, title });
    return this.rename(id, title);
  }

  deleteConversation(id: string): Promise<void> {
    this.deleteCalls.push(id);
    return this.remove(id);
  }

  async loadSettings(): Promise<SettingsView> {
    return this.settings;
  }

  async saveSettings(input: SaveSettingsInput): Promise<SettingsView> {
    this.savedSettings.push(input);
    return this.save(input);
  }

  async chooseWorkspace(): Promise<string | null> {
    this.chooseCalls += 1;
    return this.chosenWorkspace;
  }

  async validateWorkspace(path: string) {
    this.validateCalls.push(path);
    return { path, name: path.split("/").at(-1) ?? path };
  }

  async assignConversationWorkspace(id: string, path: string) {
    this.assignments.push({ id, path });
    return this.assign(id, path);
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

  await store.newChat();

  expect(api.listCalls).toBe(1);
  expect(api.loadCalls).toEqual([]);
  expect(api.askCalls).toEqual([]);
  expect(store.getState().activeConversationId).toBeNull();
  expect(store.getState().messages).toEqual([]);
});

test("first success adopts the returned id and sidebar metadata", async () => {
  const api = new FakeApi();
  api.asks.push(async (_prompt, _id, _workspace, onDelta) => {
    onDelta("Hello");
    onDelta(" desktop");
    return result(first, "Hello desktop", true);
  });
  const store = new ChatStore(api);
  await store.chooseWorkspaceForDraft();
  store.setDraft("  hello  ");

  await store.send();

  const state = store.getState();
  expect(api.askCalls).toEqual([
    { prompt: "hello", conversationId: null, workspacePath: workspace.path },
  ]);
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
  api.asks.push(async (_prompt, _id, _workspace, onDelta) => {
    onDelta("partial");
    throw new Error("provider missing");
  });
  const store = new ChatStore(api);
  await store.chooseWorkspaceForDraft();
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
  api.asks.push(async (_prompt, _id, _workspace, onDelta) => {
    onDelta("partial");
    throw new Error("temporary failure");
  });
  api.asks.push(async (_prompt, _id, _workspace, onDelta) => {
    onDelta("complete");
    return result(first, "complete", true);
  });
  const store = new ChatStore(api);
  await store.chooseWorkspaceForDraft();
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
  api.asks.push(async (_prompt, _id, _workspace, onDelta) => {
    onDelta("next answer");
    return result(updated, "next answer", false);
  });
  const store = new ChatStore(api);
  await store.initialize();
  await store.selectConversation(first.id);
  store.setDraft("next question");

  await store.send();

  expect(api.askCalls).toEqual([
    { prompt: "next question", conversationId: first.id, workspacePath: null },
  ]);
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
  api.asks.push(async (_prompt, _id, _workspace, onDelta) => {
    emit = onDelta;
    onDelta("complete");
    return result(first, "complete", true);
  });
  const store = new ChatStore(api);
  await store.chooseWorkspaceForDraft();
  store.setDraft("hello");
  await store.send();

  emit(" late");

  expect(store.getState().messages.at(-1)?.content).toBe("complete");
});

test("startup listing does not interrupt an ask that starts while it is pending", async () => {
  const api = new FakeApi();
  const listing = deferred<ConversationMetadata[]>();
  api.listConversations = () => listing.promise;
  const asking = deferred<ConversationAskResult>();
  let emit!: (text: string) => void;
  api.asks.push(async (_prompt, _id, _workspace, onDelta) => {
    emit = onDelta;
    return asking.promise;
  });
  const store = new ChatStore(api);
  await store.chooseWorkspaceForDraft();

  const initializing = store.initialize();
  store.setDraft("hello");
  const sending = store.send();
  expect(store.getState().status).toBe("streaming");

  listing.resolve([second]);
  await initializing;
  expect(store.getState().status).toBe("streaming");
  emit("still streaming");
  expect(store.getState().messages.at(-1)?.content).toBe("still streaming");

  asking.resolve(result(first, "still streaming", true));
  await sending;
});

test("a completed delete cannot erase a newer draft", async () => {
  const api = new FakeApi();
  api.conversations = [first];
  api.loads.set(first.id, async () => savedConversation(first));
  const deleting = deferred<void>();
  api.remove = () => deleting.promise;
  const store = new ChatStore(api);
  await store.initialize();
  await store.selectConversation(first.id);

  const pendingDelete = store.deleteActive();
  await store.newChat();
  store.setDraft("new work");
  deleting.resolve();
  await pendingDelete;

  expect(store.getState().activeConversationId).toBeNull();
  expect(store.getState().draftText).toBe("new work");
  expect(store.getState().conversations).toEqual([]);
});

test("startup loads conversations and settings independently", async () => {
  const api = new FakeApi();
  api.listConversations = async () => {
    throw new Error("list unavailable");
  };
  api.settings = { ...settings, defaultWorkspace: workspace };
  const store = new ChatStore(api);

  await store.initialize();

  expect(store.getState().listStatus).toBe("error");
  expect(store.getState().settings).toEqual(api.settings);
  expect(store.getState().activeWorkspace).toEqual(workspace);
});

test("new chat adopts a valid default and cancelled selection stays unassigned", async () => {
  const api = new FakeApi();
  api.settings = { ...settings, defaultWorkspace: workspace };
  const store = new ChatStore(api);
  await store.initialize();

  await store.newChat();
  expect(store.getState().activeWorkspace).toEqual(workspace);
  expect(api.chooseCalls).toBe(0);

  api.settings = settings;
  api.chosenWorkspace = null;
  const cancelled = new ChatStore(api);
  await cancelled.initialize();
  await cancelled.newChat();
  expect(cancelled.getState().activeWorkspace).toBeNull();
  expect(cancelled.getState().activeConversationId).toBeNull();
});

test("send is blocked until a workspace is selected", async () => {
  const api = new FakeApi();
  const store = new ChatStore(api);
  store.setDraft("inspect this repo");

  await store.send();

  expect(api.askCalls).toEqual([]);
  expect(store.getState().errorMessage).toContain("workspace");
  expect(store.getState().draftText).toBe("inspect this repo");
});

test("legacy conversation is readable and assignment commits only after backend success", async () => {
  const api = new FakeApi();
  const legacy = { ...first, workspace: null };
  api.conversations = [legacy];
  api.loads.set(legacy.id, async () => savedConversation(legacy));
  const assignment = deferred<ConversationMetadata>();
  api.assign = () => assignment.promise;
  const store = new ChatStore(api);
  await store.initialize();
  await store.selectConversation(legacy.id);
  store.setDraft("continue");
  await store.send();
  expect(api.askCalls).toEqual([]);

  const changing = store.changeActiveWorkspace();
  expect(store.getState().activeWorkspace).toBeNull();
  assignment.resolve({ ...legacy, workspace });
  await changing;
  expect(store.getState().activeWorkspace).toEqual(workspace);
});

test("workspace groups and members are ordered by their newest conversation", () => {
  const newestFirstWorkspace = {
    ...first,
    id: "00000000-0000-4000-8000-000000000003",
    updatedAt: "2026-10-04T13:00:00Z",
  };
  const olderSameWorkspace = { ...first, updatedAt: "2026-10-04T09:00:00Z" };
  const unassigned = { ...second, workspace: null };

  const groups = groupConversations([olderSameWorkspace, unassigned, newestFirstWorkspace]);

  expect(groups.map((group) => group.key)).toEqual([workspace.path, "__unassigned__"]);
  expect(groups[0].conversations.map((item) => item.id)).toEqual([
    newestFirstWorkspace.id,
    olderSameWorkspace.id,
  ]);
});

test("settings cancel discards edits while failure keeps them and success uses masked view", async () => {
  const api = new FakeApi();
  const store = new ChatStore(api);
  await store.initialize();
  store.openSettings();
  store.updateSettingsDraft({ model: "edited-model", apiKey: "" });
  store.cancelSettings();
  expect(store.getState().settingsDraft).toBeNull();
  expect(store.getState().settings?.model).toBe("deepseek-flash");

  store.openSettings();
  store.updateSettingsDraft({ model: "edited-model", apiKey: "" });
  api.save = async () => {
    throw new Error("save failed");
  };
  await store.saveSettings();
  expect(store.getState().screen).toBe("settings");
  expect(store.getState().settingsDraft?.model).toBe("edited-model");
  expect(api.savedSettings.at(-1)?.apiKey).toBeNull();

  const returned = { ...settings, apiKeySource: "settings" as const, model: "edited-model" };
  api.save = async () => returned;
  await store.saveSettings();
  expect(store.getState().screen).toBe("chat");
  expect(store.getState().settings).toEqual(returned);
  expect(store.getState().settingsDraft).toBeNull();
});

test("settings and workspace navigation are disabled while an ask streams", async () => {
  const api = new FakeApi();
  const pending = deferred<ConversationAskResult>();
  api.asks.push(async () => pending.promise);
  const store = new ChatStore(api);
  await store.initialize();
  await store.chooseWorkspaceForDraft();
  store.setDraft("hello");

  const sending = store.send();
  store.openSettings();
  await store.changeActiveWorkspace();

  expect(store.getState().screen).toBe("chat");
  expect(api.chooseCalls).toBe(1);
  pending.resolve(result(first, "answer", true));
  await sending;
});
