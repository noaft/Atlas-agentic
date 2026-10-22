import { taskFixtures } from "./task-fixtures.test-support";
import { afterEach, expect, test, vi } from "vitest";

import type { AtlasApi } from "./atlas-api";
import { ChatStore } from "./chat-state";
import { mountChat } from "./chat-view";
import type {
  ModelInfo,
  Conversation,
  ConversationAskResult,
  ConversationMetadata,
  SaveSettingsInput,
  SettingsView,
  ChatEvent,
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

if (!("showModal" in HTMLDialogElement.prototype)) {
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", {
    configurable: true,
    value(this: HTMLDialogElement) {
      this.setAttribute("open", "");
    },
  });
}

if (!("close" in HTMLDialogElement.prototype)) {
  Object.defineProperty(HTMLDialogElement.prototype, "close", {
    configurable: true,
    value(this: HTMLDialogElement) {
      this.removeAttribute("open");
      this.dispatchEvent(new Event("close"));
    },
  });
}

const metadata: ConversationMetadata = {
  id: "00000000-0000-4000-8000-000000000001",
  title: "Atlas runtime",
  titleAttempted: true,
  createdAt: "2026-10-04T10:00:00Z",
  updatedAt: "2026-10-04T10:00:01Z",
  messageCount: 2,
  workspace,
  permissionMode: "ask",
};

