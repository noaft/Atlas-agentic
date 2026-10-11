import { expect, test } from "vitest";

import type { AtlasApi } from "./atlas-api";
import { ChatStore, groupConversations } from "./chat-state";
import type {
  Conversation,
  ConversationAskResult,
  ConversationMetadata,
  SaveSettingsInput,
  SettingsView,
  ChatEvent,
  ActivityRecord,
  PermissionMode,
} from "./types";

const workspace = { path: "/work/atlas", name: "atlas" };
const automaticWorkspace = { path: "/home/user/Atlas", name: "Atlas" };
const settings: SettingsView = {
  apiKeySource: "missing",
  baseUrl: "https://api.deepseek.com",
  model: "deepseek-flash",
  activeModel: "deepseek-flash",
  historyLimit: 40,
  defaultWorkspace: automaticWorkspace,
  defaultWorkspaceAutomatic: true,
  defaultWorkspaceError: null,
  defaultPermissionMode: "ask",
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
  permissionMode: "ask",
};

const second: ConversationMetadata = {
  id: "00000000-0000-4000-8000-000000000002",
  title: "Second",
  titleAttempted: true,
  createdAt: "2026-10-04T11:00:00Z",
  updatedAt: "2026-10-04T11:00:01Z",
  messageCount: 2,
  workspace: { path: "/work/second", name: "second" },
  permissionMode: "workspace",
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

const thinkingActivity: ActivityRecord = {
  sequence: 0,
  kind: "status",
  tool: null,
  arguments: null,
  outcome: null,
  summary: "Thinking",
  details: null,
  createdAt: "2026-10-04T10:00:00Z",
};

class FakeApi implements AtlasApi {
  listCalls = 0;
  loadCalls: string[] = [];
  askCalls: Array<{
    prompt: string;
    conversationId: string | null;
    workspacePath: string | null;
  }> = [];
  askPermissionModes: PermissionMode[] = [];
  eventSinks: Array<(event: ChatEvent) => void> = [];
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
  selectedModels: string[] = [];
  models = [
    { id: "deepseek-flash", name: "DeepSeek Flash" },
    { id: "deepseek-v4-pro", name: "DeepSeek V4 Pro" },
  ];
  listModelCalls = 0;
  assignments: Array<{ id: string; path: string }> = [];
  permissionUpdates: Array<{ id: string; permissionMode: PermissionMode }> = [];
  approvalResolutions: Array<{ id: string; allow: boolean }> = [];
  approvalCancellations = 0;
  chooseCalls = 0;
  validateCalls: string[] = [];
  save: (input: SaveSettingsInput) => Promise<SettingsView> = async () => this.settings;
  loadSettingsResult: () => Promise<SettingsView> = async () => this.settings;
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
    permissionMode: PermissionMode,
    onEvent: (event: ChatEvent) => void,
  ): Promise<ConversationAskResult> {
    this.askCalls.push({ prompt, conversationId, workspacePath });
    this.askPermissionModes.push(permissionMode);
    this.eventSinks.push(onEvent);
    const ask = this.asks.shift();
    if (!ask) throw new Error("missing fake ask");
    return ask(prompt, conversationId, workspacePath, (text) =>
      onEvent({ event: "delta", data: { content: text } }),
    );
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
    return this.loadSettingsResult();
  }

  async saveSettings(input: SaveSettingsInput): Promise<SettingsView> {
    this.savedSettings.push(input);
    return this.save(input);
  }

  async listModels() {
    this.listModelCalls += 1;
    return this.models;
  }

  async setModel(model: string) {
    this.selectedModels.push(model);
    return { ...this.settings, model, activeModel: model };
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

  async setConversationPermissionMode(id: string, permissionMode: PermissionMode) {
    this.permissionUpdates.push({ id, permissionMode });
    const existing = this.conversations.find((item) => item.id === id) ?? first;
    return { ...existing, permissionMode };
  }

  async resolveToolApproval(id: string, allow: boolean) {
    this.approvalResolutions.push({ id, allow });
  }

  async cancelToolApprovals() {
    this.approvalCancellations += 1;
  }

  async toggleFullscreen() {
    return true;
  }
}

test("draft permission overrides stay local and new chats restore the settings default", async () => {
  const api = new FakeApi();
  api.settings = { ...settings, defaultPermissionMode: "workspace" };
  const store = new ChatStore(api);
  await store.initialize();

  expect(store.getState().activePermissionMode).toBe("workspace");
  await store.selectPermissionMode("fullAccess");
  expect(store.getState().activePermissionMode).toBe("fullAccess");
  expect(api.permissionUpdates).toEqual([]);

  await store.newChat();
  expect(store.getState().activePermissionMode).toBe("workspace");
});

test("saved chats restore and persist their own permission mode", async () => {
  const api = new FakeApi();
  api.conversations = [second];
  api.loads.set(second.id, async () => savedConversation(second));
  const store = new ChatStore(api);
  await store.initialize();
  await store.selectConversation(second.id);

  expect(store.getState().activePermissionMode).toBe("workspace");
  await store.selectPermissionMode("fullAccess");

  expect(api.permissionUpdates).toEqual([
    { id: second.id, permissionMode: "fullAccess" },
  ]);
  expect(store.getState().activePermissionMode).toBe("fullAccess");
});

test("legacy saved chats fall back to ask instead of the settings default", async () => {
  const api = new FakeApi();
  const legacy = { ...second, permissionMode: null };
  api.settings = { ...settings, defaultPermissionMode: "fullAccess" };
  api.conversations = [legacy];
  api.loads.set(legacy.id, async () => savedConversation(legacy));
  const store = new ChatStore(api);
  await store.initialize();
  await store.selectConversation(legacy.id);

  expect(store.getState().activePermissionMode).toBe("ask");
});

test("approval and tool events are scoped to the active ask", async () => {
  const api = new FakeApi();
  const pending = deferred<ConversationAskResult>();
  api.asks.push(() => pending.promise);
  const store = new ChatStore(api);
  await store.initialize();
  store.setDraft("remove note");
  const sending = store.send();
  await Promise.resolve();
  const sink = api.eventSinks[0]!;

  sink({
    event: "toolRequested",
    data: { sequence: 0, tool: "delete_file", arguments: '{"path":"note.txt"}' },
  });
  sink({
    event: "approvalRequested",
    data: {
      sequence: 0,
      request: {
        id: "approval-1",
        tool: "delete_file",
        arguments: '{"path":"note.txt"}',
        reason: "deletes or overwrites data",
      },
    },
  });
  expect(store.getState().pendingApproval?.id).toBe("approval-1");
  expect(store.getState().toolActivity[0]).toMatchObject({ status: "waiting" });

  await store.resolveToolApproval(false);
  expect(api.approvalResolutions).toEqual([{ id: "approval-1", allow: false }]);
  expect(store.getState().pendingApproval).toBeNull();

  sink({
    event: "toolDenied",
    data: { sequence: 0, tool: "delete_file", summary: "delete_file was denied" },
  });
  expect(store.getState().toolActivity[0]).toMatchObject({ status: "denied" });
  pending.resolve(result(first, "Denied", true));
  await sending;
  expect(api.askPermissionModes).toEqual(["ask"]);
  sink({
    event: "approvalRequested",
    data: {
      sequence: 0,
      request: {
        id: "stale",
        tool: "delete_file",
        arguments: "{}",
        reason: "stale",
      },
    },
  });
  expect(store.getState().pendingApproval).toBeNull();
});

test("leaving a pending approval cancels the ask and clears unsaved messages", async () => {
  const api = new FakeApi();
  const pending = deferred<ConversationAskResult>();
  api.asks.push(() => pending.promise);
  const store = new ChatStore(api);
  await store.initialize();
  store.setDraft("remove note");
  const sending = store.send();
  await Promise.resolve();
  api.eventSinks[0]!({
    event: "approvalRequested",
    data: {
      sequence: 0,
      request: {
        id: "approval-cancel",
        tool: "delete_file",
        arguments: "{}",
        reason: "deletes data",
      },
    },
  });

  await store.newChat();
  expect(api.approvalCancellations).toBe(1);
  expect(store.getState().messages).toEqual([]);
  expect(store.getState().activeConversationId).toBeNull();

  pending.reject(new Error("tool approval was cancelled"));
  await sending;
  expect(store.getState().messages).toEqual([]);
});

test("activity events update repeated tools by sequence and canonicalize on success", async () => {
  const api = new FakeApi();
  const pending = deferred<ConversationAskResult>();
  api.asks.push(() => pending.promise);
  const store = new ChatStore(api);
  await store.initialize();
  store.setDraft("search twice");
  const sending = store.send();
  await Promise.resolve();
  const sink = api.eventSinks[0]!;

  sink({ event: "runtimeStatus", data: { status: "thinking" } });
  sink({
    event: "toolRequested",
    data: { sequence: 0, tool: "search_files", arguments: '{"query":"one"}' },
  });
  sink({ event: "toolStarted", data: { sequence: 0, tool: "search_files" } });
  sink({
    event: "toolRequested",
    data: { sequence: 1, tool: "search_files", arguments: '{"query":"two"}' },
  });
  sink({ event: "toolStarted", data: { sequence: 1, tool: "search_files" } });
  sink({
    event: "toolFinished",
    data: {
      sequence: 0,
      tool: "search_files",
      success: true,
      summary: "first search succeeded",
      details: "first result",
    },
  });

  expect(store.getState().toolActivity).toEqual(
    expect.arrayContaining([
      expect.objectContaining({ sequence: 0, status: "succeeded" }),
      expect.objectContaining({ sequence: 1, status: "running" }),
    ]),
  );

  pending.resolve({ answer: "Done", metadata: first, created: true, activities: [thinkingActivity] });
  await sending;
  expect(store.getState().messages.at(-1)?.activity).toEqual([thinkingActivity]);
  expect(store.getState().toolActivity).toEqual([]);

  sink({ event: "runtimeStatus", data: { status: "preparingAnswer" } });
  expect(store.getState().messages.at(-1)?.activity).toEqual([thinkingActivity]);
});

test("restored activity is associated by assistant message index", async () => {
  const api = new FakeApi();
  api.conversations = [first];
  api.loads.set(first.id, async () => ({
    ...savedConversation(first),
    activities: [
      {
        turnId: "turn-1",
        assistantMessageIndex: 1,
        records: [thinkingActivity],
      },
    ],
  }));
  const store = new ChatStore(api);
  await store.initialize();

  await store.selectConversation(first.id);

  expect(store.getState().messages[1].activity).toEqual([thinkingActivity]);
  expect(store.getState().messages[0].activity).toBeUndefined();
});

test("failed request keeps transient activity until the next chat action", async () => {
  const api = new FakeApi();
  const pending = deferred<ConversationAskResult>();
  api.asks.push(() => pending.promise);
  const store = new ChatStore(api);
  await store.initialize();
  store.setDraft("fail after search");
  const sending = store.send();
  await Promise.resolve();
  api.eventSinks[0]!({ event: "runtimeStatus", data: { status: "thinking" } });
  pending.reject(new Error("model failed"));
  await sending;

  expect(store.getState().toolActivity).toHaveLength(1);
  await store.newChat();
  expect(store.getState().toolActivity).toEqual([]);
});

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

test("failed send preserves a new draft typed while the request is pending", async () => {
  const api = new FakeApi();
  const asking = deferred<ConversationAskResult>();
  api.asks.push(async () => asking.promise);
  const store = new ChatStore(api);
  await store.chooseWorkspaceForDraft();
  store.setDraft("original request");

  const sending = store.send();
  store.setDraft("new draft typed during streaming");
  asking.reject(new Error("provider failed"));
  await sending;

  expect(store.getState().draftText).toBe("new draft typed during streaming");
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

test("initialization_uses_automatic_workspace_without_picker", async () => {
  const api = new FakeApi();
  api.listConversations = async () => {
    throw new Error("list unavailable");
  };
  const store = new ChatStore(api);

  await store.initialize();

  expect(store.getState().listStatus).toBe("error");
  expect(store.getState().settings).toEqual(api.settings);
  expect(store.getState().activeWorkspace).toEqual(automaticWorkspace);
  expect(api.chooseCalls).toBe(0);
});

test("new_chat_restores_effective_default_without_picker", async () => {
  const api = new FakeApi();
  api.conversations = [first];
  api.loads.set(first.id, async () => savedConversation(first));
  const store = new ChatStore(api);
  await store.initialize();
  await store.selectConversation(first.id);

  await store.newChat();

  expect(store.getState().activeWorkspace).toEqual(automaticWorkspace);
  expect(api.chooseCalls).toBe(0);
  expect(store.getState().activeConversationId).toBeNull();
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

test("legacy_conversation_uses_default_until_successful_assignment", async () => {
  const api = new FakeApi();
  const legacy = { ...first, workspace: null };
  api.conversations = [legacy];
  api.loads.set(legacy.id, async () => savedConversation(legacy));
  const assigned = { ...legacy, workspace: automaticWorkspace, messageCount: 4 };
  api.asks.push(async () => result(assigned, "next answer", false));
  const store = new ChatStore(api);
  await store.initialize();
  await store.selectConversation(legacy.id);
  expect(store.getState().activeWorkspace).toEqual(automaticWorkspace);
  store.setDraft("continue");

  await store.send();

  expect(api.askCalls).toEqual([
    { prompt: "continue", conversationId: legacy.id, workspacePath: null },
  ]);
  expect(store.getState().activeWorkspace).toEqual(automaticWorkspace);
  expect(store.getState().conversations[0]).toEqual(assigned);
});

test("late_settings_do_not_override_a_selected_workspace", async () => {
  const api = new FakeApi();
  const loadingSettings = deferred<SettingsView>();
  api.loadSettingsResult = () => loadingSettings.promise;
  api.conversations = [first];
  api.loads.set(first.id, async () => savedConversation(first));
  const store = new ChatStore(api);

  const initializing = store.initialize();
  await Promise.resolve();
  await store.selectConversation(first.id);
  loadingSettings.resolve(settings);
  await initializing;

  expect(store.getState().activeConversationId).toBe(first.id);
  expect(store.getState().activeWorkspace).toEqual(workspace);
});

test("delayed_settings_supply_the_default_to_a_selected_legacy_conversation", async () => {
  const api = new FakeApi();
  const loadingSettings = deferred<SettingsView>();
  api.loadSettingsResult = () => loadingSettings.promise;
  const legacy = { ...first, workspace: null };
  api.conversations = [legacy];
  api.loads.set(legacy.id, async () => savedConversation(legacy));
  const store = new ChatStore(api);

  const initializing = store.initialize();
  await Promise.resolve();
  await store.selectConversation(legacy.id);
  expect(store.getState().activeWorkspace).toBeNull();
  loadingSettings.resolve(settings);
  await initializing;

  expect(store.getState().activeConversationId).toBe(legacy.id);
  expect(store.getState().activeWorkspace).toEqual(automaticWorkspace);
});

test("renaming_a_legacy_conversation_preserves_its_effective_default", async () => {
  const api = new FakeApi();
  const legacy = { ...first, workspace: null };
  api.conversations = [legacy];
  api.loads.set(legacy.id, async () => savedConversation(legacy));
  api.rename = async (_id, title) => ({ ...legacy, title });
  const store = new ChatStore(api);
  await store.initialize();
  await store.selectConversation(legacy.id);

  await store.renameActive("Renamed legacy");

  expect(store.getState().activeWorkspace).toEqual(automaticWorkspace);
  expect(store.getState().conversations[0].workspace).toBeNull();
});

test("deleting_or_resetting_keeps_saved_conversation_workspace_authoritative", async () => {
  const api = new FakeApi();
  api.settings = {
    ...settings,
    defaultWorkspace: automaticWorkspace,
    defaultWorkspaceAutomatic: false,
  };
  api.conversations = [first];
  api.loads.set(first.id, async () => savedConversation(first));
  api.save = async () => settings;
  const store = new ChatStore(api);
  await store.initialize();
  await store.selectConversation(first.id);
  store.openSettings();
  store.updateSettingsDraft({ defaultWorkspace: null });

  await store.saveSettings();

  expect(store.getState().activeConversationId).toBe(first.id);
  expect(store.getState().activeWorkspace).toEqual(workspace);
  expect(api.savedSettings.at(-1)?.defaultWorkspacePath).toBeNull();

  await store.deleteActive();
  expect(store.getState().activeWorkspace).toEqual(automaticWorkspace);
});

test("saving_a_new_default_refreshes_an_active_legacy_conversation", async () => {
  const api = new FakeApi();
  const legacy = { ...first, workspace: null };
  const customDefault = { path: "/work/custom", name: "custom" };
  api.conversations = [legacy];
  api.loads.set(legacy.id, async () => savedConversation(legacy));
  api.save = async () => ({
    ...settings,
    defaultWorkspace: customDefault,
    defaultWorkspaceAutomatic: false,
  });
  const store = new ChatStore(api);
  await store.initialize();
  await store.selectConversation(legacy.id);
  store.openSettings();
  store.updateSettingsDraft({ defaultWorkspace: customDefault });

  await store.saveSettings();

  expect(store.getState().activeConversationId).toBe(legacy.id);
  expect(store.getState().activeWorkspace).toEqual(customDefault);
});

test("send stays blocked for the full workspace assignment", async () => {
  const api = new FakeApi();
  api.conversations = [first];
  api.loads.set(first.id, async () => savedConversation(first));
  api.chosenWorkspace = second.workspace!.path;
  const assignment = deferred<ConversationMetadata>();
  api.assign = () => assignment.promise;
  const store = new ChatStore(api);
  await store.initialize();
  await store.selectConversation(first.id);
  store.setDraft("run in the selected folder");

  const changing = store.changeActiveWorkspace();
  expect(store.getState().workspaceBusy).toBe(true);
  await store.send();

  expect(api.askCalls).toEqual([]);
  assignment.resolve({ ...first, workspace: second.workspace });
  await changing;
  expect(store.getState().workspaceBusy).toBe(false);
  expect(store.getState().activeWorkspace).toEqual(second.workspace);
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

  const returned = {
    ...settings,
    apiKeySource: "settings" as const,
    model: "edited-model",
    activeModel: "edited-model",
  };
  api.save = async () => returned;
  await store.saveSettings();
  expect(store.getState().screen).toBe("chat");
  expect(store.getState().settings).toEqual(returned);
  expect(store.getState().settingsDraft).toBeNull();
});

test("successful settings save loads available models for the chat picker", async () => {
  const api = new FakeApi();
  const store = new ChatStore(api);
  await store.initialize();
  store.openSettings();
  api.save = async () => ({ ...settings, apiKeySource: "settings" as const });

  await store.saveSettings();

  expect(api.listModelCalls).toBe(1);
  expect(store.getState().availableModels).toEqual(api.models);
  expect(store.getState().modelListStatus).toBe("idle");
});

test("selecting a chat model saves it globally before applying it", async () => {
  const api = new FakeApi();
  api.settings = { ...settings, apiKeySource: "settings" };
  api.save = async (input) => ({
    ...api.settings,
    model: input.model,
    activeModel: input.model,
  });
  const store = new ChatStore(api);
  await store.initialize();

  await store.selectModel("deepseek-v4-pro");

  expect(api.selectedModels).toEqual(["deepseek-v4-pro"]);
  expect(api.savedSettings).toEqual([]);
  expect(store.getState().settings?.model).toBe("deepseek-v4-pro");
  expect(store.getState().settings?.activeModel).toBe("deepseek-v4-pro");
});

test("model discovery failure keeps the configured model and can retry", async () => {
  const api = new FakeApi();
  api.settings = { ...settings, apiKeySource: "settings" };
  let failing = true;
  api.listModels = async () => {
    api.listModelCalls += 1;
    if (failing) throw new Error("model catalog unavailable");
    return api.models;
  };
  const store = new ChatStore(api);

  await store.initialize();
  expect(store.getState().modelListStatus).toBe("error");
  expect(store.getState().modelListErrorMessage).toBe("model catalog unavailable");
  expect(store.getState().modelListRetryAvailable).toBe(true);
  expect(store.getState().availableModels).toEqual([
    { id: "deepseek-flash", name: "deepseek-flash" },
  ]);

  failing = false;
  await store.retryModels();
  expect(api.listModelCalls).toBe(2);
  expect(store.getState().availableModels).toEqual(api.models);
  expect(store.getState().modelListErrorMessage).toBeNull();
  expect(store.getState().modelListRetryAvailable).toBe(false);
});

test("model save failure reports the error without offering discovery retry", async () => {
  const api = new FakeApi();
  api.settings = { ...settings, apiKeySource: "settings" };
  api.setModel = async () => {
    throw new Error("model save failed");
  };
  const store = new ChatStore(api);
  await store.initialize();

  await store.selectModel("deepseek-v4-pro");

  expect(store.getState().modelListErrorMessage).toBe("model save failed");
  expect(store.getState().modelListRetryAvailable).toBe(false);
});

test("sending waits until a selected model has been persisted", async () => {
  const api = new FakeApi();
  api.settings = { ...settings, apiKeySource: "settings" };
  const pending = deferred<SettingsView>();
  api.setModel = async (model) => {
    api.selectedModels.push(model);
    return pending.promise;
  };
  const store = new ChatStore(api);
  await store.initialize();
  const selecting = store.selectModel("deepseek-v4-pro");
  store.setDraft("use the new model");

  await store.send();

  expect(api.askCalls).toEqual([]);
  expect(store.getState().draftText).toBe("use the new model");
  pending.resolve({
    ...api.settings,
    model: "deepseek-v4-pro",
    activeModel: "deepseek-v4-pro",
  });
  await selecting;
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
