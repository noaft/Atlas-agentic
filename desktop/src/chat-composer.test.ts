import { EmptySkillsApi } from "./skills-api.test-support";
import { taskFixtures } from "./task-fixtures.test-support";
import { afterEach, expect, test, vi } from "vitest";

import type { AtlasApi } from "./atlas-api";
import { createChatComposer } from "./chat-composer";
import { ChatStore } from "./chat-state";
import type {
  Conversation,
  ConversationAskResult,
  ConversationMetadata,
  SaveSettingsInput,
  SettingsView,
  ChatEvent,
  PermissionMode,
} from "./types";

const workspace = { path: "/work/atlas", name: "atlas" };
const settings: SettingsView = {
  apiKeySource: "missing",
  baseUrl: "https://api.deepseek.com",
  model: "deepseek-flash",
  activeModel: "deepseek-flash",
  historyLimit: 40,
  defaultWorkspace: workspace,
  defaultWorkspaceAutomatic: false,
  defaultWorkspaceError: null,
  defaultPermissionMode: "ask",
  environmentOverrides: [],
};

class ComposerApi extends EmptySkillsApi implements AtlasApi {
  listMcpConnections = async () => [];
  connectMcpServer = async (_server: string) => {};
  disconnectMcpServer = async (_server: string) => {};
  cancelMcpConnection = async (_server: string) => {};
  async listTasks() { return taskFixtures; }
  async runTask(taskId: string, _args: Record<string, unknown>, id: string | null, onEvent: (event: ChatEvent) => void): Promise<import("./types").TaskResult> {
    return {taskId, outcome: {kind: "compaction", result: await this.compactConversation(id!, onEvent)}};
  }
  async cancelCompaction(): Promise<void> {}
  async compactConversation(_id: string, _onEvent: (event: import("./types").ChatEvent) => void): Promise<import("./types").CompactionResult> { return { memory: null, message: "No older messages" }; }
  async listConversations() {
    return [];
  }
  async loadConversation(_id: string): Promise<Conversation> {
    throw new Error("not used");
  }
  async ask(
    _prompt: string,
    _conversationId: string | null,
    _workspacePath: string | null,
    _permissionMode: PermissionMode,
    _onEvent: (event: ChatEvent) => void,
  ): Promise<ConversationAskResult> {
    throw new Error("not used");
  }
  async renameConversation(
    _id: string,
    _title: string,
  ): Promise<ConversationMetadata> {
    throw new Error("not used");
  }
  async deleteConversation(_id: string) {}
  async loadSettings() {
    return settings;
  }
  async saveSettings(_input: SaveSettingsInput) {
    return settings;
  }
  async listModels() {
    return [];
  }
  async setModel(_model: string) {
    return settings;
  }
  async chooseWorkspace() {
    return workspace.path;
  }
  async validateWorkspace(_path: string) {
    return workspace;
  }
  async assignConversationWorkspace(
    _id: string,
    _path: string,
  ): Promise<ConversationMetadata> {
    throw new Error("not used");
  }
  async setConversationPermissionMode(
    _id: string,
    _permissionMode: PermissionMode,
  ): Promise<ConversationMetadata> {
    throw new Error("not used");
  }
  async resolveToolApproval(_id: string, _allow: boolean) {}
  async cancelToolApprovals() {}
  async toggleFullscreen() {
    return true;
  }
}

const mounted: Array<() => void> = [];

async function setup() {
  const store = new ChatStore(new ComposerApi());
  await store.initialize();
  const submit = vi.fn(async () => {});
  const runTask = vi.fn(async () => {});
  const composer = createChatComposer(store, submit, runTask);
  document.body.append(composer.element);
  mounted.push(composer.destroy);
  composer.updateState(store.getState());
  return { composer, store, submit, runTask };
}

