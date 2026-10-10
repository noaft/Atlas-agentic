import { afterEach, expect, test, vi } from "vitest";

import type { AtlasApi } from "./atlas-api";
import { ChatStore } from "./chat-state";
import { mountChat } from "./chat-view";
import type {
  Conversation,
  ConversationAskResult,
  ConversationMetadata,
  SaveSettingsInput,
  SettingsView,
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

const metadata: ConversationMetadata = {
  id: "00000000-0000-4000-8000-000000000001",
  title: "Atlas runtime",
  titleAttempted: true,
  createdAt: "2026-10-04T10:00:00Z",
  updatedAt: "2026-10-04T10:00:01Z",
  messageCount: 2,
  workspace,
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
  availableModels = [
    { id: "deepseek-flash", name: "DeepSeek Flash" },
    { id: "deepseek-v4-pro", name: "DeepSeek V4 Pro" },
  ];
  savedSettings: SaveSettingsInput[] = [];
  selectedModels: string[] = [];
  loadSettingsImpl = async () => this.settings;
  chosenWorkspace: string | null = workspace.path;
  fullscreenCalls = 0;

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
    onDelta: (text: string) => void,
  ) {
    this.askCount += 1;
    return this.askImpl(prompt, id, workspacePath, onDelta);
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

  async toggleFullscreen() {
    this.fullscreenCalls += 1;
    return this.fullscreenCalls % 2 === 1;
  }
}

function setup(api = new ViewApi()) {
  const root = document.createElement("div");
  document.body.replaceChildren(root);
  const store = new ChatStore(api);
  const unmount = mountChat(root, store);
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

  expect(composer(root).disabled).toBe(true);
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

afterEach(() => {
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

test("composition fallback enables sending when no final input event arrives", async () => {
  vi.useFakeTimers();
  try {
    const { root, store } = setup();
    await store.chooseWorkspaceForDraft();
    const input = composer(root);
    input.focus();
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    input.value = "á";
    input.setSelectionRange(1, 1);
    input.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "á" }));

    vi.runAllTimers();

    const rendered = composer(root);
    expect(rendered.value).toBe("á");
    expect(document.activeElement).toBe(rendered);
    expect(rendered.selectionStart).toBe(1);
    expect(root.querySelector<HTMLButtonElement>("[data-send]")?.disabled).toBe(false);
    expect(store.getState().draftText).toBe("á");
  } finally {
    vi.useRealTimers();
  }
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
  await flush();

  expect(root.querySelector("[data-message-role='assistant']")?.textContent).toContain(
    "Hello desktop",
  );
  expect(composer(root).disabled).toBe(true);
  expect(root.querySelector<HTMLButtonElement>("[data-new-chat]")?.disabled).toBe(true);
  pending.resolve({ answer: "Hello desktop", metadata, created: true });
  await flush();
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
  await flush();
  const shell = root.querySelector(".atlas-shell");
  const streamingComposer = composer(root);

  emit("**Hello");
  emit("** responsive desktop");

  expect(root.querySelector(".atlas-shell")).toBe(shell);
  expect(composer(root)).toBe(streamingComposer);
  expect(root.querySelector("[data-message-role='assistant']")?.textContent).toContain(
    "Hello responsive desktop",
  );
  expect(root.querySelector("[data-message-role='assistant'] strong")?.textContent).toBe(
    "Hello",
  );
  pending.resolve({ answer: "Hello responsive desktop", metadata, created: true });
  await flush();
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

test("composition defers async renders and preserves Vietnamese text and caret", async () => {
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
  expect(rendered).not.toBe(original);
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