const conversation = (title = metadata.title): Conversation => ({
  metadata: { ...metadata, title },
  messages: [
    { role: "user", content: "How does it work?", createdAt: metadata.createdAt },
    { role: "assistant", content: "Through the runtime.", createdAt: metadata.updatedAt },
  ],
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

class ViewApi implements AtlasApi {
  async listTasks() { return taskFixtures; }
  async runTask(taskId: string, _args: Record<string, unknown>, id: string | null, onEvent: (event: ChatEvent) => void): Promise<import("./types").TaskResult> {
    return {taskId, outcome: {kind: "compaction", result: await this.compactConversation(id!, onEvent)}};
  }
  async cancelCompaction(): Promise<void> {}
  async compactConversation(_id: string, _onEvent: (event: import("./types").ChatEvent) => void): Promise<import("./types").CompactionResult> { return { memory: null, message: "No older messages" }; }
  list = Promise.resolve<ConversationMetadata[]>([]);
  load = async (_id: string) => conversation();
  askImpl = async (
    _prompt: string,
    _id: string | null,
    _workspacePath: string | null,
    onDelta: (text: string) => void,
  ): Promise<ConversationAskResult> => {
    onDelta("Done");
    return { answer: "Done", metadata, created: true };
  };
  renameImpl = async (_id: string, title: string) => ({ ...metadata, title });
  deleteImpl = async (_id: string) => {};
  askCount = 0;
  loadCount = 0;
  settings = settings;
  availableModels: ModelInfo[] = [
    { id: "deepseek-flash", name: "DeepSeek Flash" },
    { id: "deepseek-v4-pro", name: "DeepSeek V4 Pro" },
  ];
  savedSettings: SaveSettingsInput[] = [];
  selectedModels: string[] = [];
  loadSettingsImpl = async () => this.settings;
  chosenWorkspace: string | null = workspace.path;
  fullscreenCalls = 0;
  eventSink: ((event: ChatEvent) => void) | null = null;
  approvalResolutions: Array<{ id: string; allow: boolean }> = [];
  approvalCancellations = 0;

  listConversations() {
    return this.list;
  }

  loadConversation(id: string) {
    this.loadCount += 1;
    return this.load(id);
  }

  ask(
    prompt: string,
    id: string | null,
    workspacePath: string | null,
    _permissionMode: PermissionMode,
    onEvent: (event: ChatEvent) => void,
  ) {
    this.askCount += 1;
    this.eventSink = onEvent;
    return this.askImpl(prompt, id, workspacePath, (text) =>
      onEvent({ event: "delta", data: { content: text } }),
    );
  }

  renameConversation(id: string, title: string) {
    return this.renameImpl(id, title);
  }

  deleteConversation(id: string) {
    return this.deleteImpl(id);
  }

  async loadSettings() {
    return this.loadSettingsImpl();
  }

  async saveSettings(input: SaveSettingsInput) {
    this.savedSettings.push(input);
    this.settings = { ...this.settings, model: input.model, activeModel: input.model };
    return this.settings;
  }

  async listModels() {
    return this.availableModels;
  }

  async setModel(model: string) {
    this.selectedModels.push(model);
    this.settings = { ...this.settings, model, activeModel: model };
    return this.settings;
  }

  async chooseWorkspace() {
    return this.chosenWorkspace;
  }

  async validateWorkspace(path: string) {
    return { path, name: path.split("/").at(-1) ?? path };
  }

  async assignConversationWorkspace(id: string, path: string) {
    return { ...metadata, id, workspace: await this.validateWorkspace(path) };
  }

  async setConversationPermissionMode(id: string, permissionMode: PermissionMode) {
    return { ...metadata, id, permissionMode };
  }

  async resolveToolApproval(id: string, allow: boolean) {
    this.approvalResolutions.push({ id, allow });
  }

  async cancelToolApprovals() {
    this.approvalCancellations += 1;
  }

  async toggleFullscreen() {
    this.fullscreenCalls += 1;
    return this.fullscreenCalls % 2 === 1;
  }
}

const mountedViews: Array<() => void> = [];

function setup(api = new ViewApi()) {
  const root = document.createElement("div");
  document.body.replaceChildren(root);
  const store = new ChatStore(api);
  const unmount = mountChat(root, store);
  mountedViews.push(unmount);
  return { api, root, store, unmount };
}

function click(root: HTMLElement, selector: string) {
  const element = root.querySelector<HTMLButtonElement>(selector);
  expect(element, selector).not.toBeNull();
  element!.click();
}

function composer(root: HTMLElement) {
  return root.querySelector<HTMLTextAreaElement>("[data-composer]")!;
}

test("composer permission picker shows all modes and warns for full access", async () => {
  const { root, store } = setup();
  await store.initialize();

  let select = root.querySelector<HTMLSelectElement>("[data-permission-select]")!;
  expect([...select.options].map((option) => option.textContent)).toEqual([
    "Ask",
    "Workspace",
    "Full access",
  ]);
  select.value = "fullAccess";
  select.dispatchEvent(new Event("change", { bubbles: true }));
  await Promise.resolve();
  select = root.querySelector<HTMLSelectElement>("[data-permission-select]")!;
  expect(select.value).toBe("fullAccess");
  expect(root.textContent).toContain("No confirmations");
});

test("approval card exposes keyboard buttons and sends the selected decision", async () => {
  const { api, root, store } = setup();
  const pending = deferred<ConversationAskResult>();
  api.askImpl = () => pending.promise;
  await store.initialize();
  store.setDraft("delete note");
  const sending = store.send();
  await Promise.resolve();

  api.eventSink!({
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
  expect(root.querySelector("[role=alertdialog]")?.textContent).toContain(
    "Allow delete_file?",
  );
  click(root, "[data-deny-approval]");
  await Promise.resolve();
  expect(api.approvalResolutions).toEqual([{ id: "approval-1", allow: false }]);

  pending.resolve({ answer: "Denied", metadata, created: true });
  await sending;
});

test("saved assistant activity renders as an accessible expandable timeline", async () => {
  const api = new ViewApi();
  api.list = Promise.resolve([metadata]);
  api.load = async () => ({
    ...conversation(),
    activities: [
      {
        turnId: "turn-1",
        assistantMessageIndex: 1,
        durationMs: 72_000,
        records: [
          {
            sequence: 0,
            kind: "status",
            tool: null,
            arguments: null,
            outcome: null,
            summary: "Thinking",
            details: null,
            createdAt: metadata.createdAt,
          },
          {
            sequence: 1,
            kind: "toolResult",
            tool: "search_files",
            arguments: '{"query":"ToolEvent"}',
            outcome: "succeeded",
            summary: "Search succeeded",
            details: "Found 14 matches",
            createdAt: metadata.updatedAt,
          },
          {
            sequence: 2,
            kind: "toolResult",
            tool: "read_file",
            arguments: null,
            outcome: "failed",
            summary: "Read failed",
            details: "File missing",
            createdAt: metadata.updatedAt,
          },
          {
            sequence: 3,
            kind: "toolResult",
            tool: "delete_file",
            arguments: null,
            outcome: "denied",
            summary: "Delete denied",
            details: null,
            createdAt: metadata.updatedAt,
          },
          {
            sequence: 4,
            kind: "toolResult",
            tool: "capture_screen",
            arguments: null,
            outcome: "succeeded",
            summary: "Captured primary display (1920x1080)",
            details: null,
            createdAt: metadata.updatedAt,
          },
        ],
      },
    ],
  });
  const { root, store } = setup(api);
  await store.initialize();
  await store.selectConversation(metadata.id);

  const disclosure = root.querySelector<HTMLDetailsElement>("[data-activity]")!;
  const assistant = disclosure.closest<HTMLElement>("[data-message-role='assistant']")!;
  const content = assistant.querySelector<HTMLElement>(".message-content")!;
  expect(disclosure.open).toBe(false);
  expect(
    [...assistant.children].indexOf(disclosure),
  ).toBeLessThan([...assistant.children].indexOf(content));
  expect(disclosure.querySelector("summary")?.textContent).toContain("Activity");
  expect(disclosure.querySelector("summary")?.textContent).toContain(
    "Worked for 1m 12s",
  );
  expect(disclosure.textContent).toContain("Thinking");
  expect(disclosure.textContent).toContain("Succeeded");
  expect(disclosure.textContent).toContain("Failed");
  expect(disclosure.textContent).toContain("Denied");
  expect(disclosure.querySelector("pre")?.textContent).toContain("ToolEvent");
  expect(disclosure.textContent).not.toContain("base64");
  expect(disclosure.getAttribute("aria-label")).toBe("Atlas activity");
});

test("live activity has a status region while approvals stay prominent", async () => {
  const api = new ViewApi();
  const pending = deferred<ConversationAskResult>();
  api.askImpl = async () => pending.promise;
  const { root, store } = setup(api);
  await store.initialize();
  await store.chooseWorkspaceForDraft();
  store.setDraft("search");
  const sending = store.send();
  await flush();

  api.eventSink!({ event: "runtimeStatus", data: { status: "thinking" } });
  api.eventSink!({
    event: "toolRequested",
    data: { sequence: 0, tool: "search_files", arguments: '{"query":"x"}' },
  });
  api.eventSink!({
    event: "approvalRequested",
    data: {
      sequence: 0,
      request: {
        id: "approval-live",
        tool: "search_files",
        arguments: '{"query":"x"}',
        reason: "reads sensitive data",
      },
    },
  });

  expect(root.querySelector("[data-live-activity][role='status']")?.textContent).toContain(
    "Thinking",
  );
  const waitingDisclosure = root.querySelector<HTMLElement>(
    "[data-live-turn-activity]",
  );
  expect(waitingDisclosure?.textContent).toContain("Waiting");
  expect(waitingDisclosure?.querySelector("pre")?.textContent).toContain(
    '"query":"x"',
  );
  expect(root.querySelector("[role='alertdialog']")?.textContent).toContain(
    "Allow search_files?",
  );
  api.eventSink!({
    event: "toolStarted",
    data: { sequence: 0, tool: "search_files" },
  });
  api.eventSink!({
    event: "toolFinished",
    data: {
      sequence: 0,
      tool: "search_files",
      success: true,
      summary: "Search completed",
      details: "Found 3 matches",
    },
  });
  await flushStreaming();
  const finishedDisclosure = root.querySelector<HTMLElement>(
    "[data-live-turn-activity]",
  );
  expect(finishedDisclosure?.textContent).toContain("Succeeded");
  expect(finishedDisclosure?.textContent).toContain("Found 3 matches");
  pending.resolve({ answer: "done", metadata, created: true, activities: [] });
  await sending;
});

function typeRawText(input: HTMLTextAreaElement, text: string) {
  for (const character of text) {
    const start = input.selectionStart ?? input.value.length;
    const end = input.selectionEnd ?? start;
    input.setRangeText(character, start, end, "end");
    input.dispatchEvent(
      new InputEvent("input", {
        bubbles: true,
        data: character,
        inputType: "insertText",
      }),
    );
  }
}

test("chat model picker shows discovered models and saves the selection", async () => {
  const api = new ViewApi();
  api.settings = { ...settings, apiKeySource: "settings" };
  const { root, store } = setup(api);
  await store.initialize();

  const picker = root.querySelector<HTMLSelectElement>("[data-model-select]");
  expect(picker).not.toBeNull();
  expect([...picker!.options].map((option) => option.textContent)).toEqual([
    "DeepSeek Flash",
    "DeepSeek V4 Pro",
  ]);
  expect(picker!.getAttribute("aria-label")).toBe("Model");
  expect(picker!.parentElement?.firstChild).toBe(picker);

  picker!.value = "deepseek-v4-pro";
  picker!.dispatchEvent(new Event("change", { bubbles: true }));
  await new Promise((resolve) => setTimeout(resolve, 0));

  expect(api.selectedModels).toEqual(["deepseek-v4-pro"]);
  expect(store.getState().settings?.model).toBe("deepseek-v4-pro");
});

test("chat composer explains when the model is locked by the environment", async () => {
  const api = new ViewApi();
  api.settings = {
    ...settings,
    apiKeySource: "environment",
    environmentOverrides: ["apiKey", "model"],
  };
  const { root, store } = setup(api);
  await store.initialize();

  const picker = root.querySelector<HTMLSelectElement>("[data-model-select]");
  expect(picker?.disabled).toBe(true);
  expect(root.querySelector("[data-model-status]")?.textContent).toBe(
    "Controlled by DEEPSEEK_MODEL",
  );
});

test("chat composer shows model loading without turning it into an error", async () => {
  const api = new ViewApi();
  api.settings = { ...settings, apiKeySource: "settings" };
  const pending = deferred<typeof api.availableModels>();
  api.listModels = () => pending.promise;
  const { root, store } = setup(api);
  const initializing = store.initialize();
  await flush();

  expect(root.querySelector("[data-model-status]")?.textContent).toBe(
    "Loading models…",
  );
  expect(root.querySelector("[data-model-feedback]")).toBeNull();

  pending.resolve(api.availableModels);
  await initializing;
});

test("model catalog errors render below the composer controls", async () => {
  const api = new ViewApi();
  api.settings = { ...settings, apiKeySource: "settings" };
  api.listModels = async () => {
    throw new Error("A very long model catalog error that must wrap on narrow windows");
  };
  const { root, store } = setup(api);
  await store.initialize();

  const feedback = root.querySelector<HTMLElement>("[data-model-feedback]");
  expect(feedback?.textContent).toContain("A very long model catalog error");
  expect(feedback?.querySelector("[data-retry-models]")).not.toBeNull();
  expect(root.querySelector(".composer-box")?.contains(feedback!)).toBe(false);
});

test("model picker is integrated into the composer controls", async () => {
  const api = new ViewApi();
  api.settings = { ...settings, apiKeySource: "settings" };
  const { root, store } = setup(api);
  await store.initialize();

  const controls = root.querySelector("[data-composer-controls]");
  expect(controls).not.toBeNull();
  expect(controls?.querySelector("[data-model-picker]")).not.toBeNull();
  expect(controls?.querySelector("[data-send]")).not.toBeNull();
});

test("chat controls stay disabled while a model selection is being saved", async () => {
  const api = new ViewApi();
  api.settings = { ...settings, apiKeySource: "settings" };
  const pending = deferred<SettingsView>();
  api.setModel = async (model) => {
    api.selectedModels.push(model);
    return pending.promise;
  };
  const { root, store } = setup(api);
  await store.initialize();
  const picker = root.querySelector<HTMLSelectElement>("[data-model-select]")!;

  picker.value = "deepseek-v4-pro";
  picker.dispatchEvent(new Event("change", { bubbles: true }));

  expect(composer(root).disabled).toBe(false);
  expect(root.querySelector<HTMLButtonElement>("[data-send]")!.disabled).toBe(true);
  expect(root.querySelector<HTMLButtonElement>("[data-open-settings]")!.disabled).toBe(true);
  pending.resolve({
    ...api.settings,
    model: "deepseek-v4-pro",
    activeModel: "deepseek-v4-pro",
  });
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(store.getState().modelListStatus).toBe("idle");
});

function inputComposer(root: HTMLElement, value: string) {
  const input = composer(root);
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  return composer(root);
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));
const flushStreaming = () => new Promise((resolve) => requestAnimationFrame(resolve));

let restoreMoveBefore: (() => void) | null = null;

function observeMoveBefore() {
  const descriptor = Object.getOwnPropertyDescriptor(Element.prototype, "moveBefore");
  const moveBefore = vi.fn(function (
    this: Element,
    node: Node,
    child: Node | null,
  ) {
    this.insertBefore(node, child);
  });
  Object.defineProperty(Element.prototype, "moveBefore", {
    configurable: true,
    value: moveBefore,
  });
  restoreMoveBefore = () => {
    if (descriptor) Object.defineProperty(Element.prototype, "moveBefore", descriptor);
    else Reflect.deleteProperty(Element.prototype, "moveBefore");
  };
  return moveBefore;
}

afterEach(() => {
  for (const unmount of mountedViews.splice(0)) unmount();
  restoreMoveBefore?.();
  restoreMoveBefore = null;
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

test("startup shows loading then saved conversations while keeping a draft", async () => {
  const api = new ViewApi();
  const pending = deferred<ConversationMetadata[]>();
  api.list = pending.promise;
  const { root, store } = setup(api);

  const initializing = store.initialize();
  expect(root.textContent).toContain("Loading conversations");
  expect(root.querySelector("[data-active-title]")?.textContent).toBe("New chat");
  pending.resolve([metadata]);
  await initializing;

  expect(root.textContent).toContain("Atlas runtime");
  expect(root.querySelector("[data-active-title]")?.textContent).toBe("New chat");
});

test("new chat clears locally without a backend create call", async () => {
  const api = new ViewApi();
  api.list = Promise.resolve([metadata]);
  const { root, store } = setup(api);
  await store.initialize();
  inputComposer(root, "unsaved text");

  click(root, "[data-new-chat]");

  expect(api.askCount).toBe(0);
  expect(api.loadCount).toBe(0);
  expect(composer(root).value).toBe("");
  expect(root.querySelector("[data-active-title]")?.textContent).toBe("New chat");
});

test("new chat and conversation selection replace the draft without replacing the composer", async () => {
  const api = new ViewApi();
  api.list = Promise.resolve([metadata]);
  api.load = async () => conversation();
  const { root, store } = setup(api);
  await store.initialize();
  const input = inputComposer(root, "draft for next chat");
  const parent = input.parentElement;

  click(root, "[data-new-chat]");
  expect(composer(root)).toBe(input);
  expect(input.parentElement).toBe(parent);
  expect(input.value).toBe("");

  input.value = "draft kept while selecting";
  input.dispatchEvent(new Event("input", { bubbles: true }));
  click(root, `[data-conversation-id="${metadata.id}"]`);
  await flush();

  expect(composer(root)).toBe(input);
  expect(input.parentElement).toBe(parent);
  expect(input.value).toBe("");
});

test("Settings round trip preserves the live draft", async () => {
  const { root, store } = setup();
  await store.initialize();
  const input = inputComposer(root, "tiếng Việt draft");
  const parent = input.parentElement;

  click(root, "[data-open-settings]");
  root.querySelector<HTMLButtonElement>(".settings-actions .secondary-button")!.click();

  expect(composer(root)).toBe(input);
  expect(input.parentElement).toBe(parent);
  expect(input.value).toBe("tiếng Việt draft");
});

test("successful send clears the live draft", async () => {
  const api = new ViewApi();
  const pending = deferred<ConversationAskResult>();
  api.askImpl = async () => pending.promise;
  const { root, store } = setup(api);
  await store.chooseWorkspaceForDraft();
  const input = inputComposer(root, "xin chào");
  const parent = input.parentElement;

  click(root, "[data-send]");
  await flush();

  expect(composer(root)).toBe(input);
  expect(input.parentElement).toBe(parent);
  expect(input.value).toBe("");

  pending.resolve({ answer: "Done", metadata, created: true });
  await flush();
  expect(input.value).toBe("");
});

test("failed send restores the live draft", async () => {
  const api = new ViewApi();
  const pending = deferred<ConversationAskResult>();
  api.askImpl = async () => pending.promise;
  const { root, store } = setup(api);
  await store.chooseWorkspaceForDraft();
  const input = inputComposer(root, "xin chào");
  const parent = input.parentElement;

  click(root, "[data-send]");
  await flush();
  expect(input.value).toBe("");

  pending.reject(new Error("send failed"));
  await flush();
  expect(composer(root)).toBe(input);
  expect(input.parentElement).toBe(parent);
  expect(input.value).toBe("xin chào");
  expect(root.querySelector<HTMLButtonElement>("[data-send]")!.disabled).toBe(false);
});

test("failed send does not overwrite text entered while streaming", async () => {
  const api = new ViewApi();
  const pending = deferred<ConversationAskResult>();
  api.askImpl = async () => pending.promise;
  const { root, store } = setup(api);
  await store.chooseWorkspaceForDraft();
  const input = inputComposer(root, "original request");

  click(root, "[data-send]");
  await flush();
  input.focus();
  input.value = "new draft typed during streaming";
  input.setSelectionRange(9, 9);
  input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
  input.dispatchEvent(new Event("input", { bubbles: true }));
  pending.reject(new Error("send failed"));
  await flush();

  expect(input.value).toBe("new draft typed during streaming");
  expect(store.getState().draftText).toBe("new draft typed during streaming");
  expect(input.selectionStart).toBe(9);
  expect(document.activeElement).toBe(input);
});

test("conversation selection shows loading then its saved messages", async () => {
  const api = new ViewApi();
  api.list = Promise.resolve([metadata]);
  const pending = deferred<Conversation>();
  api.load = () => pending.promise;
  const { root, store } = setup(api);
  await store.initialize();

  click(root, `[data-conversation-id="${metadata.id}"]`);
  expect(root.textContent).toContain("Loading conversation");
  pending.resolve(conversation());
  await flush();

  expect(root.textContent).toContain("How does it work?");
  expect(root.textContent).toContain("Through the runtime.");
});

test("delayed conversation loading does not overwrite resumed composition", async () => {
  const api = new ViewApi();
  api.list = Promise.resolve([metadata]);
  const pending = deferred<Conversation>();
  api.load = () => pending.promise;
  const { root, store } = setup(api);
  await store.initialize();
  const input = composer(root);

  click(root, `[data-conversation-id="${metadata.id}"]`);
  input.value = "đang soạn tiếng Việt";
  input.setSelectionRange(8, 8);
  input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
  input.dispatchEvent(new Event("input", { bubbles: true }));
  const valueSetter = vi.spyOn(HTMLTextAreaElement.prototype, "value", "set");
  pending.resolve(conversation());
  await flush();

  expect(input.value).toBe("đang soạn tiếng Việt");
  expect(input.selectionStart).toBe(8);
  expect(document.activeElement).toBe(input);
  expect(valueSetter).not.toHaveBeenCalled();
});

test("Enter sends while Shift Enter keeps a newline", async () => {
  const { api, root, store } = setup();
  await store.chooseWorkspaceForDraft();
  let input = inputComposer(root, "hello");

  input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", shiftKey: true, bubbles: true }));
  expect(api.askCount).toBe(0);

  input = composer(root);
  input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  await flush();

  expect(api.askCount).toBe(1);
});

test("Enter confirms IME composition without sending", async () => {
  const { api, root } = setup();
  const input = inputComposer(root, "編集中");

  input.dispatchEvent(
    new KeyboardEvent("keydown", { key: "Enter", isComposing: true, bubbles: true }),
  );
  await flush();

  expect(api.askCount).toBe(0);
});

test("typing preserves the composer's caret position", async () => {
  const { root, store } = setup();
  await store.chooseWorkspaceForDraft();
  const input = composer(root);
  input.focus();
  input.value = "hello";
  input.setSelectionRange(3, 3);
  input.dispatchEvent(new Event("input", { bubbles: true }));

  const rendered = composer(root);
  expect(rendered.value).toBe("hello");
  expect(document.activeElement).toBe(rendered);
  expect(rendered.selectionStart).toBe(3);
  expect(rendered.selectionEnd).toBe(3);
});

test("composer node and parent stay mounted across every background render", async () => {
  observeMoveBefore();
  const api = new ViewApi();
  const listing = deferred<ConversationMetadata[]>();
  api.list = listing.promise;
  const { root, store } = setup(api);
  const initializing = store.initialize();
  await flush();
  const input = composer(root);
  const parent = input.parentElement;
  input.focus();
  input.value = "tiếng Việt";
  input.setSelectionRange(5, 5);
  input.dispatchEvent(new InputEvent("input", { bubbles: true, data: "ệ" }));
  const refocus = vi.spyOn(input, "focus");

  listing.resolve([metadata]);
  await initializing;

  expect(composer(root)).toBe(input);
  expect(input.parentElement).toBe(parent);
  expect(input.isConnected).toBe(true);
  expect(document.activeElement).toBe(input);
  expect(refocus).not.toHaveBeenCalled();
  expect(input.value).toBe("tiếng Việt");
  expect(input.selectionStart).toBe(5);
});

test("background render does not touch focused composer APIs", async () => {
  const moveBefore = observeMoveBefore();
  const api = new ViewApi();
  const listing = deferred<ConversationMetadata[]>();
  api.list = listing.promise;
  const { root, store } = setup(api);
  const initializing = store.initialize();
  const input = composer(root);
  input.focus();
  input.value = "tiếng Việt";
  input.dispatchEvent(new InputEvent("input", { bubbles: true, data: "ệ" }));
  const focus = vi.spyOn(input, "focus");
  const blur = vi.spyOn(input, "blur");
  const value = vi.spyOn(HTMLTextAreaElement.prototype, "value", "set");

  listing.resolve([metadata]);
  await initializing;

  expect(composer(root)).toBe(input);
  expect(focus).not.toHaveBeenCalled();
  expect(blur).not.toHaveBeenCalled();
  expect(value).not.toHaveBeenCalled();
  expect(moveBefore).not.toHaveBeenCalled();
});

test("composed first character survives initialization", async () => {
  const api = new ViewApi();
  const listing = deferred<ConversationMetadata[]>();
  api.list = listing.promise;
  const { root, store } = setup(api);
  const initializing = store.initialize();
  const input = composer(root);
  const parent = input.parentElement;
  input.focus();
  input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
  input.value = "á";
  input.setSelectionRange(1, 1);
  input.dispatchEvent(new InputEvent("input", { bubbles: true, data: "á" }));

  listing.resolve([metadata]);
  await initializing;
  input.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "á" }));
  input.dispatchEvent(new InputEvent("input", { bubbles: true, data: "á" }));

  expect(composer(root)).toBe(input);
  expect(input.parentElement).toBe(parent);
  expect(document.activeElement).toBe(input);
  expect(input.value).toBe("á");
  expect(store.getState().draftText).toBe("á");
  expect(root.textContent).toContain("Atlas runtime");
});

test("composition end keeps the composer mounted for the final Vietnamese input", async () => {
  const { root, store } = setup();
  await store.chooseWorkspaceForDraft();
  const input = composer(root);
  input.focus();
  input.value = "a";
  input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
  input.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "á" }));

  input.value = "á";
  input.dispatchEvent(new InputEvent("input", { bubbles: true, data: "á" }));

  expect(composer(root)).toBe(input);
  expect(input.value).toBe("á");
  expect(store.getState().draftText).toBe("á");
});