afterEach(() => {
  for (const destroy of mounted.splice(0)) destroy();
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

test("native input updates the draft without transformation", async () => {
  const { composer, store } = await setup();

  composer.textarea.value = "á";
  composer.textarea.dispatchEvent(
    new InputEvent("input", { bubbles: true, data: "á", inputType: "insertText" }),
  );

  expect(composer.textarea.value).toBe("á");
  expect(store.getState().draftText).toBe("á");
});

test("state updates never mutate the live textarea", async () => {
  const { composer, store } = await setup();
  const textarea = composer.textarea;
  const parent = textarea.parentElement;
  textarea.value = "tiếng Việt";
  textarea.focus();
  textarea.setSelectionRange(5, 5);
  textarea.dispatchEvent(new Event("input", { bubbles: true }));
  const focus = vi.spyOn(textarea, "focus");
  const blur = vi.spyOn(textarea, "blur");

  composer.updateState({ ...store.getState(), status: "loading", draftText: "stale" });
  composer.updateState({ ...store.getState(), modelListStatus: "saving" });

  expect(composer.textarea).toBe(textarea);
  expect(textarea.parentElement).toBe(parent);
  expect(textarea.isConnected).toBe(true);
  expect(textarea.value).toBe("tiếng Việt");
  expect(textarea.selectionStart).toBe(5);
  expect(document.activeElement).toBe(textarea);
  expect(textarea.disabled).toBe(false);
  expect(focus).not.toHaveBeenCalled();
  expect(blur).not.toHaveBeenCalled();
});

test("composer stays editable while send availability follows state", async () => {
  const { composer, store } = await setup();
  composer.textarea.value = "xin chào";
  composer.textarea.dispatchEvent(new Event("input", { bubbles: true }));

  composer.updateState(store.getState());
  expect(composer.textarea.disabled).toBe(false);
  expect(composer.sendButton.disabled).toBe(false);

  composer.updateState({ ...store.getState(), status: "streaming" });
  expect(composer.textarea.disabled).toBe(false);
  expect(composer.sendButton.disabled).toBe(true);

  composer.updateState({ ...store.getState(), activeWorkspace: null });
  expect(composer.textarea.disabled).toBe(false);
  expect(composer.sendButton.disabled).toBe(true);
});

test("explicit draft replacement refreshes send availability", async () => {
  const { composer, store } = await setup();

  composer.replaceDraft("xin chào");

  expect(composer.sendButton.disabled).toBe(false);
  composer.replaceDraft("");
  expect(composer.sendButton.disabled).toBe(true);
  expect(store.getState().activeWorkspace).toEqual(workspace);
});

test("composition keys never submit", async () => {
  const { composer, submit } = await setup();
  composer.textarea.value = "đang gõ";
  composer.textarea.dispatchEvent(new Event("input", { bubbles: true }));

  composer.textarea.dispatchEvent(
    new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
  );
  await Promise.resolve();
  expect(submit).toHaveBeenCalledTimes(1);
  submit.mockClear();

  composer.textarea.dispatchEvent(
    new KeyboardEvent("keydown", {
      key: "Enter",
      isComposing: true,
      bubbles: true,
    }),
  );
  composer.textarea.dispatchEvent(
    new KeyboardEvent("keydown", {
      key: "Enter",
      keyCode: 229,
      bubbles: true,
    }),
  );

  expect(submit).not.toHaveBeenCalled();
});

test("Enter submits and Shift Enter remains native", async () => {
  const { composer, submit } = await setup();
  composer.textarea.value = "xin chào";
  composer.textarea.dispatchEvent(new Event("input", { bubbles: true }));

  const newline = new KeyboardEvent("keydown", {
    key: "Enter",
    shiftKey: true,
    bubbles: true,
    cancelable: true,
  });
  composer.textarea.dispatchEvent(newline);
  expect(newline.defaultPrevented).toBe(false);
  expect(submit).not.toHaveBeenCalled();

  const send = new KeyboardEvent("keydown", {
    key: "Enter",
    bubbles: true,
    cancelable: true,
  });
  composer.textarea.dispatchEvent(send);
  await Promise.resolve();
  expect(send.defaultPrevented).toBe(true);
  expect(submit).toHaveBeenCalledTimes(1);
});

test("destroy removes window listeners", async () => {
  const measuredHeight = vi
    .spyOn(HTMLTextAreaElement.prototype, "scrollHeight", "get")
    .mockReturnValue(24);
  const { composer } = await setup();
  composer.textarea.dispatchEvent(new Event("input", { bubbles: true }));
  expect(composer.textarea.style.height).toBe("24px");

  measuredHeight.mockReturnValue(80);
  window.dispatchEvent(new Event("resize"));
  expect(composer.textarea.style.height).toBe("80px");

  composer.destroy();
  measuredHeight.mockReturnValue(32);
  window.dispatchEvent(new Event("resize"));
  expect(composer.textarea.style.height).toBe("80px");
});

test("Enter selects the task directly without inserting a command or submitting chat", async () => {
  const {composer, store, submit, runTask} = await setup();
  const state = {...store.getState(), activeConversationId: "saved", settings: {...settings, apiKeySource: "settings" as const}};
  composer.updateState(state);
  composer.textarea.value = "/"; composer.textarea.focus();
  composer.textarea.dispatchEvent(new Event("input", {bubbles: true})); composer.updateState(state);
  composer.textarea.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", bubbles: true, cancelable: true}));
  expect(runTask).toHaveBeenCalledExactlyOnceWith(taskFixtures[0]);
  expect(submit).not.toHaveBeenCalled(); expect(composer.textarea.value).toBe("/");
});

test("Vietnamese replacement input never collapses the live textarea", async () => {
  const {composer, store} = await setup();
  const height = vi.spyOn(composer.textarea.style, "height", "set");
  for (const [value, inputType, data] of [
    ["tieng", "insertText", "g"],
    ["", "deleteContentBackward", null],
    ["tiếng", "insertReplacementText", "tiếng"],
    ["tiếng Việt", "insertFromPaste", " Việt"],
  ] as const) {
    composer.textarea.value = value;
    composer.textarea.dispatchEvent(new InputEvent("input", {bubbles: true, inputType, data}));
    expect(composer.textarea.value).toBe(value);
    expect(store.getState().draftText).toBe(value);
  }
  expect(height).not.toHaveBeenCalledWith("0px");
});

test("composition input and window resize leave live geometry untouched until composition ends", async () => {
  vi.spyOn(HTMLTextAreaElement.prototype, "scrollHeight", "get").mockReturnValue(50);
  const {composer, store} = await setup();
  const height = vi.spyOn(composer.textarea.style, "height", "set");
  composer.textarea.dispatchEvent(new CompositionEvent("compositionstart", {bubbles: true}));
  composer.textarea.value = "tiế";
  composer.textarea.dispatchEvent(new InputEvent("input", {bubbles: true, inputType: "insertCompositionText", data: "tiế", isComposing: true}));
  window.dispatchEvent(new Event("resize"));
  expect(height).not.toHaveBeenCalled();
  expect(store.getState().draftText).toBe("tiế");
  composer.textarea.dispatchEvent(new CompositionEvent("compositionend", {bubbles: true, data: "tiế"}));
  expect(composer.textarea.style.height).toBe("50px");
});

test("composition lifecycle blocks unflagged Enter and form submission until it ends", async () => {
  const {composer, submit} = await setup();
  composer.textarea.value = "tiếng Việt";
  composer.textarea.dispatchEvent(new Event("input", {bubbles: true}));
  composer.textarea.dispatchEvent(new CompositionEvent("compositionstart", {bubbles: true}));
  const enter = new KeyboardEvent("keydown", {key: "Enter", bubbles: true, cancelable: true});
  composer.textarea.dispatchEvent(enter);
  composer.element.dispatchEvent(new Event("submit", {bubbles: true, cancelable: true}));
  expect(submit).not.toHaveBeenCalled();
  expect(enter.defaultPrevented).toBe(false);
  expect(composer.sendButton.disabled).toBe(true);
  composer.textarea.dispatchEvent(new CompositionEvent("compositionend", {bubbles: true, data: "tiếng Việt"}));
  composer.textarea.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", bubbles: true, cancelable: true}));
  expect(submit).toHaveBeenCalledTimes(1);
});

test("an identical explicit draft replacement does not reset native editing", async () => {
  const {composer} = await setup();
  composer.textarea.value = "tiếng Việt"; composer.textarea.focus(); composer.textarea.setSelectionRange(3, 3);
  const value = vi.spyOn(composer.textarea, "value", "set");
  composer.replaceDraft("tiếng Việt");
  expect(value).not.toHaveBeenCalled();
  expect(composer.textarea.selectionStart).toBe(3);
  expect(document.activeElement).toBe(composer.textarea);
});

test("a composition session cannot run a slash task even when key flags are missing", async () => {
  const {composer, store, submit, runTask} = await setup();
  const state = {...store.getState(), activeConversationId: "saved", settings: {...settings, apiKeySource: "settings" as const}};
  composer.updateState(state);
  composer.textarea.value = "/"; composer.textarea.focus();
  composer.textarea.dispatchEvent(new Event("input", {bubbles: true})); composer.updateState(state);
  composer.textarea.dispatchEvent(new CompositionEvent("compositionstart", {bubbles: true}));
  const enter = new KeyboardEvent("keydown", {key: "Enter", bubbles: true, cancelable: true}); composer.textarea.dispatchEvent(enter);
  expect(runTask).not.toHaveBeenCalled();
  expect(submit).not.toHaveBeenCalled();
  expect(enter.defaultPrevented).toBe(false);
  expect(composer.textarea.hasAttribute("aria-activedescendant")).toBe(false);
});

test("an IME cancellation without compositionend cannot block sending after blur", async () => {
  const {composer, submit} = await setup();
  composer.textarea.value = "tiếng Việt"; composer.textarea.focus(); composer.textarea.dispatchEvent(new Event("input", {bubbles: true}));
  composer.textarea.dispatchEvent(new CompositionEvent("compositionstart", {bubbles: true}));
  composer.textarea.blur(); composer.textarea.focus();
  composer.textarea.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", bubbles: true, cancelable: true}));
  expect(submit).toHaveBeenCalledTimes(1);
});

test("switching to an equal-text session clears stale composition without rewriting the draft", async () => {
  const {composer, store, submit} = await setup();
  composer.textarea.value = "tiếng Việt"; composer.textarea.dispatchEvent(new Event("input", {bubbles: true}));
  composer.textarea.dispatchEvent(new CompositionEvent("compositionstart", {bubbles: true}));
  await store.newChat(); store.setDraft("tiếng Việt");
  const value = vi.spyOn(composer.textarea, "value", "set");
  composer.replaceDraft(store.getState().draftText); composer.updateState(store.getState());
  expect(value).not.toHaveBeenCalled();
  expect(composer.sendButton.disabled).toBe(false);
  composer.textarea.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", bubbles: true, cancelable: true}));
  expect(submit).toHaveBeenCalledTimes(1);
});
