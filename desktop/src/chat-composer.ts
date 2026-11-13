import { createTextareaLayout } from "./textarea-layout";
import { createSlashMenu } from "./slash-menu";
import { createContextMeter, type ContextMeter } from "./context-meter";
import type { ChatStore } from "./chat-state";
import type { ChatState, TaskDescriptor } from "./types";

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
    state.status === "loading" ||
    state.workspaceBusy ||
    state.permissionBusy ||
    state.modelListStatus === "saving"
  );
}

export function createChatComposer(
  store: ChatStore,
  onSubmit: () => Promise<void>,
  onTask: (task: TaskDescriptor) => Promise<void>,
): ChatComposer {
  const element = document.createElement("form");
  element.className = "composer-wrap";
  element.dataset.composerForm = "";
  element.innerHTML = `
    <div class="composer-box">
      <textarea rows="1" aria-label="Message Atlas" data-composer></textarea>
      <div class="composer-controls" data-composer-controls>
        <div class="composer-pickers"><div class="model-picker" data-model-picker></div><div class="permission-picker" data-permission-picker></div></div>
        <div class="composer-actions" data-composer-actions><span data-context-meter-host></span><select class="send-mode" aria-label="When Atlas is working" data-send-mode><option value="wait">Wait until finished</option><option value="steer">Supplement active turn</option></select><button type="button" class="stop-turn" aria-label="Stop response" data-stop-turn hidden>■</button><button type="submit" class="send-button" aria-label="Send message" data-send>↑</button></div>
      </div>
    </div>
    <div class="queued-inputs" data-queued-inputs></div>
    <div data-model-feedback-host></div>
    <p class="composer-note" data-composer-note></p>`;

  const textarea = element.querySelector<HTMLTextAreaElement>("[data-composer]")!;
  const modelHost = element.querySelector<HTMLElement>("[data-model-picker]")!;
  const permissionHost = element.querySelector<HTMLElement>("[data-permission-picker]")!;
  const feedbackHost = element.querySelector<HTMLElement>("[data-model-feedback-host]")!;
  const sendButton = element.querySelector<HTMLButtonElement>("[data-send]")!;
  const modePicker = element.querySelector<HTMLSelectElement>("[data-send-mode]")!;
  const stopButton = element.querySelector<HTMLButtonElement>("[data-stop-turn]")!;
  const queueHost = element.querySelector<HTMLElement>("[data-queued-inputs]")!;
  const note = element.querySelector<HTMLElement>("[data-composer-note]")!;
  modePicker.addEventListener("change", () => store.setSendMode(modePicker.value as "wait" | "steer"));
  stopButton.addEventListener("click", () => { void store.stopActiveRequest(); });
  let composing = false;
  let sessionId = store.getState().activeSessionId;
  const layout = createTextareaLayout(textarea);
  const contextMeter = createContextMeter();
  contextMeter.element.hidden = true;
  element.querySelector("[data-context-meter-host]")!.append(contextMeter.element);

  const updateSendAvailability = (state: Readonly<ChatState>) => {
    sendButton.disabled =
      composing || chatIsBusy(state) || !state.historyLoaded || !state.activeWorkspace || !textarea.value.trim();
  };
  const refreshLayout = () => {
    if (composing || !element.isConnected || element.closest("[hidden]")) return;
    layout.refresh();
  };

  const menuHost = document.createElement("div");
  element.append(menuHost);
  const slashMenu = createSlashMenu(textarea, menuHost, (task) => {
    void onTask(task);
  }, () => { void store.retryTasks(); });
  const updateMenu = (state: Readonly<ChatState>) => slashMenu.update({
    tasks: state.tasks, status: state.taskCatalogStatus, error: state.taskCatalogError,
    state, sessionId: state.activeSessionId,
  });
  updateMenu(store.getState());

  const resetComposition = () => { composing = false; slashMenu.setComposing(false); };
  const onCompositionCancelled = () => {
    if (!composing) return;
    resetComposition(); store.setDraft(textarea.value);
    refreshLayout(); updateSendAvailability(store.getState());
  };
  const onInput = (event: Event) => {
    if (event instanceof InputEvent && event.isComposing) {
      composing = true; slashMenu.setComposing(true);
    }
    refreshLayout();
    store.setDraft(textarea.value);
    updateSendAvailability(store.getState());
  };
  const onCompositionStart = () => {
    composing = true; slashMenu.setComposing(true);
    updateSendAvailability(store.getState());
  };
  const onCompositionEnd = () => {
    resetComposition();
    store.setDraft(textarea.value);
    refreshLayout(); updateSendAvailability(store.getState());
  };
  const onKeydown = (event: KeyboardEvent) => {
    // Some IMEs omit per-key flags while a composition session is still active.
    if (composing || event.isComposing || event.keyCode === 229) return;
    if (slashMenu.handleKeydown(event)) return;
    if (event.key !== "Enter" || event.shiftKey) return;
    event.preventDefault(); element.requestSubmit();
  };
  const onFormSubmit = (event: SubmitEvent) => {
    event.preventDefault();
    const state = store.getState();
    if (composing || chatIsBusy(state) || !state.historyLoaded || !state.activeWorkspace || !textarea.value.trim()) return;
    void onSubmit();
  };
  textarea.addEventListener("input", onInput);
  textarea.addEventListener("compositionstart", onCompositionStart);
  textarea.addEventListener("compositionend", onCompositionEnd);
  textarea.addEventListener("blur", onCompositionCancelled);
  window.addEventListener("blur", onCompositionCancelled);
  textarea.addEventListener("keydown", onKeydown);
  element.addEventListener("submit", onFormSubmit);
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
      if (sessionId !== state.activeSessionId) {
        sessionId = state.activeSessionId; resetComposition();
      }
      updateMenu(state);
      modePicker.value = state.sendMode ?? "wait";
      modePicker.hidden = state.status !== "streaming";
      modePicker.disabled = !!state.stopping;
      stopButton.hidden = state.status !== "streaming" || !!state.compactionProgress;
      stopButton.disabled = !!state.stopping;
      stopButton.title = state.stopping ? "Stopping…" : "Stop response";
      stopButton.setAttribute("aria-label", state.stopping ? "Stopping response" : "Stop response");
      queueHost.replaceChildren();
      for (const input of state.queuedInputs ?? []) {
        const row = document.createElement("div"); row.className = "queued-input";
        const text = document.createElement("span"); text.textContent = `${state.queuePaused ? "Paused" : "Queued"}: ${input.content}`;
        const remove = document.createElement("button"); remove.type = "button"; remove.textContent = "×"; remove.setAttribute("aria-label", "Remove queued message");
        remove.addEventListener("click", () => store.removeQueuedInput(input.id)); row.append(text, remove); queueHost.append(row);
      }
      if (state.queuePaused && state.queuedInputs?.length) {
        const resume = document.createElement("button"); resume.type = "button"; resume.textContent = "Resume queue";
        resume.disabled = state.status === "streaming"; resume.addEventListener("click", () => { void store.resumeQueue(); }); queueHost.append(resume);
      }
      const placeholder = state.activeWorkspace ? "Message Atlas…" : "Starting folder unavailable";
      if (textarea.placeholder !== placeholder) textarea.placeholder = placeholder;
      updateSendAvailability(state);
      note.textContent = state.activeWorkspace
        ? `Commands use ${state.activeWorkspace.path} as their starting folder and can access other locations allowed by your OS account.`
        : "Atlas needs an accessible starting folder before it can run tools.";
    },
    replaceDraft(text) {
      if (textarea.value !== text) {
        textarea.value = text; resetComposition();
      }
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
      textarea.removeEventListener("input", onInput);
      textarea.removeEventListener("compositionstart", onCompositionStart);
      textarea.removeEventListener("compositionend", onCompositionEnd);
      textarea.removeEventListener("blur", onCompositionCancelled);
      window.removeEventListener("blur", onCompositionCancelled);
      textarea.removeEventListener("keydown", onKeydown);
      element.removeEventListener("submit", onFormSubmit);
      slashMenu.destroy();
      layout.destroy();
      contextMeter.destroy();
      window.removeEventListener("resize", resizeOnWindowChange);
    },
  };
}