test("composer leaves raw Telex unchanged when the system IME is off", async () => {
  const { root, store } = setup();
  await store.chooseWorkspaceForDraft();
  const input = composer(root);
  input.focus();

  typeRawText(input, "tieengs vieetj");

  expect(input.value).toBe("tieengs vieetj");
  expect(store.getState().draftText).toBe("tieengs vieetj");
});

test("composer height follows its content", async () => {
  const measuredHeight = vi
    .spyOn(HTMLTextAreaElement.prototype, "scrollHeight", "get")
    .mockReturnValue(28);
  const { root, store } = setup();
  await store.chooseWorkspaceForDraft();

  measuredHeight.mockReturnValue(96);
  let input = inputComposer(root, "first line\nsecond line\nthird line");
  expect(input.style.height).toBe("96px");

  measuredHeight.mockReturnValue(28);
  input = inputComposer(root, "short");
  expect(input.style.height).toBe("28px");
});

test("composer measures only after it is attached", () => {
  const measuredHeight = vi
    .spyOn(HTMLTextAreaElement.prototype, "scrollHeight", "get")
    .mockImplementation(function (this: HTMLTextAreaElement) {
      return this.isConnected ? 28 : 0;
    });

  const { root } = setup();

  expect(measuredHeight).toHaveBeenCalled();
  expect(composer(root).style.height).toBe("28px");
});

