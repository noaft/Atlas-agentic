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

class ComposerApi implements AtlasApi {
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
  const composer = createChatComposer(store, submit);
  document.body.append(composer.element);
  mounted.push(composer.destroy);
  composer.updateState(store.getState());
  return { composer, store, submit };
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

test("slash selection inserts then a subsequent Enter submits", async () => {
  const {composer, store, submit} = await setup();
  const state = {...store.getState(), activeConversationId: "saved", settings: {...settings, apiKeySource: "settings" as const}};
  composer.updateState(state);
  composer.textarea.value = "/"; composer.textarea.focus();
  composer.textarea.dispatchEvent(new Event("input", {bubbles: true}));
  composer.updateState(state);
  composer.textarea.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", bubbles: true, cancelable: true}));
  expect(composer.textarea.value).toBe("/compact"); expect(submit).not.toHaveBeenCalled();
  expect(store.getState().draftText).toBe("/compact");
  composer.textarea.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", bubbles: true, cancelable: true}));
  expect(submit).toHaveBeenCalledOnce();
});
