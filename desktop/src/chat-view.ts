import type { ChatStore } from "./chat-state";
import { renderMarkdown } from "./markdown";
import { renderSettings } from "./settings-view";
import type { ChatState, ConversationMetadata, DisplayMessage } from "./types";

function activeMetadata(state: Readonly<ChatState>): ConversationMetadata | undefined {
  return state.conversations.find(
    (conversation) => conversation.id === state.activeConversationId,
  );
}

function button(label: string, className?: string): HTMLButtonElement {
  const element = document.createElement("button");
  element.type = "button";
  element.textContent = label;
  if (className) element.className = className;
  return element;
}

function renderMessage(message: DisplayMessage): HTMLElement {
  const article = document.createElement("article");
  article.className = `message message-${message.role} delivery-${message.delivery}`;
  article.dataset.messageRole = message.role;
  const label = document.createElement("div");
  label.className = "message-label";
  label.textContent = message.role === "assistant" ? "Atlas" : "You";
  const content = document.createElement("div");
  content.className = "message-content";
  const renderedContent =
    message.content || (message.delivery === "pending" ? "Thinking…" : "");
  if (message.role === "assistant" && message.content) {
    renderMarkdown(content, renderedContent);
  } else {
    content.textContent = renderedContent;
  }
  article.append(label, content);
  if (message.delivery === "failed") {
    const failed = document.createElement("div");
    failed.className = "message-failed";
    failed.textContent = "Not saved — retry from the composer.";
    article.append(failed);
  }
  return article;
}