test("hidden settings resize cannot collapse the composer", async () => {
  let measuredHeight = 28;
  vi.spyOn(HTMLTextAreaElement.prototype, "scrollHeight", "get").mockImplementation(
    () => measuredHeight,
  );
  const { root, store } = setup();
  await store.initialize();
  expect(composer(root).style.height).toBe("28px");

  click(root, "[data-open-settings]");
  measuredHeight = 0;
  window.dispatchEvent(new Event("resize"));
  expect(composer(root).style.height).toBe("28px");

  measuredHeight = 72;
  root.querySelector<HTMLButtonElement>(".settings-actions .secondary-button")!.click();
  expect(composer(root).style.height).toBe("72px");
});

test("composer recalculates its height when the window resizes", async () => {
  const measuredHeight = vi
    .spyOn(HTMLTextAreaElement.prototype, "scrollHeight", "get")
    .mockReturnValue(28);
  const { root, store } = setup();
  await store.chooseWorkspaceForDraft();

  measuredHeight.mockReturnValue(72);
  window.dispatchEvent(new Event("resize"));

  expect(composer(root).style.height).toBe("72px");
});

test("streaming renders ordered assistant text and disables mutation controls", async () => {
  const api = new ViewApi();
  const pending = deferred<ConversationAskResult>();
  api.askImpl = async (_prompt, _id, _workspace, onDelta) => {
    onDelta("Hello");
    onDelta(" desktop");
    return pending.promise;
  };
  const { root, store } = setup(api);
  await store.chooseWorkspaceForDraft();
  inputComposer(root, "hello");

  click(root, "[data-send]");
  await flushStreaming();

  expect(root.querySelector("[data-message-role='assistant']")?.textContent).toContain(
    "Hello desktop",
  );
  expect(composer(root).disabled).toBe(false);
  expect(root.querySelector<HTMLButtonElement>("[data-send]")!.disabled).toBe(true);
  expect(root.querySelector<HTMLButtonElement>("[data-new-chat]")?.disabled).toBe(false);
  pending.resolve({ answer: "Hello desktop", metadata, created: true });
  await flushStreaming();
  expect(composer(root).disabled).toBe(false);
});

test("streaming_deltas_update_the_answer_without_rebuilding_the_ui", async () => {
  const api = new ViewApi();
  const pending = deferred<ConversationAskResult>();
  let emit!: (text: string) => void;
  api.askImpl = async (_prompt, _id, _workspace, onDelta) => {
    emit = onDelta;
    return pending.promise;
  };
  const { root, store } = setup(api);
  await store.chooseWorkspaceForDraft();
  inputComposer(root, "hello");
  click(root, "[data-send]");
  await flushStreaming();
  const shell = root.querySelector(".atlas-shell");
  const streamingComposer = composer(root);

  emit("**Hello");
  emit("** responsive desktop");
  await flushStreaming();

  expect(root.querySelector(".atlas-shell")).toBe(shell);
  expect(composer(root)).toBe(streamingComposer);
  expect(root.querySelector("[data-message-role='assistant']")?.textContent).toContain(
    "Hello responsive desktop",
  );
  expect(root.querySelector("[data-message-role='assistant'] strong")?.textContent).toBe(
    "Hello",
  );
  pending.resolve({ answer: "Hello responsive desktop", metadata, created: true });
  await flushStreaming();
});

test("assistant_messages_render_markdown_while_user_messages_stay_literal", async () => {
  const api = new ViewApi();
  api.list = Promise.resolve([metadata]);
  api.load = async () => ({
    metadata,
    messages: [
      { role: "user", content: "# Keep this literal", createdAt: metadata.createdAt },
      {
        role: "assistant",
        content: "## Result\n\n- First\n- **Second**\n\n```bash\necho atlas\n```",
        createdAt: metadata.updatedAt,
      },
    ],
  });
  const { root, store } = setup(api);
  await store.initialize();

  await store.selectConversation(metadata.id);

  const user = root.querySelector("[data-message-role='user']")!;
  const assistant = root.querySelector("[data-message-role='assistant']")!;
  expect(user.querySelector("h1")).toBeNull();
  expect(user.textContent).toContain("# Keep this literal");
  expect(assistant.querySelector("h2")?.textContent).toBe("Result");
  expect(assistant.querySelectorAll("li")).toHaveLength(2);
  expect(assistant.querySelector("strong")?.textContent).toBe("Second");
  expect(assistant.querySelector("pre code")?.textContent).toContain("echo atlas");
});

test("assistant_markdown_removes_unsafe_html_and_link_protocols", async () => {
  const api = new ViewApi();
  api.list = Promise.resolve([metadata]);
  api.load = async () => ({
    metadata,
    messages: [
      {
        role: "assistant",
        content:
          "Safe [link](https://example.com) and [unsafe](javascript:alert(1)).\n\n<img src=x onerror=alert(1)><script>alert(1)</script>",
        createdAt: metadata.updatedAt,
      },
    ],
  });
  const { root, store } = setup(api);
  await store.initialize();

  await store.selectConversation(metadata.id);

  const assistant = root.querySelector("[data-message-role='assistant']")!;
  const links = assistant.querySelectorAll("a");
  expect(links).toHaveLength(2);
  expect(links[0].getAttribute("href")).toBe("https://example.com");
  expect(links[0].getAttribute("rel")).toBe("noopener noreferrer");
  expect(links[1].hasAttribute("href")).toBe(false);
  expect(assistant.querySelector("img")).toBeNull();
  expect(assistant.querySelector("script")).toBeNull();
});

test("rename keeps the dialog on failure and updates title on success", async () => {
  const api = new ViewApi();
  api.list = Promise.resolve([metadata]);
  const { root, store } = setup(api);
  await store.initialize();
  await store.selectConversation(metadata.id);
  const showModal = vi
    .spyOn(HTMLDialogElement.prototype, "showModal")
    .mockImplementation(function (this: HTMLDialogElement) {
      this.setAttribute("open", "");
    });
  api.renameImpl = async () => {
    throw new Error("rename failed");
  };

  click(root, "[data-conversation-menu]");
  click(root, "[data-rename]");
  expect(showModal).toHaveBeenCalledOnce();
  const title = root.querySelector<HTMLInputElement>("[data-rename-input]")!;
  title.value = "  Renamed  ";
  title.dispatchEvent(new Event("input", { bubbles: true }));
  click(root, "[data-save-rename]");
  await flush();
  expect(root.querySelector("[role='dialog']")?.hasAttribute("open")).toBe(true);
  expect(root.textContent).toContain("rename failed");

  api.renameImpl = async (_id, value) => ({ ...metadata, title: value });
  click(root, "[data-save-rename]");
  await flush();
  expect(root.querySelector("[data-active-title]")?.textContent).toBe("Renamed");
});

