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
    <div class="queued-inputs" data-queued-inputs hidden>
      <div class="queue-header"><span data-queue-label></span><button type="button" class="queue-resume" data-resume-queue hidden>Resume queue</button></div>
      <div class="queue-list" data-queue-list></div>
    </div>
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
  const queueHost = element.querySelector<HTMLElement>("[data-queued-inputs]")!;
  const queueList = element.querySelector<HTMLElement>("[data-queue-list]")!;
  const queueLabel = element.querySelector<HTMLElement>("[data-queue-label]")!;
  const resumeButton = element.querySelector<HTMLButtonElement>("[data-resume-queue]")!;
  const note = element.querySelector<HTMLElement>("[data-composer-note]")!;
  const queueRows = new Map<string, {row: HTMLElement; text: HTMLElement; steer: HTMLButtonElement}>();
  resumeButton.addEventListener("click", () => { void store.resumeQueue(); });
  const renderQueue = (state: Readonly<ChatState>) => {
    const inputs = state.queuedInputs ?? [];
    queueHost.hidden = !inputs.length;
    queueLabel.textContent = `${state.queuePaused ? "Paused" : "Queued"} · ${inputs.length}`;
    resumeButton.hidden = !state.queuePaused;
    resumeButton.disabled = state.status === "streaming";
    const ids = new Set(inputs.map(input => input.id));
    for (const [id, entry] of queueRows) {
      if (!ids.has(id)) { entry.row.remove(); queueRows.delete(id); }
    }
    inputs.forEach((input, index) => {
      let entry = queueRows.get(input.id);
      if (!entry) {
        const row = document.createElement("div"); row.className = "queued-input";
        const text = document.createElement("span"); text.className = "queued-input-text";
        const steer = document.createElement("button"); steer.type = "button"; steer.className = "queue-steer"; steer.textContent = "Steer";
        steer.title = "Send this message to the active turn";
        steer.addEventListener("click", () => { void store.steerQueuedInput(input.id); });
        const remove = document.createElement("button"); remove.type = "button"; remove.className = "queue-remove"; remove.textContent = "×";
        remove.title = "Remove queued message"; remove.setAttribute("aria-label", "Remove queued message");
        remove.addEventListener("click", () => store.removeQueuedInput(input.id));
        row.append(text, steer, remove); entry = {row, text, steer}; queueRows.set(input.id, entry);
      }
      entry.text.textContent = input.content; entry.text.title = input.content;
      entry.steer.hidden = state.status !== "streaming" || !!state.compactionProgress;
      entry.steer.disabled = !!state.stopping;
      if (queueList.children[index] !== entry.row) queueList.insertBefore(entry.row, queueList.children[index] ?? null);
    });
  };
  let composing = false;
  let sessionId = store.getState().activeSessionId;
  const layout = createTextareaLayout(textarea);
  const contextMeter = createContextMeter();
  contextMeter.element.hidden = true;
  element.querySelector("[data-context-meter-host]")!.append(contextMeter.element);

  const canStop = (state: Readonly<ChatState>) => state.status === "streaming" && !state.compactionProgress && !textarea.value.trim();
  const updateSendAvailability = (state: Readonly<ChatState>) => {
    const stop = canStop(state);
    const label = stop ? (state.stopping ? "Stopping response" : "Stop response") : state.status === "streaming" ? "Queue message" : "Send message";
    sendButton.classList.toggle("is-stop", stop);
    sendButton.classList.toggle("is-stopping", stop && !!state.stopping);
    sendButton.textContent = stop ? "" : "↑";
    sendButton.setAttribute("aria-label", label); sendButton.title = label;
    sendButton.disabled = composing || chatIsBusy(state) || !state.historyLoaded || !state.activeWorkspace ||
      (stop ? !!state.stopping : !textarea.value.trim());
  };
  const onPrimaryClick = (event: MouseEvent) => {
    if (!canStop(store.getState())) return;
    event.preventDefault();
    if (!composing && !sendButton.disabled) void store.stopActiveRequest();
  };
  sendButton.addEventListener("click", onPrimaryClick);
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
      renderQueue(state);
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
      sendButton.removeEventListener("click", onPrimaryClick);
      slashMenu.destroy();
      layout.destroy();
      contextMeter.destroy();
      window.removeEventListener("resize", resizeOnWindowChange);
    },
  };
}
