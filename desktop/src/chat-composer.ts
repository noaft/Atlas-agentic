import { createSlashMenu } from "./slash-menu";
import { createContextMeter, type ContextMeter } from "./context-meter";
import type { ChatStore } from "./chat-state";
import type { ChatState } from "./types";

export interface ChatComposer {
  element: HTMLFormElement;
  textarea: HTMLTextAreaElement;
  modelHost: HTMLElement;
  permissionHost: HTMLElement;
  feedbackHost: HTMLElement;
  contextMeter: ContextMeter;
  sendButton: HTMLButtonElement;
  updateState(state: Readonly<ChatState>): void;
  replaceDraft(text: string): void;
  refreshLayout(): void;
  focus(): void;
  destroy(): void;
}

function chatIsBusy(state: Readonly<ChatState>) {
  return (
    state.status === "streaming" ||
    state.status === "loading" ||
    state.workspaceBusy ||
    state.permissionBusy ||
    state.modelListStatus === "saving"
  );
}

function resize(textarea: HTMLTextAreaElement) {
  textarea.style.height = "0px";
  textarea.style.height = `${textarea.scrollHeight}px`;
}

export function createChatComposer(
  store: ChatStore,
  onSubmit: () => Promise<void>,
): ChatComposer {
  const element = document.createElement("form");
  element.className = "composer-wrap";
  element.dataset.composerForm = "";
  element.innerHTML = `
    <div class="composer-box">
      <textarea rows="1" aria-label="Message Atlas" data-composer></textarea>
      <div class="composer-controls" data-composer-controls>
        <div class="composer-pickers"><div class="model-picker" data-model-picker></div><div class="permission-picker" data-permission-picker></div></div>
        <div class="composer-actions" data-composer-actions><span data-context-meter-host></span><button type="submit" class="send-button" aria-label="Send message" data-send>↑</button></div>
      </div>
    </div>
    <div data-model-feedback-host></div>
    <p class="composer-note" data-composer-note></p>`;

  const textarea = element.querySelector<HTMLTextAreaElement>("[data-composer]")!;
  const modelHost = element.querySelector<HTMLElement>("[data-model-picker]")!;
  const permissionHost = element.querySelector<HTMLElement>("[data-permission-picker]")!;
  const feedbackHost = element.querySelector<HTMLElement>("[data-model-feedback-host]")!;
  const sendButton = element.querySelector<HTMLButtonElement>("[data-send]")!;
  const note = element.querySelector<HTMLElement>("[data-composer-note]")!;
  const contextMeter = createContextMeter();
  contextMeter.element.hidden = true;
  element.querySelector("[data-context-meter-host]")!.append(contextMeter.element);

  const updateSendAvailability = (state: Readonly<ChatState>) => {
    sendButton.disabled =
      chatIsBusy(state) || !state.historyLoaded || !state.activeWorkspace || !textarea.value.trim();
  };
  const refreshLayout = () => {
    if (!element.isConnected || element.closest("[hidden]")) return;
    resize(textarea);
  };

  const menuHost = document.createElement("div");
  element.append(menuHost);
  const slashMenu = createSlashMenu(textarea, menuHost, (command) => {
    store.setDraft(command);
    refreshLayout();
    updateSendAvailability(store.getState());
  }, () => { void store.retryTasks(); });
  const updateMenu = (state: Readonly<ChatState>) => slashMenu.update({
    tasks: state.tasks, status: state.taskCatalogStatus, error: state.taskCatalogError,
    state, sessionId: state.activeSessionId,
  });
  updateMenu(store.getState());

  textarea.addEventListener("input", () => {
    refreshLayout();
    store.setDraft(textarea.value);
    updateSendAvailability(store.getState());
  });
  textarea.addEventListener("keydown", (event) => {
    if (slashMenu.handleKeydown(event)) return;
    if (
      event.key !== "Enter" ||
      event.shiftKey ||
      event.isComposing ||
      event.keyCode === 229
    )
      return;
    event.preventDefault();
    element.requestSubmit();
  });
  element.addEventListener("submit", (event) => {
    event.preventDefault();
    const state = store.getState();
    if (
      chatIsBusy(state) ||
      !state.historyLoaded ||
      !state.activeWorkspace ||
      !textarea.value.trim()
    )
      return;
    void onSubmit();
  });
  const resizeOnWindowChange = () => refreshLayout();
  window.addEventListener("resize", resizeOnWindowChange);

  return {
    element,
    textarea,
    modelHost,
    permissionHost,
    feedbackHost,
    contextMeter,
    sendButton,
    updateState(state) {
      updateMenu(state);
      textarea.placeholder = state.activeWorkspace
        ? "Message Atlas…"
        : "Starting folder unavailable";
      updateSendAvailability(state);
      note.textContent = state.activeWorkspace
        ? `Commands use ${state.activeWorkspace.path} as their starting folder and can access other locations allowed by your OS account.`
        : "Atlas needs an accessible starting folder before it can run tools.";
    },
    replaceDraft(text) {
      textarea.value = text;
      updateMenu(store.getState());
      refreshLayout();
      updateSendAvailability(store.getState());
    },
    refreshLayout() {
      refreshLayout();
    },
    focus() {
      textarea.focus();
      textarea.setSelectionRange(textarea.value.length, textarea.value.length);
    },
    destroy() {
      slashMenu.destroy();
      contextMeter.destroy();
      window.removeEventListener("resize", resizeOnWindowChange);
    },
  };
}