test("rename closes through the dialog API and Escape does not reopen it", async () => {
  const api = new ViewApi();
  api.list = Promise.resolve([metadata]);
  const { root, store } = setup(api);
  await store.initialize();
  await store.selectConversation(metadata.id);
  const showModal = vi
    .spyOn(HTMLDialogElement.prototype, "showModal")
    .mockImplementation(function (this: HTMLDialogElement) {
      this.setAttribute("open", "");
    });
  const close = vi
    .spyOn(HTMLDialogElement.prototype, "close")
    .mockImplementation(function (this: HTMLDialogElement) {
      this.removeAttribute("open");
      this.dispatchEvent(new Event("close"));
    });

  click(root, "[data-conversation-menu]");
  click(root, "[data-rename]");
  const dialog = root.querySelector<HTMLDialogElement>("[role='dialog']")!;
  dialog.removeAttribute("open");
  dialog.dispatchEvent(new Event("close"));
  click(root, "[data-sidebar-toggle]");

  expect(showModal).toHaveBeenCalledTimes(1);

  click(root, "[data-conversation-menu]");
  click(root, "[data-rename]");
  click(root, "[data-cancel-rename]");
  expect(close).toHaveBeenCalledOnce();
});

test("delete requires confirmation and returns the active view to a draft", async () => {
  const api = new ViewApi();
  api.list = Promise.resolve([metadata]);
  const { root, store } = setup(api);
  await store.initialize();
  await store.selectConversation(metadata.id);
  const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);

  click(root, "[data-conversation-menu]");
  click(root, "[data-delete]");
  expect(root.querySelector("[data-active-title]")?.textContent).toBe("Atlas runtime");

  confirm.mockReturnValue(true);
  click(root, "[data-delete]");
  await flush();
  expect(root.querySelector("[data-active-title]")?.textContent).toBe("New chat");
  expect(root.querySelectorAll("[data-conversation-id]")).toHaveLength(0);
});

test("mobile sidebar toggle and composer focus remain keyboard accessible", async () => {
  vi.stubGlobal("matchMedia", () => ({ matches: true }));
  const { root } = setup();
  const toggle = root.querySelector<HTMLButtonElement>("[data-sidebar-toggle]")!;
  expect(toggle.getAttribute("aria-expanded")).toBe("false");

  toggle.click();
  expect(root.querySelector("[data-sidebar]")?.classList.contains("open")).toBe(true);
  expect(root.querySelector("[data-sidebar-toggle]")?.getAttribute("aria-expanded")).toBe("true");

  click(root, "[data-new-chat]");
  await flush();
  expect(document.activeElement).toBe(composer(root));
});

test("desktop sidebar toggle collapses and restores the sidebar", () => {
  vi.stubGlobal("matchMedia", () => ({ matches: false }));
  const { root } = setup();

  expect(root.querySelector(".atlas-shell")?.classList.contains("sidebar-collapsed")).toBe(
    false,
  );
  expect(root.querySelector("[data-sidebar-toggle]")?.getAttribute("aria-expanded")).toBe(
    "true",
  );

  click(root, "[data-sidebar-toggle]");
  expect(root.querySelector(".atlas-shell")?.classList.contains("sidebar-collapsed")).toBe(
    true,
  );
  expect(root.querySelector("[data-sidebar-toggle]")?.getAttribute("aria-expanded")).toBe(
    "false",
  );
  expect(root.querySelector("[data-sidebar-toggle]")?.getAttribute("aria-label")).toBe(
    "Show sidebar",
  );

  click(root, "[data-sidebar-toggle]");
  expect(root.querySelector(".atlas-shell")?.classList.contains("sidebar-collapsed")).toBe(
    false,
  );
});

test("fullscreen_control_and_f11_toggle_the_native_window", async () => {
  const { api, root } = setup();

  click(root, "[data-toggle-fullscreen]");
  await flush();
  window.dispatchEvent(new KeyboardEvent("keydown", { key: "F11", bubbles: true }));
  await flush();

  expect(api.fullscreenCalls).toBe(2);
  expect(
    root.querySelector("[data-toggle-fullscreen]")?.getAttribute("aria-label"),
  ).toBe("Toggle fullscreen");
});

test("hostile titles and messages render only as literal text", async () => {
  const api = new ViewApi();
  const hostile = "<img src=x onerror='window.hacked=true'>";
  const unsafeMetadata = { ...metadata, title: hostile };
  api.list = Promise.resolve([unsafeMetadata]);
  api.load = async () => ({
    metadata: unsafeMetadata,
    messages: [
      { role: "user", content: hostile, createdAt: metadata.createdAt },
      { role: "assistant", content: hostile, createdAt: metadata.updatedAt },
    ],
  });
  const { root, store } = setup(api);
  await store.initialize();
  await store.selectConversation(metadata.id);

  expect(root.textContent).toContain(hostile);
  expect(root.querySelectorAll("img")).toHaveLength(1);
  expect(root.querySelector("img:not(.brand-mark)")).toBeNull();
});

test("a failed conversation list can be retried from the sidebar", async () => {
  const api = new ViewApi();
  api.list = Promise.reject(new Error("list failed"));
  const { root, store } = setup(api);
  await store.initialize();

  expect(root.textContent).toContain("list failed");
  api.list = Promise.resolve([metadata]);
  click(root, "[data-retry-list]");
  await flush();

  expect(root.textContent).toContain("Atlas runtime");
});

test("a failed settings load is visible and can be retried", async () => {
  const api = new ViewApi();
  api.loadSettingsImpl = async () => {
    throw new Error("settings failed");
  };
  const { root, store } = setup(api);
  await store.initialize();

  expect(root.querySelector("[role='alert']")?.textContent).toContain("settings failed");
  expect(root.querySelector<HTMLButtonElement>("[data-open-settings]")?.disabled).toBe(true);

  api.loadSettingsImpl = async () => settings;
  click(root, "[data-retry-settings]");
  await flush();

  expect(root.querySelector("[data-retry-settings]")).toBeNull();
  expect(root.querySelector<HTMLButtonElement>("[data-open-settings]")?.disabled).toBe(false);
});

test("renders a ready general assistant with an optional starting folder action", async () => {
  const api = new ViewApi();
  const other = {
    ...metadata,
    id: "00000000-0000-4000-8000-000000000002",
    title: "Other workspace",
    workspace: { path: "/work/other", name: "other" },
  };
  api.list = Promise.resolve([metadata, other]);
  const { root, store } = setup(api);
  await store.initialize();

  expect(root.textContent).toContain("Computer assistant");
  expect(root.textContent).toContain("How can I help?");
  expect(root.querySelector("[aria-label='Active workspace']")).not.toBeNull();
  expect(root.querySelector("[data-workspace-action]")).not.toBeNull();
  expect(root.querySelector("[data-open-settings]")?.textContent).toContain("Settings");
  expect(root.textContent).toContain("Atlas");
  expect(root.textContent).toContain("other");
  expect(composer(root).disabled).toBe(false);
  expect(root.querySelector("[data-composer-note]")?.textContent).toContain("starting folder");
  expect(root.querySelector("[data-composer-note]")?.textContent).toContain("other locations");

  await store.selectConversation(metadata.id);
  expect(root.textContent).toContain("/work/atlas");
});

test("cancelled and successful folder choices preserve composer focus", async () => {
  const api = new ViewApi();
  api.chosenWorkspace = null;
  const { root } = setup(api);
  click(root, "[data-choose-workspace]");
  await flush();
  expect(document.activeElement).toBe(composer(root));

  api.chosenWorkspace = workspace.path;
  click(root, "[data-choose-workspace]");
  await flush();
  expect(document.activeElement).toBe(composer(root));
  expect(root.textContent).toContain("/work/atlas");
});

test("mobile sidebar contains workspace and Settings controls", () => {
  vi.stubGlobal("matchMedia", () => ({ matches: true }));
  const { root } = setup();
  click(root, "[data-sidebar-toggle]");

  const sidebar = root.querySelector("[data-sidebar]")!;
  expect(sidebar.classList.contains("open")).toBe(true);
  expect(sidebar.querySelector("[data-workspace-action]")).not.toBeNull();
  expect(sidebar.querySelector("[data-open-settings]")).not.toBeNull();
});