export function mountChat(root: HTMLElement, store: ChatStore): () => void {
  let sidebarOpen = false;
  let actionsOpen = false;
  let renameOpen = false;
  let renameValue = "";
  let composing = false;
  let compositionSettling = false;
  let compositionSettleTimer: ReturnType<typeof setTimeout> | null = null;
  let pendingState: Readonly<ChatState> | null = null;
  let renderedState: Readonly<ChatState> | null = null;

  const focusComposer = (selection?: [number, number]) => {
    const composer = root.querySelector<HTMLTextAreaElement>("[data-composer]");
    if (!composer || composer.disabled) return;
    composer.focus();
    const start = Math.min(selection?.[0] ?? composer.value.length, composer.value.length);
    const end = Math.min(selection?.[1] ?? start, composer.value.length);
    composer.setSelectionRange(start, end);
  };

  const renderNow = (state: Readonly<ChatState>) => {
    const focusedComposer =
      document.activeElement instanceof HTMLTextAreaElement &&
      document.activeElement.hasAttribute("data-composer")
        ? document.activeElement
        : null;
    const composerSelection: [number, number] | undefined = focusedComposer
      ? [focusedComposer.selectionStart, focusedComposer.selectionEnd]
      : undefined;
    const metadata = activeMetadata(state);
    const isStreaming = state.status === "streaming";
    const isBusy =
      isStreaming ||
      state.status === "loading" ||
      state.workspaceBusy ||
      state.modelListStatus === "saving";

    root.innerHTML = `
      <div class="atlas-shell">
        <aside class="sidebar${sidebarOpen ? " open" : ""}" data-sidebar>
          <div class="brand"><img class="brand-mark" src="/atlas-compass.svg" alt="" /><span><strong>Atlas</strong><small>Computer assistant</small></span></div>
          <button type="button" class="new-chat" data-new-chat><span>＋</span> New chat</button>
          <section class="workspace-card" aria-label="Active workspace" data-workspace-card>
            <div class="workspace-card-icon" aria-hidden="true">⌘</div>
            <div class="workspace-card-copy"><span>Workspace</span><strong data-workspace-name></strong><small data-workspace-path></small></div>
            <button type="button" class="workspace-card-action" data-workspace-action aria-label="Choose workspace folder">⌄</button>
          </section>
          <div class="section-label">Conversations</div>
          <nav class="conversation-list" aria-label="Saved conversations" data-conversation-list></nav>
          <div class="sidebar-footer">
            <button type="button" class="settings-nav" data-open-settings><span aria-hidden="true">⚙</span> Settings</button>
            <p class="local-note">Private and stored locally on this device.</p>
          </div>
        </aside>
        <main class="chat-main" data-main></main>
      </div>`;

    const sidebar = root.querySelector<HTMLElement>("[data-sidebar]")!;
    const main = root.querySelector<HTMLElement>("[data-main]")!;
    const workspaceName = root.querySelector<HTMLElement>("[data-workspace-name]")!;
    const workspacePath = root.querySelector<HTMLElement>("[data-workspace-path]")!;
    workspaceName.textContent = state.activeWorkspace?.name ?? "No folder selected";
    workspacePath.textContent = state.activeWorkspace?.path ?? "Choose a folder to begin";

    const workspaceAction = root.querySelector<HTMLButtonElement>("[data-workspace-action]")!;
    workspaceAction.disabled = isBusy;
    workspaceAction.addEventListener("click", async () => {
      if (state.activeConversationId) await store.changeActiveWorkspace();
      else await store.chooseWorkspaceForDraft();
      focusComposer();
    });

    const newChat = root.querySelector<HTMLButtonElement>("[data-new-chat]")!;
    newChat.disabled = isBusy;
    newChat.addEventListener("click", async () => {
      actionsOpen = false;
      renameOpen = false;
      sidebarOpen = false;
      await store.newChat();
      focusComposer();
    });

    const openSettings = root.querySelector<HTMLButtonElement>("[data-open-settings]")!;
    openSettings.disabled = isBusy || !state.settings;
    openSettings.addEventListener("click", () => {
      sidebarOpen = false;
      store.openSettings();
    });
    if (state.settingsErrorMessage && !state.settings) {
      const settingsError = document.createElement("p");
      settingsError.className = "sidebar-status sidebar-error";
      settingsError.setAttribute("role", "alert");
      settingsError.textContent = state.settingsErrorMessage;
      const retry = button("Retry settings", "sidebar-retry");
      retry.dataset.retrySettings = "";
      retry.disabled = isBusy || state.settingsStatus === "loading";
      retry.addEventListener("click", () => void store.retrySettings());
      openSettings.insertAdjacentElement("afterend", settingsError);
      settingsError.insertAdjacentElement("afterend", retry);
    }

    const list = root.querySelector<HTMLElement>("[data-conversation-list]")!;
    if (state.listStatus === "loading" && state.conversations.length === 0) {
      const loading = document.createElement("p");
      loading.className = "sidebar-status";
      loading.textContent = "Loading conversations…";
      list.append(loading);
    } else if (state.listStatus === "error" && state.conversations.length === 0) {
      const error = document.createElement("p");
      error.className = "sidebar-status sidebar-error";
      error.textContent = state.listErrorMessage ?? "Could not load conversations.";
      const retry = button("Retry", "sidebar-retry");
      retry.dataset.retryList = "";
      retry.addEventListener("click", () => void store.initialize());
      list.append(error, retry);
    } else if (state.conversationGroups.length === 0) {
      const empty = document.createElement("p");
      empty.className = "sidebar-status";
      empty.textContent = "No saved conversations yet.";
      list.append(empty);
    } else {
      for (const group of state.conversationGroups) {
        const heading = document.createElement("h2");
        heading.className = "workspace-group-title";
        heading.textContent = group.workspace?.name ?? "Unassigned";
        list.append(heading);
        for (const conversation of group.conversations) {
          const item = button(conversation.title, "conversation-item");
          item.dataset.conversationId = conversation.id;
          item.disabled = isBusy;
          item.classList.toggle("active", conversation.id === state.activeConversationId);
          item.addEventListener("click", async () => {
            actionsOpen = false;
            renameOpen = false;
            sidebarOpen = false;
            await store.selectConversation(conversation.id);
            focusComposer();
          });
          list.append(item);
        }
      }
    }

    if (state.screen === "settings") {
      main.classList.add("settings-main");
      const mobileHeader = document.createElement("header");
      mobileHeader.className = "settings-mobile-header";
      const toggle = button("☰", "sidebar-toggle");
      toggle.dataset.sidebarToggle = "";
      toggle.setAttribute("aria-label", "Toggle conversations");
      toggle.setAttribute("aria-expanded", String(sidebarOpen));
      toggle.addEventListener("click", () => {
        sidebarOpen = !sidebarOpen;
        renderNow(store.getState());
      });
      mobileHeader.append(toggle);
      const settingsHost = document.createElement("div");
      settingsHost.className = "settings-scroll";
      main.append(mobileHeader, settingsHost);
      renderSettings(settingsHost, state, store);
      return;
    }

    main.innerHTML = `
      <header class="chat-header">
        <button type="button" class="sidebar-toggle" aria-label="Toggle conversations" aria-expanded="${sidebarOpen}" data-sidebar-toggle>☰</button>
        <div class="header-title"><h1 data-active-title></h1><button type="button" data-header-workspace></button></div>
        <div class="conversation-actions"><button type="button" class="icon-button" aria-label="Toggle fullscreen" title="Toggle fullscreen (F11)" data-toggle-fullscreen>⛶</button><button type="button" class="icon-button" aria-label="Conversation actions" data-conversation-menu>•••</button><div class="action-menu" data-action-menu></div></div>
      </header>
      <div class="notice-area" data-notices></div>
      <section class="messages" aria-label="Conversation messages" aria-live="polite" data-messages></section>
      <form class="composer-wrap" data-composer-form>
        <div class="model-picker" data-model-picker></div>
        <div class="composer-box"><textarea rows="1" aria-label="Message Atlas" data-composer></textarea><button type="submit" class="send-button" aria-label="Send message" data-send>↑</button></div>
        <p class="composer-note" data-composer-note></p>
      </form>
      <dialog class="rename-dialog" role="dialog" aria-labelledby="rename-title"><form method="dialog" class="dialog-card"><h2 id="rename-title">Rename conversation</h2><label for="rename-input">Title</label><input id="rename-input" autocomplete="off" data-rename-input /><div class="dialog-actions"><button type="button" class="secondary-button" data-cancel-rename>Cancel</button><button type="submit" class="primary-button" data-save-rename>Save</button></div></form></dialog>`;

    const toggle = root.querySelector<HTMLButtonElement>("[data-sidebar-toggle]")!;
    toggle.addEventListener("click", () => {
      sidebarOpen = !sidebarOpen;
      renderNow(store.getState());
    });
    const title = root.querySelector<HTMLElement>("[data-active-title]")!;
    title.textContent = metadata?.title ?? "New chat";
    const headerWorkspace = root.querySelector<HTMLButtonElement>("[data-header-workspace]")!;
    headerWorkspace.textContent = state.activeWorkspace
      ? `${state.activeWorkspace.name} · Change folder`
      : "Choose folder";
    headerWorkspace.disabled = isBusy;
    headerWorkspace.addEventListener("click", async () => {
      if (state.activeConversationId) await store.changeActiveWorkspace();
      else await store.chooseWorkspaceForDraft();
      focusComposer();
    });

    const menuButton = root.querySelector<HTMLButtonElement>("[data-conversation-menu]")!;
    root
      .querySelector<HTMLButtonElement>("[data-toggle-fullscreen]")!
      .addEventListener("click", () => void store.toggleFullscreen());
    menuButton.disabled = !metadata || isBusy;
    menuButton.addEventListener("click", () => {
      actionsOpen = !actionsOpen;
      renderNow(store.getState());
    });
    const menu = root.querySelector<HTMLElement>("[data-action-menu]")!;
    if (actionsOpen && metadata) {
      menu.classList.add("open");
      const change = button("Change folder", "menu-item");
      change.addEventListener("click", () => void store.changeActiveWorkspace());
      const rename = button("Rename", "menu-item");
      rename.dataset.rename = "";
      rename.addEventListener("click", () => {
        renameValue = metadata.title;
        renameOpen = true;
        actionsOpen = false;
        renderNow(store.getState());
        root.querySelector<HTMLInputElement>("[data-rename-input]")?.focus();
      });
      const remove = button("Delete", "menu-item danger");
      remove.dataset.delete = "";
      remove.addEventListener("click", async () => {
        if (!window.confirm("Delete this conversation? Its history will be moved to Atlas trash.")) return;
        actionsOpen = false;
        await store.deleteActive();
        focusComposer();
      });
      menu.append(change, rename, remove);
    }

    const notices = root.querySelector<HTMLElement>("[data-notices]")!;
    if (state.errorMessage) {
      const error = document.createElement("div");
      error.className = "error-notice";
      error.setAttribute("role", "alert");
      error.textContent = state.errorMessage;
      notices.append(error);
    } else if (state.status === "loading" || isStreaming) {
      const status = document.createElement("div");
      status.className = "status-notice";
      status.setAttribute("role", "status");
      status.textContent = isStreaming ? "Atlas is responding…" : "Loading conversation…";
      notices.append(status);
    }

    const messages = root.querySelector<HTMLElement>("[data-messages]")!;
    if (state.messages.length === 0) {
      const welcome = document.createElement("div");
      welcome.className = "welcome";
      const eyebrow = document.createElement("span");
      eyebrow.textContent = state.activeWorkspace ? "Ready to help" : "Starting folder unavailable";
      const heading = document.createElement("h2");
      heading.textContent = state.activeWorkspace
        ? "How can I help?"
        : "Atlas needs a starting folder";
      const copy = document.createElement("p");
      copy.textContent = state.activeWorkspace
        ? "Atlas can chat, organize files, run commands, help with applications, and work on code."
        : "Check Settings or choose a folder before sending a message.";
      welcome.append(eyebrow, heading, copy);
      if (!state.activeWorkspace) {
        const choose = button("Choose folder", "primary-button folder-first-action");
        choose.dataset.chooseWorkspace = "";
        choose.addEventListener("click", async () => {
          await store.chooseWorkspaceForDraft();
          focusComposer();
        });
        welcome.append(choose);
      }
      messages.append(welcome);
    } else {
      for (const message of state.messages) messages.append(renderMessage(message));
      messages.scrollTop = messages.scrollHeight;
    }

    const modelPicker = root.querySelector<HTMLElement>("[data-model-picker]")!;
    if (state.settings && state.settings.apiKeySource !== "missing") {
      const label = document.createElement("label");
      label.textContent = "Model";
      const select = document.createElement("select");
      select.dataset.modelSelect = "";
      select.setAttribute("aria-label", "Model");
      const models = state.availableModels.length
        ? state.availableModels
        : [{ id: state.settings.activeModel, name: state.settings.activeModel }];
      for (const model of models) {
        const option = document.createElement("option");
        option.value = model.id;
        option.textContent = model.name;
        select.append(option);
      }
      select.value = state.settings.activeModel;
      select.disabled =
        isBusy ||
        state.modelListStatus === "loading" ||
        state.settings.environmentOverrides.includes("model");
      select.addEventListener("change", () => void store.selectModel(select.value));
      label.append(select);
      modelPicker.append(label);
      if (state.modelListErrorMessage) {
        const error = document.createElement("span");
        error.className = "model-picker-error";
        error.setAttribute("role", "alert");
        error.textContent = state.modelListErrorMessage;
        modelPicker.append(error);
        if (state.modelListRetryAvailable) {
          const retry = button("Retry", "model-picker-retry");
          retry.type = "button";
          retry.disabled = isBusy || state.modelListStatus === "loading";
          retry.addEventListener("click", () => void store.retryModels());
          modelPicker.append(retry);
        }
      }
    } else {
      modelPicker.remove();
    }

    const textarea = root.querySelector<HTMLTextAreaElement>("[data-composer]")!;
    const send = root.querySelector<HTMLButtonElement>("[data-send]")!;
    textarea.value = state.draftText;
    textarea.placeholder = state.activeWorkspace
      ? "Message Atlas…"
      : "Starting folder unavailable";
    textarea.disabled = isBusy;
    send.disabled = isBusy || !state.activeWorkspace || !state.draftText.trim();
    root.querySelector<HTMLElement>("[data-composer-note]")!.textContent = state.activeWorkspace
      ? `Commands use ${state.activeWorkspace.path} as their starting folder and can access other locations allowed by your OS account.`
      : "Atlas needs an accessible starting folder before it can run tools.";
    textarea.addEventListener("compositionstart", () => {
      if (compositionSettleTimer !== null) clearTimeout(compositionSettleTimer);
      compositionSettleTimer = null;
      composing = true;
      compositionSettling = false;
      pendingState = null;
    });
    textarea.addEventListener("compositionend", () => {
      const value = textarea.value;
      store.setDraft(value);
      composing = false;
      compositionSettling = true;
      compositionSettleTimer = setTimeout(() => {
        compositionSettleTimer = null;
        compositionSettling = false;
        const selection: [number, number] = [textarea.selectionStart, textarea.selectionEnd];
        pendingState = null;
        renderNow(store.getState());
        focusComposer(selection);
      }, 0);
    });
    textarea.addEventListener("input", () => {
      if (composing) return;
      store.setDraft(textarea.value);
      if (compositionSettling) {
        if (compositionSettleTimer !== null) clearTimeout(compositionSettleTimer);
        compositionSettleTimer = null;
        compositionSettling = false;
        if (pendingState) {
          const selection: [number, number] = [textarea.selectionStart, textarea.selectionEnd];
          pendingState = null;
          renderNow(store.getState());
          focusComposer(selection);
          return;
        }
      }
      send.disabled = isBusy || !state.activeWorkspace || !textarea.value.trim();
    });
    textarea.addEventListener("keydown", (event) => {
      if (
        event.key === "Enter" &&
        !event.shiftKey &&
        !event.isComposing &&
        !composing &&
        event.keyCode !== 229
      ) {
        event.preventDefault();
        root.querySelector<HTMLFormElement>("[data-composer-form]")?.requestSubmit();
      }
    });
    root.querySelector<HTMLFormElement>("[data-composer-form]")!.addEventListener(
      "submit",
      async (event) => {
        event.preventDefault();
        await store.send();
        focusComposer();
      },
    );

    const dialog = root.querySelector<HTMLDialogElement>("dialog")!;
    if (renameOpen) {
      dialog.addEventListener("close", () => {
        renameOpen = false;
      });
      dialog.showModal();
      const renameInput = root.querySelector<HTMLInputElement>("[data-rename-input]")!;
      renameInput.value = renameValue;
      renameInput.addEventListener("input", () => {
        renameValue = renameInput.value;
      });
      root.querySelector<HTMLButtonElement>("[data-cancel-rename]")!.addEventListener(
        "click",
        () => {
          renameOpen = false;
          renderNow(store.getState());
        },
      );
      root.querySelector<HTMLFormElement>(".dialog-card")!.addEventListener(
        "submit",
        async (event) => {
          event.preventDefault();
          if (!renameValue.trim()) return renameInput.focus();
          await store.renameActive(renameValue);
          if (store.getState().status !== "error") renameOpen = false;
          renderNow(store.getState());
        },
      );
    }
    if (composerSelection && !textarea.disabled) focusComposer(composerSelection);
  };

  const patchStreamingDelta = (
    previous: Readonly<ChatState>,
    state: Readonly<ChatState>,
  ): boolean => {
    if (previous.status !== "streaming" || state.status !== "streaming") return false;
    for (const key of Object.keys(previous) as Array<keyof ChatState>) {
      if (key !== "messages" && previous[key] !== state[key]) return false;
    }
    if (previous.messages.length !== state.messages.length || state.messages.length === 0) {
      return false;
    }
    const lastIndex = state.messages.length - 1;
    if (previous.messages.slice(0, lastIndex).some((message, index) => message !== state.messages[index])) {
      return false;
    }
    const previousMessage = previous.messages[lastIndex];
    const message = state.messages[lastIndex];
    if (
      previousMessage.role !== "assistant" ||
      previousMessage.delivery !== "pending" ||
      message.role !== "assistant" ||
      message.delivery !== "pending" ||
      !message.content.startsWith(previousMessage.content)
    ) {
      return false;
    }
    const messages = root.querySelector<HTMLElement>("[data-messages]");
    const articles = messages?.querySelectorAll<HTMLElement>("[data-message-role]");
    const content = articles?.item(lastIndex).querySelector<HTMLElement>(".message-content");
    if (!messages || !content) return false;
    if (message.content) renderMarkdown(content, message.content);
    else content.textContent = "Thinking…";
    messages.scrollTop = messages.scrollHeight;
    return true;
  };

  const render = (state: Readonly<ChatState>) => {
    if (composing || compositionSettling) {
      pendingState = state;
      return;
    }
    pendingState = null;
    if (renderedState && patchStreamingDelta(renderedState, state)) {
      renderedState = state;
      return;
    }
    renderNow(state);
    renderedState = state;
  };
  const onWindowKeydown = (event: KeyboardEvent) => {
    if (event.key !== "F11" || event.repeat) return;
    event.preventDefault();
    void store.toggleFullscreen();
  };
  window.addEventListener("keydown", onWindowKeydown);
  const unsubscribe = store.subscribe(render);
  return () => {
    if (compositionSettleTimer !== null) clearTimeout(compositionSettleTimer);
    window.removeEventListener("keydown", onWindowKeydown);
    unsubscribe();
    root.replaceChildren();
  };
}
