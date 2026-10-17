import {filterTasks, taskUnavailableReason} from "./slash-commands";
import type {ChatState, TaskDescriptor} from "./types";

interface MenuData {
  tasks: TaskDescriptor[];
  status: ChatState["taskCatalogStatus"];
  error: string | null;
  state: Pick<ChatState, "activeConversationId" | "settings">;
  sessionId: string;
}
let nextMenuId = 0;

export function createSlashMenu(
  textarea: HTMLTextAreaElement,
  host: HTMLElement,
  onChoose: (command: string) => void,
  onRetry: () => void,
) {
  const id = `slash-tasks-${++nextMenuId}`;
  const announcement = document.createElement("span");
  announcement.className = "sr-only";
  announcement.setAttribute("role", "status");
  announcement.setAttribute("aria-live", "polite");
  const popup = document.createElement("div");
  popup.className = "slash-menu";
  popup.id = id;
  popup.setAttribute("role", "listbox");
  popup.setAttribute("aria-label", "Tasks");
  let data: MenuData | null = null;
  let rows: TaskDescriptor[] = [];
  let active = 0;
  let activeId: string | null = null;
  let dismissed: string | null = null;
  let session = "";
  let query = "";
  let open = false;
  let destroyed = false;

  function clearAria() {
    for (const attribute of ["aria-controls", "aria-expanded", "aria-activedescendant", "aria-autocomplete", "aria-haspopup"]) textarea.removeAttribute(attribute);
  }
  function close() {
    open = false;
    popup.remove();
    clearAria();
  }
  function dismiss() {
    dismissed = textarea.value;
    close();
  }
  function position() {
    if (!open) return;
    const rect = textarea.getBoundingClientRect();
    const width = Math.max(0, Math.min(Math.max(rect.width, 280), window.innerWidth - 24));
    popup.style.width = `${width}px`;
    popup.style.left = `${Math.max(12, Math.min(rect.left, window.innerWidth - width - 12))}px`;
    // Anchor the bottom above the textarea, with scrolling inside the visible area.
    const bottom = Math.max(12, Math.min(window.innerHeight - rect.top + 8, window.innerHeight - 12));
    popup.style.bottom = `${bottom}px`;
    popup.style.maxHeight = `${Math.max(0, Math.min(320, window.innerHeight - bottom - 12))}px`;
  }
  function choose(task: TaskDescriptor) {
    if (!data) return;
    const reason = taskUnavailableReason(task, data.state);
    if (reason) {announcement.textContent = reason; return;}
    const match = /^(\s*)\/[^\s]*/.exec(textarea.value);
    const prefix = match?.[1] ?? "";
    const suffix = match ? textarea.value.slice(match[0].length) : "";
    const command = `${prefix}/${task.command}${suffix}`;
    textarea.value = command;
    dismissed = command;
    close();
    onChoose(command);
    textarea.focus();
    textarea.setSelectionRange(prefix.length + task.command.length + 1, prefix.length + task.command.length + 1);
  }
  function render() {
    if (destroyed || !data) return;
    if (session !== data.sessionId) {
      session = data.sessionId; dismissed = null; active = 0; activeId = null; query = ""; close();
    }
    if (dismissed !== null && textarea.value !== dismissed) dismissed = null;
    const token = /^(\s*)\/([A-Za-z][A-Za-z0-9_-]*)?(?=\s|$)/.exec(textarea.value);
    const caret = textarea.selectionStart;
    if (document.activeElement !== textarea || textarea.value === dismissed || !token ||
        caret < token[1].length + 1 || caret > token[0].length || textarea.selectionEnd !== caret) {
      close(); return;
    }
    const nextQuery = (token[2] ?? "").toLowerCase();
    if (query !== nextQuery) {active = 0; activeId = null; query = nextQuery;}
    rows = filterTasks(data.tasks, query);
    const retained = activeId ? rows.findIndex(task => task.id === activeId) : -1;
    active = retained >= 0 ? retained : Math.min(active, Math.max(0, rows.length - 1));
    activeId = rows[active]?.id ?? null;
    popup.replaceChildren();
    open = true;
    host.append(popup, announcement);
    textarea.setAttribute("aria-controls", id);
    textarea.setAttribute("aria-expanded", "true");
    textarea.setAttribute("aria-haspopup", "listbox");
    textarea.setAttribute("aria-autocomplete", "list");
    if (data.status !== "ready") {
      textarea.removeAttribute("aria-activedescendant");
      const message = document.createElement("p");
      message.className = "slash-menu-message";
      message.textContent = data.status === "error" ? `Could not load tasks: ${data.error ?? "Unknown error"}` : "Loading tasks…";
      popup.append(message);
      if (data.status === "error") {
        const retry = document.createElement("button"); retry.type = "button";
        retry.className = "slash-menu-retry"; retry.textContent = "Retry tasks (Enter)";
        retry.addEventListener("mousedown", event => event.preventDefault());
        retry.addEventListener("click", () => {textarea.focus(); onRetry();});
        popup.append(retry);
      }
      announcement.textContent = message.textContent;
    } else {
      announcement.textContent = `${rows.length} ${rows.length === 1 ? "task" : "tasks"} available`;
      if (!rows.length) {
        textarea.removeAttribute("aria-activedescendant");
        const message = document.createElement("p"); message.className = "slash-menu-message";
        message.textContent = "No matching tasks. Try another command."; popup.append(message);
      }
      rows.forEach((task, index) => {
        const option = document.createElement("div");
        option.id = `${id}-${index}`; option.className = "slash-menu-option";
        option.setAttribute("role", "option");
        option.setAttribute("aria-selected", String(index === active));
        const reason = taskUnavailableReason(task, data!.state);
        option.setAttribute("aria-disabled", String(!!reason));
        const title = document.createElement("div"); title.className = "slash-menu-title";
        const command = document.createElement("strong"); command.textContent = `/${task.command}`;
        const label = document.createElement("span"); label.textContent = task.title;
        title.append(command, label);
        const description = document.createElement("p"); description.textContent = task.description;
        option.append(title, description);
        if (reason) {const hint = document.createElement("small"); hint.textContent = reason; option.append(hint);}
        option.addEventListener("mousedown", event => event.preventDefault());
        option.addEventListener("click", () => choose(task));
        popup.append(option);
        if (index === active) textarea.setAttribute("aria-activedescendant", option.id);
      });
    }
    position();
  }
  function handleKeydown(event: KeyboardEvent): boolean {
    if (!open || event.isComposing || event.keyCode === 229 || event.shiftKey || event.ctrlKey || event.metaKey || event.altKey) return false;
    if (event.key === "Enter" && data?.status === "error") {
      event.preventDefault(); onRetry(); return true;
    }
    if (event.key === "Escape") {event.preventDefault(); dismiss(); return true;}
    if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key) && rows.length && data?.status === "ready") {
      event.preventDefault();
      if (event.key === "Home") active = 0;
      else if (event.key === "End") active = rows.length - 1;
      else active = (active + (event.key === "ArrowDown" ? 1 : -1) + rows.length) % rows.length;
      activeId = rows[active].id; render();
      popup.querySelector<HTMLElement>('[aria-selected="true"]')?.scrollIntoView?.({block: "nearest"});
      return true;
    }
    if ((event.key === "Enter" || event.key === "Tab") && data?.status === "ready" && rows.length) {
      const task = rows[active];
      if (event.key === "Tab" && taskUnavailableReason(task, data.state)) {dismiss(); return false;}
      event.preventDefault(); choose(task); return true;
    }
    return false;
  }
  function outside(event: Event) {
    if (!host.contains(event.target as Node) && event.target !== textarea) dismiss();
  }
  const events = ["input", "click", "keyup", "select", "focus"];
  events.forEach(event => textarea.addEventListener(event, render));
  function blur(event: FocusEvent) {
    if (!host.contains(event.relatedTarget as Node | null)) close();
  }
  textarea.addEventListener("blur", blur);
  document.addEventListener("pointerdown", outside);
  window.addEventListener("resize", position);
  window.addEventListener("scroll", position, true);
  return {
    update(next: MenuData) {data = next; render();},
    handleKeydown,
    destroy() {
      destroyed = true; close(); announcement.remove();
      events.forEach(event => textarea.removeEventListener(event, render));
      textarea.removeEventListener("blur", blur);
      document.removeEventListener("pointerdown", outside);
      window.removeEventListener("resize", position);
      window.removeEventListener("scroll", position, true);
    },
  };
}