test("composition survives async renders and preserves Vietnamese text and caret", async () => {
  const api = new ViewApi();
  const listing = deferred<ConversationMetadata[]>();
  api.list = listing.promise;
  const { root, store } = setup(api);
  await store.chooseWorkspaceForDraft();
  const original = composer(root);
  original.focus();
  original.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
  original.value = "tiếng Việt";
  original.setSelectionRange(5, 5);
  original.dispatchEvent(new InputEvent("input", { bubbles: true, data: "ệ" }));

  const initializing = store.initialize();
  listing.resolve([metadata]);
  await initializing;
  expect(composer(root)).toBe(original);
  expect(original.value).toBe("tiếng Việt");

  original.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true }));
  original.dispatchEvent(new InputEvent("input", { bubbles: true, data: "ệ" }));
  const rendered = composer(root);
  expect(rendered).toBe(original);
  expect(rendered.value).toBe("tiếng Việt");
  expect(store.getState().draftText).toBe("tiếng Việt");
  expect(rendered.selectionStart).toBe(5);
  expect(document.activeElement).toBe(rendered);
  expect(root.textContent).toContain("Atlas runtime");
});

test("Enter and key code 229 never send during composition", async () => {
  const { api, root, store } = setup();
  await store.chooseWorkspaceForDraft();
  const input = inputComposer(root, "đang gõ");
  input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
  input.dispatchEvent(
    new KeyboardEvent("keydown", {
      key: "Enter",
      isComposing: true,
      keyCode: 229,
      bubbles: true,
    }),
  );
  await flush();
  expect(api.askCount).toBe(0);
});

test("navigation remains available and a local running chat can be reopened", async () => {
  const { api, root, store } = setup();
  const pending = deferred<ConversationAskResult>();
  api.askImpl = () => pending.promise;
  await store.initialize();
  inputComposer(root, "work in background");
  click(root, "[data-send]");
  await flush();
  const owner = store.getState().activeSessionId;
  expect(root.querySelector<HTMLButtonElement>("[data-new-chat]")!.disabled).toBe(false);
  expect(root.querySelector<HTMLButtonElement>("[data-open-settings]")!.disabled).toBe(false);
  click(root, "[data-new-chat]");
  await flush();
  inputComposer(root, "unrelated draft");
  const local = root.querySelector<HTMLButtonElement>(`[data-session-id='${owner}']`)!;
  expect(local.textContent).toContain("Working");
  local.click();
  await flush();
  expect(store.getState().activeSessionId).toBe(owner);
  expect(composer(root).value).toBe("");
  pending.resolve({ answer: "done", metadata, created: true });
  await flush();
  expect(root.querySelectorAll(`[data-conversation-id='${metadata.id}']`)).toHaveLength(1);
});

test("background request events preserve settings inputs and focus", async () => {
  const { api, root, store } = setup();
  const pending = deferred<ConversationAskResult>();
  api.askImpl = () => pending.promise;
  await store.initialize();
  inputComposer(root, "task");
  click(root, "[data-send]");
  click(root, "[data-open-settings]");
  const key = root.querySelector<HTMLInputElement>("[data-settings-api-key]")!;
  expect(key).not.toBeNull();
  key.focus();
  key.value = "draft credential";
  key.dispatchEvent(new Event("input", { bubbles: true }));
  api.eventSink!({ event: "runtimeStatus", data: { status: "thinking" } });
  api.eventSink!({ event: "approvalRequested", data: { sequence: 0,
    request: { id: "wait", tool: "exec_command", arguments: "{}", reason: "command" } } });
  api.eventSink!({ event: "delta", data: { content: "background text" } });
  pending.resolve({ answer: "done", metadata, created: true });
  await flush();
  expect(root.querySelector("[data-settings-api-key]")).toBe(key);
  expect(key.value).toBe("draft credential");
  expect(document.activeElement).toBe(key);
  expect(store.getState().screen).toBe("settings");
});

test("streaming batches text updates and preserves a reader's scroll position", async () => {
  const frames = new Map<number, FrameRequestCallback>();
  let nextFrame = 0;
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    frames.set(++nextFrame, callback);
    return nextFrame;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
  const { api, root, store } = setup();
  const pending = deferred<ConversationAskResult>();
  api.askImpl = () => pending.promise;
  await store.initialize();
  inputComposer(root, "long task");
  click(root, "[data-send]");
  const sidebarButton = root.querySelector("[data-new-chat]");
  const messages = root.querySelector<HTMLElement>("[data-messages]")!;
  Object.defineProperty(messages, "scrollHeight", { configurable: true, value: 2000 });
  Object.defineProperty(messages, "clientHeight", { configurable: true, value: 500 });
  messages.scrollTop = 300;
  for (const content of ["one", " two", " three"]) {
    api.eventSink!({ event: "delta", data: { content } });
  }
  expect(root.querySelector("[data-message-role='assistant']")!.textContent).not.toContain("one two three");
  expect(frames.size).toBe(1);
  const callbacks = [...frames.values()];
  frames.clear();
  callbacks.forEach((callback) => callback(0));
  expect(root.querySelector("[data-message-role='assistant']")!.textContent).toContain("one two three");
  expect(messages.scrollTop).toBe(300);
  expect(root.querySelector("[data-new-chat]")).toBe(sidebarButton);
  api.eventSink!({ event: "toolRequested", data: { sequence: 0, tool: "pwd", arguments: "{}" } });
  const activityFrames = [...frames.values()];
  frames.clear();
  activityFrames.forEach((callback) => callback(1));
  expect(root.querySelector("[data-new-chat]")).toBe(sidebarButton);
  expect(messages.scrollTop).toBe(300);
  pending.resolve({ answer: "one two three", metadata, created: true });
  await flush();
  expect(messages.scrollTop).toBe(300);
  vi.unstubAllGlobals();
});

test("a late send completion cannot replace another chat's composer", async () => {
  const { api, root, store } = setup();
  const pending = deferred<ConversationAskResult>();
  api.askImpl = () => pending.promise;
  await store.initialize();
  inputComposer(root, "first request");
  click(root, "[data-send]");
  await store.newChat();
  inputComposer(root, "keep this text");
  const input = composer(root);
  input.focus();
  input.setSelectionRange(3, 3);
  pending.reject(new Error("background failure"));
  await flush();
  expect(composer(root)).toBe(input);
  expect(input.value).toBe("keep this text");
  expect(document.activeElement).toBe(input);
  expect(input.selectionStart).toBe(3);
  const failed = root.querySelector<HTMLButtonElement>("[data-session-id]")!;
  expect(failed.textContent).toContain("Failed");
});

test("background approvals do not replace another chat's rename input", async () => {
  const api = new ViewApi();
  api.list = Promise.resolve([metadata]);
  const pending = deferred<ConversationAskResult>();
  api.askImpl = () => pending.promise;
  const { root, store } = setup(api);
  await store.initialize();
  inputComposer(root, "background task");
  click(root, "[data-send]");
  const backgroundEvents = api.eventSink!;
  await store.selectConversation(metadata.id);
  click(root, "[data-conversation-menu]");
  click(root, "[data-rename]");
  const input = root.querySelector<HTMLInputElement>("[data-rename-input]")!;
  input.value = "an edited title";
  input.dispatchEvent(new Event("input", { bubbles: true }));
  input.focus();
  input.setSelectionRange(3, 3);
  backgroundEvents({ event: "approvalRequested", data: { sequence: 0,
    request: { id: "background-approval", tool: "exec_command", arguments: "{}", reason: "command" } } });
  expect(root.querySelector("[data-rename-input]")).toBe(input);
  expect(document.activeElement).toBe(input);
  expect(input.selectionStart).toBe(3);
  expect(root.querySelector("[data-session-id]")?.textContent).toContain("Needs approval");
  pending.reject(new Error("approval expired"));
  await flush();
});

test("Settings credential removal and replacement stay consistent while inputs keep focus", async () => {
  const { api, root, store } = setup();
  api.settings = { ...settings, apiKeySource: "settings" };
  await store.initialize();
  click(root, "[data-open-settings]");
  const key = root.querySelector<HTMLInputElement>("[data-settings-api-key]")!;
  const clear = root.querySelector<HTMLInputElement>(".settings-check input")!;
  key.focus();
  key.value = "replacement";
  key.dispatchEvent(new Event("input", { bubbles: true }));
  clear.focus();
  clear.click();
  expect(store.getState().settingsDraft?.apiKey).toBe("");
  expect(key.value).toBe("");
  expect(document.activeElement).toBe(clear);
  key.focus();
  key.value = "another key";
  key.dispatchEvent(new Event("input", { bubbles: true }));
  expect(store.getState().settingsDraft?.clearApiKey).toBe(false);
  expect(clear.checked).toBe(false);
  expect(document.activeElement).toBe(key);
});

test("a failed history load disables sending until the conversation can be reloaded", async () => {
  const { api, root, store } = setup();
  api.list = Promise.resolve([metadata]);
  api.load = async () => { throw new Error("read failed"); };
  await store.initialize();
  await store.selectConversation(metadata.id);
  inputComposer(root, "continue");
  expect(root.querySelector<HTMLButtonElement>("[data-send]")!.disabled).toBe(true);
  expect(root.textContent).toContain("read failed");
  api.load = async () => conversation();
  await store.selectConversation(metadata.id);
  expect(root.querySelector<HTMLButtonElement>("[data-send]")!.disabled).toBe(false);
});


