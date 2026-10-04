import type { ChatStore } from "./chat-state";
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
  article.append(label);

  const content = document.createElement("div");
  content.className = "message-content";
  content.textContent = message.content || (message.delivery === "pending" ? "Thinking…" : "");
  article.append(content);

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

  const focusComposer = () => {
    const composer = root.querySelector<HTMLTextAreaElement>("[data-composer]");
    composer?.focus();
    composer?.setSelectionRange(composer.value.length, composer.value.length);
  };

  const render = (state: Readonly<ChatState>) => {
    const composerWasFocused = document.activeElement?.hasAttribute("data-composer") ?? false;
    const metadata = activeMetadata(state);
    const isStreaming = state.status === "streaming";
    const isBusy = isStreaming || state.status === "loading";

    root.innerHTML = `
      <div class="atlas-shell">
        <aside class="sidebar${sidebarOpen ? " open" : ""}" data-sidebar>
          <div class="brand"><span class="brand-mark" aria-hidden="true">A</span><span>Atlas</span></div>
          <button type="button" class="new-chat" data-new-chat>+ New chat</button>
          <div class="section-label">Recent</div>
          <nav class="conversation-list" aria-label="Saved conversations" data-conversation-list></nav>
          <p class="local-note">Conversations are stored locally on this device.</p>
        </aside>
        <main class="chat-main">
          <header class="chat-header">
            <button type="button" class="sidebar-toggle" aria-label="Toggle conversations" aria-expanded="${sidebarOpen}" data-sidebar-toggle>☰</button>
            <h1 data-active-title></h1>
            <div class="conversation-actions">
              <button type="button" class="icon-button" aria-label="Conversation actions" data-conversation-menu>•••</button>
              <div class="action-menu" data-action-menu></div>
            </div>
          </header>
          <div class="notice-area" data-notices></div>
          <section class="messages" aria-label="Conversation messages" aria-live="polite" data-messages></section>
          <form class="composer-wrap" data-composer-form>
            <div class="composer-box">
              <textarea rows="1" aria-label="Message Atlas" placeholder="Message Atlas…" data-composer></textarea>
              <button type="submit" class="send-button" aria-label="Send message" data-send>↑</button>
            </div>
            <p class="composer-note">A new chat is saved after its first successful response.</p>
          </form>
        </main>
      </div>
      <dialog class="rename-dialog" role="dialog" aria-labelledby="rename-title">
        <form method="dialog" class="dialog-card">
          <h2 id="rename-title">Rename conversation</h2>
          <label for="rename-input">Title</label>
          <input id="rename-input" autocomplete="off" data-rename-input />
          <div class="dialog-actions">
            <button type="button" class="secondary-button" data-cancel-rename>Cancel</button>
            <button type="button" class="primary-button" data-save-rename>Save</button>
          </div>
        </form>
      </dialog>
    `;

    const sidebar = root.querySelector<HTMLElement>("[data-sidebar]")!;
    const toggle = root.querySelector<HTMLButtonElement>("[data-sidebar-toggle]")!;
    toggle.addEventListener("click", () => {
      sidebarOpen = !sidebarOpen;
      render(store.getState());
    });

    const title = root.querySelector<HTMLElement>("[data-active-title]")!;
    title.textContent = metadata?.title ?? "New chat";

    const newChat = root.querySelector<HTMLButtonElement>("[data-new-chat]")!;
    newChat.disabled = isStreaming;
    newChat.addEventListener("click", () => {
      actionsOpen = false;
      renameOpen = false;
      sidebarOpen = false;
      store.newChat();
      focusComposer();
    });

    const list = root.querySelector<HTMLElement>("[data-conversation-list]")!;
    if (state.status === "loading" && state.conversations.length === 0) {
      const loading = document.createElement("p");
      loading.className = "sidebar-status";
      loading.textContent = "Loading conversations…";
      list.append(loading);
    } else if (state.conversations.length === 0) {
      const empty = document.createElement("p");
      empty.className = "sidebar-status";
      empty.textContent = "No saved conversations yet.";
      list.append(empty);
    } else {
      for (const conversation of state.conversations) {
        const item = button(conversation.title, "conversation-item");
        item.dataset.conversationId = conversation.id;
        item.disabled = isStreaming;
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

    const menuButton = root.querySelector<HTMLButtonElement>("[data-conversation-menu]")!;
    menuButton.disabled = !metadata || isBusy;
    menuButton.addEventListener("click", () => {
      actionsOpen = !actionsOpen;
      render(store.getState());
    });
    const menu = root.querySelector<HTMLElement>("[data-action-menu]")!;
    if (actionsOpen && metadata) {
      menu.classList.add("open");
      const rename = button("Rename", "menu-item");
      rename.dataset.rename = "";
      rename.addEventListener("click", () => {
        renameValue = metadata.title;
        renameOpen = true;
        actionsOpen = false;
        render(store.getState());
        root.querySelector<HTMLInputElement>("[data-rename-input]")?.focus();
      });
      const remove = button("Delete", "menu-item danger");
      remove.dataset.delete = "";
      remove.addEventListener("click", async () => {
        if (!window.confirm("Delete this conversation? Its history will be moved to Atlas trash.")) {
          return;
        }
        actionsOpen = false;
        await store.deleteActive();
        focusComposer();
      });
      menu.append(rename, remove);
    }

    const notices = root.querySelector<HTMLElement>("[data-notices]")!;
    if (state.errorMessage) {
      const error = document.createElement("div");
      error.className = "error-notice";
      error.setAttribute("role", "alert");
      error.textContent = state.errorMessage;
      notices.append(error);
    } else if (state.status === "loading") {
      const loading = document.createElement("div");
      loading.className = "status-notice";
      loading.setAttribute("role", "status");
      loading.textContent = "Loading conversation…";
      notices.append(loading);
    } else if (isStreaming) {
      const streaming = document.createElement("div");
      streaming.className = "status-notice";
      streaming.setAttribute("role", "status");
      streaming.textContent = "Atlas is responding…";
      notices.append(streaming);
    }

    const messages = root.querySelector<HTMLElement>("[data-messages]")!;
    if (state.messages.length === 0) {
      const welcome = document.createElement("div");
      welcome.className = "welcome";
      const heading = document.createElement("h2");
      heading.textContent = "What are we building?";
      const copy = document.createElement("p");
      copy.textContent = "Ask Atlas to inspect, explain, debug, or change your code.";
      welcome.append(heading, copy);
      messages.append(welcome);
    } else {
      for (const message of state.messages) messages.append(renderMessage(message));
      messages.scrollTop = messages.scrollHeight;
    }

    const textarea = root.querySelector<HTMLTextAreaElement>("[data-composer]")!;
    textarea.value = state.draftText;
    textarea.disabled = isBusy;
    textarea.addEventListener("input", () => {
      const value = textarea.value;
      store.setDraft(value);
      focusComposer();
    });
    textarea.addEventListener("keydown", (event) => {
      if (event.key === "Enter" && !event.shiftKey) {
        event.preventDefault();
        root.querySelector<HTMLFormElement>("[data-composer-form]")?.requestSubmit();
      }
    });

    const send = root.querySelector<HTMLButtonElement>("[data-send]")!;
    send.disabled = isBusy || !state.draftText.trim();
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
      dialog.setAttribute("open", "");
      const renameInput = root.querySelector<HTMLInputElement>("[data-rename-input]")!;
      renameInput.value = renameValue;
      renameInput.addEventListener("input", () => {
        renameValue = renameInput.value;
      });
      root.querySelector<HTMLButtonElement>("[data-cancel-rename]")!.addEventListener(
        "click",
        () => {
          renameOpen = false;
          render(store.getState());
        },
      );
      root.querySelector<HTMLButtonElement>("[data-save-rename]")!.addEventListener(
        "click",
        async () => {
          await store.renameActive(renameValue);
          if (store.getState().status !== "error") renameOpen = false;
          render(store.getState());
        },
      );
    }

    if (composerWasFocused && !textarea.disabled) focusComposer();
  };

  const unsubscribe = store.subscribe(render);
  return () => {
    unsubscribe();
    root.replaceChildren();
  };
}
