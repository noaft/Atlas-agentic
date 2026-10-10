import type { ChatStore } from "./chat-state";
import type { ChatState } from "./types";

export interface ChatComposer {
  element: HTMLFormElement;
  textarea: HTMLTextAreaElement;
  modelHost: HTMLElement;
  feedbackHost: HTMLElement;
  sendButton: HTMLButtonElement;
  updateState(state: Readonly<ChatState>): void;
  replaceDraft(text: string): void;
  focus(): void;
  destroy(): void;
}

function chatIsBusy(state: Readonly<ChatState>) {
  return (
    state.status === "streaming" ||
    state.status === "loading" ||
    state.workspaceBusy ||
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
        <div class="model-picker" data-model-picker></div>
        <button type="submit" class="send-button" aria-label="Send message" data-send>↑</button>
      </div>
    </div>
    <div data-model-feedback-host></div>
    <p class="composer-note" data-composer-note></p>`;

  const textarea = element.querySelector<HTMLTextAreaElement>("[data-composer]")!;
  const modelHost = element.querySelector<HTMLElement>("[data-model-picker]")!;
  const feedbackHost = element.querySelector<HTMLElement>("[data-model-feedback-host]")!;
  const sendButton = element.querySelector<HTMLButtonElement>("[data-send]")!;
  const note = element.querySelector<HTMLElement>("[data-composer-note]")!;

  textarea.addEventListener("input", () => {
    resize(textarea);
    store.setDraft(textarea.value);
    const state = store.getState();
    sendButton.disabled =
      chatIsBusy(state) || !state.activeWorkspace || !textarea.value.trim();
  });
  textarea.addEventListener("keydown", (event) => {
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
      !state.activeWorkspace ||
      !textarea.value.trim()
    )
      return;
    void onSubmit();
  });
  const resizeOnWindowChange = () => resize(textarea);
  window.addEventListener("resize", resizeOnWindowChange);

  return {
    element,
    textarea,
    modelHost,
    feedbackHost,
    sendButton,
    updateState(state) {
      textarea.placeholder = state.activeWorkspace
        ? "Message Atlas…"
        : "Starting folder unavailable";
      sendButton.disabled =
        chatIsBusy(state) || !state.activeWorkspace || !textarea.value.trim();
      note.textContent = state.activeWorkspace
        ? `Commands use ${state.activeWorkspace.path} as their starting folder and can access other locations allowed by your OS account.`
        : "Atlas needs an accessible starting folder before it can run tools.";
    },
    replaceDraft(text) {
      textarea.value = text;
      resize(textarea);
    },
    focus() {
      textarea.focus();
      textarea.setSelectionRange(textarea.value.length, textarea.value.length);
    },
    destroy() {
      window.removeEventListener("resize", resizeOnWindowChange);
    },
  };
}