test("model names stay clean while the context ring follows the selected limit", async () => {
  const api = new ViewApi();
  api.settings = { ...settings, apiKeySource: "settings" };
  api.availableModels = [
    { id: "deepseek-flash", name: "Flash", contextWindow: 1048576, maxOutputTokens: 393216 },
    { id: "deepseek-v4-pro", name: "Pro", contextWindow: 128000 },
  ];
  const { root, store } = setup(api);
  await store.initialize();
  const picker = root.querySelector<HTMLSelectElement>("[data-model-select]")!;
  expect([...picker.options].map((option) => option.textContent)).toEqual(["Flash", "Pro"]);
  const ring = root.querySelector<HTMLButtonElement>("[data-context-meter]");
  expect(ring).not.toBeNull();
  expect(root.querySelector("[data-composer-controls]")?.contains(ring)).toBe(true);
  expect(root.querySelector("[data-model-picker]")?.contains(ring)).toBe(false);
  expect(root.querySelector("[data-context-percent]")?.textContent).toBe("—");
  expect(root.querySelector("[data-model-feedback-host] [data-model-context]")).toBeNull();
  expect(root.querySelector("[data-model-context]")?.textContent).toBe("Max context: 1,048,576 tokens (input + output)");
  picker.value = "deepseek-v4-pro";
  picker.dispatchEvent(new Event("change", { bubbles: true }));
  await flush();
  expect(root.querySelector("[data-model-context]")?.textContent).toBe("Max context: 128,000 tokens (input + output)");
});

test("missing model metadata stays unknown and discovery retry supplies the limit", async () => {
  const api = new ViewApi();
  api.settings = { ...settings, apiKeySource: "settings" };
  api.listModels = async () => { throw new Error("catalog unavailable"); };
  const { root, store } = setup(api);
  await store.initialize();
  expect(root.querySelector("[data-model-context]")?.textContent).toBe("Max context: Unknown");
  api.listModels = async () => [{ id: "deepseek-flash", name: "Flash", contextWindow: 64000 }];
  await store.retryModels();
  expect(root.querySelector("[data-model-context]")?.textContent).toContain("64,000");
});

test("invalid or unlisted model context limits are never guessed", async () => {
  const api = new ViewApi();
  api.settings = { ...settings, apiKeySource: "environment", environmentOverrides: ["model"] };
  api.availableModels = [{ id: "deepseek-flash", name: "Flash", contextWindow: 0 }];
  const { root, store } = setup(api);
  await store.initialize();
  expect(root.querySelector("[data-model-context]")?.textContent).toBe("Max context: Unknown");
  api.availableModels = [{ id: "another-model", name: "Another", contextWindow: 128000 }];
  await store.retryModels();
  expect(root.querySelector("[data-model-context]")?.textContent).toBe("Max context: Unknown");
});


test("the context ring opens details without sending and survives streamed updates", async () => {
  const api = new ViewApi();
  api.settings = { ...settings, apiKeySource: "settings" };
  api.availableModels = [{ id: "deepseek-flash", name: "Flash", contextWindow: 128000 }];
  const pending = deferred<ConversationAskResult>();
  api.askImpl = () => pending.promise;
  const { root, store } = setup(api);
  await store.initialize();
  inputComposer(root, "work");
  const ring = root.querySelector<HTMLButtonElement>("[data-context-meter]")!;
  ring.click();
  expect(api.askCount).toBe(0);
  expect(root.querySelector<HTMLElement>("[data-context-tooltip]")!.hidden).toBe(false);
  expect(root.querySelector("[data-context-ratio]")?.textContent).toBe("— / 128,000 tokens");
  click(root, "[data-send]");
  expect(root.querySelector("[data-context-meter]")).toBe(ring);
  api.eventSink!({ event: "delta", data: { content: "working" } });
  await flushStreaming();
  expect(root.querySelector("[data-context-meter]")).toBe(ring);
  pending.resolve({ answer: "done", metadata, created: true });
  await flush();
});

test("the ring shows last request input plus output and ignores another model's usage", async () => {
  const api = new ViewApi();
  api.settings = { ...settings, apiKeySource: "settings", source: {provider:"deepseek",sourceId:"current"} };
  api.availableModels = [
    { id: "deepseek-flash", name: "Flash", contextWindow: 128000 },
    { id: "deepseek-v4-pro", name: "Pro", contextWindow: 128000 },
  ];
  const pending = deferred<ConversationAskResult>();
  api.askImpl = () => pending.promise;
  const { root, store } = setup(api);
  await store.initialize();
  inputComposer(root, "hello");
  click(root, "[data-send]");
  const usage = { source: {provider:"deepseek" as const,sourceId:"current"}, model: "deepseek-flash", inputTokens: 15000, outputTokens: 1000, totalTokens: 16000 };
  api.eventSink!({ event: "usage", data: { usage } });
  expect(root.querySelector("[data-context-percent]")?.textContent).toBe("13%");
  expect(root.querySelector("[data-context-ratio]")?.textContent).toBe("16,000 / 128,000 tokens");
  expect(root.querySelector("[data-context-tooltip]")?.textContent).toContain("Input: 15,000");
  expect(root.querySelector("[data-context-tooltip]")?.textContent).toContain("Output: 1,000");
  pending.resolve({ answer: "done", metadata, created: true, usage });
  await flush();
  await store.selectModel("deepseek-v4-pro");
  expect(root.querySelector("[data-context-percent]")?.textContent).toBe("—");
  await store.selectModel("deepseek-flash");
  expect(root.querySelector("[data-context-percent]")?.textContent).toBe("13%");
});


test("custom composer menus save model and access through visible options", async () => {
  const api = new ViewApi();
  api.settings = { ...settings, apiKeySource: "settings" };
  const { root, store } = setup(api);
  await store.initialize();
  const model = root.querySelector<HTMLButtonElement>('.composer-picker-trigger[data-picker-label="Model"]')!;
  expect(model).not.toBeNull();
  model.click();
  const pro = [...root.querySelectorAll<HTMLElement>('[role="option"]')].find((row) => row.textContent?.includes("DeepSeek V4 Pro"))!;
  pro.click();
  await flush();
  expect(api.selectedModels).toEqual(["deepseek-v4-pro"]);
  const access = root.querySelector<HTMLButtonElement>('.composer-picker-trigger[data-picker-label="Tool access"]')!;
  access.click();
  const workspace = [...root.querySelectorAll<HTMLElement>('[role="option"]')].find((row) => row.textContent?.includes("Workspace"))!;
  workspace.click();
  await flush();
  expect(store.getState().activePermissionMode).toBe("workspace");
});

test("rerendering an open picker restores focus to its trigger", async () => {
  const api = new ViewApi();
  api.settings = { ...settings, apiKeySource: "settings" };
  const { root, store } = setup(api);
  await store.initialize();
  root.querySelector<HTMLButtonElement>('[data-picker-label="Model"]')!.click();
  expect(document.activeElement?.getAttribute("role")).toBe("listbox");
  await store.selectPermissionMode("workspace");
  expect(document.activeElement).toBe(root.querySelector('[data-picker-label="Model"]'));
});

test("the same memory disclosure stays open while the composer changes", async () => {
 const api=new ViewApi();const saved=conversation();
 saved.memory={version:1,conversationId:metadata.id,checkpointId:"saved",createdAt:metadata.createdAt,model:"fake",compactedMessageCount:1,blocks:[{startIndex:0,endIndexExclusive:1,content:"Remember the goal"}]};
 api.load=async()=>saved;
 const {root,store}=setup(api);await store.initialize();await store.selectConversation(metadata.id);
 const details=root.querySelector<HTMLDetailsElement>("[data-memory-checkpoint]")!;
 details.open=true;
 store.setDraft("typing");
 await store.selectPermissionMode("workspace");
 expect(root.querySelector<HTMLDetailsElement>("[data-memory-checkpoint]")?.open).toBe(true);
});

test("automatic compaction exposes cancellation while the answer is pending", async () => {
 const api=new ViewApi();const pending=deferred<ConversationAskResult>();api.askImpl=()=>pending.promise;
 let cancelled=0;api.cancelCompaction=async()=>{cancelled++;};
 const {root,store}=setup(api);await store.initialize();await store.selectConversation(metadata.id);
 store.setDraft("next");const sending=store.send();
 api.eventSink!({event:"compaction",data:{progress:{completedBlocks:0,totalBlocks:3}}});
 const cancel=[...root.querySelectorAll<HTMLButtonElement>("button")].filter((button)=>button.textContent==="Cancel compaction");
 expect(cancel).toHaveLength(1);cancel[0].click();await Promise.resolve();expect(cancelled).toBe(1);
 pending.resolve({answer:"done",metadata,created:false});await sending;
});

