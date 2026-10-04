import { afterEach, expect, test, vi } from "vitest";

import type { AtlasApi } from "./atlas-api";
import { ChatStore } from "./chat-state";
import { mountChat } from "./chat-view";
import type {
  Conversation,
  ConversationAskResult,
  ConversationMetadata,
} from "./types";

const metadata: ConversationMetadata = {
  id: "00000000-0000-4000-8000-000000000001",
  title: "Atlas runtime",
  titleAttempted: true,
  createdAt: "2026-10-04T10:00:00Z",
  updatedAt: "2026-10-04T10:00:01Z",
  messageCount: 2,
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
    onDelta: (text: string) => void,
  ): Promise<ConversationAskResult> => {
    onDelta("Done");
    return { answer: "Done", metadata, created: true };
  };
  renameImpl = async (_id: string, title: string) => ({ ...metadata, title });
  deleteImpl = async (_id: string) => {};
  askCount = 0;
  loadCount = 0;

  listConversations() {
    return this.list;
  }

  loadConversation(id: string) {
    this.loadCount += 1;
    return this.load(id);
  }

  ask(prompt: string, id: string | null, onDelta: (text: string) => void) {
    this.askCount += 1;
    return this.askImpl(prompt, id, onDelta);
  }

  renameConversation(id: string, title: string) {
    return this.renameImpl(id, title);
  }

  deleteConversation(id: string) {
    return this.deleteImpl(id);
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

function inputComposer(root: HTMLElement, value: string) {
  const input = composer(root);
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  return composer(root);
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

afterEach(() => {
  vi.restoreAllMocks();
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
  const { api, root } = setup();
  let input = inputComposer(root, "hello");

  input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", shiftKey: true, bubbles: true }));
  expect(api.askCount).toBe(0);

  input = composer(root);
  input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  await flush();

  expect(api.askCount).toBe(1);
});

test("streaming renders ordered assistant text and disables mutation controls", async () => {
  const api = new ViewApi();
  const pending = deferred<ConversationAskResult>();
  api.askImpl = async (_prompt, _id, onDelta) => {
    onDelta("Hello");
    onDelta(" desktop");
    return pending.promise;
  };
  const { root } = setup(api);
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

test("rename keeps the dialog on failure and updates title on success", async () => {
  const api = new ViewApi();
  api.list = Promise.resolve([metadata]);
  const { root, store } = setup(api);
  await store.initialize();
  await store.selectConversation(metadata.id);
  api.renameImpl = async () => {
    throw new Error("rename failed");
  };

  click(root, "[data-conversation-menu]");
  click(root, "[data-rename]");
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
  const { root } = setup();
  const toggle = root.querySelector<HTMLButtonElement>("[data-sidebar-toggle]")!;
  expect(toggle.getAttribute("aria-expanded")).toBe("false");

  toggle.click();
  expect(root.querySelector("[data-sidebar]")?.classList.contains("open")).toBe(true);
  expect(root.querySelector("[data-sidebar-toggle]")?.getAttribute("aria-expanded")).toBe("true");

  click(root, "[data-new-chat]");
  expect(document.activeElement).toBe(composer(root));
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
  expect(root.querySelector("img")).toBeNull();
});