test.each(["Enter", "click"])("selecting a task with %s runs immediately and shows progress", async (selection) => {
  const api = new ViewApi(); api.settings = {...settings, apiKeySource: "settings"};
  const pending = deferred<import("./types").TaskResult>();
  let calls = 0; api.runTask = (taskId, args, id, _onEvent) => {
    expect(taskId).toBe("conversation.compact"); expect(args).toEqual({}); expect(id).toBe(metadata.id);
    calls++; return pending.promise;
  };
  const {root, store} = setup(api); await store.initialize(); await store.selectConversation(metadata.id);
  const before = store.getState().messages;
  const textarea = root.querySelector<HTMLTextAreaElement>("[data-composer]")!;
  textarea.value = selection === "click" ? "/SUMMARIZE" : "/"; textarea.focus(); textarea.dispatchEvent(new Event("input", {bubbles: true}));
  expect(root.textContent).toContain("Summarize older context");
  if (selection === "Enter") textarea.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", bubbles: true, cancelable: true}));
  else root.querySelector<HTMLElement>('.slash-menu [role="option"]')!.click();
  expect(calls).toBe(1); expect(textarea.value).toBe(""); expect(api.askCount).toBe(0);
  expect(root.textContent).toContain("Compacting context"); expect(store.getState().messages).toEqual(before);
  pending.resolve({taskId: "conversation.compact", outcome: {kind: "compaction", result: {memory: null, message: "Compacted"}}});
  await flush(); expect(root.textContent).toContain("Compacted");
});

test("Send or Enter after dismissing the menu sends literal slash text to the agent", async () => {
  const api = new ViewApi(); api.settings = {...settings, apiKeySource: "settings"};
  let tasks = 0; api.runTask = async () => {tasks++; throw new Error("must not run");};
  const {root, store} = setup(api); await store.initialize();
  const textarea = root.querySelector<HTMLTextAreaElement>("[data-composer]")!;
  textarea.value = "/compact"; textarea.focus(); textarea.dispatchEvent(new Event("input", {bubbles: true}));
  root.querySelector<HTMLButtonElement>("[data-send]")!.click(); await flush();
  expect(api.askCount).toBe(1); expect(tasks).toBe(0);
  expect(store.getState().messages.some(message => message.role === "user" && message.content === "/compact")).toBe(true);
  textarea.value = "/compact"; textarea.focus(); textarea.dispatchEvent(new Event("input", {bubbles: true}));
  textarea.dispatchEvent(new KeyboardEvent("keydown", {key: "Escape", bubbles: true, cancelable: true}));
  textarea.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", bubbles: true, cancelable: true})); await flush();
  expect(api.askCount).toBe(2); expect(tasks).toBe(0);
});

test("a selected task cannot replace the composer after switching chats", async () => {
  const api = new ViewApi(); api.settings = {...settings, apiKeySource: "settings"};
  const pending = deferred<import("./types").TaskResult>(); api.runTask = () => pending.promise;
  const {root, store} = setup(api); await store.initialize(); await store.selectConversation(metadata.id);
  const textarea = root.querySelector<HTMLTextAreaElement>("[data-composer]")!;
  textarea.value = "/"; textarea.focus(); textarea.dispatchEvent(new Event("input", {bubbles: true}));
  root.querySelector<HTMLElement>('.slash-menu [role="option"]')!.click();
  await store.newChat(); textarea.value = "other chat draft"; textarea.dispatchEvent(new Event("input", {bubbles: true}));
  pending.resolve({taskId: "conversation.compact", outcome: {kind: "compaction", result: {memory: null, message: "Compacted"}}}); await flush();
  expect(textarea.value).toBe("other chat draft"); expect(store.getState().draftText).toBe("other chat draft");
  expect(root.textContent).not.toContain("Compacted");
});

test("failed compaction has a retry card without duplicating the raw error banner", async () => {
  const api = new ViewApi(); api.settings = {...settings, apiKeySource: "settings"};
  let count = 0; const pending = deferred<import("./types").TaskResult>();
  api.runTask = (taskId, args, id) => {
    count++; expect(taskId).toBe("conversation.compact"); expect(args).toEqual({}); expect(id).toBe(metadata.id);
    return count === 1 ? Promise.reject(new Error("Summary reached its 2048-token output limit")) : pending.promise;
  };
  const {root, store} = setup(api); await store.initialize(); await store.selectConversation(metadata.id);
  store.setDraft("/"); await store.runTask(taskFixtures[0], {});
  const card = root.querySelector<HTMLElement>('[data-compaction-status]')!;
  expect(card).not.toBeNull(); expect(card.textContent).toContain("Compaction failed");
  expect(root.querySelector('.error-notice')).toBeNull();
  const textarea = composer(root); textarea.value = "my next message"; textarea.dispatchEvent(new Event("input", {bubbles: true}));
  card.querySelector<HTMLButtonElement>('[data-retry-compaction]')!.click();
  expect(count).toBe(2); expect(root.textContent).toContain("Compacting context");
  expect(textarea.value).toBe("my next message");
  pending.resolve({taskId: "conversation.compact", outcome: {kind: "compaction", result: {memory: null, message: "Compacted"}}}); await flush();
  expect(root.textContent).toContain("Compaction finished"); expect(textarea.value).toBe("my next message");
  expect(store.getState().draftText).toBe("my next message");
});

test("automatic compaction failures show the compaction card and keep the failed prompt", async () => {
  const api = new ViewApi(); const pending = deferred<ConversationAskResult>(); api.askImpl = () => pending.promise;
  const {root, store} = setup(api); await store.initialize(); await store.selectConversation(metadata.id);
  store.setDraft("continue working"); const sending = store.send();
  api.eventSink!({event: "compaction", data: {progress: {completedBlocks: 0, totalBlocks: 3}}});
  pending.reject(new Error("Summary response was empty; original memory retained")); await sending;
  expect(root.querySelector('[data-compaction-status]')?.textContent).toContain("Compaction failed");
  expect(root.querySelector('.error-notice')).toBeNull(); expect(store.getState().draftText).toBe("continue working");
});

test("a background compaction failure and retry remain in their owning chat", async () => {
  const api = new ViewApi(); api.settings = {...settings, apiKeySource: "settings"};
  const pending = deferred<import("./types").TaskResult>(); api.runTask = () => pending.promise;
  const {root, store} = setup(api); await store.initialize(); await store.selectConversation(metadata.id);
  const owner = store.getState().activeSessionId;
  const running = store.runTask(taskFixtures[0], {}); await store.newChat(); store.setDraft("other draft");
  pending.reject(new Error("Summary response was empty")); await running;
  expect(root.querySelector<HTMLElement>('[data-compaction-status]')?.hidden).toBe(true);
  expect(store.getState().draftText).toBe("other draft");
  await store.selectSession(owner);
  expect(root.querySelector('[data-compaction-status]')?.textContent).toContain("Compaction failed");
  expect(root.querySelector('[data-retry-compaction]')).not.toBeNull();
});

test("visible compaction stays in the existing notice grid row", async () => {
  const api = new ViewApi(); const pending = deferred<ConversationAskResult>(); api.askImpl = () => pending.promise;
  const {root, store} = setup(api); await store.initialize(); await store.selectConversation(metadata.id);
  store.setDraft("next"); const sending = store.send();
  api.eventSink!({event: "compaction", data: {progress: {completedBlocks: 0, totalBlocks: 3}}});
  const card = root.querySelector<HTMLElement>('[data-compaction-status]')!;
  const notices = root.querySelector<HTMLElement>('[data-notices]')!;
  expect(card.hidden).toBe(false);
  // display:contents exposes direct children to the four-row main grid.
  // Card and notices must share its one feedback item, leaving messages/composer tracks intact.
  expect(card.parentElement).toBe(notices.parentElement);
  expect(card.parentElement?.classList.contains("notice-area")).toBe(true);
  pending.resolve({answer: "done", metadata, created: false}); await sending;
});

test("same model usage requires matching provider source and legacy unknown is hidden", async () => {
  const api = new ViewApi();
  const source = {provider:"deepseek" as const,sourceId:"current"};
  api.settings = { ...settings, apiKeySource:"settings", source };
  api.availableModels = [{id:"deepseek-flash",name:"Flash",contextWindow:6000}];
  const pending = deferred<ConversationAskResult>(); api.askImpl = () => pending.promise;
  const {root,store} = setup(api); await store.initialize();
  inputComposer(root,"hello"); click(root,"[data-send]");
  const reading={model:"deepseek-flash",inputTokens:1200,outputTokens:300,totalTokens:1500};
  for (const wrong of [undefined,{provider:"openai" as const,sourceId:"another"},{provider:"deepseek" as const,sourceId:"old-endpoint"}]) {
    api.eventSink!({event:"usage",data:{usage:{...reading,source:wrong}}});
    expect(root.querySelector("[data-context-percent]")?.textContent).toBe("—");
  }
  const usage={...reading,source};
  api.eventSink!({event:"usage",data:{usage}});
  expect(root.querySelector("[data-context-percent]")?.textContent).toBe("25%");
  pending.resolve({answer:"done",metadata,created:true,usage}); await flush();
});
